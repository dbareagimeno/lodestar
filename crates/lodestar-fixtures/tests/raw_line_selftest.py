#!/usr/bin/env python3
"""Regresiones L12 del contrato de ``LodestarSession.raw_line``.

Cada selector es invocado por un test Rust con nombre propio. Los doubles sólo cubren el
contrato del lector; el caso ``--prefetched`` usa un ``Popen`` real para que TextIOWrapper
pueda demostrar el buffer anticipado que pierde el lector basado únicamente en selector.
"""

from __future__ import annotations

import importlib.util
import json
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path


TIMEOUT = 0.05
# Margen de pared portable para runners CI lentos. Sigue siendo muy inferior a la
# lectura bloqueada, por lo que una regresión que espere al reader no puede pasar.
STRICT_LIMIT = 0.30
BLOCKED_READ = 0.75


def load_harness():
    repo = Path(__file__).resolve().parents[3]
    path = repo / "docs/qa/testbench/lodestar_harness.py"
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("lodestar_testbench_harness", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"no se pudo cargar el arnés: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FakeStdin:
    def __init__(self):
        self.lines: list[str] = []

    def write(self, line: str) -> None:
        self.lines.append(line)

    def flush(self) -> None:
        return None


class FailingBarrierStdin:
    """Registra el write de barrera y falla antes de que pueda enviarse la operación pública."""

    def __init__(self, error_type, marker: str):
        self.error_type = error_type
        self.marker = marker
        self.lines: list[str] = []
        self.flush_calls = 0

    def write(self, line: str) -> None:
        self.lines.append(line)
        raise self.error_type(self.marker)

    def flush(self) -> None:
        self.flush_calls += 1


class ObservingQueue(queue.Queue):
    """Acredita que el lector insertó un frame real, no sólo el sentinel EOF."""

    def __init__(self):
        super().__init__()
        self.frame_queued = threading.Event()
        self.frame_count = 0
        self.frame_count_changed = threading.Condition()

    def put(self, item, block=True, timeout=None):
        super().put(item, block=block, timeout=timeout)
        if isinstance(item, str):
            self.frame_queued.set()
            with self.frame_count_changed:
                self.frame_count += 1
                self.frame_count_changed.notify_all()

    def wait_for_frames(self, expected: int, timeout: float) -> bool:
        with self.frame_count_changed:
            return self.frame_count_changed.wait_for(
                lambda: self.frame_count >= expected,
                timeout=timeout,
            )


class ControlledMonotonic:
    """Reloj determinista para cruzar deadlines sin bloquear tiempo de pared."""

    def __init__(self):
        self.value = 100.0
        self.crossed = False

    def monotonic(self) -> float:
        return self.value

    def cross_deadline(self) -> None:
        self.advance(1.0)

    def advance(self, seconds: float) -> None:
        self.value += seconds
        self.crossed = True


class DeadlineCrossingQueue:
    """El primer get bloqueante entrega un frame sólo después de vencer su timeout."""

    def __init__(self, clock, late_frame: str, queued_frame: str):
        self.clock = clock
        self.late_frame = late_frame
        self.queued_frames = [queued_frame]
        self.blocking_timeouts: list[float] = []
        self.blocking_returns = 0
        self.late_returned_at: float | None = None
        self.nonblocking_calls = 0

    def get_nowait(self):
        self.nonblocking_calls += 1
        # La primera operación debe entrar realmente en el camino bloqueante.
        if self.nonblocking_calls == 1:
            raise queue.Empty
        if self.queued_frames:
            return self.queued_frames.pop(0)
        raise queue.Empty

    def get(self, block=True, timeout=None):
        if timeout is None or timeout <= 0:
            raise AssertionError(f"get bloqueante sin plazo positivo: {timeout!r}")
        self.blocking_timeouts.append(timeout)
        if self.blocking_returns == 0:
            self.clock.advance(timeout + 1.0)
            self.blocking_returns += 1
            self.late_returned_at = self.clock.monotonic()
            return self.late_frame
        raise queue.Empty


class CountingPopenStdin:
    """Proxy que acredita intentos de write y puede coordinar el primer EOF terminal."""

    def __init__(
        self,
        stream,
        process,
        wait_after_first_flush: bool,
        queued_frame_event=None,
        flush_error_type=None,
        flush_error_marker: str | None = None,
        before_flush_error=None,
        queued_frames_waiter=None,
        wait_for_process_exit: bool | None = None,
    ):
        self.stream = stream
        self.process = process
        self.wait_after_first_flush = wait_after_first_flush
        self.queued_frame_event = queued_frame_event
        self.flush_error_type = flush_error_type
        self.flush_error_marker = flush_error_marker
        self.before_flush_error = before_flush_error
        self.queued_frames_waiter = queued_frames_waiter
        self.wait_for_process_exit = (
            wait_after_first_flush
            if wait_for_process_exit is None
            else wait_for_process_exit
        )
        self.write_attempts: list[str] = []
        self.flush_calls = 0

    def write(self, line: str):
        self.write_attempts.append(line)
        return self.stream.write(line)

    def flush(self) -> None:
        self.flush_calls += 1
        self.stream.flush()
        if self.wait_after_first_flush and self.flush_calls == 1:
            if self.queued_frames_waiter is not None:
                self.queued_frames_waiter()
            elif self.queued_frame_event is not None and not self.queued_frame_event.wait(1):
                raise AssertionError("el lector no encoló la respuesta antes del exit coordinado")
        if self.wait_for_process_exit and self.flush_calls == 1:
            self.process.wait(timeout=1)
        if self.flush_error_type is not None and self.flush_calls == 1:
            if self.before_flush_error is not None:
                self.before_flush_error()
            raise self.flush_error_type(self.flush_error_marker)

    def close(self) -> None:
        self.stream.close()


class OneFrameStdout:
    def __init__(self, frame: str):
        self.frame = frame
        self.reads = 0

    def readline(self):
        self.reads += 1
        if self.reads == 1:
            return self.frame
        return ""


class BlockingStdout:
    def readline(self):
        time.sleep(BLOCKED_READ)
        return ""


class InjectedIdDomainStdout:
    """Double coordinado que inyecta frames para ejercitar la clasificación del arnés.

    No pretende demostrar qué emite el binario rmcp real. Publica, después de observar
    la petición escrita, un ``-32600`` sin ``id`` para ids fuera de string/i64 y una
    respuesta correlacionable para los ids admitidos. Así aísla únicamente la decisión
    de ``raw_line`` ante esos frames ya observados.
    """

    def __init__(self, stdin):
        self.stdin = stdin
        self.reads = 0
        self.observed_line: str | None = None

    def readline(self):
        self.reads += 1
        if self.reads != 1 or not self.stdin.lines:
            return ""
        self.observed_line = self.stdin.lines[-1].rstrip("\n")
        request = json.loads(self.observed_line)
        request_id = request.get("id")
        is_i64 = type(request_id) is int and -(2**63) <= request_id <= 2**63 - 1
        if isinstance(request_id, str) or is_i64:
            return json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {"marker": "accepted-number-or-string"},
                }
            ) + "\n"
        return (
            '{"jsonrpc":"2.0","error":'
            '{"code":-32600,"message":"Invalid request"}}\n'
        )


class SimulatedProcess:
    def __init__(self, stdout):
        self.stdin = FakeStdin()
        self.stdout = stdout
        self.returncode = None

    def poll(self):
        return self.returncode


class WriteFailureProcess:
    def __init__(self, stdin):
        self.stdin = stdin
        self.stdout = BlockingStdout()
        self.stderr = None
        self.returncode = None

    def poll(self):
        return self.returncode


class CoordinatedStdout:
    """Cola bloqueante: cada frame sólo existe después del ``write`` acreditado."""

    def __init__(self):
        self.frames = queue.Queue()

    def push(self, response) -> None:
        self.frames.put(json.dumps(response) + "\n")

    def readline(self):
        frame = self.frames.get()
        return "" if frame is None else frame

    def close(self) -> None:
        self.frames.put(None)


class FirstBarrierFlushFailureStdin:
    """El primer write sale y publica ACK, pero su flush falla una sola vez."""

    def __init__(self, stdout, error_type, marker: str):
        self.stdout = stdout
        self.error_type = error_type
        self.marker = marker
        self.lines: list[str] = []
        self.flush_calls = 0
        self.barrier_ids: list[str] = []

    def write(self, line: str) -> None:
        self.lines.append(line)
        request = json.loads(line)
        request_id = request.get("id")
        if request.get("method") == "ping" and isinstance(request_id, str):
            self.barrier_ids.append(request_id)
            self.stdout.push(
                {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "marker": f"BARRIER_ACK_{len(self.barrier_ids)}",
                    },
                }
            )
        elif type(request_id) is int:
            self.stdout.push(
                {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {"marker": "PUBLIC_RPC"},
                }
            )

    def flush(self) -> None:
        self.flush_calls += 1
        if self.flush_calls == 1:
            raise self.error_type(self.marker)


class CoordinatedFlushFailureProcess:
    def __init__(self, error_type, marker: str):
        self.stdout = CoordinatedStdout()
        self.stdin = FirstBarrierFlushFailureStdin(self.stdout, error_type, marker)
        self.stderr = None
        self.returncode = None

    def poll(self):
        return self.returncode


def session(harness, stdout):
    instance = object.__new__(harness.LodestarSession)
    instance.proc = SimulatedProcess(stdout)
    return instance


def injected_id_domain_session(harness):
    instance = object.__new__(harness.LodestarSession)
    stdin = FakeStdin()
    stdout = InjectedIdDomainStdout(stdin)
    process = SimulatedProcess(stdout)
    process.stdin = stdin
    instance.proc = process
    return instance, stdin, stdout


def check_injected_rejected_id_domain_returns_observed_invalid_request() -> None:
    """El arnés conserva el frame inyectado para ids fuera de string/i64."""
    harness = load_harness()
    invalid_ids = [
        ("object", {}),
        ("float", 1.5),
        ("boolean", True),
        ("null", None),
        ("integer-above-i64", 2**63),
    ]
    valid_ids = [("string", "request-7"), ("integer", 7)]
    failures: list[str] = []

    for label, request_id in invalid_ids:
        instance, stdin, stdout = injected_id_domain_session(harness)
        line = json.dumps({"jsonrpc": "2.0", "id": request_id, "method": "ping"})
        response = instance.raw_line(line, timeout=TIMEOUT)
        if stdin.lines != [line + "\n"]:
            failures.append(f"{label}: petición no escrita exactamente una vez: {stdin.lines!r}")
        if stdout.observed_line != line or stdout.reads < 1:
            failures.append(
                f"{label}: el double no observó la petición enviada: "
                f"line={stdout.observed_line!r} reads={stdout.reads}"
            )
        if not isinstance(response, dict) or response.get("error", {}).get("code") != -32600:
            failures.append(f"{label}: esperaba -32600 sin id observado, obtuvo {response!r}")
        elif "id" in response:
            failures.append(f"{label}: el -32600 inyectado debe omitir id: {response!r}")

    for label, request_id in valid_ids:
        instance, stdin, stdout = injected_id_domain_session(harness)
        line = json.dumps({"jsonrpc": "2.0", "id": request_id, "method": "ping"})
        response = instance.raw_line(line, timeout=TIMEOUT)
        if stdin.lines != [line + "\n"] or stdout.observed_line != line:
            failures.append(f"{label}: la guarda válida no recorrió el transporte real del double")
        if not isinstance(response, dict) or response.get("id") != request_id:
            failures.append(f"{label}: id válido dejó de correlacionar: {response!r}")
        elif response.get("result", {}).get("marker") != "accepted-number-or-string":
            failures.append(f"{label}: respuesta válida vacía o ajena: {response!r}")

    assert not failures, "dominio de ids JSON-RPC mal correlacionado:\n- " + "\n- ".join(failures)


class SequenceStdout:
    """Publica una secuencia finita y permite comprobar qué frames consumió el arnés."""

    def __init__(self, frames: list[str]):
        self.frames = list(frames)
        self.reads = 0

    def readline(self):
        self.reads += 1
        if self.frames:
            return self.frames.pop(0)
        return ""


class NullParamsPingStdout:
    """Publica la respuesta id=77 sólo tras observar el ping con ``params:null``."""

    def __init__(self, stdin):
        self.stdin = stdin
        self.reads = 0
        self.observed_request: dict | None = None

    def readline(self):
        self.reads += 1
        if self.reads != 1 or not self.stdin.lines:
            return ""
        self.observed_request = json.loads(self.stdin.lines[-1])
        if self.observed_request != {
            "jsonrpc": "2.0",
            "id": 77,
            "method": "ping",
            "params": None,
        }:
            return ""
        return '{"jsonrpc":"2.0","id":77,"result":{}}\n'


def check_null_params_is_absent_and_correlates_integer_id() -> None:
    """rmcp trata ``params:null`` como ausencia y conserva la correlación del id.

    Los escalares y listas siguen siendo requests inválidas; su negativo observable está
    fijado por ``fresh-idless-invalid-params``. Esta guarda evita ampliar ese rechazo a
    ``null``, que el servidor acepta igual que si el campo ``params`` estuviera ausente.
    """
    harness = load_harness()
    instance = object.__new__(harness.LodestarSession)
    stdin = FakeStdin()
    stdout = NullParamsPingStdout(stdin)
    process = SimulatedProcess(stdout)
    process.stdin = stdin
    instance.proc = process
    line = '{"jsonrpc":"2.0","id":77,"method":"ping","params":null}'

    response = instance.raw_line(line, timeout=TIMEOUT)

    assert stdin.lines == [line + "\n"], (
        f"el ping debe escribirse exactamente una vez: {stdin.lines!r}"
    )
    assert stdout.observed_request is not None and stdout.reads >= 1, (
        "el double coordinado debe observar la petición antes de publicar el frame"
    )
    assert response == {"jsonrpc": "2.0", "id": 77, "result": {}}, (
        "params:null debe comportarse como params ausente y devolver el frame "
        f"correspondiente a id=77, no silencio: {response!r}"
    )


def check_read_response_integer_id_rejects_bool_float_and_string_aliases() -> None:
    """El lector RPC comparte la correlación estricta de ids de ``raw_line``."""
    harness = load_harness()
    stdout = SequenceStdout(
        [
            '{"jsonrpc":"2.0","id":true,"result":{"marker":"bool-alias"}}\n',
            '{"jsonrpc":"2.0","id":1.0,"result":{"marker":"float-alias"}}\n',
            '{"jsonrpc":"2.0","id":"1","result":{"marker":"string-alias"}}\n',
            '{"jsonrpc":"2.0","id":1,"result":{"marker":"integer-match"}}\n',
        ]
    )
    instance = session(harness, stdout)

    response = instance._read_response(1, timeout=0.2)

    assert isinstance(response, dict), f"debe alcanzar el frame entero final: {response!r}"
    assert type(response.get("id")) is int, (
        f"bool/float/string no pueden correlacionar con el id entero: {response!r}"
    )
    assert response.get("result", {}).get("marker") == "integer-match", (
        "_read_response correlacionó un alias previo en vez del id entero exacto: "
        f"{response!r}"
    )
    assert stdout.reads >= 4, (
        f"debe consumir tres aliases antes del entero; lecturas={stdout.reads}"
    )


def check_fresh_request_returns_observed_idless_invalid_params_error() -> None:
    """Un -32600 sin id observado en una request fresca no se fabrica como silencio.

    El double sólo inyecta el frame indicado por la spec; este test no afirma que el
    binario rmcp real produzca ese error para todos estos valores de ``params``.
    """
    harness = load_harness()
    params_cases = [
        ("number", 1),
        ("string", "scalar"),
        ("list", ["item"]),
    ]
    failures: list[str] = []

    for label, params in params_cases:
        expected = {
            "jsonrpc": "2.0",
            "error": {
                "code": -32600,
                "message": f"invalid injected params: {label}",
            },
        }
        stdout = OneFrameStdout(json.dumps(expected) + "\n")
        instance = session(harness, stdout)
        line = json.dumps(
            {"jsonrpc": "2.0", "id": 7, "method": "ping", "params": params}
        )
        response = instance.raw_line(line, timeout=TIMEOUT)
        if instance.proc.stdin.lines != [line + "\n"]:
            failures.append(
                f"{label}: request no escrita exactamente una vez: "
                f"{instance.proc.stdin.lines!r}"
            )
        if stdout.reads < 1:
            failures.append(f"{label}: el double no llegó a publicar el frame")
        if response != expected:
            failures.append(
                f"{label}: -32600 sin id observado se perdió o cambió: {response!r}"
            )

    assert not failures, "params no-objeto mal clasificados:\n- " + "\n- ".join(failures)


def check_integer_id_rejects_bool_float_and_string_aliases() -> None:
    """Los aliases iguales según Python no correlacionan con un id JSON entero."""
    harness = load_harness()
    frames = [
        '{"jsonrpc":"2.0","id":true,"result":{"marker":"bool-alias"}}\n',
        '{"jsonrpc":"2.0","id":1.0,"result":{"marker":"float-alias"}}\n',
        '{"jsonrpc":"2.0","id":"1","result":{"marker":"string-alias"}}\n',
        '{"jsonrpc":"2.0","id":1,"result":{"marker":"integer-match"}}\n',
    ]
    stdout = SequenceStdout(frames)
    instance = session(harness, stdout)
    line = '{"jsonrpc":"2.0","id":1,"method":"ping"}'
    response = instance.raw_line(line, timeout=0.2)

    assert instance.proc.stdin.lines == [line + "\n"], (
        f"la request con id entero debe escribirse exactamente una vez: {instance.proc.stdin.lines!r}"
    )
    assert isinstance(response, dict), f"debe alcanzar el frame id entero final: {response!r}"
    assert type(response.get("id")) is int, (
        f"bool/float/string no pueden hacerse pasar por el id entero: {response!r}"
    )
    assert response.get("result", {}).get("marker") == "integer-match", (
        "raw_line correlacionó un alias previo de id en vez del entero exacto: "
        f"{response!r}"
    )
    assert stdout.reads >= 4, (
        f"debe descartar tres aliases antes del entero; lecturas={stdout.reads}"
    )


def check_timeout_is_bounded_when_timer_delivery_is_unavailable() -> None:
    """Un timeout de 50 ms no puede convertirse en una espera bloqueante de 750 ms.

    Simula un runtime donde las APIs de timer existen por compatibilidad pero no entregan
    interrupciones (la propiedad que el arnés debe soportar fuera de POSIX). Así la guarda
    temporal no depende de que el host dispare ``SIGALRM`` de verdad.
    """
    harness = load_harness()

    class NoDeliveryTimer:
        SIGALRM = 14
        ITIMER_REAL = 0

        @staticmethod
        def getsignal(_signal):
            return None

        @staticmethod
        def setitimer(_which, _seconds, _interval=0):
            return (0, 0)

        @staticmethod
        def signal(_signal, _handler):
            return None

    original_signal = harness.signal
    harness.signal = NoDeliveryTimer()
    try:
        started = time.monotonic()
        response = session(harness, BlockingStdout()).raw_line(
            "{json roto sin cerrar", timeout=TIMEOUT
        )
        elapsed = time.monotonic() - started
    finally:
        harness.signal = original_signal
    assert response is None, f"JSON ilegible debe ser silencio, no {response!r}"
    assert elapsed <= STRICT_LIMIT, (
        f"el timeout nominal {TIMEOUT:.3f}s no puede aceptar el bloqueo de "
        f"{BLOCKED_READ:.3f}s: debe retornar en <= {STRICT_LIMIT:.3f}s "
        f"(duró {elapsed:.3f}s)"
    )


def check_invalid_request_error_is_not_silenced() -> None:
    harness = load_harness()
    response = session(
        harness,
        OneFrameStdout(
            '{"jsonrpc":"2.0","error":{"code":-32600,"message":"Invalid Request"}}\n'
        ),
    ).raw_line('{"foo":"bar"}', timeout=TIMEOUT)
    assert isinstance(response, dict), "objeto JSON no JSON-RPC debe producir respuesta"
    assert response.get("error", {}).get("code") == -32600, (
        f"objeto JSON bien formado sin jsonrpc exige -32600: {response!r}"
    )


def check_non_string_method_returns_observed_invalid_request_instead_of_silence() -> None:
    """Un error sin id sigue perteneciendo a la request inválida que acaba de enviarse."""
    harness = load_harness()
    expected = {
        "jsonrpc": "2.0",
        "error": {"code": -32600, "message": "Invalid request"},
    }
    stdout = OneFrameStdout(
        '{"jsonrpc":"2.0","error":{"code":-32600,"message":"Invalid request"}}\n'
    )
    response = session(harness, stdout).raw_line(
        '{"jsonrpc":"2.0","id":7,"method":1}', timeout=TIMEOUT
    )
    assert response == expected, (
        "el -32600 sin id realmente emitido para method no-string no puede "
        f"descartarse hasta fabricar silencio: {response!r}"
    )


def check_malformed_json_returns_observed_server_parse_error_instead_of_silence() -> None:
    """Un frame defectuoso observado gana frente al silencio inferido del request."""
    harness = load_harness()
    expected = {
        "jsonrpc": "2.0",
        "id": None,
        "error": {
            "code": -32700,
            "message": "parse error emitido indebidamente",
            "data": {"origin": "malformed-input"},
        },
    }
    stdout = OneFrameStdout(
        '{"jsonrpc":"2.0","id":null,"error":{"code":-32700,'
        '"message":"parse error emitido indebidamente",'
        '"data":{"origin":"malformed-input"}}}\n'
    )
    response = session(harness, stdout).raw_line("{json roto sin cerrar", timeout=TIMEOUT)
    assert response == expected, (
        "un -32700 realmente emitido no puede descartarse para fabricar silencio: "
        f"{response!r}"
    )
    assert stdout.reads == 1, "la respuesta observada debe retornarse en su primer frame"


def check_notification_returns_observed_server_method_error_instead_of_silence() -> None:
    """Una notification es silenciosa sólo mientras el servidor no haya emitido frame."""
    harness = load_harness()
    expected = {
        "jsonrpc": "2.0",
        "id": None,
        "error": {
            "code": -32601,
            "message": "notification rechazada indebidamente",
            "data": {"origin": "id-less-notification"},
        },
    }
    stdout = OneFrameStdout(
        '{"jsonrpc":"2.0","id":null,"error":{"code":-32601,'
        '"message":"notification rechazada indebidamente",'
        '"data":{"origin":"id-less-notification"}}}\n'
    )
    response = session(harness, stdout).raw_line(
        '{"jsonrpc":"2.0","method":"metodo_que_no_existe"}', timeout=TIMEOUT
    )
    assert response == expected, (
        "un -32601 realmente emitido no puede descartarse para fabricar silencio: "
        f"{response!r}"
    )
    assert stdout.reads == 1, "la respuesta observada debe retornarse en su primer frame"


PREFETCH_CHILD = r'''
import sys
import time
sys.stdin.readline()
time.sleep(0.15)
sys.stdout.write(
    '{"jsonrpc":"2.0","id":1,"result":{"marker":"first"}}\n'
    '{"jsonrpc":"2.0","id":2,"result":{"marker":"second"}}\n'
)
sys.stdout.flush()
# Mantiene el descriptor del pipe abierto: selector no puede confundir EOF con datos.
# El padre cierra stdin durante el cleanup y entonces este read termina.
sys.stdin.read()
'''


LATE_IDLESS_ERROR_CHILD = r'''
import json
import sys

# La primera entrada no recibe respuesta. Si el arnés delimita el timeout mediante un
# ping interno, el error tardío se publica antes de su ack; sin barrera, se publica al
# recibir la segunda entrada como antes. En ambos casos sigue siendo causalmente previo.
sys.stdin.readline()
candidate = sys.stdin.readline().rstrip("\n")
parsed = json.loads(candidate)
is_barrier = (
    isinstance(parsed, dict)
    and parsed.get("jsonrpc") == "2.0"
    and parsed.get("method") == "ping"
    and "id" in parsed
    and parsed != {"jsonrpc": "2.0", "id": 2, "method": "ping"}
)
if is_barrier:
    sys.stdout.write(
        '{"jsonrpc":"2.0","error":{"code":-32600,"message":"late invalid request"}}\n'
        + json.dumps({"jsonrpc": "2.0", "id": parsed["id"], "result": {}})
        + "\n"
    )
    sys.stdout.flush()
    sys.stdin.readline()
else:
    sys.stdout.write(
        '{"jsonrpc":"2.0","error":{"code":-32600,"message":"late invalid request"}}\n'
    )
sys.stdout.write('{"jsonrpc":"2.0","id":2,"result":{"marker":"current request"}}\n')
sys.stdout.flush()
sys.stdin.read()
'''


LATE_EXPIRED_ID_CHILD = r'''
import json
import sys

# El propio pipe fija el orden: la respuesta id=1 sólo se publica después de que el
# padre haya enviado la segunda entrada silenciosa. No hay sleeps ni una carrera de reloj.
first = sys.stdin.readline().rstrip("\n")
second = sys.stdin.readline().rstrip("\n")
sys.stdout.write(
    '{"jsonrpc":"2.0","id":1,"result":{"marker":"expired-id-1"}}\n'
)
sys.stdout.flush()
sys.stderr.write(json.dumps({"first": first, "second": second}) + "\n")
sys.stderr.flush()
# Conserva stdout abierto: devolver None debe deberse al timeout, no a EOF.
sys.stdin.read()
'''


LATE_EXPIRED_IDLESS_ERROR_CHILD = r'''
import json
import sys

# El pipe coordina sin sleeps. Una barrera futura puede enviar un ping interno: en ese
# caso el error tardío sale antes del ack y la segunda línea de usuario se lee después.
# Sin barrera se conserva la secuencia histórica, publicándolo tras la segunda entrada.
first = sys.stdin.readline().rstrip("\n")
candidate = sys.stdin.readline().rstrip("\n")
try:
    parsed = json.loads(candidate)
except json.JSONDecodeError:
    parsed = None
is_barrier = (
    isinstance(parsed, dict)
    and parsed.get("jsonrpc") == "2.0"
    and parsed.get("method") == "ping"
    and "id" in parsed
)
late = (
    '{"jsonrpc":"2.0","error":{"code":-32600,'
    '"message":"late invalid request"}}\n'
)
if is_barrier:
    sys.stdout.write(
        late
        + json.dumps({"jsonrpc": "2.0", "id": parsed["id"], "result": {}})
        + "\n"
    )
    sys.stdout.flush()
    second = sys.stdin.readline().rstrip("\n")
else:
    second = candidate
    sys.stdout.write(late)
    sys.stdout.flush()
sys.stderr.write(json.dumps({"first": first, "second": second}) + "\n")
sys.stderr.flush()
# Mantener stdout abierto prueba que None procede del plazo, no de EOF.
sys.stdin.read()
'''


REJECTED_BOOL_THEN_FRESH_INVALID_CHILD = r'''
import json
import sys

# Reproduce la secuencia observable de rmcp sin sleeps: el id booleano rechazado no
# produce frame; el objeto inválido siguiente produce su propio -32600 sin id; el ping
# final acredita que el transporte sigue vivo y que la correlación no se desplazó.
first = sys.stdin.readline().rstrip("\n")
second = sys.stdin.readline().rstrip("\n")
if json.loads(first) != {"jsonrpc": "2.0", "id": True, "method": "ping"}:
    raise SystemExit(81)
if json.loads(second) != {"foo": "bar"}:
    raise SystemExit(82)
sys.stdout.write(
    '{"jsonrpc":"2.0","error":{"code":-32600,'
    '"message":"fresh invalid request"}}\n'
)
sys.stdout.flush()

third = sys.stdin.readline().rstrip("\n")
if json.loads(third) != {"jsonrpc": "2.0", "id": 91, "method": "ping"}:
    raise SystemExit(83)
sys.stdout.write(
    '{"jsonrpc":"2.0","id":91,"result":{"marker":"session-alive"}}\n'
)
sys.stdout.flush()
sys.stderr.write(json.dumps({"first": first, "second": second, "third": third}) + "\n")
sys.stderr.flush()
'''


FRESH_IDFUL_DURING_SILENCE_CHILD = r'''
import json
import sys

# El pipe coordina la causalidad sin sleeps: id=77 se publica sólo después de leer
# la primera y única entrada. Mantener stdout abierto impide confundir el resultado
# con EOF y acredita que el frame fue descartado por la correlación del arnés.
observed = sys.stdin.readline().rstrip("\n")
sys.stdout.write(
    '{"jsonrpc":"2.0","id":77,'
    '"result":{"marker":"fresh-during-silence"}}\n'
)
sys.stdout.flush()
sys.stderr.write(json.dumps({"observed": observed}) + "\n")
sys.stderr.flush()
sys.stdin.read()
'''


EXPIRED_INTEGER_THEN_STRING_ALIAS_CHILD = r'''
import json
import sys

# La request id entero 1 vence sin frame. Tras la notificación se publica id string
# "1": ambos son ids válidos, pero no son el mismo id JSON-RPC por tipo+valor.
first = sys.stdin.readline().rstrip("\n")
second = sys.stdin.readline().rstrip("\n")
sys.stdout.write(
    '{"jsonrpc":"2.0","id":"1",'
    '"result":{"marker":"string-id-is-fresh"}}\n'
)
sys.stdout.flush()
sys.stderr.write(json.dumps({"first": first, "second": second}) + "\n")
sys.stderr.flush()
sys.stdin.read()
'''


IDLESS_CAUSAL_BOUNDARY_CHILD = r'''
import json
import sys

mode = sys.argv[1]
barriers = []

def next_user_line():
    while True:
        raw = sys.stdin.readline()
        if not raw:
            raise SystemExit(91)
        line = raw.rstrip("\n")
        try:
            parsed = json.loads(line)
        except json.JSONDecodeError:
            return line
        valid_id = isinstance(parsed.get("id"), str) or type(parsed.get("id")) is int
        if (
            isinstance(parsed, dict)
            and parsed.get("jsonrpc") == "2.0"
            and parsed.get("method") == "ping"
            and "id" in parsed
            and valid_id
        ):
            barriers.append(line)
            sys.stdout.write(
                json.dumps({"jsonrpc": "2.0", "id": parsed["id"], "result": {}})
                + "\n"
            )
            sys.stdout.flush()
            continue
        return line

# Los acks anteriores son exclusivamente infraestructura para una barrera interna futura;
# el primer invalid no genera error. El único frame de negocio se publica al segundo evento.
first = next_user_line()
second = next_user_line()
if json.loads(first) != {"foo": "first-invalid"}:
    raise SystemExit(92)
if mode == "second-invalid":
    if json.loads(second) != {"foo": "second-invalid"}:
        raise SystemExit(93)
    sys.stdout.write(
        '{"jsonrpc":"2.0","error":{"code":-32600,'
        '"message":"fresh second invalid"}}\n'
    )
elif mode == "notification":
    if json.loads(second) != {"jsonrpc": "2.0", "method": "notify/fresh-idless"}:
        raise SystemExit(94)
    sys.stdout.write(
        '{"jsonrpc":"2.0","id":null,"error":{"code":-32601,'
        '"message":"fresh notification frame"}}\n'
    )
else:
    raise SystemExit(95)
sys.stdout.flush()
sys.stderr.write(
    json.dumps({"first": first, "second": second, "barriers": barriers}) + "\n"
)
sys.stderr.flush()
# El descriptor sigue vivo: un None observado procede del descarte/timeout del arnés.
sys.stdin.read()
'''


RAW_ID_COLLISION_WITH_RPC_CHILD = r'''
import json
import sys

# La respuesta raw vencida sólo aparece tras recibir el rpc siguiente. Sale primero
# para que reutilizar id=2 produzca una falsa correlación observable (STALE_RAW).
raw_line = sys.stdin.readline().rstrip("\n")
rpc_line = sys.stdin.readline().rstrip("\n")
raw_request = json.loads(raw_line)
rpc_request = json.loads(rpc_line)
if raw_request != {"jsonrpc": "2.0", "id": 2, "method": "ping"}:
    raise SystemExit(101)
if rpc_request.get("jsonrpc") != "2.0" or rpc_request.get("method") != "ping":
    raise SystemExit(102)
rpc_id = rpc_request.get("id")
sys.stdout.write(
    '{"jsonrpc":"2.0","id":2,"result":{"marker":"STALE_RAW"}}\n'
    + json.dumps(
        {"jsonrpc": "2.0", "id": rpc_id, "result": {"marker": "FRESH_RPC"}}
    )
    + "\n"
)
sys.stdout.flush()
sys.stderr.write(
    json.dumps({"raw": raw_line, "rpc": rpc_line, "rpc_id": rpc_id}) + "\n"
)
sys.stderr.flush()
sys.stdin.read()
'''


FRESH_FRAME_DURING_BARRIER_CHILD = r'''
import json
import sys

# Se crean dos deudas reales: una respuesta id string pendiente y un invalid idless
# que necesita barrera. Durante el ping se publica primero un frame fresco id=77 y
# después el ACK; la notification pública se acredita antes de mantener stdout vivo.
pending_line = sys.stdin.readline().rstrip("\n")
invalid_line = sys.stdin.readline().rstrip("\n")
barrier_line = sys.stdin.readline().rstrip("\n")
pending = json.loads(pending_line)
invalid = json.loads(invalid_line)
barrier = json.loads(barrier_line)
if pending.get("method") != "ping" or not isinstance(pending.get("id"), str):
    raise SystemExit(111)
if invalid != {"foo": "invalid-needs-boundary"}:
    raise SystemExit(112)
if (
    barrier.get("jsonrpc") != "2.0"
    or barrier.get("method") != "ping"
    or not isinstance(barrier.get("id"), str)
):
    raise SystemExit(113)
sys.stdout.write(
    '{"jsonrpc":"2.0","id":77,'
    '"result":{"marker":"FRESH_DURING_BARRIER"}}\n'
    + json.dumps({"jsonrpc": "2.0", "id": barrier["id"], "result": {}})
    + "\n"
)
sys.stdout.flush()
notification_line = sys.stdin.readline().rstrip("\n")
notification = json.loads(notification_line)
if notification != {"jsonrpc": "2.0", "method": "notify/after-boundary"}:
    raise SystemExit(114)
sys.stderr.write(
    json.dumps(
        {
            "pending": pending_line,
            "invalid": invalid_line,
            "barrier": barrier_line,
            "barrier_id": barrier["id"],
            "notification": notification_line,
        }
    )
    + "\n"
)
sys.stderr.flush()
sys.stdin.read()
'''


FOREIGN_FRAMES_BEFORE_CURRENT_RESPONSE_CHILD = r'''
import json
import sys

mode = sys.argv[1]
first_line = sys.stdin.readline().rstrip("\n")
first = json.loads(first_line)
if first.get("jsonrpc") != "2.0" or first.get("method") != "ping":
    raise SystemExit(121)
current_id = first.get("id")
if type(current_id) is not int:
    raise SystemExit(122)
if mode == "raw" and current_id != 1:
    raise SystemExit(123)
if mode not in ("raw", "rpc"):
    raise SystemExit(124)

# Orden causal acreditable: un frame ajeno id=77, la respuesta que permite cerrar la
# operación actual y otro ajeno id=78. Las dos raw posteriores deben consumir 77 y 78
# en FIFO, aunque 78 permanezca todavía en el pipe cuando 77 ya esté en backlog.
current_marker = "CURRENT_RAW" if mode == "raw" else "CURRENT_RPC"
sys.stdout.write(
    '{"jsonrpc":"2.0","id":77,"result":{"marker":"FOREIGN_FIFO_1"}}\n'
    + json.dumps(
        {"jsonrpc": "2.0", "id": current_id, "result": {"marker": current_marker}}
    )
    + "\n"
    + '{"jsonrpc":"2.0","id":78,"result":{"marker":"FOREIGN_FIFO_2"}}\n'
)
sys.stdout.flush()
follow_1 = sys.stdin.readline().rstrip("\n")
follow_2 = sys.stdin.readline().rstrip("\n")
sys.stderr.write(
    json.dumps(
        {
            "first": first_line,
            "current_id": current_id,
            "follow_1": follow_1,
            "follow_2": follow_2,
            "emitted_ids": [77, current_id, 78],
        }
    )
    + "\n"
)
sys.stderr.flush()
sys.stdin.read()
'''


BARRIER_IDLESS_CLASSIFICATION_CHILD = r'''
import json
import sys

mode = sys.argv[1]
invalid_line = sys.stdin.readline().rstrip("\n")
barrier_line = sys.stdin.readline().rstrip("\n")
invalid = json.loads(invalid_line)
barrier = json.loads(barrier_line)
if invalid.get("foo") not in (
    "invalid-before-fresh-barrier",
    "invalid-before-double-32600",
):
    raise SystemExit(131)
if (
    barrier.get("jsonrpc") != "2.0"
    or barrier.get("method") != "ping"
    or not isinstance(barrier.get("id"), str)
):
    raise SystemExit(132)

if mode == "fresh-id-null":
    sys.stdout.write(
        '{"jsonrpc":"2.0","id":null,"error":{"code":-32601,'
        '"message":"fresh during barrier",'
        '"data":{"marker":"FRESH_DURING_BARRIER"}}}\n'
    )
elif mode == "at-most-one-32600":
    # Una sola entrada invalid no puede causar dos respuestas: como máximo el primer
    # -32600 es atribuible. El segundo debe conservarse como frame fresco en FIFO.
    sys.stdout.write(
        '{"jsonrpc":"2.0","error":{"code":-32600,'
        '"message":"late attributable invalid",'
        '"data":{"marker":"STALE_ATTRIBUTABLE_INVALID"}}}\n'
        '{"jsonrpc":"2.0","id":null,"error":{"code":-32600,'
        '"message":"second idless is fresh",'
        '"data":{"marker":"SECOND_32600_NOT_ATTRIBUTABLE"}}}\n'
    )
else:
    raise SystemExit(133)
sys.stdout.write(
    json.dumps({"jsonrpc": "2.0", "id": barrier["id"], "result": {}}) + "\n"
)
sys.stdout.flush()

follow_1 = sys.stdin.readline().rstrip("\n")
follow_2 = None
if mode == "at-most-one-32600":
    follow_2 = sys.stdin.readline().rstrip("\n")
sys.stderr.write(
    json.dumps(
        {
            "mode": mode,
            "invalid": invalid_line,
            "barrier": barrier_line,
            "barrier_id": barrier["id"],
            "follow_1": follow_1,
            "follow_2": follow_2,
        }
    )
    + "\n"
)
sys.stderr.flush()
sys.stdin.read()
'''


SERVER_REQUEST_SAME_PENDING_ID_CHILD = r'''
import json
import sys

raw_line = sys.stdin.readline().rstrip("\n")
silent_1 = sys.stdin.readline().rstrip("\n")
if json.loads(raw_line) != {"jsonrpc": "2.0", "id": 1, "method": "ping"}:
    raise SystemExit(141)
sys.stdout.write(
    '{"jsonrpc":"2.0","id":1,"method":"server/fresh",'
    '"params":{"marker":"NOT_A_RESPONSE"}}\n'
)
sys.stdout.flush()

silent_2 = sys.stdin.readline().rstrip("\n")
sys.stdout.write(
    '{"jsonrpc":"2.0","id":1,'
    '"result":{"marker":"REAL_PENDING_RESPONSE"}}\n'
)
sys.stdout.flush()

silent_3 = sys.stdin.readline().rstrip("\n")
sys.stdout.write(
    '{"jsonrpc":"2.0","id":1,"method":"server/after-real",'
    '"params":{"marker":"AFTER_PENDING_CLEARED"}}\n'
)
sys.stdout.flush()
sys.stderr.write(
    json.dumps(
        {
            "raw": raw_line,
            "silent_1": silent_1,
            "silent_2": silent_2,
            "silent_3": silent_3,
        }
    )
    + "\n"
)
sys.stderr.flush()
sys.stdin.read()
'''


PENDING_RESPONSE_SHAPE_MATRIX_CHILD = r'''
import json
import sys

kind = sys.argv[1]
candidate = json.loads(sys.argv[2])
raw_line = sys.stdin.readline().rstrip("\n")
silent_1 = sys.stdin.readline().rstrip("\n")
if json.loads(raw_line) != {"jsonrpc": "2.0", "id": 1, "method": "ping"}:
    raise SystemExit(151)
sys.stdout.write(json.dumps(candidate, separators=(",", ":")) + "\n")
sys.stdout.flush()

silent_2 = sys.stdin.readline().rstrip("\n")
if kind == "nonresponse":
    control = {
        "jsonrpc": "2.0",
        "id": 1,
        "result": {"marker": "REAL_PENDING_RESPONSE"},
    }
elif kind == "response":
    control = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "server/after-valid-response",
        "params": {"marker": "PENDING_WAS_CLEARED"},
    }
else:
    raise SystemExit(152)
sys.stdout.write(json.dumps(control, separators=(",", ":")) + "\n")
sys.stdout.flush()
sys.stderr.write(
    json.dumps(
        {
            "kind": kind,
            "candidate": candidate,
            "raw": raw_line,
            "silent_1": silent_1,
            "silent_2": silent_2,
        }
    )
    + "\n"
)
sys.stderr.flush()
sys.stdin.read()
'''


REUSED_PENDING_RAW_ID_CHILD = r'''
import json
import sys

mode = sys.argv[1]
first_line = sys.stdin.readline().rstrip("\n")
candidate_line = sys.stdin.readline().rstrip("\n")
first = json.loads(first_line)
candidate = json.loads(candidate_line)
if first != {"jsonrpc": "2.0", "id": 1, "method": "ping"}:
    raise SystemExit(161)

barrier_line = None
barrier_id = None
is_barrier = (
    candidate.get("jsonrpc") == "2.0"
    and candidate.get("method") == "ping"
    and isinstance(candidate.get("id"), str)
)
emitted = []
if is_barrier:
    barrier_line = candidate_line
    barrier_id = candidate["id"]
    if mode == "stale-before-ack":
        sys.stdout.write(
            '{"jsonrpc":"2.0","id":1,'
            '"result":{"marker":"STALE_FIRST"}}\n'
        )
        emitted.append("STALE_FIRST")
    sys.stdout.write(
        json.dumps({"jsonrpc": "2.0", "id": barrier_id, "result": {}}) + "\n"
    )
    emitted.append("BARRIER_ACK")
    sys.stdout.flush()
    second_line = sys.stdin.readline().rstrip("\n")
    second = json.loads(second_line)
else:
    second_line = candidate_line
    second = candidate

expected_second_id = 2 if mode == "noncolliding-id-2" else 1
if second != {"jsonrpc": "2.0", "id": expected_second_id, "method": "ping"}:
    raise SystemExit(162)
if mode == "stale-before-ack" and not is_barrier:
    sys.stdout.write(
        '{"jsonrpc":"2.0","id":1,"result":{"marker":"STALE_FIRST"}}\n'
    )
    emitted.append("STALE_FIRST")
second_marker = "FRESH_ID2" if mode == "noncolliding-id-2" else "FRESH_SECOND"
sys.stdout.write(
    json.dumps(
        {
            "jsonrpc": "2.0",
            "id": expected_second_id,
            "result": {"marker": second_marker},
        }
    )
    + "\n"
)
emitted.append(second_marker)
sys.stdout.flush()

postcheck_line = sys.stdin.readline().rstrip("\n")
sys.stdout.write(
    '{"jsonrpc":"2.0","id":1,'
    '"result":{"marker":"AFTER_SEQUENCE"}}\n'
)
emitted.append("AFTER_SEQUENCE")
sys.stdout.flush()
sys.stderr.write(
    json.dumps(
        {
            "mode": mode,
            "first": first_line,
            "barrier": barrier_line,
            "barrier_id": barrier_id,
            "second": second_line,
            "postcheck": postcheck_line,
            "emitted": emitted,
        }
    )
    + "\n"
)
sys.stderr.flush()
sys.stdin.read()
'''


NONFINITE_RPC_RESPONSE_CHILD = r'''
import json
import sys

rpc_line = sys.stdin.readline().rstrip("\n")
rpc = json.loads(rpc_line)
rpc_id = rpc.get("id")
if rpc.get("jsonrpc") != "2.0" or rpc.get("method") != "ping" or type(rpc_id) is not int:
    raise SystemExit(171)
frames = [
    '{"jsonrpc":"2.0","id":%d,"result":NaN}' % rpc_id,
    '{"jsonrpc":"2.0","id":%d,"result":{"nested":Infinity}}' % rpc_id,
    '{"jsonrpc":"2.0","id":%d,"result":[0,-Infinity]}' % rpc_id,
]
sys.stdout.write("\n".join(frames) + "\n")
sys.stdout.write(
    json.dumps(
        {"jsonrpc": "2.0", "id": rpc_id, "result": {"marker": "VALID_RPC"}}
    )
    + "\n"
)
sys.stdout.flush()
raw_lines = [sys.stdin.readline().rstrip("\n") for _ in range(3)]
sys.stderr.write(
    json.dumps(
        {
            "rpc": rpc_line,
            "rpc_id": rpc_id,
            "frames": frames,
            "raw_lines": raw_lines,
        }
    )
    + "\n"
)
sys.stderr.flush()
sys.stdin.read()
'''


STRICT_RAW_RESPONSE_TEXT_CHILD = r'''
import json
import sys

kind = sys.argv[1]
candidate = sys.argv[2]
raw_line = sys.stdin.readline().rstrip("\n")
silent_1 = sys.stdin.readline().rstrip("\n")
if json.loads(raw_line) != {"jsonrpc": "2.0", "id": 1, "method": "ping"}:
    raise SystemExit(181)
sys.stdout.write(candidate + "\n")
sys.stdout.flush()
silent_2 = sys.stdin.readline().rstrip("\n")
if kind == "nonfinite":
    control = '{"jsonrpc":"2.0","id":1,"result":{"marker":"REAL_VALID_RESPONSE"}}'
elif kind == "valid":
    control = (
        '{"jsonrpc":"2.0","id":1,"method":"server/after-valid",'
        '"params":{"marker":"PENDING_CLEARED"}}'
    )
else:
    raise SystemExit(182)
sys.stdout.write(control + "\n")
sys.stdout.flush()
sys.stderr.write(
    json.dumps(
        {
            "kind": kind,
            "candidate": candidate,
            "raw": raw_line,
            "silent_1": silent_1,
            "silent_2": silent_2,
        }
    )
    + "\n"
)
sys.stderr.flush()
sys.stdin.read()
'''


STRICT_RAW_INPUT_CHILD = r'''
import json
import sys

kind = sys.argv[1]
line = sys.stdin.readline().rstrip("\n")
if kind == "valid":
    sys.stdout.write('{"jsonrpc":"2.0","id":7,"result":{"marker":"VALID_INPUT"}}\n')
    sys.stdout.flush()
elif kind != "nonfinite":
    raise SystemExit(191)
sys.stderr.write(json.dumps({"kind": kind, "line": line}) + "\n")
sys.stderr.flush()
sys.stdin.read()
'''


CURRENT_INVALID_FRESH_BEFORE_ATTRIBUTABLE_CHILD = r'''
import json
import sys

id_form = sys.argv[1]
invalid_line = sys.stdin.readline().rstrip("\n")
if json.loads(invalid_line) != {"foo": "current-invalid"}:
    raise SystemExit(201)
id_field = "" if id_form == "absent" else '"id":null,'
fresh = (
    '{"jsonrpc":"2.0",'
    + id_field
    + '"error":{"code":-32601,"message":"fresh idless",'
    '"data":{"marker":"FRESH_IDLESS_32601"}}}'
)
attributable = (
    '{"jsonrpc":"2.0",'
    + id_field
    + '"error":{"code":-32600,"message":"current invalid",'
    '"data":{"marker":"ATTRIBUTABLE_CURRENT"}}}'
)
sys.stdout.write(fresh + "\n" + attributable + "\n")
sys.stdout.flush()
follow_line = sys.stdin.readline().rstrip("\n")
sys.stderr.write(
    json.dumps(
        {
            "id_form": id_form,
            "invalid": invalid_line,
            "follow": follow_line,
            "frames": [fresh, attributable],
        }
    )
    + "\n"
)
sys.stderr.flush()
sys.stdin.read()
'''


CURRENT_INVALID_CORRELATION_MATRIX_CHILD = r'''
import json
import sys

kind = sys.argv[1]
candidate = sys.argv[2]
invalid_line = sys.stdin.readline().rstrip("\n")
if json.loads(invalid_line) != {"foo": "matrix-current-invalid"}:
    raise SystemExit(211)
sys.stdout.write(candidate + "\n")
attributable = None
if kind == "nonattributable":
    attributable = (
        '{"jsonrpc":"2.0","error":{"code":-32600,'
        '"message":"matrix attributable",'
        '"data":{"marker":"ATTRIBUTABLE_MATRIX"}}}'
    )
    sys.stdout.write(attributable + "\n")
elif kind != "attributable":
    raise SystemExit(212)
sys.stdout.flush()
follow_line = sys.stdin.readline().rstrip("\n")
sys.stderr.write(
    json.dumps(
        {
            "kind": kind,
            "candidate": candidate,
            "attributable": attributable,
            "invalid": invalid_line,
            "follow": follow_line,
        }
    )
    + "\n"
)
sys.stderr.flush()
sys.stdin.read()
'''


IDLESS_PARAMS_CLASSIFIER_CHILD = r'''
import json
import sys

behavior = sys.argv[1]
id_form = sys.argv[2]
line = sys.stdin.readline().rstrip("\n")
if behavior == "invalid-response":
    id_field = "" if id_form == "absent" else '"id":null,'
    sys.stdout.write(
        '{"jsonrpc":"2.0",'
        + id_field
        + '"error":{"code":-32600,"message":"invalid params idless"}}\n'
    )
    sys.stdout.flush()
elif behavior != "silence":
    raise SystemExit(221)
sys.stderr.write(
    json.dumps({"behavior": behavior, "id_form": id_form, "line": line}) + "\n"
)
sys.stderr.flush()
sys.stdin.read()
'''


RESYNC_FAILURE_CHILD = r'''
import json
import sys

mode = sys.argv[1]
exit_code = int(sys.argv[2])
first = sys.stdin.readline().rstrip("\n")
barrier = sys.stdin.readline().rstrip("\n")
record = {"mode": mode, "first": first, "barrier": barrier, "rest": []}
if mode == "timeout":
    rest = [line.rstrip("\n") for line in sys.stdin.readlines()]
    record["rest"] = rest
    sys.stderr.write(json.dumps(record) + "\n")
    sys.stderr.flush()
elif mode == "eof":
    sys.stderr.write(json.dumps(record) + "\n")
    sys.stderr.flush()
    raise SystemExit(exit_code)
else:
    raise SystemExit(222)
'''


READ_RESPONSE_EOF_CHILD = r'''
import sys

sys.stderr.write("read-response-eof-marker\n")
sys.stderr.flush()
raise SystemExit(23)
'''


RAW_LINE_EOF_CHILD = r'''
import sys

line = sys.stdin.readline()
if not line:
    raise SystemExit(91)
sys.stderr.write("raw-line-observed=" + line)
sys.stderr.flush()
raise SystemExit(17)
'''


PERSISTENT_LIVE_EOF_CHILD = r'''
import json
import os
import sys

first = sys.stdin.readline().rstrip("\n")
if not first:
    raise SystemExit(92)
os.close(sys.stdout.fileno())
lines = [first]
lines.extend(line.rstrip("\n") for line in sys.stdin.readlines())
sys.stderr.write(json.dumps({"lines": lines}) + "\n")
sys.stderr.flush()
'''


TERMINAL_EOF_CHILD = r'''
import sys

mode = sys.argv[1]
marker = sys.argv[2]
exit_code = int(sys.argv[3])
if mode == "after-first":
    line = sys.stdin.readline().rstrip("\n")
    if not line:
        raise SystemExit(93)
    sys.stderr.write(marker + ";observed=" + line + "\n")
elif mode == "already-exited":
    sys.stderr.write(marker + ";observed=<none>\n")
else:
    raise SystemExit(94)
sys.stderr.flush()
raise SystemExit(exit_code)
'''


QUEUED_RESPONSE_THEN_EXIT_CHILD = r'''
import json
import sys

marker = sys.argv[1]
line = sys.stdin.readline().rstrip("\n")
if not line:
    raise SystemExit(95)
request = json.loads(line)
request_id = request.get("id")
response = {
    "jsonrpc": "2.0",
    "id": request_id,
    "result": {"marker": marker},
}
sys.stdout.write(json.dumps(response) + "\n")
sys.stdout.flush()
sys.stderr.write(marker + ";observed=" + line + "\n")
sys.stderr.flush()
raise SystemExit(0)
'''


CORRELATED_THEN_FOREIGN_EXIT_CHILD = r'''
import json
import sys

marker = sys.argv[1]
line = sys.stdin.readline().rstrip("\n")
if not line:
    raise SystemExit(99)
request = json.loads(line)
request_id = request.get("id")
correlated = {
    "jsonrpc": "2.0",
    "id": request_id,
    "result": {"marker": "CORRELATED_BEFORE_FOREIGN"},
}
foreign = {
    "jsonrpc": "2.0",
    "id": 77,
    "method": "server/foreign-after-correlated",
    "params": {"marker": marker},
}
frames = [correlated, foreign]
for frame in frames:
    sys.stdout.write(json.dumps(frame) + "\n")
sys.stdout.flush()
sys.stderr.write(
    json.dumps({"line": line, "frames": frames, "marker": marker}) + "\n"
)
sys.stderr.flush()
raise SystemExit(0)
'''


CORRELATED_THREE_PRE_EOF_FRAMES_CHILD = r'''
import json
import sys

marker = sys.argv[1]
line = sys.stdin.readline().rstrip("\n")
if not line:
    raise SystemExit(96)
request = json.loads(line)
correlated = {
    "jsonrpc": "2.0",
    "id": request.get("id"),
    "result": {"marker": "CORRELATED_BEFORE_THREE"},
}
foreign_77 = {
    "jsonrpc": "2.0",
    "id": 77,
    "result": {"marker": marker + ":77"},
}
unparseable = "NOT_JSON_PRE_EOF:" + marker
foreign_78 = {
    "jsonrpc": "2.0",
    "id": 78,
    "result": {"marker": marker + ":78"},
}
wire_frames = [
    json.dumps(correlated),
    json.dumps(foreign_77),
    unparseable,
    json.dumps(foreign_78),
]
for frame in wire_frames:
    sys.stdout.write(frame + "\n")
sys.stdout.flush()
sys.stderr.write(
    json.dumps({"line": line, "wire_frames": wire_frames, "marker": marker}) + "\n"
)
sys.stderr.flush()
raise SystemExit(0)
'''


LIVE_CORRELATED_THEN_FOREIGN_CHILD = r'''
import json
import sys

marker = sys.argv[1]
first_line = sys.stdin.readline().rstrip("\n")
if not first_line:
    raise SystemExit(97)
first_request = json.loads(first_line)
correlated = {
    "jsonrpc": "2.0",
    "id": first_request.get("id"),
    "result": {"marker": "LIVE_CORRELATED_ID5"},
}
foreign = {
    "jsonrpc": "2.0",
    "id": 77,
    "result": {"marker": marker},
}
frames = [correlated, foreign]
for frame in frames:
    sys.stdout.write(json.dumps(frame) + "\n")
sys.stdout.flush()

# El proceso permanece vivo y stdout abierto hasta acreditar la segunda request id6.
second_line = sys.stdin.readline().rstrip("\n")
if not second_line:
    raise SystemExit(98)
second_request = json.loads(second_line)
sys.stderr.write(
    json.dumps(
        {
            "lines": [first_line, second_line],
            "ids": [first_request.get("id"), second_request.get("id")],
            "frames": frames,
            "marker": marker,
        }
    )
    + "\n"
)
sys.stderr.flush()
raise SystemExit(0)
'''


QUEUED_BARRIER_ACK_THEN_EXIT_CHILD = r'''
import json
import sys

marker = sys.argv[1]
line = sys.stdin.readline().rstrip("\n")
if not line:
    raise SystemExit(96)
barrier = json.loads(line)
barrier_id = barrier.get("id")
response = {
    "jsonrpc": "2.0",
    "id": barrier_id,
    "result": {"marker": marker},
}
sys.stdout.write(json.dumps(response) + "\n")
sys.stdout.flush()
sys.stderr.write(marker + ";observed=" + line + "\n")
sys.stderr.flush()
raise SystemExit(0)
'''


NO_ACK_LIVE_AFTER_BARRIER_CHILD = r'''
import json
import sys

first = sys.stdin.readline().rstrip("\n")
if not first:
    raise SystemExit(97)
rest = [line.rstrip("\n") for line in sys.stdin.readlines()]
sys.stderr.write(json.dumps({"first": first, "rest": rest}) + "\n")
sys.stderr.flush()
'''


FOREIGN_THEN_ACK_LIVE_CHILD = r'''
import json
import sys

marker = sys.argv[1]
first = sys.stdin.readline().rstrip("\n")
if not first:
    raise SystemExit(98)
barrier = json.loads(first)
barrier_id = barrier.get("id")
foreign = {
    "jsonrpc": "2.0",
    "id": 77,
    "method": "server/foreign",
    "params": {"marker": marker},
}
ack = {
    "jsonrpc": "2.0",
    "id": barrier_id,
    "result": {"marker": "STRICT_BARRIER_ACK"},
}
frames = [foreign, ack]
for frame in frames:
    sys.stdout.write(json.dumps(frame) + "\n")
sys.stdout.flush()
rest = [line.rstrip("\n") for line in sys.stdin.readlines()]
sys.stderr.write(
    json.dumps(
        {
            "first": first,
            "rest": rest,
            "frames": frames,
        }
    )
    + "\n"
)
sys.stderr.flush()
'''


CLOSE_LIFECYCLE_CHILD = r'''
import sys

sys.stdout.write("reader-ready\n")
sys.stdout.flush()
sys.stdin.read()
'''


def check_read_response_eof_reports_context_without_waiting_for_timeout() -> None:
    """EOF es un fallo inmediato y conserva id, stderr y código de salida."""
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", READ_RESPONSE_EOF_CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    process.wait(timeout=1)

    started = time.monotonic()
    try:
        instance._read_response(73, timeout=2.0)
    except RuntimeError as error:
        elapsed = time.monotonic() - started
        message = str(error)
    else:
        raise AssertionError("EOF antes de id=73 debe producir RuntimeError")
    finally:
        if process.stdin is not None:
            process.stdin.close()
        reader = getattr(instance, "_stdout_reader", None)
        if reader is not None:
            reader.join(timeout=1)

    normalized = message.lower()
    assert elapsed <= STRICT_LIMIT, (
        "EOF ya observado debe fallar inmediatamente, no consumir el timeout de 2s: "
        f"{elapsed:.3f}s"
    )
    assert "eof" in normalized and "73" in message, (
        f"el error debe identificar EOF y el id esperado: {message!r}"
    )
    assert "read-response-eof-marker" in message, (
        f"el diagnóstico debe conservar stderr: {message!r}"
    )
    assert ("exit=23" in normalized or "returncode=23" in normalized), (
        f"el diagnóstico debe conservar el código de salida 23: {message!r}"
    )


def check_raw_line_eof_reports_server_exit_instead_of_silence() -> None:
    """La caída real tras consumir la línea no puede confundirse con silencio JSON-RPC."""
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", RAW_LINE_EOF_CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    process.stdin = CountingPopenStdin(
        process.stdin,
        process,
        wait_after_first_flush=False,
        wait_for_process_exit=True,
    )
    instance.proc = process
    line = '{"jsonrpc":"2.0","method":"notify/eof"}'
    try:
        response = instance.raw_line(line, timeout=0.5)
        process.wait(timeout=1)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=1)
        if process.stdin is not None:
            process.stdin.close()
        reader = getattr(instance, "_stdout_reader", None)
        if reader is not None:
            reader.join(timeout=1)

    assert process.returncode == 17, (
        "la guarda sólo es válida si el hijo consumió la línea y terminó por la ruta 17; "
        f"returncode={process.returncode}"
    )
    assert response == {"server_exited": 17}, (
        "EOF/proceso caído debe conservar el resultado observable histórico, no devolver "
        f"None como silencio: {response!r}"
    )


def persistent_live_eof_session(harness):
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", PERSISTENT_LIVE_EOF_CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    counter = CountingPopenStdin(
        process.stdin,
        process,
        wait_after_first_flush=False,
    )
    process.stdin = counter
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    return instance, process, counter


def finish_persistent_live_eof_session(instance, process):
    if process.stdin is not None:
        process.stdin.close()
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=1)
    stderr = process.stderr.read() if process.stderr is not None else ""
    reader = getattr(instance, "_stdout_reader", None)
    if reader is not None:
        reader.join(timeout=1)
    try:
        observed = json.loads(stderr.strip())
    except json.JSONDecodeError:
        observed = {"decode_error": stderr}
    return observed


def check_raw_line_persists_live_transport_eof_across_calls_and_resync() -> None:
    """EOF vivo se consulta en preflight sin nuevos writes públicos ni de barrera."""
    harness = load_harness()
    failures: list[str] = []
    public_lines = [
        '{"jsonrpc":"2.0","method":"notify/eof-one"}',
        '{"jsonrpc":"2.0","method":"notify/eof-two"}',
        '{"jsonrpc":"2.0","method":"notify/eof-three"}',
    ]
    instance, process, counter = persistent_live_eof_session(harness)
    outcomes = []
    try:
        for index, line in enumerate(public_lines, start=1):
            started = time.monotonic()
            try:
                value = instance.raw_line(line, timeout=0.5)
                outcome = ("return", value)
            except Exception as error:
                outcome = ("error", error)
            elapsed = time.monotonic() - started
            outcomes.append((outcome, elapsed, process.poll()))
    finally:
        observed = finish_persistent_live_eof_session(instance, process)

    if observed.get("lines") != public_lines[:1]:
        failures.append(f"raw: hijo recibió writes posteriores al EOF: {observed!r}")
    if counter.write_attempts != [public_lines[0] + "\n"]:
        failures.append(
            f"raw: write count creció tras conocer EOF vivo: {counter.write_attempts!r}"
        )
    for index, (outcome, elapsed, returncode) in enumerate(outcomes, start=1):
        prefix = f"raw/call-{index}"
        if outcome != ("return", {"server_exited": None}):
            failures.append(
                f"{prefix}: EOF vivo no fue estable: outcome={outcome!r}"
            )
        if elapsed > STRICT_LIMIT:
            failures.append(
                f"{prefix}: EOF persistente consumió timeout: {elapsed:.3f}s"
            )
        if returncode is not None:
            failures.append(f"{prefix}: hijo no seguía vivo leyendo stdin: exit={returncode}")

    resync_public = '{"jsonrpc":"2.0","method":"notify/must-not-cross-eof"}'
    resync_instance, resync_process, resync_counter = persistent_live_eof_session(
        harness
    )
    resync_instance._raw_pending_idless = 1
    resync_instance._raw_needs_resync = True
    resync_instance._raw_backlog = []
    resync_instance._raw_pending_ids = []
    resync_instance._raw_reserved_ids = []
    resync_outcomes = []
    try:
        for index in range(2):
            started = time.monotonic()
            try:
                value = resync_instance.raw_line(resync_public, timeout=0.5)
                outcome = ("return", value)
            except Exception as error:
                outcome = ("error", error)
            elapsed = time.monotonic() - started
            resync_outcomes.append((outcome, elapsed, resync_process.poll()))
    finally:
        resync_observed = finish_persistent_live_eof_session(
            resync_instance, resync_process
        )

    barrier_lines = resync_observed.get("lines", [])
    if len(barrier_lines) != 1:
        failures.append(
            f"resync: se escribió después de conocer EOF vivo: {resync_observed!r}"
        )
    else:
        barrier = json.loads(barrier_lines[0])
        if (
            barrier.get("jsonrpc") != "2.0"
            or barrier.get("method") != "ping"
            or not isinstance(barrier.get("id"), str)
        ):
            failures.append(f"resync: primera línea no era barrera: {barrier!r}")
        if resync_public in barrier_lines:
            failures.append("resync: operación pública cruzó el EOF de la barrera")
    if len(resync_counter.write_attempts) != 1:
        failures.append(
            "resync: segunda llamada escribió otra barrera/public tras EOF: "
            f"{resync_counter.write_attempts!r}"
        )
    for index, (outcome, elapsed, returncode) in enumerate(resync_outcomes, start=1):
        prefix = f"resync/call-{index}"
        if outcome[0] != "error" or type(outcome[1]) is not RuntimeError:
            failures.append(f"{prefix}: EOF no produjo RuntimeError: {outcome!r}")
        else:
            message = str(outcome[1]).lower()
            if "resync" not in message or "eof" not in message:
                failures.append(f"{prefix}: error no identifica resync EOF: {outcome[1]!r}")
        if elapsed > STRICT_LIMIT:
            failures.append(f"{prefix}: EOF persistente consumió timeout: {elapsed:.3f}s")
        if returncode is not None:
            failures.append(f"{prefix}: hijo no seguía vivo leyendo stdin: exit={returncode}")

    assert not failures, "EOF raw/resync no persistió:\n- " + "\n- ".join(failures)


def check_rpc_persists_live_transport_eof_across_repeated_calls() -> None:
    """RPC/_read_response consultan EOF vivo sin nuevos writes ni bloquear stderr."""
    harness = load_harness()
    instance, process, counter = persistent_live_eof_session(harness)
    instance._next_id = 41
    outcomes = []

    def capture(label, expected_id, operation) -> None:
        started = time.monotonic()
        try:
            value = operation()
            outcome = ("return", value)
        except Exception as error:
            outcome = ("error", error)
        outcomes.append(
            (label, expected_id, outcome, time.monotonic() - started, process.poll())
        )

    try:
        capture("rpc-first", 41, lambda: instance.rpc("persistent/eof-1"))
        capture("read-response-42", 42, lambda: instance._read_response(42, timeout=0.5))
        capture("read-response-43", 43, lambda: instance._read_response(43, timeout=0.5))
        capture("rpc-repeated", 42, lambda: instance.rpc("persistent/must-not-write"))
    finally:
        observed = finish_persistent_live_eof_session(instance, process)

    failures: list[str] = []
    written_lines = observed.get("lines", [])
    if len(written_lines) != 1:
        failures.append(f"hijo recibió rpc posteriores al EOF: {observed!r}")
    else:
        request = json.loads(written_lines[0])
        if (
            request.get("jsonrpc") != "2.0"
            or request.get("id") != 41
            or request.get("method") != "persistent/eof-1"
        ):
            failures.append(f"primera línea rpc no acreditada: {request!r}")
    if len(counter.write_attempts) != 1:
        failures.append(
            f"rpc escribió después de conocer EOF vivo: {counter.write_attempts!r}"
        )
    for label, expected_id, outcome, elapsed, returncode in outcomes:
        prefix = f"rpc/{label}"
        if outcome[0] != "error" or type(outcome[1]) is not RuntimeError:
            failures.append(f"{prefix}: EOF devolvió valor normal: {outcome!r}")
        else:
            message = str(outcome[1])
            normalized = message.lower()
            if "eof" not in normalized or str(expected_id) not in message:
                failures.append(f"{prefix}: error perdió EOF/id: {message!r}")
            if "exit=none" not in normalized or "stderr=" not in normalized:
                failures.append(
                    f"{prefix}: error vivo debe conservar exit=None/stderr no bloqueante: "
                    f"{message!r}"
                )
        if elapsed > STRICT_LIMIT:
            failures.append(f"{prefix}: EOF persistente consumió timeout: {elapsed:.3f}s")
        if returncode is not None:
            failures.append(f"{prefix}: hijo no seguía vivo leyendo stdin: exit={returncode}")

    assert not failures, "EOF rpc no persistió:\n- " + "\n- ".join(failures)


def terminal_eof_session(harness, mode: str, marker: str, exit_code: int):
    process = subprocess.Popen(
        [
            sys.executable,
            "-u",
            "-c",
            TERMINAL_EOF_CHILD,
            mode,
            marker,
            str(exit_code),
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    if mode == "already-exited":
        process.wait(timeout=1)
    counter = CountingPopenStdin(
        process.stdin,
        process,
        wait_after_first_flush=mode == "after-first",
    )
    process.stdin = counter
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    return instance, process, counter


def finish_terminal_eof_session(instance, process, counter) -> str:
    try:
        counter.close()
    except (BrokenPipeError, OSError, ValueError):
        pass
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=1)
    stderr = process.stderr.read() if process.stderr is not None else ""
    reader = getattr(instance, "_stdout_reader", None)
    if reader is not None:
        reader.join(timeout=1)
    return stderr


def check_raw_line_terminal_eof_preflight_is_stable_and_never_rewrites() -> None:
    """Tras conocer EOF+exit, raw_line reutiliza el terminal sin tocar stdin."""
    harness = load_harness()
    marker = "TERMINAL_RAW_STDERR_17"
    first_line = '{"jsonrpc":"2.0","method":"notify/terminal-first"}'
    repeated_lines = [
        '{"jsonrpc":"2.0","method":"notify/terminal-second"}',
        '{"jsonrpc":"2.0","method":"notify/terminal-third"}',
    ]
    instance, process, counter = terminal_eof_session(
        harness, "after-first", marker, 17
    )
    outcomes = []
    try:
        for line in [first_line, *repeated_lines]:
            started = time.monotonic()
            try:
                value = instance.raw_line(line, timeout=0.5)
                outcome = ("return", value)
            except Exception as error:
                outcome = ("error", error)
            outcomes.append((outcome, time.monotonic() - started))
    finally:
        stderr = finish_terminal_eof_session(instance, process, counter)

    failures: list[str] = []
    expected_terminal = {"server_exited": 17}
    for index, (outcome, elapsed) in enumerate(outcomes, start=1):
        if outcome != ("return", expected_terminal):
            failures.append(
                f"raw/call-{index}: terminal no fue estable {expected_terminal!r}: {outcome!r}"
            )
        if elapsed > STRICT_LIMIT:
            failures.append(f"raw/call-{index}: preflight terminal tardó {elapsed:.3f}s")
    if counter.write_attempts != [first_line + "\n"]:
        failures.append(
            "raw: write count creció después de conocer terminal: "
            f"{counter.write_attempts!r}"
        )
    if marker not in stderr or f"observed={first_line}" not in stderr:
        failures.append(f"raw: hijo no acreditó primera línea/marker: {stderr!r}")

    race_marker = "TERMINAL_RAW_ALREADY_EXITED_17"
    race_line = '{"jsonrpc":"2.0","method":"notify/already-exited"}'
    race_instance, race_process, race_counter = terminal_eof_session(
        harness, "already-exited", race_marker, 17
    )
    started = time.monotonic()
    try:
        try:
            race_value = race_instance.raw_line(race_line, timeout=0.5)
            race_outcome = ("return", race_value)
        except Exception as error:
            race_outcome = ("error", error)
        race_elapsed = time.monotonic() - started
    finally:
        race_stderr = finish_terminal_eof_session(
            race_instance, race_process, race_counter
        )
    if race_outcome != ("return", expected_terminal):
        failures.append(f"raw/already-exited: no devolvió terminal: {race_outcome!r}")
    if race_elapsed > STRICT_LIMIT:
        failures.append(f"raw/already-exited: preflight tardó {race_elapsed:.3f}s")
    if race_counter.write_attempts:
        failures.append(
            f"raw/already-exited: intentó escribir en proceso muerto: {race_counter.write_attempts!r}"
        )
    if race_marker not in race_stderr or "observed=<none>" not in race_stderr:
        failures.append(f"raw/already-exited: hijo no acreditado: {race_stderr!r}")

    assert not failures, "preflight raw terminal defectuoso:\n- " + "\n- ".join(failures)


def check_rpc_terminal_eof_preflight_caches_diagnostic_and_never_rewrites() -> None:
    """RPC y _read_response repiten EOF/id/exit/stderr sin releer ni escribir."""
    harness = load_harness()
    marker = "TERMINAL_RPC_STDERR_23"
    instance, process, counter = terminal_eof_session(
        harness, "after-first", marker, 23
    )
    instance._next_id = 71
    outcomes = []

    def capture(label, expected_id, operation) -> None:
        started = time.monotonic()
        try:
            value = operation()
            outcome = ("return", value)
        except Exception as error:
            outcome = ("error", error)
        outcomes.append((label, expected_id, outcome, time.monotonic() - started))

    try:
        capture("rpc-first", 71, lambda: instance.rpc("terminal/first"))
        capture("read-response-72", 72, lambda: instance._read_response(72, timeout=0.5))
        capture("read-response-73", 73, lambda: instance._read_response(73, timeout=0.5))
        capture("rpc-repeated", 72, lambda: instance.rpc("terminal/must-not-write"))
    finally:
        stderr_tail = finish_terminal_eof_session(instance, process, counter)

    failures: list[str] = []
    for label, expected_id, outcome, elapsed in outcomes:
        if outcome[0] != "error" or type(outcome[1]) is not RuntimeError:
            failures.append(f"{label}: terminal EOF no produjo RuntimeError: {outcome!r}")
        else:
            message = str(outcome[1])
            normalized = message.lower()
            if "eof" not in normalized or str(expected_id) not in message:
                failures.append(f"{label}: diagnóstico perdió EOF/id={expected_id}: {message!r}")
            if "exit=23" not in normalized and "returncode=23" not in normalized:
                failures.append(f"{label}: diagnóstico perdió exit 23: {message!r}")
            if marker not in message:
                failures.append(f"{label}: diagnóstico stderr no fue estable: {message!r}")
        if elapsed > STRICT_LIMIT:
            failures.append(f"{label}: preflight terminal tardó {elapsed:.3f}s")

    if len(counter.write_attempts) != 1:
        failures.append(
            "rpc: write count creció después de conocer terminal: "
            f"{counter.write_attempts!r}"
        )
    else:
        first_request = json.loads(counter.write_attempts[0])
        if (
            first_request.get("jsonrpc") != "2.0"
            or first_request.get("id") != 71
            or first_request.get("method") != "terminal/first"
        ):
            failures.append(f"rpc: primera línea pública no acreditada: {first_request!r}")
    first_line = counter.write_attempts[0].rstrip("\n") if counter.write_attempts else ""
    first_message = str(outcomes[0][2][1]) if outcomes and outcomes[0][2][0] == "error" else ""
    if f"observed={first_line}" not in first_message:
        failures.append(f"rpc: hijo no acreditó primera línea en stderr: {first_message!r}")
    if stderr_tail:
        failures.append(
            f"rpc: stderr terminal no se capturó de forma única/canónica: {stderr_tail!r}"
        )

    assert not failures, "preflight rpc terminal defectuoso:\n- " + "\n- ".join(failures)


def queued_response_terminal_session(harness, marker: str):
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", QUEUED_RESPONSE_THEN_EXIT_CHILD, marker],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    observed_queue = ObservingQueue()
    instance._stdout_queue = observed_queue
    counter = CountingPopenStdin(
        process.stdin,
        process,
        wait_after_first_flush=True,
        queued_frame_event=observed_queue.frame_queued,
    )
    process.stdin = counter
    instance._next_id = 5
    instance._ensure_stdout_reader()
    return instance, process, counter, observed_queue


def finish_queued_response_terminal_session(instance, process, counter) -> str:
    try:
        counter.close()
    except (BrokenPipeError, OSError, ValueError):
        pass
    process.wait(timeout=1)
    read_ack = getattr(instance, "_stdout_read_ack", None)
    if read_ack is not None:
        read_ack.set()
    reader = getattr(instance, "_stdout_reader", None)
    if reader is not None:
        reader.join(timeout=1)
    return process.stderr.read() if process.stderr is not None else ""


def queued_text_frames(instance):
    stdout_queue = instance._stdout_queue
    with stdout_queue.mutex:
        return [item for item in list(stdout_queue.queue) if isinstance(item, str)]


def check_raw_line_drains_queued_response_before_terminal_preflight() -> None:
    """Una respuesta id5 ya en cola gana al exit0 observado en la misma llamada raw."""
    harness = load_harness()
    marker = "RAW_RESPONSE_QUEUED_BEFORE_EXIT"
    request_line = '{"jsonrpc":"2.0","id":5,"method":"queue/raw"}'
    follow_lines = [
        '{"jsonrpc":"2.0","method":"notify/terminal-after-response-1"}',
        '{"jsonrpc":"2.0","method":"notify/terminal-after-response-2"}',
    ]
    expected_response = {
        "jsonrpc": "2.0",
        "id": 5,
        "result": {"marker": marker},
    }
    instance, process, counter, observed_queue = queued_response_terminal_session(
        harness, marker
    )
    try:
        first = instance.raw_line(request_line, timeout=0.5)
        poll_after_first = process.poll()
        follows = []
        for line in follow_lines:
            started = time.monotonic()
            try:
                value = instance.raw_line(line, timeout=0.5)
                outcome = ("return", value)
            except Exception as error:
                outcome = ("error", error)
            follows.append((outcome, time.monotonic() - started))
        queued_after_calls = queued_text_frames(instance)
        backlog_after_calls = list(instance._raw_backlog)
    finally:
        stderr = finish_queued_response_terminal_session(instance, process, counter)

    failures: list[str] = []
    if not observed_queue.frame_queued.is_set() or poll_after_first != 0:
        failures.append(
            "raw: guarda no coordinó respuesta en cola antes de poll exit0: "
            f"queued={observed_queue.frame_queued.is_set()} poll={poll_after_first!r}"
        )
    if first != expected_response:
        failures.append(
            f"raw: terminal adelantó a respuesta correlacionada en cola: {first!r}"
        )
    for index, (outcome, elapsed) in enumerate(follows, start=1):
        if outcome != ("return", {"server_exited": 0}):
            failures.append(f"raw/follow-{index}: terminal no fue estable: {outcome!r}")
        if elapsed > STRICT_LIMIT:
            failures.append(f"raw/follow-{index}: terminal tardó {elapsed:.3f}s")
    if counter.write_attempts != [request_line + "\n"]:
        failures.append(
            f"raw: escribió después del terminal conocido: {counter.write_attempts!r}"
        )
    if queued_after_calls:
        failures.append(f"raw: respuesta quedó abandonada en cola FIFO: {queued_after_calls!r}")
    if backlog_after_calls:
        failures.append(f"raw: backlog terminal no quedó drenado: {backlog_after_calls!r}")
    if marker not in stderr or f"observed={request_line}" not in stderr:
        failures.append(f"raw: stderr/primera línea no acreditados: {stderr!r}")

    assert not failures, "respuesta raw en cola perdió contra terminal:\n- " + "\n- ".join(
        failures
    )


def check_rpc_drains_queued_response_before_terminal_preflight() -> None:
    """RPC id5 devuelve su respuesta en cola antes de cachear exit0/stderr terminal."""
    harness = load_harness()
    marker = "RPC_RESPONSE_QUEUED_BEFORE_EXIT"
    expected_response = {
        "jsonrpc": "2.0",
        "id": 5,
        "result": {"marker": marker},
    }
    instance, process, counter, observed_queue = queued_response_terminal_session(
        harness, marker
    )
    first_outcome = None
    terminal_outcomes = []

    def capture(label, expected_id, operation) -> None:
        started = time.monotonic()
        try:
            value = operation()
            outcome = ("return", value)
        except Exception as error:
            outcome = ("error", error)
        terminal_outcomes.append(
            (label, expected_id, outcome, time.monotonic() - started)
        )

    try:
        try:
            first_outcome = ("return", instance.rpc("queue/rpc"))
        except Exception as error:
            first_outcome = ("error", error)
        poll_after_first = process.poll()
        capture("rpc-follow", 6, lambda: instance.rpc("terminal/must-not-write"))
        capture("read-response-follow", 7, lambda: instance._read_response(7, timeout=0.5))
        queued_after_calls = queued_text_frames(instance)
        backlog_after_calls = list(instance._raw_backlog)
    finally:
        stderr_tail = finish_queued_response_terminal_session(instance, process, counter)

    failures: list[str] = []
    if not observed_queue.frame_queued.is_set() or poll_after_first != 0:
        failures.append(
            "rpc: guarda no coordinó respuesta en cola antes de poll exit0: "
            f"queued={observed_queue.frame_queued.is_set()} poll={poll_after_first!r}"
        )
    if first_outcome != ("return", expected_response):
        failures.append(
            f"rpc: terminal adelantó a respuesta correlacionada en cola: {first_outcome!r}"
        )
    for label, expected_id, outcome, elapsed in terminal_outcomes:
        if outcome[0] != "error" or type(outcome[1]) is not RuntimeError:
            failures.append(f"rpc/{label}: terminal no produjo RuntimeError: {outcome!r}")
        else:
            message = str(outcome[1])
            normalized = message.lower()
            if "eof" not in normalized or str(expected_id) not in message:
                failures.append(f"rpc/{label}: perdió EOF/id={expected_id}: {message!r}")
            if "exit=0" not in normalized or marker not in message:
                failures.append(f"rpc/{label}: perdió exit0/stderr cacheado: {message!r}")
        if elapsed > STRICT_LIMIT:
            failures.append(f"rpc/{label}: terminal tardó {elapsed:.3f}s")
    if len(counter.write_attempts) != 1:
        failures.append(
            f"rpc: escribió después del terminal conocido: {counter.write_attempts!r}"
        )
    else:
        request = json.loads(counter.write_attempts[0])
        if (
            request.get("jsonrpc") != "2.0"
            or request.get("id") != 5
            or request.get("method") != "queue/rpc"
        ):
            failures.append(f"rpc: primera petición id5 no acreditada: {request!r}")
    if queued_after_calls:
        failures.append(f"rpc: respuesta quedó abandonada en cola FIFO: {queued_after_calls!r}")
    if backlog_after_calls:
        failures.append(f"rpc: backlog terminal no quedó drenado: {backlog_after_calls!r}")
    if stderr_tail:
        failures.append(f"rpc: stderr cacheado se releyó/desdobló: {stderr_tail!r}")

    assert not failures, "respuesta rpc en cola perdió contra terminal:\n- " + "\n- ".join(
        failures
    )


def correlated_then_foreign_terminal_session(harness, marker: str):
    """Coordina dos frames escritos antes de exit0, con backpressure entre ambos."""
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", CORRELATED_THEN_FOREIGN_EXIT_CHILD, marker],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    observed_queue = ObservingQueue()
    instance._stdout_queue = observed_queue
    counter = CountingPopenStdin(
        process.stdin,
        process,
        wait_after_first_flush=True,
        queued_frame_event=observed_queue.frame_queued,
    )
    process.stdin = counter
    instance._next_id = 5
    instance._ensure_stdout_reader()
    return instance, process, counter, observed_queue


def finish_correlated_then_foreign_terminal_session(instance, process, counter):
    """Libera cualquier ACK restante sólo para no dejar el reader del test vivo."""
    try:
        counter.close()
    except (BrokenPipeError, OSError, ValueError):
        pass
    process.wait(timeout=1)
    reader = getattr(instance, "_stdout_reader", None)
    read_ack = getattr(instance, "_stdout_read_ack", None)
    if reader is not None:
        for _ in range(3):
            if not reader.is_alive():
                break
            if read_ack is not None:
                read_ack.set()
            reader.join(timeout=STRICT_LIMIT)
    stderr = process.stderr.read() if process.stderr is not None else ""
    return stderr, reader is not None and reader.is_alive()


def run_correlated_then_foreign_before_terminal(operation: str) -> None:
    """El frame ajeno posterior a la respuesta debe ganar a terminal, sin otro write."""
    harness = load_harness()
    marker = f"FOREIGN_AFTER_{operation.upper()}_CORRELATED"
    raw_request = '{"jsonrpc":"2.0","id":5,"method":"queue/raw-two-frames"}'
    observation_lines = (
        '{"jsonrpc":"2.0","method":"notify/observe-foreign"}',
        '{"jsonrpc":"2.0","method":"notify/observe-terminal"}',
    )
    expected_correlated = {
        "jsonrpc": "2.0",
        "id": 5,
        "result": {"marker": "CORRELATED_BEFORE_FOREIGN"},
    }
    expected_foreign = {
        "jsonrpc": "2.0",
        "id": 77,
        "method": "server/foreign-after-correlated",
        "params": {"marker": marker},
    }
    instance, process, counter, observed_queue = (
        correlated_then_foreign_terminal_session(harness, marker)
    )
    first_outcome = None
    foreign_outcome = None
    terminal_outcome = None
    foreign_elapsed = 0.0
    terminal_elapsed = 0.0
    queued_after_calls = []
    backlog_after_calls = []
    reader_alive_before_cleanup = True
    waiting_ack_before_cleanup = True
    eof_seen_before_cleanup = False
    frame_count_before_cleanup = 0
    try:
        try:
            if operation == "raw":
                first_outcome = ("return", instance.raw_line(raw_request, timeout=0.5))
            else:
                first_outcome = ("return", instance.rpc("queue/rpc-two-frames"))
        except Exception as error:
            first_outcome = ("error", error)
        poll_after_first = process.poll()

        started = time.monotonic()
        try:
            foreign_outcome = (
                "return",
                instance.raw_line(observation_lines[0], timeout=0.5),
            )
        except Exception as error:
            foreign_outcome = ("error", error)
        foreign_elapsed = time.monotonic() - started

        started = time.monotonic()
        try:
            terminal_outcome = (
                "return",
                instance.raw_line(observation_lines[1], timeout=0.5),
            )
        except Exception as error:
            terminal_outcome = ("error", error)
        terminal_elapsed = time.monotonic() - started

        queued_after_calls = queued_text_frames(instance)
        backlog_after_calls = list(instance._raw_backlog)
        reader = getattr(instance, "_stdout_reader", None)
        if reader is not None:
            reader.join(timeout=STRICT_LIMIT)
            reader_alive_before_cleanup = reader.is_alive()
        waiting_ack_before_cleanup = getattr(instance, "_stdout_waiting_ack", False)
        eof_seen = getattr(instance, "_stdout_eof_seen", None)
        eof_seen_before_cleanup = eof_seen is not None and eof_seen.is_set()
        frame_count_before_cleanup = observed_queue.frame_count
        writes = list(counter.write_attempts)
    finally:
        stderr, reader_alive_after_cleanup = (
            finish_correlated_then_foreign_terminal_session(instance, process, counter)
        )

    failures: list[str] = []
    prefix = operation
    if not observed_queue.frame_queued.is_set() or poll_after_first != 0:
        failures.append(
            f"{prefix}: guarda no fijó primer frame en cola + exit0: "
            f"queued={observed_queue.frame_queued.is_set()} poll={poll_after_first!r}"
        )
    if first_outcome != ("return", expected_correlated):
        failures.append(
            f"{prefix}: primera operación no devolvió su respuesta correlacionada: "
            f"{first_outcome!r}"
        )
    if foreign_outcome != ("return", expected_foreign):
        failures.append(
            f"{prefix}: terminal ocultó el frame ajeno FIFO: {foreign_outcome!r}"
        )
    if terminal_outcome != ("return", {"server_exited": 0}):
        failures.append(
            f"{prefix}: terminal posterior al frame ajeno no fue estable: "
            f"{terminal_outcome!r}"
        )
    if foreign_elapsed > STRICT_LIMIT or terminal_elapsed > STRICT_LIMIT:
        failures.append(
            f"{prefix}: observación no fue inmediata: "
            f"foreign={foreign_elapsed:.3f}s terminal={terminal_elapsed:.3f}s"
        )
    if len(writes) != 1:
        failures.append(
            f"{prefix}: observar frame/terminal causó writes públicos: {writes!r}"
        )
        sent_line = ""
    else:
        sent_line = writes[0].rstrip("\n")
        sent = json.loads(sent_line)
        expected_method = (
            "queue/raw-two-frames" if operation == "raw" else "queue/rpc-two-frames"
        )
        if (
            sent.get("jsonrpc") != "2.0"
            or sent.get("id") != 5
            or sent.get("method") != expected_method
        ):
            failures.append(f"{prefix}: primera línea/id no acreditados: {sent!r}")
    if frame_count_before_cleanup < 2:
        failures.append(
            f"{prefix}: reader no drenó el segundo frame antes del terminal: "
            f"frames={frame_count_before_cleanup}"
        )
    if queued_after_calls:
        failures.append(f"{prefix}: cola textual no quedó FIFO/drenada: {queued_after_calls!r}")
    if backlog_after_calls:
        failures.append(f"{prefix}: backlog no quedó drenado: {backlog_after_calls!r}")
    if reader_alive_before_cleanup or waiting_ack_before_cleanup:
        failures.append(
            f"{prefix}: reader quedó bloqueado esperando ACK: "
            f"alive={reader_alive_before_cleanup} waiting_ack={waiting_ack_before_cleanup}"
        )
    if not eof_seen_before_cleanup:
        failures.append(f"{prefix}: reader no publicó EOF después de drenar ambos frames")
    if reader_alive_after_cleanup:
        failures.append(f"{prefix}: cleanup acotado no pudo cerrar el reader")
    try:
        proof = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        failures.append(f"{prefix}: stderr del hijo no fue evidencia JSON: {stderr!r}; {error}")
    else:
        if (
            proof.get("line") != sent_line
            or proof.get("frames") != [expected_correlated, expected_foreign]
            or proof.get("marker") != marker
        ):
            failures.append(f"{prefix}: hijo no acreditó línea/orden de frames: {proof!r}")

    assert not failures, "correlated+foreign perdió contra terminal:\n- " + "\n- ".join(
        failures
    )


def check_raw_line_preserves_foreign_after_correlated_before_terminal() -> None:
    run_correlated_then_foreign_before_terminal("raw")


def check_rpc_preserves_foreign_after_correlated_before_terminal() -> None:
    run_correlated_then_foreign_before_terminal("rpc")


def correlated_three_pre_eof_frames_session(harness, marker: str):
    process = subprocess.Popen(
        [
            sys.executable,
            "-u",
            "-c",
            CORRELATED_THREE_PRE_EOF_FRAMES_CHILD,
            marker,
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    observed_queue = ObservingQueue()
    instance._stdout_queue = observed_queue
    counter = CountingPopenStdin(
        process.stdin,
        process,
        wait_after_first_flush=True,
        queued_frame_event=observed_queue.frame_queued,
    )
    process.stdin = counter
    instance._next_id = 5
    instance._ensure_stdout_reader()
    return instance, process, counter, observed_queue


def run_terminal_drains_three_pre_eof_frames(operation: str) -> None:
    """Un solo drain terminal libera y preserva todos los frames tras la correlacionada."""
    harness = load_harness()
    marker = f"THREE_PRE_EOF_{operation.upper()}"
    first_raw = '{"jsonrpc":"2.0","id":5,"method":"multi/raw-first"}'
    probes = (
        '{"jsonrpc":"2.0","method":"notify/multi-observe-77"}',
        '{"jsonrpc":"2.0","method":"notify/multi-observe-unparseable"}',
        '{"jsonrpc":"2.0","method":"notify/multi-observe-78"}',
        '{"jsonrpc":"2.0","method":"notify/multi-observe-terminal"}',
    )
    expected_correlated = {
        "jsonrpc": "2.0",
        "id": 5,
        "result": {"marker": "CORRELATED_BEFORE_THREE"},
    }
    expected_77 = {
        "jsonrpc": "2.0",
        "id": 77,
        "result": {"marker": marker + ":77"},
    }
    invalid_raw = "NOT_JSON_PRE_EOF:" + marker
    expected_unparseable = {"unparseable_response": invalid_raw}
    expected_78 = {
        "jsonrpc": "2.0",
        "id": 78,
        "result": {"marker": marker + ":78"},
    }
    instance, process, counter, observed_queue = correlated_three_pre_eof_frames_session(
        harness, marker
    )
    first_outcome = None
    outcomes = []
    backlog_after_first_preserved = None
    queue_after_first_preserved = None
    reader_alive_after_first_preserved = True
    waiting_ack_after_first_preserved = True
    eof_after_first_preserved = False
    frame_count_after_first_preserved = 0
    queued_at_end = []
    backlog_at_end = []
    try:
        try:
            if operation == "raw":
                first_outcome = ("return", instance.raw_line(first_raw, timeout=0.5))
            else:
                first_outcome = ("return", instance.rpc("multi/rpc-first"))
        except Exception as error:
            first_outcome = ("error", error)
        poll_after_correlated = process.poll()

        for index, probe in enumerate(probes):
            started = time.monotonic()
            try:
                outcome = ("return", instance.raw_line(probe, timeout=0.5))
            except Exception as error:
                outcome = ("error", error)
            outcomes.append((outcome, time.monotonic() - started))
            if index == 0:
                backlog_after_first_preserved = list(instance._raw_backlog)
                queue_after_first_preserved = queued_text_frames(instance)
                reader = getattr(instance, "_stdout_reader", None)
                if reader is not None:
                    reader.join(timeout=STRICT_LIMIT)
                    reader_alive_after_first_preserved = reader.is_alive()
                waiting_ack_after_first_preserved = getattr(
                    instance, "_stdout_waiting_ack", False
                )
                eof_seen = getattr(instance, "_stdout_eof_seen", None)
                eof_after_first_preserved = eof_seen is not None and eof_seen.is_set()
                frame_count_after_first_preserved = observed_queue.frame_count

        writes = list(counter.write_attempts)
        queued_at_end = queued_text_frames(instance)
        backlog_at_end = list(instance._raw_backlog)
    finally:
        stderr, reader_alive_after_cleanup = (
            finish_correlated_then_foreign_terminal_session(instance, process, counter)
        )

    failures: list[str] = []
    prefix = operation
    expected_outcomes = [
        ("return", expected_77),
        ("return", expected_unparseable),
        ("return", expected_78),
        ("return", {"server_exited": 0}),
    ]
    if not observed_queue.frame_queued.is_set() or poll_after_correlated != 0:
        failures.append(
            f"{prefix}: correlacionada no quedó en cola antes de exit0: "
            f"queued={observed_queue.frame_queued.is_set()} poll={poll_after_correlated!r}"
        )
    if first_outcome != ("return", expected_correlated):
        failures.append(f"{prefix}: primera respuesta no fue correlacionada: {first_outcome!r}")
    for index, ((outcome, elapsed), expected) in enumerate(
        zip(outcomes, expected_outcomes), start=1
    ):
        if outcome != expected:
            failures.append(
                f"{prefix}/probe-{index}: FIFO pre-EOF incorrecto: "
                f"esperado={expected!r} obtenido={outcome!r}"
            )
        if elapsed > STRICT_LIMIT:
            failures.append(f"{prefix}/probe-{index}: tardó {elapsed:.3f}s")

    expected_backlog_after_first = [
        expected_unparseable,
        json.dumps(expected_78) + "\n",
    ]
    if backlog_after_first_preserved != expected_backlog_after_first:
        failures.append(
            f"{prefix}: primer drain no preservó de una vez los dos frames restantes: "
            f"{backlog_after_first_preserved!r}"
        )
    if queue_after_first_preserved:
        failures.append(
            f"{prefix}: primer drain dejó frames de texto en cola: "
            f"{queue_after_first_preserved!r}"
        )
    if (
        reader_alive_after_first_preserved
        or waiting_ack_after_first_preserved
        or not eof_after_first_preserved
        or frame_count_after_first_preserved != 4
    ):
        failures.append(
            f"{prefix}: primer drain no liberó reader hasta EOF: "
            f"alive={reader_alive_after_first_preserved} "
            f"waiting_ack={waiting_ack_after_first_preserved} "
            f"eof={eof_after_first_preserved} frames={frame_count_after_first_preserved}"
        )
    if queued_at_end or backlog_at_end:
        failures.append(
            f"{prefix}: estado terminal no quedó vacío: "
            f"queue={queued_at_end!r} backlog={backlog_at_end!r}"
        )
    if len(writes) != 1:
        failures.append(f"{prefix}: probes FIFO/terminal escribieron: {writes!r}")
        sent_line = ""
    else:
        sent_line = writes[0].rstrip("\n")
        request = json.loads(sent_line)
        expected_method = "multi/raw-first" if operation == "raw" else "multi/rpc-first"
        if (
            request.get("jsonrpc") != "2.0"
            or request.get("id") != 5
            or request.get("method") != expected_method
        ):
            failures.append(f"{prefix}: única línea pública no acreditada: {request!r}")
    expected_wire = [
        json.dumps(expected_correlated),
        json.dumps(expected_77),
        invalid_raw,
        json.dumps(expected_78),
    ]
    try:
        proof = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        failures.append(f"{prefix}: stderr del hijo no fue JSON: {stderr!r}; {error}")
    else:
        if (
            proof.get("line") != sent_line
            or proof.get("wire_frames") != expected_wire
            or proof.get("marker") != marker
        ):
            failures.append(f"{prefix}: hijo no acreditó FIFO exacto: {proof!r}")
    if reader_alive_after_cleanup:
        failures.append(f"{prefix}: cleanup acotado no cerró reader")

    assert not failures, "drain terminal no agotó todos los frames pre-EOF:\n- " + "\n- ".join(
        failures
    )


def check_raw_line_terminal_drains_three_pre_eof_frames() -> None:
    run_terminal_drains_three_pre_eof_frames("raw")


def check_rpc_terminal_drains_three_pre_eof_frames() -> None:
    run_terminal_drains_three_pre_eof_frames("rpc")


def live_correlated_then_foreign_session(harness, marker: str):
    """Deja al hijo vivo hasta que reciba la segunda request raw id6."""
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", LIVE_CORRELATED_THEN_FOREIGN_CHILD, marker],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    observed_queue = ObservingQueue()
    instance._stdout_queue = observed_queue
    counter = CountingPopenStdin(
        process.stdin,
        process,
        wait_after_first_flush=True,
        queued_frame_event=observed_queue.frame_queued,
        wait_for_process_exit=False,
    )
    process.stdin = counter
    instance._next_id = 5
    instance._ensure_stdout_reader()
    return instance, process, counter, observed_queue


def finish_live_correlated_then_foreign_session(instance, process, counter):
    try:
        counter.close()
    except (BrokenPipeError, OSError, ValueError):
        pass
    process.wait(timeout=1)
    reader = getattr(instance, "_stdout_reader", None)
    read_ack = getattr(instance, "_stdout_read_ack", None)
    if reader is not None:
        for _ in range(3):
            if not reader.is_alive():
                break
            if read_ack is not None:
                read_ack.set()
            reader.join(timeout=STRICT_LIMIT)
    stderr = process.stderr.read() if process.stderr is not None else ""
    return stderr, reader is not None and reader.is_alive()


def run_live_eof_preserves_foreign_before_terminal(operation: str) -> None:
    """En EOF, el backlog ajeno ya preservado precede al estado terminal."""
    harness = load_harness()
    marker = f"LIVE_FOREIGN_AFTER_{operation.upper()}"
    first_raw = '{"jsonrpc":"2.0","id":5,"method":"live/raw-first"}'
    second_raw = '{"jsonrpc":"2.0","id":6,"method":"live/raw-second"}'
    terminal_probe = '{"jsonrpc":"2.0","method":"notify/live-terminal-probe"}'
    expected_correlated = {
        "jsonrpc": "2.0",
        "id": 5,
        "result": {"marker": "LIVE_CORRELATED_ID5"},
    }
    expected_foreign = {
        "jsonrpc": "2.0",
        "id": 77,
        "result": {"marker": marker},
    }
    instance, process, counter, observed_queue = live_correlated_then_foreign_session(
        harness, marker
    )
    first_outcome = None
    second_outcome = None
    terminal_outcome = None
    second_elapsed = 0.0
    terminal_elapsed = 0.0
    queued_after_calls = []
    backlog_after_calls = []
    reader_alive_before_cleanup = True
    waiting_ack_before_cleanup = True
    eof_seen_before_cleanup = False
    frame_count_before_cleanup = 0
    try:
        try:
            if operation == "raw":
                first_outcome = ("return", instance.raw_line(first_raw, timeout=0.5))
            else:
                first_outcome = ("return", instance.rpc("live/rpc-first"))
        except Exception as error:
            first_outcome = ("error", error)
        poll_after_first = process.poll()

        started = time.monotonic()
        try:
            second_outcome = ("return", instance.raw_line(second_raw, timeout=0.5))
        except Exception as error:
            second_outcome = ("error", error)
        second_elapsed = time.monotonic() - started
        poll_after_second = process.wait(timeout=1)

        started = time.monotonic()
        try:
            terminal_outcome = (
                "return",
                instance.raw_line(terminal_probe, timeout=0.5),
            )
        except Exception as error:
            terminal_outcome = ("error", error)
        terminal_elapsed = time.monotonic() - started

        writes = list(counter.write_attempts)
        queued_after_calls = queued_text_frames(instance)
        backlog_after_calls = list(instance._raw_backlog)
        reader = getattr(instance, "_stdout_reader", None)
        if reader is not None:
            reader.join(timeout=STRICT_LIMIT)
            reader_alive_before_cleanup = reader.is_alive()
        waiting_ack_before_cleanup = getattr(instance, "_stdout_waiting_ack", False)
        eof_seen = getattr(instance, "_stdout_eof_seen", None)
        eof_seen_before_cleanup = eof_seen is not None and eof_seen.is_set()
        frame_count_before_cleanup = observed_queue.frame_count
    finally:
        stderr, reader_alive_after_cleanup = finish_live_correlated_then_foreign_session(
            instance, process, counter
        )

    failures: list[str] = []
    prefix = operation
    if not observed_queue.frame_queued.is_set() or poll_after_first is not None:
        failures.append(
            f"{prefix}: ventana viva no quedó coordinada tras id5: "
            f"queued={observed_queue.frame_queued.is_set()} poll={poll_after_first!r}"
        )
    if first_outcome != ("return", expected_correlated):
        failures.append(
            f"{prefix}: primera operación no devolvió id5 correlacionado: "
            f"{first_outcome!r}"
        )
    if second_outcome != ("return", expected_foreign):
        failures.append(
            f"{prefix}: EOF ganó al backlog FIFO ya preservado: {second_outcome!r}"
        )
    if terminal_outcome != ("return", {"server_exited": 0}):
        failures.append(
            f"{prefix}: tercera operación no observó terminal estable: "
            f"{terminal_outcome!r}"
        )
    if poll_after_second != 0:
        failures.append(f"{prefix}: hijo no salió tras acreditar id6: {poll_after_second!r}")
    if second_elapsed > STRICT_LIMIT or terminal_elapsed > STRICT_LIMIT:
        failures.append(
            f"{prefix}: entrega FIFO/terminal no fue inmediata: "
            f"second={second_elapsed:.3f}s terminal={terminal_elapsed:.3f}s"
        )
    if len(writes) != 2:
        failures.append(
            f"{prefix}: esperaba sólo writes id5/id6, nunca el probe terminal: {writes!r}"
        )
        sent_lines = []
    else:
        sent_lines = [line.rstrip("\n") for line in writes]
        sent = [json.loads(line) for line in sent_lines]
        expected_first_method = "live/raw-first" if operation == "raw" else "live/rpc-first"
        if [request.get("id") for request in sent] != [5, 6]:
            failures.append(f"{prefix}: orden exacto de ids escritos no fue 5,6: {sent!r}")
        if [request.get("method") for request in sent] != [
            expected_first_method,
            "live/raw-second",
        ]:
            failures.append(f"{prefix}: líneas públicas no acreditadas: {sent!r}")
    if frame_count_before_cleanup < 2:
        failures.append(
            f"{prefix}: lector no materializó correlacionada+ajena: "
            f"frames={frame_count_before_cleanup}"
        )
    if queued_after_calls:
        failures.append(f"{prefix}: cola textual no quedó drenada: {queued_after_calls!r}")
    if backlog_after_calls:
        failures.append(f"{prefix}: backlog terminal no quedó vacío: {backlog_after_calls!r}")
    if reader_alive_before_cleanup or waiting_ack_before_cleanup or not eof_seen_before_cleanup:
        failures.append(
            f"{prefix}: ciclo del reader no cerró tras EOF: "
            f"alive={reader_alive_before_cleanup} waiting_ack={waiting_ack_before_cleanup} "
            f"eof={eof_seen_before_cleanup}"
        )
    if reader_alive_after_cleanup:
        failures.append(f"{prefix}: cleanup acotado no cerró el reader")
    try:
        proof = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        failures.append(f"{prefix}: stderr del hijo no acreditó la secuencia: {stderr!r}; {error}")
    else:
        if (
            proof.get("lines") != sent_lines
            or proof.get("ids") != [5, 6]
            or proof.get("frames") != [expected_correlated, expected_foreign]
            or proof.get("marker") != marker
        ):
            failures.append(f"{prefix}: evidencia de líneas/frames no fue exacta: {proof!r}")

    assert not failures, "EOF ocultó backlog extranjero en ventana viva:\n- " + "\n- ".join(
        failures
    )


def check_raw_line_live_eof_preserves_foreign_before_terminal() -> None:
    run_live_eof_preserves_foreign_before_terminal("raw")


def check_rpc_then_raw_live_eof_preserves_foreign_before_terminal() -> None:
    run_live_eof_preserves_foreign_before_terminal("rpc")


def queued_barrier_terminal_session(harness, marker: str):
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", QUEUED_BARRIER_ACK_THEN_EXIT_CHILD, marker],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    observed_queue = ObservingQueue()
    instance._stdout_queue = observed_queue
    counter = CountingPopenStdin(
        process.stdin,
        process,
        wait_after_first_flush=True,
        queued_frame_event=observed_queue.frame_queued,
    )
    process.stdin = counter
    instance._raw_pending_idless = 1
    instance._raw_needs_resync = True
    instance._raw_backlog = []
    instance._raw_pending_ids = []
    instance._raw_reserved_ids = []
    instance._next_id = 5
    instance._ensure_stdout_reader()
    return instance, process, counter, observed_queue


def assert_clean_barrier_cohort(instance, barrier_id, prefix: str, failures: list[str]) -> None:
    if instance._raw_id_in_use(barrier_id):
        failures.append(f"{prefix}: ACK no retiró id de barrera {barrier_id!r}")
    if instance._raw_pending_ids:
        failures.append(
            f"{prefix}: pending ids terminal no quedó vacío: {instance._raw_pending_ids!r}"
        )
    if instance._raw_pending_idless != 0:
        failures.append(
            f"{prefix}: deuda idless no se cerró con ACK: {instance._raw_pending_idless!r}"
        )
    if instance._raw_reserved_ids:
        failures.append(
            f"{prefix}: reservas de barrera no quedaron limpias: {instance._raw_reserved_ids!r}"
        )
    if instance._raw_needs_resync:
        failures.append(f"{prefix}: ACK en cola no cerró needs_resync")
    if instance._raw_backlog:
        failures.append(f"{prefix}: ACK contaminó backlog: {instance._raw_backlog!r}")
    queued = queued_text_frames(instance)
    if queued:
        failures.append(f"{prefix}: ACK quedó abandonado en cola: {queued!r}")


def check_raw_line_resync_drains_queued_ack_before_terminal_public_preflight() -> None:
    """Resync consume ACK en cola; la raw pública nueva sólo observa exit0."""
    harness = load_harness()
    marker = "RAW_BARRIER_ACK_QUEUED_BEFORE_EXIT"
    public_line = '{"jsonrpc":"2.0","method":"notify/public-after-barrier"}'
    follow_line = '{"jsonrpc":"2.0","method":"notify/terminal-stable"}'
    instance, process, counter, observed_queue = queued_barrier_terminal_session(
        harness, marker
    )
    try:
        try:
            first_value = instance.raw_line(public_line, timeout=0.5)
            first_outcome = ("return", first_value)
        except Exception as error:
            first_outcome = ("error", error)
        poll_after_first = process.poll()
        state_after_first = (
            list(instance._raw_pending_ids),
            instance._raw_pending_idless,
            list(instance._raw_reserved_ids),
            instance._raw_needs_resync,
            list(instance._raw_backlog),
            queued_text_frames(instance),
        )
        started = time.monotonic()
        try:
            follow_value = instance.raw_line(follow_line, timeout=0.5)
            follow_outcome = ("return", follow_value)
        except Exception as error:
            follow_outcome = ("error", error)
        follow_elapsed = time.monotonic() - started
        writes = list(counter.write_attempts)
        if writes:
            barrier = json.loads(writes[0])
            barrier_id = barrier.get("id")
        else:
            barrier = None
            barrier_id = None
        state_before_finish = (
            list(instance._raw_pending_ids),
            instance._raw_pending_idless,
            list(instance._raw_reserved_ids),
            instance._raw_needs_resync,
            list(instance._raw_backlog),
            queued_text_frames(instance),
        )
    finally:
        stderr = finish_queued_response_terminal_session(instance, process, counter)

    failures: list[str] = []
    if not observed_queue.frame_queued.is_set() or poll_after_first != 0:
        failures.append(
            "raw/resync: guarda no fijó ACK en cola + exit0: "
            f"queued={observed_queue.frame_queued.is_set()} poll={poll_after_first!r}"
        )
    if (
        not isinstance(barrier, dict)
        or barrier.get("jsonrpc") != "2.0"
        or barrier.get("method") != "ping"
        or not isinstance(barrier_id, str)
    ):
        failures.append(f"raw/resync: primera línea no fue barrera: {barrier!r}")
    if len(writes) != 1:
        failures.append(f"raw/resync: public/follow cruzó terminal: {writes!r}")
    if first_outcome != ("return", {"server_exited": 0}):
        failures.append(
            f"raw/resync: operación pública no observó terminal nuevo: {first_outcome!r}"
        )
    if follow_outcome != ("return", {"server_exited": 0}):
        failures.append(f"raw/resync: terminal posterior no fue estable: {follow_outcome!r}")
    if follow_elapsed > STRICT_LIMIT:
        failures.append(f"raw/resync: terminal posterior tardó {follow_elapsed:.3f}s")
    if barrier_id is not None:
        assert_clean_barrier_cohort(instance, barrier_id, "raw/resync", failures)
    expected_state = ([], 0, [], False, [], [])
    if state_after_first != expected_state:
        failures.append(
            f"raw/resync: cohorte sucia justo después de la llamada pública: "
            f"{state_after_first!r}"
        )
    if state_before_finish != expected_state:
        failures.append(
            f"raw/resync: estado terminal/cohorte inexacto: {state_before_finish!r}"
        )
    barrier_raw = writes[0].rstrip("\n") if writes else ""
    if marker not in stderr or f"observed={barrier_raw}" not in stderr:
        failures.append(f"raw/resync: stderr no acreditó barrera: {stderr!r}")

    assert not failures, "ACK raw de barrera perdió contra terminal:\n- " + "\n- ".join(
        failures
    )


def check_rpc_resync_drains_queued_ack_before_terminal_public_preflight() -> None:
    """Resync consume ACK; el rpc público id5 falla después con EOF/stderr estable."""
    harness = load_harness()
    marker = "RPC_BARRIER_ACK_QUEUED_BEFORE_EXIT"
    instance, process, counter, observed_queue = queued_barrier_terminal_session(
        harness, marker
    )
    try:
        try:
            value = instance.rpc("public/after-barrier")
            first_outcome = ("return", value)
        except Exception as error:
            first_outcome = ("error", error)
        poll_after_first = process.poll()
        state_after_first = (
            list(instance._raw_pending_ids),
            instance._raw_pending_idless,
            list(instance._raw_reserved_ids),
            instance._raw_needs_resync,
            list(instance._raw_backlog),
            queued_text_frames(instance),
        )
        started = time.monotonic()
        try:
            value = instance._read_response(6, timeout=0.5)
            follow_outcome = ("return", value)
        except Exception as error:
            follow_outcome = ("error", error)
        follow_elapsed = time.monotonic() - started
        writes = list(counter.write_attempts)
        if writes:
            barrier = json.loads(writes[0])
            barrier_id = barrier.get("id")
        else:
            barrier = None
            barrier_id = None
        state_before_finish = (
            list(instance._raw_pending_ids),
            instance._raw_pending_idless,
            list(instance._raw_reserved_ids),
            instance._raw_needs_resync,
            list(instance._raw_backlog),
            queued_text_frames(instance),
        )
    finally:
        stderr_tail = finish_queued_response_terminal_session(instance, process, counter)

    failures: list[str] = []
    if not observed_queue.frame_queued.is_set() or poll_after_first != 0:
        failures.append(
            "rpc/resync: guarda no fijó ACK en cola + exit0: "
            f"queued={observed_queue.frame_queued.is_set()} poll={poll_after_first!r}"
        )
    if (
        not isinstance(barrier, dict)
        or barrier.get("jsonrpc") != "2.0"
        or barrier.get("method") != "ping"
        or not isinstance(barrier_id, str)
    ):
        failures.append(f"rpc/resync: primera línea no fue barrera: {barrier!r}")
    if len(writes) != 1:
        failures.append(f"rpc/resync: rpc público cruzó terminal: {writes!r}")
    for label, expected_id, outcome in (
        ("public", 5, first_outcome),
        ("follow", 6, follow_outcome),
    ):
        if outcome[0] != "error" or type(outcome[1]) is not RuntimeError:
            failures.append(f"rpc/resync/{label}: no produjo RuntimeError EOF: {outcome!r}")
        else:
            message = str(outcome[1])
            normalized = message.lower()
            if "eof" not in normalized or str(expected_id) not in message:
                failures.append(
                    f"rpc/resync/{label}: error perdió EOF/id={expected_id}: {message!r}"
                )
            if normalized.startswith("resync "):
                failures.append(
                    f"rpc/resync/{label}: ACK no separó la operación pública: {message!r}"
                )
            if "exit=0" not in normalized or marker not in message:
                failures.append(
                    f"rpc/resync/{label}: perdió exit0/stderr estable: {message!r}"
                )
    if follow_elapsed > STRICT_LIMIT:
        failures.append(f"rpc/resync: EOF posterior tardó {follow_elapsed:.3f}s")
    if barrier_id is not None:
        assert_clean_barrier_cohort(instance, barrier_id, "rpc/resync", failures)
    expected_state = ([], 0, [], False, [], [])
    if state_after_first != expected_state:
        failures.append(
            f"rpc/resync: cohorte sucia justo después de la llamada pública: "
            f"{state_after_first!r}"
        )
    if state_before_finish != expected_state:
        failures.append(
            f"rpc/resync: estado terminal/cohorte inexacto: {state_before_finish!r}"
        )
    if stderr_tail:
        failures.append(f"rpc/resync: stderr estable se releyó: {stderr_tail!r}")

    assert not failures, "ACK rpc de barrera perdió contra terminal:\n- " + "\n- ".join(
        failures
    )


def flush_error_barrier_session(harness, marker: str, error_type, ack_then_exit: bool):
    child = (
        QUEUED_BARRIER_ACK_THEN_EXIT_CHILD
        if ack_then_exit
        else NO_ACK_LIVE_AFTER_BARRIER_CHILD
    )
    argv = [sys.executable, "-u", "-c", child]
    if ack_then_exit:
        argv.append(marker)
    process = subprocess.Popen(
        argv,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    observed_queue = ObservingQueue()
    instance._stdout_queue = observed_queue
    counter = CountingPopenStdin(
        process.stdin,
        process,
        wait_after_first_flush=ack_then_exit,
        queued_frame_event=observed_queue.frame_queued if ack_then_exit else None,
        flush_error_type=error_type,
        flush_error_marker=marker,
    )
    process.stdin = counter
    instance._raw_pending_idless = 1
    instance._raw_needs_resync = True
    instance._raw_backlog = []
    instance._raw_pending_ids = []
    instance._raw_reserved_ids = []
    instance._next_id = 5
    if ack_then_exit:
        instance._ensure_stdout_reader()
    return instance, process, counter, observed_queue


def finish_live_no_ack_flush_session(instance, process, counter):
    try:
        counter.close()
    except (BrokenPipeError, OSError, ValueError):
        pass
    process.wait(timeout=1)
    read_ack = getattr(instance, "_stdout_read_ack", None)
    if read_ack is not None:
        read_ack.set()
    reader = getattr(instance, "_stdout_reader", None)
    if reader is not None:
        reader.join(timeout=1)
    stderr = process.stderr.read() if process.stderr is not None else ""
    try:
        return json.loads(stderr.strip())
    except json.JSONDecodeError:
        return {"decode_error": stderr}


def assert_live_no_ack_flush_control(
    harness, operation: str, label: str, error_type, failures: list[str]
) -> None:
    marker = f"NO_ACK_LIVE_FLUSH_{operation.upper()}_{label.upper()}"
    instance, process, counter, observed_queue = flush_error_barrier_session(
        harness, marker, error_type, ack_then_exit=False
    )
    public_raw = '{"jsonrpc":"2.0","method":"notify/no-ack-must-not-send"}'
    caught = None
    try:
        try:
            if operation == "raw":
                instance.raw_line(public_raw, timeout=0.5)
            else:
                instance.rpc("no-ack/must-not-send")
        except Exception as error:
            caught = error
        alive_after_error = process.poll() is None
        writes = list(counter.write_attempts)
        pending = list(instance._raw_pending_ids)
        reserved = list(instance._raw_reserved_ids)
        pending_idless = instance._raw_pending_idless
        needs_resync = instance._raw_needs_resync
        backlog = list(instance._raw_backlog)
    finally:
        observed = finish_live_no_ack_flush_session(instance, process, counter)

    prefix = f"{operation}/control-{label}"
    if type(caught) is not RuntimeError:
        failures.append(f"{prefix}: flush sin ACK no produjo RuntimeError: {caught!r}")
    else:
        message = str(caught).lower()
        if "resync" not in message or "flush" not in message or marker.lower() not in message:
            failures.append(f"{prefix}: RuntimeError no identifica flush causal: {caught!r}")
        cause = caught.__cause__
        if not isinstance(cause, error_type) or marker not in str(cause):
            failures.append(f"{prefix}: causa de flush no preservada: {cause!r}")
    if not alive_after_error:
        failures.append(f"{prefix}: control no mantuvo hijo vivo")
    if len(writes) != 1:
        failures.append(f"{prefix}: escribió barrera/public inesperada: {writes!r}")
        barrier = None
    else:
        barrier = json.loads(writes[0])
        if barrier.get("method") != "ping" or not isinstance(barrier.get("id"), str):
            failures.append(f"{prefix}: único write no era barrera: {barrier!r}")
    expected_pending = [barrier.get("id")] if isinstance(barrier, dict) else []
    if pending != expected_pending:
        failures.append(f"{prefix}: barrera enviada no quedó pending: {pending!r}")
    if reserved or pending_idless != 1 or not needs_resync or backlog:
        failures.append(
            f"{prefix}: estado tras flush sin ACK incorrecto: pending={pending!r} "
            f"reserved={reserved!r} idless={pending_idless!r} "
            f"needs={needs_resync!r} backlog={backlog!r}"
        )
    if observed_queue.frame_queued.is_set():
        failures.append(f"{prefix}: control publicó ACK inesperado")
    first_raw = writes[0].rstrip("\n") if writes else ""
    if observed.get("first") != first_raw or observed.get("rest") != []:
        failures.append(f"{prefix}: hijo no acreditó sólo barrera: {observed!r}")


def run_acknowledged_flush_error_beats_terminal(operation: str) -> None:
    """ACK en cola prueba entrega y domina al error tardío de flush."""
    harness = load_harness()
    failures: list[str] = []
    error_cases = (
        ("broken-pipe", BrokenPipeError),
        ("os-error", OSError),
        ("value-error", ValueError),
    )
    control_label, control_error = error_cases[0 if operation == "raw" else 2]
    assert_live_no_ack_flush_control(
        harness, operation, control_label, control_error, failures
    )

    for label, error_type in error_cases:
        ack_marker = f"ACK_BEFORE_FLUSH_{operation.upper()}_{label.upper()}"
        flush_marker = f"FLUSH_AFTER_ACK_{operation.upper()}_{label.upper()}"
        instance, process, counter, observed_queue = flush_error_barrier_session(
            harness, ack_marker, error_type, ack_then_exit=True
        )
        counter.flush_error_marker = flush_marker
        public_raw = '{"jsonrpc":"2.0","method":"notify/public-after-flush-ack"}'
        try:
            try:
                if operation == "raw":
                    value = instance.raw_line(public_raw, timeout=0.5)
                    first_outcome = ("return", value)
                else:
                    value = instance.rpc("public/after-flush-ack")
                    first_outcome = ("return", value)
            except Exception as error:
                first_outcome = ("error", error)
            poll_after_first = process.poll()
            state_after_first = (
                list(instance._raw_pending_ids),
                instance._raw_pending_idless,
                list(instance._raw_reserved_ids),
                instance._raw_needs_resync,
                list(instance._raw_backlog),
                queued_text_frames(instance),
            )
            started = time.monotonic()
            try:
                if operation == "raw":
                    value = instance.raw_line(public_raw, timeout=0.5)
                    follow_outcome = ("return", value)
                else:
                    value = instance._read_response(6, timeout=0.5)
                    follow_outcome = ("return", value)
            except Exception as error:
                follow_outcome = ("error", error)
            follow_elapsed = time.monotonic() - started
            writes = list(counter.write_attempts)
            barrier = json.loads(writes[0]) if writes else None
            barrier_id = barrier.get("id") if isinstance(barrier, dict) else None
        finally:
            stderr_tail = finish_queued_response_terminal_session(
                instance, process, counter
            )

        prefix = f"{operation}/{label}"
        if not observed_queue.frame_queued.is_set() or poll_after_first != 0:
            failures.append(
                f"{prefix}: guarda no fijó ACK en cola + exit0: "
                f"queued={observed_queue.frame_queued.is_set()} poll={poll_after_first!r}"
            )
        if (
            not isinstance(barrier, dict)
            or barrier.get("method") != "ping"
            or not isinstance(barrier_id, str)
        ):
            failures.append(f"{prefix}: único write no era barrera: {barrier!r}")
        if len(writes) != 1:
            failures.append(f"{prefix}: operación pública escribió tras terminal: {writes!r}")
        if operation == "raw":
            expected = ("return", {"server_exited": 0})
            if first_outcome != expected:
                failures.append(
                    f"{prefix}: ACK no dominó flush; raw pública no vio terminal: "
                    f"{first_outcome!r}"
                )
            if follow_outcome != expected:
                failures.append(f"{prefix}: terminal raw no fue estable: {follow_outcome!r}")
            if ack_marker not in stderr_tail:
                failures.append(f"{prefix}: stderr marker no quedó observable: {stderr_tail!r}")
        else:
            for phase, expected_id, outcome in (
                ("public", 5, first_outcome),
                ("follow", 6, follow_outcome),
            ):
                if outcome[0] != "error" or type(outcome[1]) is not RuntimeError:
                    failures.append(f"{prefix}/{phase}: no produjo RuntimeError EOF: {outcome!r}")
                    continue
                message = str(outcome[1])
                normalized = message.lower()
                if "eof" not in normalized or str(expected_id) not in message:
                    failures.append(
                        f"{prefix}/{phase}: perdió EOF/id={expected_id}: {message!r}"
                    )
                if normalized.startswith("resync ") or flush_marker in message:
                    failures.append(
                        f"{prefix}/{phase}: expuso flush/resync pese al ACK: {message!r}"
                    )
                if "exit=0" not in normalized or ack_marker not in message:
                    failures.append(
                        f"{prefix}/{phase}: perdió exit0/stderr estable: {message!r}"
                    )
            if stderr_tail:
                failures.append(f"{prefix}: stderr cacheado se releyó: {stderr_tail!r}")
        if follow_elapsed > STRICT_LIMIT:
            failures.append(f"{prefix}: terminal posterior tardó {follow_elapsed:.3f}s")
        expected_state = ([], 0, [], False, [], [])
        if state_after_first != expected_state:
            failures.append(
                f"{prefix}: cohorte sucia tras ACK+flush: {state_after_first!r}"
            )
        if barrier_id is not None:
            assert_clean_barrier_cohort(instance, barrier_id, prefix, failures)

    assert not failures, f"{operation}: ACK no dominó error de flush:\n- " + "\n- ".join(
        failures
    )


def check_raw_line_acknowledged_barrier_beats_flush_error_and_terminal() -> None:
    run_acknowledged_flush_error_beats_terminal("raw")


def check_rpc_acknowledged_barrier_beats_flush_error_and_terminal() -> None:
    run_acknowledged_flush_error_beats_terminal("rpc")


def install_tiny_rpc_resync_deadline(instance, clock) -> None:
    original_resync = instance._resync_before_operation

    def resync_with_tiny_deadline(_deadline, colliding_request_id=None):
        return original_resync(
            clock.monotonic() + 0.001,
            colliding_request_id=colliding_request_id,
        )

    instance._resync_before_operation = resync_with_tiny_deadline


def assert_past_deadline_no_ack_control(
    harness, operation: str, error_type, failures: list[str]
) -> None:
    clock = ControlledMonotonic()
    harness.time = clock
    marker = f"PAST_DEADLINE_NO_ACK_{operation.upper()}"
    instance, process, counter, observed_queue = flush_error_barrier_session(
        harness, marker, error_type, ack_then_exit=False
    )
    counter.before_flush_error = clock.cross_deadline
    if operation == "rpc":
        install_tiny_rpc_resync_deadline(instance, clock)
    public_raw = '{"jsonrpc":"2.0","method":"notify/past-deadline-no-ack"}'
    caught = None
    try:
        try:
            if operation == "raw":
                instance.raw_line(public_raw, timeout=0.001)
            else:
                instance.rpc("past-deadline/no-ack")
        except Exception as error:
            caught = error
        alive = process.poll() is None
        writes = list(counter.write_attempts)
        state = (
            list(instance._raw_pending_ids),
            instance._raw_pending_idless,
            list(instance._raw_reserved_ids),
            instance._raw_needs_resync,
            list(instance._raw_backlog),
        )
    finally:
        observed = finish_live_no_ack_flush_session(instance, process, counter)

    prefix = f"{operation}/past-deadline-no-ack"
    if not clock.crossed:
        failures.append(f"{prefix}: reloj no cruzó deadline antes del error")
    if type(caught) is not RuntimeError:
        failures.append(f"{prefix}: ausencia de ACK no produjo RuntimeError: {caught!r}")
    else:
        message = str(caught).lower()
        if "resync" not in message or "flush" not in message or marker.lower() not in message:
            failures.append(f"{prefix}: error no conservó flush causal: {caught!r}")
        if not isinstance(caught.__cause__, error_type):
            failures.append(f"{prefix}: causa no preservada: {caught.__cause__!r}")
    if not alive:
        failures.append(f"{prefix}: control requería hijo vivo")
    barrier = json.loads(writes[0]) if len(writes) == 1 else None
    if (
        not isinstance(barrier, dict)
        or barrier.get("method") != "ping"
        or not isinstance(barrier.get("id"), str)
    ):
        failures.append(f"{prefix}: único write no era barrera: {writes!r}")
        expected_pending = []
    else:
        expected_pending = [barrier["id"]]
    expected_state = (expected_pending, 1, [], True, [])
    if state != expected_state:
        failures.append(f"{prefix}: estado pending incorrecto: {state!r}")
    if observed_queue.frame_queued.is_set():
        failures.append(f"{prefix}: control sin ACK materializó un frame")
    first_raw = writes[0].rstrip("\n") if writes else ""
    if observed.get("first") != first_raw or observed.get("rest") != []:
        failures.append(f"{prefix}: hijo no acreditó sólo barrera: {observed!r}")


def run_past_deadline_ack_beats_flush_error(operation: str) -> None:
    """Incluso vencido el plazo se inspecciona una vez la cola no bloqueante."""
    harness = load_harness()
    failures: list[str] = []
    error_cases = (("os-error", OSError), ("value-error", ValueError))
    assert_past_deadline_no_ack_control(
        harness,
        operation,
        OSError if operation == "raw" else ValueError,
        failures,
    )

    for label, error_type in error_cases:
        clock = ControlledMonotonic()
        harness.time = clock
        ack_marker = f"ACK_PAST_DEADLINE_{operation.upper()}_{label.upper()}"
        flush_marker = f"FLUSH_PAST_DEADLINE_{operation.upper()}_{label.upper()}"
        instance, process, counter, observed_queue = flush_error_barrier_session(
            harness, ack_marker, error_type, ack_then_exit=True
        )
        counter.flush_error_marker = flush_marker
        counter.before_flush_error = clock.cross_deadline
        if operation == "rpc":
            install_tiny_rpc_resync_deadline(instance, clock)
        public_raw = '{"jsonrpc":"2.0","method":"notify/public-past-deadline"}'
        try:
            try:
                if operation == "raw":
                    value = instance.raw_line(public_raw, timeout=0.001)
                    first_outcome = ("return", value)
                else:
                    value = instance.rpc("public/past-deadline")
                    first_outcome = ("return", value)
            except Exception as error:
                first_outcome = ("error", error)
            poll_after_first = process.poll()
            state_after_first = (
                list(instance._raw_pending_ids),
                instance._raw_pending_idless,
                list(instance._raw_reserved_ids),
                instance._raw_needs_resync,
                list(instance._raw_backlog),
                queued_text_frames(instance),
            )
            started = time.monotonic()
            try:
                if operation == "raw":
                    value = instance.raw_line(public_raw, timeout=0.001)
                    follow_outcome = ("return", value)
                else:
                    value = instance._read_response(6, timeout=0.001)
                    follow_outcome = ("return", value)
            except Exception as error:
                follow_outcome = ("error", error)
            follow_elapsed = time.monotonic() - started
            writes = list(counter.write_attempts)
            barrier = json.loads(writes[0]) if writes else None
            barrier_id = barrier.get("id") if isinstance(barrier, dict) else None
        finally:
            stderr_tail = finish_queued_response_terminal_session(
                instance, process, counter
            )

        prefix = f"{operation}/{label}"
        if not clock.crossed:
            failures.append(f"{prefix}: fake monotonic no cruzó deadline")
        if not observed_queue.frame_queued.is_set() or poll_after_first != 0:
            failures.append(
                f"{prefix}: ACK no estaba en cola antes del error/exit: "
                f"queued={observed_queue.frame_queued.is_set()} poll={poll_after_first!r}"
            )
        if (
            not isinstance(barrier, dict)
            or barrier.get("method") != "ping"
            or not isinstance(barrier_id, str)
        ):
            failures.append(f"{prefix}: único write no era barrera: {barrier!r}")
        if len(writes) != 1:
            failures.append(f"{prefix}: public escribió tras terminal: {writes!r}")
        if operation == "raw":
            expected = ("return", {"server_exited": 0})
            if first_outcome != expected:
                failures.append(
                    f"{prefix}: no inspeccionó ACK tras deadline; obtuvo {first_outcome!r}"
                )
            if follow_outcome != expected:
                failures.append(f"{prefix}: terminal raw no fue estable: {follow_outcome!r}")
            if ack_marker not in stderr_tail:
                failures.append(f"{prefix}: stderr marker no observable: {stderr_tail!r}")
        else:
            for phase, expected_id, outcome in (
                ("public", 5, first_outcome),
                ("follow", 6, follow_outcome),
            ):
                if outcome[0] != "error" or type(outcome[1]) is not RuntimeError:
                    failures.append(f"{prefix}/{phase}: no produjo EOF: {outcome!r}")
                    continue
                message = str(outcome[1])
                normalized = message.lower()
                if "eof" not in normalized or str(expected_id) not in message:
                    failures.append(
                        f"{prefix}/{phase}: perdió EOF/id={expected_id}: {message!r}"
                    )
                if normalized.startswith("resync ") or flush_marker in message:
                    failures.append(
                        f"{prefix}/{phase}: expuso deadline/flush de barrera: {message!r}"
                    )
                if "exit=0" not in normalized or ack_marker not in message:
                    failures.append(
                        f"{prefix}/{phase}: perdió exit0/stderr: {message!r}"
                    )
            if stderr_tail:
                failures.append(f"{prefix}: stderr cacheado se releyó: {stderr_tail!r}")
        if follow_elapsed > STRICT_LIMIT:
            failures.append(f"{prefix}: terminal posterior tardó {follow_elapsed:.3f}s")
        expected_state = ([], 0, [], False, [], [])
        if state_after_first != expected_state:
            failures.append(f"{prefix}: cohorte no quedó limpia: {state_after_first!r}")
        if barrier_id is not None:
            assert_clean_barrier_cohort(instance, barrier_id, prefix, failures)

    assert not failures, f"{operation}: deadline anuló ACK en cola:\n- " + "\n- ".join(
        failures
    )


def check_raw_line_past_deadline_still_drains_ack_after_flush_error() -> None:
    run_past_deadline_ack_beats_flush_error("raw")


def check_rpc_past_deadline_still_drains_ack_after_flush_error() -> None:
    run_past_deadline_ack_beats_flush_error("rpc")


def foreign_then_ack_past_deadline_session(harness, marker: str, error_type):
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", FOREIGN_THEN_ACK_LIVE_CHILD, marker],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    observed_queue = ObservingQueue()
    instance._stdout_queue = observed_queue

    def wait_for_both_frames() -> None:
        if not observed_queue.wait_for_frames(1, 1):
            raise AssertionError("foreign no llegó a la cola")
        instance._stdout_read_ack.set()
        if not observed_queue.wait_for_frames(2, 1):
            raise AssertionError("ACK no llegó detrás del foreign")

    counter = CountingPopenStdin(
        process.stdin,
        process,
        wait_after_first_flush=True,
        queued_frames_waiter=wait_for_both_frames,
        wait_for_process_exit=False,
        flush_error_type=error_type,
        flush_error_marker=marker,
    )
    process.stdin = counter
    instance._raw_pending_idless = 1
    instance._raw_needs_resync = True
    instance._raw_backlog = []
    instance._raw_pending_ids = []
    instance._raw_reserved_ids = []
    instance._next_id = 5
    instance._ensure_stdout_reader()
    return instance, process, counter, observed_queue


def finish_foreign_then_ack_session(instance, process, counter):
    try:
        counter.close()
    except (BrokenPipeError, OSError, ValueError):
        pass
    process.wait(timeout=1)
    read_ack = getattr(instance, "_stdout_read_ack", None)
    if read_ack is not None:
        read_ack.set()
    reader = getattr(instance, "_stdout_reader", None)
    if reader is not None:
        reader.join(timeout=1)
    stderr = process.stderr.read() if process.stderr is not None else ""
    try:
        return json.loads(stderr.strip())
    except json.JSONDecodeError:
        return {"decode_error": stderr}


def run_past_deadline_foreign_stops_before_ack(operation: str) -> None:
    """Fuera de plazo se inspecciona uno: foreign se preserva y ACK queda en cola."""
    harness = load_harness()
    failures: list[str] = []
    error_cases = (("os-error", OSError), ("value-error", ValueError))
    public_raw = '{"jsonrpc":"2.0","method":"notify/must-not-send-after-foreign"}'

    for label, error_type in error_cases:
        clock = ControlledMonotonic()
        harness.time = clock
        marker = f"FOREIGN_THEN_ACK_{operation.upper()}_{label.upper()}"
        instance, process, counter, observed_queue = foreign_then_ack_past_deadline_session(
            harness, marker, error_type
        )
        counter.before_flush_error = lambda clock=clock: clock.advance(31.0)
        if operation == "rpc":
            install_tiny_rpc_resync_deadline(instance, clock)
        caught = None
        returned = None
        try:
            try:
                if operation == "raw":
                    returned = instance.raw_line(public_raw, timeout=0.001)
                else:
                    returned = instance.rpc("must-not-send/after-foreign")
            except Exception as error:
                caught = error
            alive = process.poll() is None
            writes = list(counter.write_attempts)
            backlog = list(instance._raw_backlog)
            queued = queued_text_frames(instance)
            pending = list(instance._raw_pending_ids)
            pending_idless = instance._raw_pending_idless
            reserved = list(instance._raw_reserved_ids)
            needs_resync = instance._raw_needs_resync
        finally:
            observed = finish_foreign_then_ack_session(instance, process, counter)

        prefix = f"{operation}/{label}"
        if returned is not None or type(caught) is not RuntimeError:
            failures.append(
                f"{prefix}: debía propagar RuntimeError flush, obtuvo "
                f"returned={returned!r} caught={caught!r}"
            )
        else:
            message = str(caught).lower()
            if "resync" not in message or "flush" not in message or marker.lower() not in message:
                failures.append(f"{prefix}: perdió error original de flush: {caught!r}")
            cause = caught.__cause__
            if not isinstance(cause, error_type) or marker not in str(cause):
                failures.append(f"{prefix}: perdió causa exacta de flush: {cause!r}")
        if not clock.crossed:
            failures.append(f"{prefix}: reloj no cruzó deadline antes del error")
        if not alive:
            failures.append(f"{prefix}: hijo debía seguir vivo para excluir EOF")
        if observed_queue.frame_count < 2:
            failures.append(
                f"{prefix}: no se acreditó queue [foreign, ACK]: "
                f"count={observed_queue.frame_count}"
            )
        barrier = json.loads(writes[0]) if len(writes) == 1 else None
        if (
            not isinstance(barrier, dict)
            or barrier.get("method") != "ping"
            or not isinstance(barrier.get("id"), str)
        ):
            failures.append(f"{prefix}: único write no era barrera: {writes!r}")
            barrier_id = None
        else:
            barrier_id = barrier["id"]
        frames = observed.get("frames", [])
        if len(frames) != 2:
            failures.append(f"{prefix}: hijo no acreditó dos frames: {observed!r}")
            foreign = None
            ack = None
        else:
            foreign, ack = frames
        if len(backlog) != 1 or foreign is None or json.loads(backlog[0]) != foreign:
            failures.append(f"{prefix}: foreign no quedó primero en backlog FIFO: {backlog!r}")
        if len(queued) != 1 or ack is None or json.loads(queued[0]) != ack:
            failures.append(f"{prefix}: ACK no quedó segundo y pendiente en queue: {queued!r}")
        if barrier_id is not None:
            expected_pending = [barrier_id]
            if ack is not None and ack.get("id") != barrier_id:
                failures.append(f"{prefix}: ACK no correlaciona barrera: {ack!r}")
        else:
            expected_pending = []
        if (
            pending != expected_pending
            or pending_idless != 1
            or reserved
            or not needs_resync
        ):
            failures.append(
                f"{prefix}: estado cambió pese a ACK fuera del límite: "
                f"pending={pending!r} idless={pending_idless!r} "
                f"reserved={reserved!r} needs={needs_resync!r}"
            )
        first_raw = writes[0].rstrip("\n") if writes else ""
        if observed.get("first") != first_raw or observed.get("rest") != []:
            failures.append(
                f"{prefix}: operación pública se escribió después de la barrera: {observed!r}"
            )

    assert not failures, f"{operation}: drenaje excedió deadline tras foreign:\n- " + "\n- ".join(
        failures
    )


def check_raw_line_past_deadline_foreign_stops_before_queued_ack() -> None:
    run_past_deadline_foreign_stops_before_ack("raw")


def check_rpc_past_deadline_foreign_stops_before_queued_ack() -> None:
    run_past_deadline_foreign_stops_before_ack("rpc")


def run_post_deadline_probe_is_exactly_nonblocking(operation: str) -> None:
    """El sondeo extraordinario posterior al deadline usa timeout exactamente cero."""
    harness = load_harness()
    failures: list[str] = []
    public_raw = '{"jsonrpc":"2.0","method":"notify/nonblocking-probe"}'

    for label, error_type in (("os-error", OSError), ("value-error", ValueError)):
        clock = ControlledMonotonic()
        harness.time = clock
        marker = f"NONBLOCKING_PROBE_{operation.upper()}_{label.upper()}"
        instance, process, counter, observed_queue = flush_error_barrier_session(
            harness, marker, error_type, ack_then_exit=False
        )
        counter.before_flush_error = lambda clock=clock: clock.advance(31.0)
        if operation == "rpc":
            install_tiny_rpc_resync_deadline(instance, clock)
        original_stdout_item = instance._stdout_item
        observed_timeouts: list[float] = []

        def stdout_item_spy(timeout):
            observed_timeouts.append(timeout)
            if timeout != 0:
                raise AssertionError(
                    f"inspección post-deadline bloqueante: timeout={timeout!r}"
                )
            return original_stdout_item(timeout)

        instance._stdout_item = stdout_item_spy
        returned = None
        caught = None
        started = time.monotonic()
        try:
            try:
                if operation == "raw":
                    returned = instance.raw_line(public_raw, timeout=0.001)
                else:
                    returned = instance.rpc("nonblocking/probe")
            except Exception as error:
                caught = error
            elapsed = time.monotonic() - started
            alive = process.poll() is None
            writes = list(counter.write_attempts)
            pending = list(instance._raw_pending_ids)
            state = (
                instance._raw_pending_idless,
                list(instance._raw_reserved_ids),
                instance._raw_needs_resync,
                list(instance._raw_backlog),
            )
        finally:
            observed = finish_live_no_ack_flush_session(instance, process, counter)

        prefix = f"{operation}/{label}"
        if observed_timeouts != [0]:
            failures.append(
                f"{prefix}: sondeo debía ser único y no bloqueante [0], "
                f"obtuvo {observed_timeouts!r}; caught={caught!r}"
            )
        if returned is not None or type(caught) is not RuntimeError:
            failures.append(
                f"{prefix}: debía propagar RuntimeError flush tras sondeo vacío: "
                f"returned={returned!r} caught={caught!r}"
            )
        else:
            message = str(caught).lower()
            if "resync" not in message or "flush" not in message or marker.lower() not in message:
                failures.append(f"{prefix}: error original de flush se perdió: {caught!r}")
            cause = caught.__cause__
            if not isinstance(cause, error_type) or marker not in str(cause):
                failures.append(f"{prefix}: causa exacta se perdió: {cause!r}")
        if elapsed > STRICT_LIMIT:
            failures.append(f"{prefix}: sondeo no fue acotado: {elapsed:.3f}s")
        if not clock.crossed or not alive:
            failures.append(
                f"{prefix}: guarda inválida: crossed={clock.crossed!r} alive={alive!r}"
            )
        barrier = json.loads(writes[0]) if len(writes) == 1 else None
        if (
            not isinstance(barrier, dict)
            or barrier.get("method") != "ping"
            or not isinstance(barrier.get("id"), str)
        ):
            failures.append(f"{prefix}: único write no era barrera: {writes!r}")
            expected_pending = []
        else:
            expected_pending = [barrier["id"]]
        if pending != expected_pending or state != (1, [], True, []):
            failures.append(
                f"{prefix}: sondeo vacío alteró deuda: pending={pending!r} state={state!r}"
            )
        if observed_queue.frame_queued.is_set():
            failures.append(f"{prefix}: control no-ACK materializó frame")
        first_raw = writes[0].rstrip("\n") if writes else ""
        if observed.get("first") != first_raw or observed.get("rest") != []:
            failures.append(f"{prefix}: public se escribió indebidamente: {observed!r}")

    assert not failures, f"{operation}: sondeo post-deadline no fue timeout=0:\n- " + "\n- ".join(
        failures
    )


def check_raw_line_post_deadline_probe_calls_stdout_item_with_zero_timeout() -> None:
    run_post_deadline_probe_is_exactly_nonblocking("raw")


def check_rpc_post_deadline_probe_calls_stdout_item_with_zero_timeout() -> None:
    run_post_deadline_probe_is_exactly_nonblocking("rpc")


def check_close_terminates_real_process_and_stdout_reader_bounded() -> None:
    """close libera el Popen real y su único lector sin dejar thread vivo."""
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", CLOSE_LIFECYCLE_CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    ready = instance._stdout_item(timeout=1)
    reader = instance._stdout_reader
    assert ready == "reader-ready\n" and reader.is_alive(), (
        "la guarda exige un Popen real con el lector activo antes de close"
    )

    started = time.monotonic()
    try:
        instance.close()
        elapsed = time.monotonic() - started
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=1)

    assert elapsed <= 0.5, f"close excedió su espera acotada: {elapsed:.3f}s"
    assert process.returncode == 0, (
        f"cerrar stdin debe terminar limpiamente el hijo real: {process.returncode}"
    )
    assert not reader.is_alive(), "close dejó vivo el thread lodestar-stdout-reader"
    assert process.stdin is not None and process.stdin.closed, "close no cerró stdin"
    assert process.stdout is not None and process.stdout.closed, "close no cerró stdout"


def check_real_pipe_preserves_prefetched_second_frame() -> None:
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", PREFETCH_CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    try:
        first_started = time.monotonic()
        first = instance.raw_line('{"jsonrpc":"2.0","id":1,"method":"first"}', timeout=TIMEOUT)
        first_elapsed = time.monotonic() - first_started
        # El hijo escribe ambos frames después de que caduque id=1 pero antes de id=2.
        time.sleep(0.18)
        second_started = time.monotonic()
        second = instance.raw_line('{"jsonrpc":"2.0","id":2,"method":"second"}', timeout=TIMEOUT)
        second_elapsed = time.monotonic() - second_started
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    assert first is None, (
        f"la respuesta tardía id=1 debe caducar y no entregarse: {first!r}"
    )
    assert first_elapsed <= STRICT_LIMIT, (
        f"id=1 debe caducar en <= {STRICT_LIMIT:.3f}s, no en {first_elapsed:.3f}s"
    )
    assert second and second.get("id") == 2 and second["result"]["marker"] == "second", (
        f"el frame prebufferizado id=2 se perdió/desincronizó: {second!r}"
    )
    assert second_elapsed < STRICT_LIMIT, (
        f"el segundo frame ya escrito no debe esperar {second_elapsed:.3f}s"
    )


def check_raw_line_preserves_frame_returned_after_blocking_deadline_fifo() -> None:
    """Un get que termina tarde no autoriza a devolver el frame fuera del plazo."""
    harness = load_harness()
    clock = ControlledMonotonic()
    harness.time = clock
    initial_time = clock.monotonic()
    late_frame = (
        '{"jsonrpc":"2.0","id":101,'
        '"result":{"marker":"LATE_AFTER_BLOCKING_DEADLINE"}}\n'
    )
    queued_frame = (
        '{"jsonrpc":"2.0","id":102,'
        '"result":{"marker":"ALREADY_QUEUED_SECOND"}}\n'
    )
    stdout_queue = DeadlineCrossingQueue(clock, late_frame, queued_frame)
    instance = object.__new__(harness.LodestarSession)
    instance.proc = SimulatedProcess(BlockingStdout())
    instance._stdout_queue = stdout_queue
    instance._stdout_eof = object()
    instance._stdout_reader_lock = threading.Lock()
    # Sentinel no nulo: la prueba controla la cola y no arranca un reader concurrente.
    instance._stdout_reader = object()
    instance._stdout_read_ack = threading.Event()
    instance._stdout_eof_seen = threading.Event()
    instance._stdout_waiting_ack = False

    raw_lines = [
        '{"jsonrpc":"2.0","method":"deadline/first"}',
        '{"jsonrpc":"2.0","method":"deadline/second"}',
        '{"jsonrpc":"2.0","method":"deadline/third"}',
    ]
    expired = instance.raw_line(raw_lines[0], timeout=TIMEOUT)
    recovered_late = instance.raw_line(raw_lines[1], timeout=TIMEOUT)
    recovered_next = instance.raw_line(raw_lines[2], timeout=TIMEOUT)

    assert stdout_queue.blocking_returns == 1, (
        "la guarda no forzó exactamente un retorno desde el get bloqueante"
    )
    assert len(stdout_queue.blocking_timeouts) >= 1, (
        "raw_line no recorrió la obtención bloqueante de stdout"
    )
    assert 0 < stdout_queue.blocking_timeouts[0] <= TIMEOUT, (
        f"plazo bloqueante inesperado: {stdout_queue.blocking_timeouts[0]!r}"
    )
    assert (
        clock.crossed
        and stdout_queue.late_returned_at is not None
        and stdout_queue.late_returned_at > initial_time + TIMEOUT
    ), "el frame no fue devuelto después del deadline acreditado"
    assert expired is None, (
        "raw_line devolvió fuera de plazo el frame obtenido tras el deadline: "
        f"{expired!r}"
    )
    observed = [recovered_late, recovered_next]
    expected = [json.loads(late_frame), json.loads(queued_frame)]
    assert observed == expected, (
        "el frame tardío debe conservarse y preceder al que ya estaba en cola: "
        f"observed={observed!r} expected={expected!r}"
    )
    assert instance.proc.stdin.lines == [line + "\n" for line in raw_lines], (
        f"las tres operaciones públicas no se escribieron exactamente una vez: "
        f"{instance.proc.stdin.lines!r}"
    )


def check_real_pipe_discards_late_idless_error_before_current_response() -> None:
    """Un error sin id sólo pertenece a la llamada inválida aún dentro de su plazo.

    El negativo complementa ``non-string-method-observed-frame``: aquel fija que el
    ``-32600`` actual sí se entrega; éste impide atribuir ese mismo tipo de frame a una
    request posterior una vez caducada la entrada que lo causó.
    """
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", LATE_IDLESS_ERROR_CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    try:
        expired = instance.raw_line('{"foo":"bar"}', timeout=TIMEOUT)
        current = instance.raw_line(
            '{"jsonrpc":"2.0","id":2,"method":"ping"}', timeout=1.0
        )
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()

    assert expired is None, (
        f"la entrada inválida debía caducar antes de que el hijo respondiera: {expired!r}"
    )
    assert current and current.get("id") == 2, (
        "el -32600 tardío sin id debe descartarse al esperar la request actual id=2; "
        f"respuesta observada: {current!r}"
    )
    assert current.get("result", {}).get("marker") == "current request", (
        f"la respuesta id=2 debe ser exactamente la emitida para la segunda request: {current!r}"
    )


def check_silent_input_discards_late_expired_request_response() -> None:
    """Una entrada silenciosa no adopta la respuesta de una request ya caducada.

    Los negativos ``malformed-observed-frame`` y ``notification-observed-frame`` siguen
    fijando el caso opuesto: un frame sin id emitido durante la entrada silenciosa sí es
    observable. Aquí el frame lleva el id=1 de la llamada anterior y debe descartarse.
    """
    harness = load_harness()
    silent_cases = [
        ("malformed", "{json roto sin cerrar"),
        ("notification", '{"jsonrpc":"2.0","method":"notify/late"}'),
    ]
    failures: list[str] = []

    for label, silent_line in silent_cases:
        process = subprocess.Popen(
            [sys.executable, "-u", "-c", LATE_EXPIRED_ID_CHILD],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        instance = object.__new__(harness.LodestarSession)
        instance.proc = process
        expired_line = '{"jsonrpc":"2.0","id":1,"method":"first"}'
        stderr = ""
        try:
            expired = instance.raw_line(expired_line, timeout=TIMEOUT)
            silent_started = time.monotonic()
            silent = instance.raw_line(silent_line, timeout=TIMEOUT)
            silent_elapsed = time.monotonic() - silent_started
        finally:
            if process.stdin is not None:
                process.stdin.close()
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            if process.stderr is not None:
                stderr = process.stderr.read()

        if expired is not None:
            failures.append(f"{label}: id=1 debía caducar antes de la segunda línea: {expired!r}")
        if silent is not None:
            failures.append(
                f"{label}: la entrada silenciosa adoptó la respuesta tardía id=1: {silent!r}"
            )
        if silent_elapsed < TIMEOUT * 0.75:
            failures.append(
                f"{label}: devolvió silencio antes de agotar el plazo: {silent_elapsed:.3f}s"
            )
        if silent_elapsed > STRICT_LIMIT:
            failures.append(
                f"{label}: excedió el límite portable de {STRICT_LIMIT:.3f}s: "
                f"{silent_elapsed:.3f}s"
            )
        try:
            observed = json.loads(stderr.strip())
        except json.JSONDecodeError:
            failures.append(f"{label}: el hijo no acreditó la publicación: stderr={stderr!r}")
        else:
            if observed != {"first": expired_line, "second": silent_line}:
                failures.append(
                    f"{label}: el pipe no observó ambas entradas exactas: {observed!r}"
                )

    assert not failures, "respuesta expirada mal atribuida al silencio:\n- " + "\n- ".join(failures)


def check_silent_input_discards_late_idless_error_from_expired_invalid_input() -> None:
    """Un silencio no adopta el ``-32600`` tardío de una entrada inválida caducada.

    La primera llamada agota su plazo mientras el hijo espera la segunda línea. Sólo
    entonces publica el error sin ``id`` causado por ``first-invalid``. Los casos
    ``malformed-observed-frame`` y ``notification-observed-frame`` conservan la guarda
    opuesta: sin una entrada anterior pendiente, un frame fresco durante silencio sí es
    observable.
    """
    harness = load_harness()
    silent_cases = [
        ("malformed", "{json roto sin cerrar"),
        ("notification", '{"jsonrpc":"2.0","method":"notify/late-idless"}'),
    ]
    expired_line = '{"foo":"first-invalid"}'
    failures: list[str] = []

    for label, silent_line in silent_cases:
        process = subprocess.Popen(
            [sys.executable, "-u", "-c", LATE_EXPIRED_IDLESS_ERROR_CHILD],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        instance = object.__new__(harness.LodestarSession)
        instance.proc = process
        stderr = ""
        try:
            expired = instance.raw_line(expired_line, timeout=TIMEOUT)
            silent_started = time.monotonic()
            silent = instance.raw_line(silent_line, timeout=TIMEOUT)
            silent_elapsed = time.monotonic() - silent_started
        finally:
            if process.stdin is not None:
                process.stdin.close()
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            if process.stderr is not None:
                stderr = process.stderr.read()

        if expired is not None:
            failures.append(
                f"{label}: la primera entrada inválida debía caducar: {expired!r}"
            )
        if silent is not None:
            failures.append(
                f"{label}: el silencio adoptó el -32600 tardío de first-invalid: {silent!r}"
            )
        if silent_elapsed < TIMEOUT * 0.75:
            failures.append(
                f"{label}: la llamada silenciosa no respetó el plazo completo: "
                f"{silent_elapsed:.3f}s"
            )
        if silent_elapsed > STRICT_LIMIT:
            failures.append(
                f"{label}: excedió el límite portable de {STRICT_LIMIT:.3f}s: "
                f"{silent_elapsed:.3f}s"
            )
        try:
            observed = json.loads(stderr.strip())
        except json.JSONDecodeError:
            failures.append(f"{label}: el hijo no acreditó la publicación: stderr={stderr!r}")
        else:
            if observed != {"first": expired_line, "second": silent_line}:
                failures.append(
                    f"{label}: el pipe no observó ambas entradas exactas: {observed!r}"
                )

    assert not failures, "error sin id expirado mal atribuido al silencio:\n- " + "\n- ".join(failures)


def check_rejected_bool_id_does_not_consume_next_fresh_idless_error() -> None:
    """Un id booleano silencioso no crea deuda para el siguiente error sin id.

    El hijo coordinado publica cero frames para ``id:true``, el ``-32600`` sólo tras
    recibir el objeto inválido posterior y una respuesta correlacionada para el ping.
    Así el test distingue un frame tardío real de una deuda inventada por clasificación.
    ``injected-id-domain`` conserva la guarda opuesta: si el request con id rechazado sí
    recibe un frame fresco, el arnés debe exponerlo.
    """
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", REJECTED_BOOL_THEN_FRESH_INVALID_CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    bool_line = '{"jsonrpc":"2.0","id":true,"method":"ping"}'
    invalid_line = '{"foo":"bar"}'
    ping_line = '{"jsonrpc":"2.0","id":91,"method":"ping"}'
    stderr = ""
    try:
        rejected = instance.raw_line(bool_line, timeout=TIMEOUT)
        fresh_invalid = instance.raw_line(invalid_line, timeout=0.5)
        ping = instance.raw_line(ping_line, timeout=0.5)
        pending = getattr(instance, "_raw_pending_idless", None)
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()

    assert rejected is None, (
        f"rmcp-silent id:true debe agotar el plazo sin frame: {rejected!r}"
    )
    assert fresh_invalid and fresh_invalid.get("error", {}).get("code") == -32600, (
        "el -32600 fresco de {foo:bar} no pertenece al id:true anterior y debe "
        f"devolverse: {fresh_invalid!r}"
    )
    assert "id" not in fresh_invalid, (
        f"la reproducción exige el error fresco sin id que emite rmcp: {fresh_invalid!r}"
    )
    assert pending == 0, f"la secuencia terminó con deuda idless fantasma: {pending!r}"
    assert ping == {
        "jsonrpc": "2.0",
        "id": 91,
        "result": {"marker": "session-alive"},
    }, f"el ping posterior no correlacionó en la misma sesión: {ping!r}"
    try:
        observed = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        raise AssertionError(f"el hijo no acreditó las tres entradas: {stderr!r}") from error
    assert observed == {
        "first": bool_line,
        "second": invalid_line,
        "third": ping_line,
    }, f"el Popen no observó la secuencia exacta: {observed!r}"


def check_silent_input_returns_fresh_valid_id_response_without_expired_request() -> None:
    """Una entrada silenciosa no descarta un id válido que no tiene deuda previa."""
    harness = load_harness()
    silent_cases = [
        ("notification", '{"jsonrpc":"2.0","method":"notify/fresh-id"}'),
        ("malformed", "{json roto sin cerrar"),
    ]
    expected = {
        "jsonrpc": "2.0",
        "id": 77,
        "result": {"marker": "fresh-during-silence"},
    }
    failures: list[str] = []

    for label, silent_line in silent_cases:
        process = subprocess.Popen(
            [sys.executable, "-u", "-c", FRESH_IDFUL_DURING_SILENCE_CHILD],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        instance = object.__new__(harness.LodestarSession)
        instance.proc = process
        stderr = ""
        try:
            pending_before = getattr(instance, "_raw_pending_ids", None)
            response = instance.raw_line(silent_line, timeout=TIMEOUT)
        finally:
            if process.stdin is not None:
                process.stdin.close()
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            if process.stderr is not None:
                stderr = process.stderr.read()

        if pending_before not in (None, [], set(), {}):
            failures.append(f"{label}: la sesión empezó con ids vencidos: {pending_before!r}")
        if response != expected:
            failures.append(
                f"{label}: el frame fresco id=77 fue descartado sin deuda previa: {response!r}"
            )
        try:
            observed = json.loads(stderr.strip())
        except json.JSONDecodeError:
            failures.append(f"{label}: el hijo no acreditó la entrada: {stderr!r}")
        else:
            if observed != {"observed": silent_line}:
                failures.append(
                    f"{label}: el hijo no observó la entrada silenciosa exacta: {observed!r}"
                )

    assert not failures, "frame idful fresco descartado durante silencio:\n- " + "\n- ".join(failures)


def check_silent_input_preserves_valid_type_alias_of_expired_id() -> None:
    """La deuda de id entero 1 no autoriza descartar el id string ``\"1\"``."""
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", EXPIRED_INTEGER_THEN_STRING_ALIAS_CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    expired_line = '{"jsonrpc":"2.0","id":1,"method":"ping"}'
    silent_line = '{"jsonrpc":"2.0","method":"notify/type-alias"}'
    stderr = ""
    try:
        expired = instance.raw_line(expired_line, timeout=TIMEOUT)
        response = instance.raw_line(silent_line, timeout=TIMEOUT)
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()

    assert expired is None, f"la request id entero 1 debía vencer sin frame: {expired!r}"
    assert response == {
        "jsonrpc": "2.0",
        "id": "1",
        "result": {"marker": "string-id-is-fresh"},
    }, (
        "una request vencida id entero 1 no permite descartar el frame fresco id string "
        f"'1': {response!r}"
    )
    try:
        observed = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        raise AssertionError(f"el hijo no acreditó ambas entradas: {stderr!r}") from error
    assert observed == {"first": expired_line, "second": silent_line}, (
        f"el Popen no observó la secuencia exacta: {observed!r}"
    )


def run_idless_causal_boundary_case(mode: str, second_line: str):
    """Ejecuta dos eventos reales tolerando sólo pings internos correlacionables.

    Una barrera futura puede insertar esos pings para cerrar causalmente el primer
    timeout. El hijo los acredita pero no los confunde con eventos del escenario ni
    publica el frame fresco antes de observar la segunda línea de usuario.
    """
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", IDLESS_CAUSAL_BOUNDARY_CHILD, mode],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    first_line = '{"foo":"first-invalid"}'
    stderr = ""
    try:
        first = instance.raw_line(first_line, timeout=TIMEOUT)
        second = instance.raw_line(second_line, timeout=TIMEOUT)
        process_alive = process.poll() is None
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()

    try:
        observed = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        raise AssertionError(f"el hijo no acreditó los eventos: {stderr!r}") from error
    assert observed.get("first") == first_line and observed.get("second") == second_line, (
        f"el Popen no observó las dos líneas de usuario exactas: {observed!r}"
    )
    for barrier in observed.get("barriers", []):
        parsed = json.loads(barrier)
        assert parsed.get("jsonrpc") == "2.0" and parsed.get("method") == "ping", (
            f"el hijo toleró como barrera una línea no-ping: {barrier!r}"
        )
        assert isinstance(parsed.get("id"), str) or type(parsed.get("id")) is int, (
            f"la barrera tolerada no era correlacionable: {barrier!r}"
        )
    assert first is None, f"el primer invalid debía vencer sin emitir frame: {first!r}"
    assert process_alive, "el hijo/stdout debían seguir vivos tras publicar el frame fresco"
    return second


def check_second_invalid_observes_its_fresh_idless_error_after_silent_invalid() -> None:
    """El timeout sin salida del invalid #1 no puede consumir el error del invalid #2."""
    response = run_idless_causal_boundary_case(
        "second-invalid", '{"foo":"second-invalid"}'
    )
    assert response == {
        "jsonrpc": "2.0",
        "error": {"code": -32600, "message": "fresh second invalid"},
    }, f"la deuda fantasma del invalid #1 consumió el -32600 fresco del #2: {response!r}"


def check_notification_observes_fresh_id_null_frame_after_silent_invalid() -> None:
    """El timeout sin salida de un invalid no oculta un frame fresco de notification."""
    response = run_idless_causal_boundary_case(
        "notification", '{"jsonrpc":"2.0","method":"notify/fresh-idless"}'
    )
    assert response == {
        "jsonrpc": "2.0",
        "id": None,
        "error": {"code": -32601, "message": "fresh notification frame"},
    }, f"la deuda fantasma del invalid previo consumió el frame id:null fresco: {response!r}"


def check_rpc_does_not_reuse_expired_raw_request_id_or_accept_stale_response() -> None:
    """Un rpc evita ids raw pendientes y alcanza su respuesta fresca."""
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", RAW_ID_COLLISION_WITH_RPC_CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    # Fuerza el candidato que colisiona con el id raw vencido. La implementación debe
    # saltarlo antes de emitir el rpc, no confiar en el orden fortuito de las respuestas.
    instance._next_id = 2
    raw_line = '{"jsonrpc":"2.0","id":2,"method":"ping"}'
    stderr = ""
    try:
        expired = instance.raw_line(raw_line, timeout=TIMEOUT)
        rpc_response = instance.rpc("ping")
        process_alive = process.poll() is None
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()

    try:
        observed = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        raise AssertionError(f"el hijo no acreditó raw+rpc: {stderr!r}") from error
    failures: list[str] = []
    if expired is not None:
        failures.append(f"la request raw id=2 debía vencer sin frame: {expired!r}")
    if observed.get("raw") != raw_line:
        failures.append(f"el hijo no observó la request raw exacta: {observed!r}")
    rpc_line = json.loads(observed.get("rpc", "null"))
    rpc_id = observed.get("rpc_id")
    if rpc_line.get("id") != rpc_id or rpc_line.get("method") != "ping":
        failures.append(f"el hijo no acreditó el rpc y su id: {observed!r}")
    if type(rpc_id) is not int or rpc_id == 2:
        failures.append(f"rpc reutilizó el id entero 2 todavía pendiente: {rpc_id!r}")
    if rpc_response.get("result", {}).get("marker") != "FRESH_RPC":
        failures.append(f"rpc aceptó STALE_RAW en vez de su respuesta fresca: {rpc_response!r}")
    if rpc_response.get("id") != rpc_id:
        failures.append(
            f"la respuesta final no correlaciona con el id realmente emitido: {rpc_response!r}"
        )
    if not process_alive:
        failures.append("el hijo/stdout no seguían vivos tras publicar ambas respuestas")
    assert not failures, "colisión entre id raw pendiente y rpc:\n- " + "\n- ".join(failures)


def check_barrier_preserves_fresh_frame_and_avoids_pending_string_id() -> None:
    """La barrera no colisiona con deuda string ni pierde frames frescos anteriores al ACK."""
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", FRESH_FRAME_DURING_BARRIER_CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    collision_token = "collision-token"
    pending_id = f"lodestar-testbench-resync:{collision_token}"
    pending_line = json.dumps(
        {"jsonrpc": "2.0", "id": pending_id, "method": "ping"}, separators=(",", ":")
    )
    invalid_line = '{"foo":"invalid-needs-boundary"}'
    notification_line = '{"jsonrpc":"2.0","method":"notify/after-boundary"}'
    tokens = iter((collision_token, "safe-token"))
    original_uuid4 = harness.uuid.uuid4
    stderr = ""
    try:
        harness.uuid.uuid4 = lambda: next(tokens)
        pending = instance.raw_line(pending_line, timeout=TIMEOUT)
        invalid = instance.raw_line(invalid_line, timeout=TIMEOUT)
        response = instance.raw_line(notification_line, timeout=TIMEOUT)
        process_alive = process.poll() is None
    finally:
        harness.uuid.uuid4 = original_uuid4
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()

    try:
        observed = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        raise AssertionError(f"el hijo no acreditó deuda+barrera+operación: {stderr!r}") from error
    failures: list[str] = []
    if pending is not None or invalid is not None:
        failures.append(
            f"las dos precondiciones debían vencer sin frame: pending={pending!r} invalid={invalid!r}"
        )
    if observed.get("pending") != pending_line or observed.get("invalid") != invalid_line:
        failures.append(f"el hijo no acreditó las precondiciones exactas: {observed!r}")
    if observed.get("notification") != notification_line:
        failures.append(f"la operación posterior no llegó al hijo: {observed!r}")
    barrier = json.loads(observed.get("barrier", "null"))
    barrier_id = observed.get("barrier_id")
    if barrier.get("method") != "ping" or barrier.get("id") != barrier_id:
        failures.append(f"la barrera no fue un ping correlacionable acreditado: {observed!r}")
    if not isinstance(barrier_id, str) or barrier_id == pending_id:
        failures.append(
            f"la barrera colisionó con el id string todavía pendiente: {barrier_id!r}"
        )
    if response != {
        "jsonrpc": "2.0",
        "id": 77,
        "result": {"marker": "FRESH_DURING_BARRIER"},
    }:
        failures.append(f"la barrera perdió el frame fresco anterior a su ACK: {response!r}")
    if not process_alive:
        failures.append("el hijo/stdout no seguían vivos tras la operación posterior")
    assert not failures, "barrera JSON-RPC no preserva correlación:\n- " + "\n- ".join(failures)


def run_foreign_frame_fifo_case(mode: str):
    """Ejecuta una operación seguida de dos raw silenciosas sobre el mismo Popen."""
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", FOREIGN_FRAMES_BEFORE_CURRENT_RESPONSE_CHILD, mode],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    first_line = '{"jsonrpc":"2.0","id":1,"method":"ping"}'
    follow_1_line = '{"jsonrpc":"2.0","method":"notify/backlog-1"}'
    follow_2_line = '{"jsonrpc":"2.0","method":"notify/backlog-2"}'
    stderr = ""
    try:
        if mode == "raw":
            current = instance.raw_line(first_line, timeout=TIMEOUT)
        elif mode == "rpc":
            instance._next_id = 10
            current = instance.rpc("ping")
        else:
            raise AssertionError(f"modo de backlog desconocido: {mode}")
        follow_1 = instance.raw_line(follow_1_line, timeout=TIMEOUT)
        follow_2 = instance.raw_line(follow_2_line, timeout=TIMEOUT)
        process_alive = process.poll() is None
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()

    try:
        observed = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        raise AssertionError(f"el hijo no acreditó operación+FIFO raw: {stderr!r}") from error
    expected_current_id = 1 if mode == "raw" else 10
    expected_first = first_line if mode == "raw" else json.dumps(
        {"jsonrpc": "2.0", "id": 10, "method": "ping"}
    )
    failures: list[str] = []
    if observed.get("first") != expected_first or observed.get("current_id") != expected_current_id:
        failures.append(f"el hijo no acreditó la primera operación y su id: {observed!r}")
    if observed.get("follow_1") != follow_1_line or observed.get("follow_2") != follow_2_line:
        failures.append(f"el hijo no recibió las dos raw posteriores exactas: {observed!r}")
    if observed.get("emitted_ids") != [77, expected_current_id, 78]:
        failures.append(f"el hijo no acreditó el orden emitido: {observed!r}")
    expected_marker = "CURRENT_RAW" if mode == "raw" else "CURRENT_RPC"
    if current != {
        "jsonrpc": "2.0",
        "id": expected_current_id,
        "result": {"marker": expected_marker},
    }:
        failures.append(f"la operación actual no alcanzó su respuesta propia: {current!r}")
    if follow_1 != {
        "jsonrpc": "2.0",
        "id": 77,
        "result": {"marker": "FOREIGN_FIFO_1"},
    }:
        failures.append(f"la primera raw posterior no observó id=77 del backlog: {follow_1!r}")
    if follow_2 != {
        "jsonrpc": "2.0",
        "id": 78,
        "result": {"marker": "FOREIGN_FIFO_2"},
    }:
        failures.append(f"el backlog no conservó FIFO hasta id=78: {follow_2!r}")
    if not process_alive:
        failures.append("el hijo/stdout no seguían vivos tras consumir el FIFO")
    assert not failures, f"frames ajenos perdidos por {mode}:\n- " + "\n- ".join(failures)


def check_raw_request_preserves_foreign_frame_for_later_raw_fifo() -> None:
    """raw_line no oculta frames ajenos mientras espera su id correlacionado."""
    run_foreign_frame_fifo_case("raw")


def check_rpc_read_response_preserves_foreign_frame_for_later_raw_fifo() -> None:
    """rpc/_read_response deja los frames ajenos disponibles para raw_line en FIFO."""
    run_foreign_frame_fifo_case("rpc")


def run_barrier_idless_classification_case(mode: str):
    """Ejecuta invalid, barrera y una o dos operaciones raw sobre un Popen vivo."""
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", BARRIER_IDLESS_CLASSIFICATION_CHILD, mode],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    if mode == "fresh-id-null":
        invalid_line = '{"foo":"invalid-before-fresh-barrier"}'
    elif mode == "at-most-one-32600":
        invalid_line = '{"foo":"invalid-before-double-32600"}'
    else:
        raise AssertionError(f"modo idless de barrera desconocido: {mode}")
    follow_1_line = '{"jsonrpc":"2.0","method":"notify/barrier-idless-1"}'
    follow_2_line = '{"jsonrpc":"2.0","method":"notify/barrier-idless-2"}'
    stderr = ""
    try:
        expired = instance.raw_line(invalid_line, timeout=TIMEOUT)
        follow_1 = instance.raw_line(follow_1_line, timeout=TIMEOUT)
        follow_2 = None
        if mode == "at-most-one-32600":
            follow_2 = instance.raw_line(follow_2_line, timeout=TIMEOUT)
        process_alive = process.poll() is None
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()

    try:
        observed = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        raise AssertionError(f"el hijo no acreditó invalid+barrera+raw: {stderr!r}") from error
    failures: list[str] = []
    if expired is not None:
        failures.append(f"el invalid inicial debía vencer sin frame: {expired!r}")
    if observed.get("mode") != mode or observed.get("invalid") != invalid_line:
        failures.append(f"el hijo no acreditó el invalid exacto: {observed!r}")
    barrier = json.loads(observed.get("barrier", "null"))
    if barrier.get("method") != "ping" or barrier.get("id") != observed.get("barrier_id"):
        failures.append(f"el hijo no acreditó el ping de barrera y su id: {observed!r}")
    if observed.get("follow_1") != follow_1_line:
        failures.append(f"la primera operación pública no llegó al hijo: {observed!r}")
    if mode == "at-most-one-32600" and observed.get("follow_2") != follow_2_line:
        failures.append(f"la segunda operación pública no llegó al hijo: {observed!r}")
    if not process_alive:
        failures.append("el hijo/stdout no seguían vivos tras las operaciones públicas")
    return follow_1, follow_2, failures


def check_barrier_preserves_fresh_id_null_error_for_next_public_raw() -> None:
    """Un id:null/-32601 fresco previo al ACK entra al backlog, no a la deuda invalid."""
    response, _, failures = run_barrier_idless_classification_case("fresh-id-null")
    expected = {
        "jsonrpc": "2.0",
        "id": None,
        "error": {
            "code": -32601,
            "message": "fresh during barrier",
            "data": {"marker": "FRESH_DURING_BARRIER"},
        },
    }
    if response != expected:
        failures.append(f"la barrera perdió el id:null/-32601 fresco: {response!r}")
    assert not failures, "frame idless fresco perdido en barrera:\n- " + "\n- ".join(failures)


def check_barrier_discards_at_most_one_attributable_idless_32600() -> None:
    """Una deuda invalid consume un solo -32600; el segundo queda observable en FIFO."""
    first, second, failures = run_barrier_idless_classification_case(
        "at-most-one-32600"
    )
    expected_fresh = {
        "jsonrpc": "2.0",
        "id": None,
        "error": {
            "code": -32600,
            "message": "second idless is fresh",
            "data": {"marker": "SECOND_32600_NOT_ATTRIBUTABLE"},
        },
    }
    if first != expected_fresh:
        failures.append(
            "la barrera no descartó exactamente el primer -32600 atribuible o perdió "
            f"el segundo: {first!r}"
        )
    if second is not None:
        failures.append(
            "el -32600 atribuible quedó indebidamente en backlog tras consumir el fresco: "
            f"{second!r}"
        )
    assert not failures, "atribución idless demasiado amplia:\n- " + "\n- ".join(failures)


def check_pending_id_does_not_consume_server_request_with_same_id() -> None:
    """Un request servidor→cliente no satisface una deuda aunque reutilice exactamente su id."""
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", SERVER_REQUEST_SAME_PENDING_ID_CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    raw_line = '{"jsonrpc":"2.0","id":1,"method":"ping"}'
    silent_lines = [
        '{"jsonrpc":"2.0","method":"notify/observe-server-request"}',
        '{"jsonrpc":"2.0","method":"notify/consume-real-response"}',
        '{"jsonrpc":"2.0","method":"notify/after-pending-cleared"}',
    ]
    stderr = ""
    try:
        expired = instance.raw_line(raw_line, timeout=TIMEOUT)
        server_request = instance.raw_line(silent_lines[0], timeout=TIMEOUT)
        pending_after_request = list(getattr(instance, "_raw_pending_ids", []))
        real_response = instance.raw_line(silent_lines[1], timeout=TIMEOUT)
        pending_after_real = list(getattr(instance, "_raw_pending_ids", []))
        after_cleared = instance.raw_line(silent_lines[2], timeout=TIMEOUT)
        process_alive = process.poll() is None
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()

    try:
        observed = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        raise AssertionError(f"el hijo no acreditó la secuencia request/response: {stderr!r}") from error
    expected_request = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "server/fresh",
        "params": {"marker": "NOT_A_RESPONSE"},
    }
    expected_after = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "server/after-real",
        "params": {"marker": "AFTER_PENDING_CLEARED"},
    }
    failures: list[str] = []
    if expired is not None:
        failures.append(f"la request raw id=1 debía vencer sin frame: {expired!r}")
    if server_request != expected_request:
        failures.append(
            f"el request servidor→cliente fresco se ocultó como respuesta pendiente: {server_request!r}"
        )
    if pending_after_request != [1]:
        failures.append(f"el request fresco consumió indebidamente la deuda id=1: {pending_after_request!r}")
    if real_response is not None:
        failures.append(f"la respuesta real pendiente debía drenarse, no observarse: {real_response!r}")
    if pending_after_real:
        failures.append(f"la respuesta real no limpió la deuda id=1: {pending_after_real!r}")
    if after_cleared != expected_after:
        failures.append(f"tras limpiar la deuda, el request id=1 debe observarse: {after_cleared!r}")
    if observed != {
        "raw": raw_line,
        "silent_1": silent_lines[0],
        "silent_2": silent_lines[1],
        "silent_3": silent_lines[2],
    }:
        failures.append(f"el Popen no acreditó las cuatro líneas exactas: {observed!r}")
    if not process_alive:
        failures.append("el hijo/stdout no seguían vivos al cerrar la secuencia")
    assert not failures, "mensaje con id pendiente mal clasificado como respuesta:\n- " + "\n- ".join(failures)


def check_pending_id_consumption_requires_strict_jsonrpc_response_shape() -> None:
    """Solo JSON-RPC sin method y con exactamente result XOR error satisface una deuda."""
    harness = load_harness()
    cases = [
        (
            "notification",
            "nonresponse",
            {"jsonrpc": "2.0", "method": "server/notification"},
        ),
        (
            "wrong-jsonrpc",
            "nonresponse",
            {"jsonrpc": "1.0", "id": 1, "result": {"marker": "MALFORMED"}},
        ),
        (
            "result-and-error",
            "nonresponse",
            {"jsonrpc": "2.0", "id": 1, "result": {}, "error": {"code": -1}},
        ),
        (
            "neither-result-nor-error",
            "nonresponse",
            {"jsonrpc": "2.0", "id": 1, "extra": "MALFORMED"},
        ),
        (
            "valid-result",
            "response",
            {"jsonrpc": "2.0", "id": 1, "result": {"marker": "VALID_RESULT"}},
        ),
        (
            "valid-error",
            "response",
            {"jsonrpc": "2.0", "id": 1, "error": {"code": -32000, "message": "VALID_ERROR"}},
        ),
    ]
    raw_line = '{"jsonrpc":"2.0","id":1,"method":"ping"}'
    silent_1 = '{"jsonrpc":"2.0","method":"notify/shape-candidate"}'
    silent_2 = '{"jsonrpc":"2.0","method":"notify/shape-control"}'
    failures: list[str] = []

    for label, kind, candidate in cases:
        process = subprocess.Popen(
            [
                sys.executable,
                "-u",
                "-c",
                PENDING_RESPONSE_SHAPE_MATRIX_CHILD,
                kind,
                json.dumps(candidate, separators=(",", ":")),
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        instance = object.__new__(harness.LodestarSession)
        instance.proc = process
        stderr = ""
        try:
            expired = instance.raw_line(raw_line, timeout=TIMEOUT)
            candidate_result = instance.raw_line(silent_1, timeout=TIMEOUT)
            control_result = instance.raw_line(silent_2, timeout=TIMEOUT)
            process_alive = process.poll() is None
        finally:
            if process.stdin is not None:
                process.stdin.close()
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            if process.stderr is not None:
                stderr = process.stderr.read()

        try:
            observed = json.loads(stderr.strip())
        except json.JSONDecodeError:
            failures.append(f"{label}: el hijo no acreditó la matriz: {stderr!r}")
            continue
        if expired is not None:
            failures.append(f"{label}: la request id=1 no venció: {expired!r}")
        if observed.get("candidate") != candidate or observed.get("kind") != kind:
            failures.append(f"{label}: candidato no acreditado: {observed!r}")
        if observed.get("raw") != raw_line or observed.get("silent_1") != silent_1 or observed.get("silent_2") != silent_2:
            failures.append(f"{label}: líneas de usuario no acreditadas: {observed!r}")
        if kind == "nonresponse":
            if candidate_result != candidate:
                failures.append(f"{label}: mensaje no-response quedó oculto: {candidate_result!r}")
            if control_result is not None:
                failures.append(
                    f"{label}: la respuesta real pendiente no se drenó después: {control_result!r}"
                )
        else:
            expected_control = {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "server/after-valid-response",
                "params": {"marker": "PENDING_WAS_CLEARED"},
            }
            if candidate_result is not None:
                failures.append(f"{label}: la respuesta válida pendiente no se drenó: {candidate_result!r}")
            if control_result != expected_control:
                failures.append(f"{label}: la deuda no quedó limpia tras respuesta válida: {control_result!r}")
        if not process_alive:
            failures.append(f"{label}: el hijo/stdout no seguían vivos")

    assert not failures, "forma de respuesta JSON-RPC mal validada:\n- " + "\n- ".join(failures)


def run_reused_pending_raw_id_case(mode: str):
    """Ejecuta id=1 vencido, segunda request y postcheck sobre un pipe coordinado."""
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", REUSED_PENDING_RAW_ID_CHILD, mode],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    first_line = '{"jsonrpc":"2.0","id":1,"method":"ping"}'
    second_id = 2 if mode == "noncolliding-id-2" else 1
    second_line = json.dumps(
        {"jsonrpc": "2.0", "id": second_id, "method": "ping"}, separators=(",", ":")
    )
    postcheck_line = '{"jsonrpc":"2.0","method":"notify/reuse-postcheck"}'
    stderr = ""
    try:
        expired = instance.raw_line(first_line, timeout=TIMEOUT)
        second = instance.raw_line(second_line, timeout=0.5)
        pending_after_second = list(getattr(instance, "_raw_pending_ids", []))
        postcheck = instance.raw_line(postcheck_line, timeout=TIMEOUT)
        pending_after_postcheck = list(getattr(instance, "_raw_pending_ids", []))
        process_alive = process.poll() is None
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()

    try:
        observed = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        raise AssertionError(f"el hijo no acreditó first/barrera/second: {stderr!r}") from error
    failures: list[str] = []
    if expired is not None:
        failures.append(f"la primera raw id=1 debía vencer: {expired!r}")
    if observed.get("first") != first_line or observed.get("second") != second_line:
        failures.append(f"el hijo no acreditó ambas requests raw exactas: {observed!r}")
    if observed.get("postcheck") != postcheck_line:
        failures.append(f"el hijo no acreditó la operación postcheck: {observed!r}")
    if not process_alive:
        failures.append("el hijo/stdout no seguían vivos tras la secuencia")
    return second, postcheck, pending_after_second, pending_after_postcheck, observed, failures


def check_reused_pending_raw_id_barrier_drains_stale_before_fresh_second() -> None:
    """Reutilizar id=1 exige barrera: STALE_FIRST no puede satisfacer la segunda raw."""
    second, postcheck, pending_second, pending_post, observed, failures = (
        run_reused_pending_raw_id_case("stale-before-ack")
    )
    barrier = json.loads(observed["barrier"]) if observed.get("barrier") else None
    if not isinstance(barrier, dict) or barrier.get("method") != "ping":
        failures.append(f"la segunda raw id=1 se envió sin barrera correlacionable: {observed!r}")
    if second != {
        "jsonrpc": "2.0",
        "id": 1,
        "result": {"marker": "FRESH_SECOND"},
    }:
        failures.append(f"la segunda raw aceptó STALE_FIRST: {second!r}")
    if pending_second:
        failures.append(f"la barrera no extinguió la deuda id=1 previa: {pending_second!r}")
    if postcheck != {
        "jsonrpc": "2.0",
        "id": 1,
        "result": {"marker": "AFTER_SEQUENCE"},
    }:
        failures.append(f"id=1 siguió marcado pendiente tras la segunda respuesta: {postcheck!r}")
    if pending_post:
        failures.append(f"el postcheck dejó deuda residual: {pending_post!r}")
    expected_emitted = ["STALE_FIRST", "BARRIER_ACK", "FRESH_SECOND", "AFTER_SEQUENCE"]
    if observed.get("emitted") != expected_emitted:
        failures.append(f"el hijo no acreditó el orden stale/ACK/fresh: {observed!r}")
    assert not failures, "reutilización raw aceptó respuesta stale:\n- " + "\n- ".join(failures)


def check_reused_pending_raw_id_barrier_ack_extinguishes_unresolved_debt() -> None:
    """Si no llega STALE_FIRST, el ACK cierra la deuda antes de enviar el id=1 nuevo."""
    second, postcheck, pending_second, pending_post, observed, failures = (
        run_reused_pending_raw_id_case("ack-without-stale")
    )
    if observed.get("barrier") is None:
        failures.append(f"id=1 se reutilizó sin esperar un ACK de barrera: {observed!r}")
    if not isinstance(second, dict) or second.get("result", {}).get("marker") != "FRESH_SECOND":
        failures.append(f"la segunda raw no alcanzó FRESH_SECOND: {second!r}")
    if pending_second:
        failures.append(f"el ACK no extinguió la deuda id=1 sin respuesta tardía: {pending_second!r}")
    if not isinstance(postcheck, dict) or postcheck.get("result", {}).get("marker") != "AFTER_SEQUENCE":
        failures.append(f"la deuda extinguida consumió el postcheck id=1: {postcheck!r}")
    if pending_post:
        failures.append(f"quedó deuda residual tras el postcheck: {pending_post!r}")
    assert not failures, "ACK no cerró deuda raw reutilizada:\n- " + "\n- ".join(failures)


def check_noncolliding_raw_id_does_not_barrier_other_pending_id() -> None:
    """Una deuda id=1 no fuerza barrera para la nueva raw id=2."""
    second, postcheck, pending_second, pending_post, observed, failures = (
        run_reused_pending_raw_id_case("noncolliding-id-2")
    )
    if observed.get("barrier") is not None:
        failures.append(f"raw id=2 no colisionante envió barrera innecesaria: {observed!r}")
    if second != {
        "jsonrpc": "2.0",
        "id": 2,
        "result": {"marker": "FRESH_ID2"},
    }:
        failures.append(f"raw id=2 no alcanzó su respuesta fresca: {second!r}")
    if pending_second != [1]:
        failures.append(f"raw id=2 alteró la deuda ajena id=1: {pending_second!r}")
    if postcheck is not None:
        failures.append(f"la respuesta postcheck id=1 pendiente debía drenarse: {postcheck!r}")
    if pending_post:
        failures.append(f"la respuesta real id=1 no limpió la deuda: {pending_post!r}")
    assert not failures, "raw no colisionante sincronizó deuda ajena:\n- " + "\n- ".join(failures)


def run_pending_response_shape_case(kind: str, candidate):
    """Ejecuta candidato y control real sobre una deuda id=1 con un Popen vivo."""
    harness = load_harness()
    process = subprocess.Popen(
        [
            sys.executable,
            "-u",
            "-c",
            PENDING_RESPONSE_SHAPE_MATRIX_CHILD,
            kind,
            json.dumps(candidate, separators=(",", ":")),
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    raw_line = '{"jsonrpc":"2.0","id":1,"method":"ping"}'
    silent_1 = '{"jsonrpc":"2.0","method":"notify/error-shape-candidate"}'
    silent_2 = '{"jsonrpc":"2.0","method":"notify/error-shape-control"}'
    stderr = ""
    try:
        expired = instance.raw_line(raw_line, timeout=TIMEOUT)
        candidate_result = instance.raw_line(silent_1, timeout=TIMEOUT)
        pending_after_candidate = list(getattr(instance, "_raw_pending_ids", []))
        control_result = instance.raw_line(silent_2, timeout=TIMEOUT)
        pending_after_control = list(getattr(instance, "_raw_pending_ids", []))
        process_alive = process.poll() is None
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()

    try:
        observed = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        raise AssertionError(f"el hijo no acreditó candidato/control error: {stderr!r}") from error
    base_failures: list[str] = []
    if expired is not None:
        base_failures.append(f"la request id=1 debía vencer sin frame: {expired!r}")
    if observed.get("kind") != kind or observed.get("candidate") != candidate:
        base_failures.append(f"el hijo no acreditó el candidato exacto: {observed!r}")
    if observed.get("raw") != raw_line or observed.get("silent_1") != silent_1 or observed.get("silent_2") != silent_2:
        base_failures.append(f"el hijo no acreditó las tres líneas: {observed!r}")
    if not process_alive:
        base_failures.append("el hijo/stdout no seguían vivos tras el control")
    return (
        candidate_result,
        control_result,
        pending_after_candidate,
        pending_after_control,
        base_failures,
    )


def check_pending_id_preserves_error_null_until_real_valid_response() -> None:
    """``error:null`` no satisface id=1; la respuesta válida posterior sí."""
    candidate = {"jsonrpc": "2.0", "id": 1, "error": None}
    observed, control, pending_candidate, pending_control, failures = (
        run_pending_response_shape_case("nonresponse", candidate)
    )
    if observed != candidate:
        failures.append(f"error:null se ocultó como si fuera respuesta válida: {observed!r}")
    if pending_candidate != [1]:
        failures.append(f"error:null consumió la deuda id=1: {pending_candidate!r}")
    if control is not None:
        failures.append(f"la respuesta válida real no se drenó después: {control!r}")
    if pending_control:
        failures.append(f"la respuesta válida real no limpió la deuda: {pending_control!r}")
    assert not failures, "error:null mal clasificado como respuesta:\n- " + "\n- ".join(failures)


def check_pending_id_validates_error_object_and_accepts_any_result_json() -> None:
    """Error exige code int exacto y message string; result admite cualquier JSON."""
    invalid_errors = [
        ("error-null", None),
        ("error-string", "not-an-object"),
        ("error-array", [-32000, "not-an-object"]),
        ("error-empty", {}),
        ("error-missing-code", {"message": "missing code"}),
        ("error-missing-message", {"code": -32000}),
        ("error-bool-code", {"code": True, "message": "bool is not int"}),
        ("error-float-code", {"code": -32000.0, "message": "float is not exact int"}),
        ("error-nonstring-message", {"code": -32000, "message": 7}),
    ]
    valid_errors = [
        ("valid-error", {"code": -32000, "message": "valid"}),
        (
            "valid-error-with-data",
            {"code": -32001, "message": "valid with data", "data": {"detail": [1, None]}},
        ),
    ]
    result_values = [
        ("result-null", None),
        ("result-bool", False),
        ("result-int", 7),
        ("result-float", 1.25),
        ("result-string", "ok"),
        ("result-array", [1, None, "x"]),
        ("result-object", {"nested": {"ok": True}}),
    ]
    failures: list[str] = []

    for label, error_value in invalid_errors:
        candidate = {"jsonrpc": "2.0", "id": 1, "error": error_value}
        observed, control, pending_candidate, pending_control, base = (
            run_pending_response_shape_case("nonresponse", candidate)
        )
        failures.extend(f"{label}: {failure}" for failure in base)
        if observed != candidate:
            failures.append(f"{label}: error inválido quedó oculto: {observed!r}")
        if pending_candidate != [1]:
            failures.append(f"{label}: error inválido consumió id=1: {pending_candidate!r}")
        if control is not None or pending_control:
            failures.append(
                f"{label}: la respuesta real posterior no cerró la deuda: "
                f"control={control!r} pending={pending_control!r}"
            )

    valid_candidates = [
        (label, {"jsonrpc": "2.0", "id": 1, "error": error_value})
        for label, error_value in valid_errors
    ] + [
        (label, {"jsonrpc": "2.0", "id": 1, "result": result_value})
        for label, result_value in result_values
    ]
    expected_control = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "server/after-valid-response",
        "params": {"marker": "PENDING_WAS_CLEARED"},
    }
    for label, candidate in valid_candidates:
        observed, control, pending_candidate, pending_control, base = (
            run_pending_response_shape_case("response", candidate)
        )
        failures.extend(f"{label}: {failure}" for failure in base)
        if observed is not None:
            failures.append(f"{label}: respuesta válida no se drenó: {observed!r}")
        if pending_candidate:
            failures.append(f"{label}: respuesta válida no limpió id=1: {pending_candidate!r}")
        if control != expected_control:
            failures.append(f"{label}: mensaje posterior quedó oculto: {control!r}")
        if pending_control:
            failures.append(f"{label}: reapareció deuda tras control: {pending_control!r}")

    assert not failures, "semántica de error/result JSON-RPC incorrecta:\n- " + "\n- ".join(failures)


def check_rpc_preserves_nonfinite_response_frames_as_unparseable_fifo() -> None:
    """rpc ignora JSON no finito, alcanza su respuesta y conserva el raw en FIFO."""
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", NONFINITE_RPC_RESPONSE_CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    instance._next_id = 10
    raw_lines = [
        '{"jsonrpc":"2.0","method":"notify/nonfinite-fifo-1"}',
        '{"jsonrpc":"2.0","method":"notify/nonfinite-fifo-2"}',
        '{"jsonrpc":"2.0","method":"notify/nonfinite-fifo-3"}',
    ]
    stderr = ""
    try:
        rpc_response = instance.rpc("ping")
        observed_fifo = [instance.raw_line(line, timeout=TIMEOUT) for line in raw_lines]
        process_alive = process.poll() is None
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()

    try:
        observed = json.loads(stderr.strip())
    except json.JSONDecodeError as error:
        raise AssertionError(f"el hijo no acreditó rpc+FIFO no finito: {stderr!r}") from error
    expected_frames = [
        '{"jsonrpc":"2.0","id":10,"result":NaN}',
        '{"jsonrpc":"2.0","id":10,"result":{"nested":Infinity}}',
        '{"jsonrpc":"2.0","id":10,"result":[0,-Infinity]}',
    ]
    failures: list[str] = []
    if rpc_response != {
        "jsonrpc": "2.0",
        "id": 10,
        "result": {"marker": "VALID_RPC"},
    }:
        failures.append(f"rpc correlacionó JSON no finito en vez de VALID_RPC: {rpc_response!r}")
    expected_fifo = [{"unparseable_response": frame} for frame in expected_frames]
    if observed_fifo != expected_fifo:
        failures.append(
            f"los raws no finitos no quedaron observables en FIFO: {observed_fifo!r}"
        )
    if observed.get("rpc_id") != 10 or observed.get("frames") != expected_frames:
        failures.append(f"el hijo no acreditó los tres frames exactos: {observed!r}")
    if observed.get("raw_lines") != raw_lines:
        failures.append(f"el hijo no recibió las tres raw posteriores: {observed!r}")
    if not process_alive:
        failures.append("el hijo/stdout no seguían vivos tras consumir el FIFO")
    assert not failures, "rpc aceptó constantes JSON no finitas:\n- " + "\n- ".join(failures)


def run_strict_raw_response_text_case(kind: str, candidate: str):
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", STRICT_RAW_RESPONSE_TEXT_CHILD, kind, candidate],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    raw_line = '{"jsonrpc":"2.0","id":1,"method":"ping"}'
    silent_1 = '{"jsonrpc":"2.0","method":"notify/strict-response-candidate"}'
    silent_2 = '{"jsonrpc":"2.0","method":"notify/strict-response-control"}'
    stderr = ""
    try:
        expired = instance.raw_line(raw_line, timeout=TIMEOUT)
        candidate_result = instance.raw_line(silent_1, timeout=TIMEOUT)
        pending_candidate = list(getattr(instance, "_raw_pending_ids", []))
        control_result = instance.raw_line(silent_2, timeout=TIMEOUT)
        pending_control = list(getattr(instance, "_raw_pending_ids", []))
        process_alive = process.poll() is None
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()
    observed = json.loads(stderr.strip())
    failures = []
    if expired is not None:
        failures.append(f"request id1 no venció: {expired!r}")
    if observed.get("candidate") != candidate or observed.get("kind") != kind:
        failures.append(f"candidato raw no acreditado: {observed!r}")
    if observed.get("raw") != raw_line or observed.get("silent_1") != silent_1 or observed.get("silent_2") != silent_2:
        failures.append(f"líneas de usuario no acreditadas: {observed!r}")
    if not process_alive:
        failures.append("hijo/stdout no seguían vivos")
    return candidate_result, control_result, pending_candidate, pending_control, failures


def run_strict_raw_input_case(kind: str, line: str):
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", STRICT_RAW_INPUT_CHILD, kind],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    stderr = ""
    try:
        classification = instance._raw_request_kind(line)
        response = instance.raw_line(line, timeout=TIMEOUT)
        pending = list(getattr(instance, "_raw_pending_ids", []))
        needs_resync = getattr(instance, "_raw_needs_resync", False)
        process_alive = process.poll() is None
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()
    observed = json.loads(stderr.strip())
    failures = []
    if observed != {"kind": kind, "line": line}:
        failures.append(f"input raw no acreditado: {observed!r}")
    if not process_alive:
        failures.append("hijo/stdout no seguían vivos")
    return classification, response, pending, needs_resync, failures


def check_raw_line_strict_json_rejects_nonfinite_responses_and_inputs() -> None:
    """raw_line usa JSON estricto tanto para frames entrantes como para input crudo."""
    nonfinite_responses = [
        ("NaN", '{"jsonrpc":"2.0","id":1,"result":NaN}'),
        ("Infinity", '{"jsonrpc":"2.0","id":1,"result":{"nested":Infinity}}'),
        ("-Infinity", '{"jsonrpc":"2.0","id":1,"result":[-Infinity]}'),
    ]
    valid_responses = [
        ("finite-float", '{"jsonrpc":"2.0","id":1,"result":1.25}'),
        ("null", '{"jsonrpc":"2.0","id":1,"result":null}'),
    ]
    failures: list[str] = []
    for label, candidate in nonfinite_responses:
        observed, control, pending_candidate, pending_control, base = (
            run_strict_raw_response_text_case("nonfinite", candidate)
        )
        failures.extend(f"response-{label}: {failure}" for failure in base)
        if observed != {"unparseable_response": candidate}:
            failures.append(f"response-{label}: no preservó raw ilegible: {observed!r}")
        if pending_candidate != [1]:
            failures.append(f"response-{label}: consumió deuda id1: {pending_candidate!r}")
        if control is not None or pending_control:
            failures.append(
                f"response-{label}: respuesta real no cerró deuda: "
                f"control={control!r} pending={pending_control!r}"
            )
    expected_after_valid = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "server/after-valid",
        "params": {"marker": "PENDING_CLEARED"},
    }
    for label, candidate in valid_responses:
        observed, control, pending_candidate, pending_control, base = (
            run_strict_raw_response_text_case("valid", candidate)
        )
        failures.extend(f"response-{label}: {failure}" for failure in base)
        if observed is not None or pending_candidate:
            failures.append(
                f"response-{label}: respuesta JSON válida no se consumió: "
                f"observed={observed!r} pending={pending_candidate!r}"
            )
        if control != expected_after_valid or pending_control:
            failures.append(f"response-{label}: control posterior oculto: {control!r}")

    nonfinite_inputs = [
        ("NaN", '{"jsonrpc":"2.0","id":7,"method":"ping","params":{"value":NaN}}'),
        ("Infinity", '{"jsonrpc":"2.0","id":7,"method":"ping","params":{"value":Infinity}}'),
        ("-Infinity", '{"jsonrpc":"2.0","id":7,"method":"ping","params":{"value":-Infinity}}'),
    ]
    valid_inputs = [
        ("finite-float", '{"jsonrpc":"2.0","id":7,"method":"ping","params":{"value":1.25}}'),
        ("null", '{"jsonrpc":"2.0","id":7,"method":"ping","params":{"value":null}}'),
    ]
    for label, line in nonfinite_inputs:
        classification, response, pending, needs_resync, base = run_strict_raw_input_case(
            "nonfinite", line
        )
        failures.extend(f"input-{label}: {failure}" for failure in base)
        if classification != ("silence", None):
            failures.append(f"input-{label}: clasificó como request válida: {classification!r}")
        if response is not None or pending or needs_resync:
            failures.append(
                f"input-{label}: JSON no finito dejó estado de request: "
                f"response={response!r} pending={pending!r} resync={needs_resync!r}"
            )
    for label, line in valid_inputs:
        classification, response, pending, needs_resync, base = run_strict_raw_input_case(
            "valid", line
        )
        failures.extend(f"input-{label}: {failure}" for failure in base)
        if classification != ("request", 7):
            failures.append(f"input-{label}: JSON finito no clasificó request: {classification!r}")
        if response != {
            "jsonrpc": "2.0",
            "id": 7,
            "result": {"marker": "VALID_INPUT"},
        } or pending or needs_resync:
            failures.append(
                f"input-{label}: control válido falló: response={response!r} "
                f"pending={pending!r} resync={needs_resync!r}"
            )

    assert not failures, "JSON no finito aceptado por raw_line:\n- " + "\n- ".join(failures)


def check_current_invalid_preserves_fresh_idless_before_attributable_32600_fifo() -> None:
    """Un -32601 fresco no puede adelantarse al -32600 atribuible al invalid actual."""
    harness = load_harness()
    invalid_line = '{"foo":"current-invalid"}'
    follow_line = '{"jsonrpc":"2.0","method":"notify/observe-fresh-idless"}'
    failures: list[str] = []
    for id_form in ("absent", "null"):
        process = subprocess.Popen(
            [
                sys.executable,
                "-u",
                "-c",
                CURRENT_INVALID_FRESH_BEFORE_ATTRIBUTABLE_CHILD,
                id_form,
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        instance = object.__new__(harness.LodestarSession)
        instance.proc = process
        stderr = ""
        try:
            current = instance.raw_line(invalid_line, timeout=0.5)
            fresh = instance.raw_line(follow_line, timeout=TIMEOUT)
            process_alive = process.poll() is None
        finally:
            if process.stdin is not None:
                process.stdin.close()
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            if process.stderr is not None:
                stderr = process.stderr.read()

        try:
            observed = json.loads(stderr.strip())
        except json.JSONDecodeError:
            failures.append(f"{id_form}: hijo no acreditó la secuencia: {stderr!r}")
            continue
        frames = [json.loads(frame) for frame in observed.get("frames", [])]
        if observed.get("invalid") != invalid_line or observed.get("follow") != follow_line:
            failures.append(f"{id_form}: líneas de usuario no acreditadas: {observed!r}")
        if len(frames) != 2:
            failures.append(f"{id_form}: frames emitidos no acreditados: {observed!r}")
            continue
        if current != frames[1]:
            failures.append(
                f"{id_form}: invalid actual no devolvió su -32600 atribuible: {current!r}"
            )
        if fresh != frames[0]:
            failures.append(
                f"{id_form}: -32601 fresco no quedó observable en backlog FIFO: {fresh!r}"
            )
        if frames[0].get("error", {}).get("data", {}).get("marker") != "FRESH_IDLESS_32601":
            failures.append(f"{id_form}: primer frame no era el fresco esperado: {frames[0]!r}")
        if frames[1].get("error", {}).get("data", {}).get("marker") != "ATTRIBUTABLE_CURRENT":
            failures.append(f"{id_form}: segundo frame no era atribuible: {frames[1]!r}")
        if id_form == "absent" and any("id" in frame for frame in frames):
            failures.append(f"{id_form}: rmcp idless no debe requerir clave id: {frames!r}")
        if id_form == "null" and any(frame.get("id", "missing") is not None for frame in frames):
            failures.append(f"{id_form}: ambos frames debían llevar id:null: {frames!r}")
        if not process_alive:
            failures.append(f"{id_form}: hijo/stdout no seguían vivos")

    assert not failures, "invalid actual invirtió frame fresco y atribuible:\n- " + "\n- ".join(failures)


def run_current_invalid_correlation_case(kind: str, candidate: str):
    harness = load_harness()
    process = subprocess.Popen(
        [
            sys.executable,
            "-u",
            "-c",
            CURRENT_INVALID_CORRELATION_MATRIX_CHILD,
            kind,
            candidate,
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    invalid_line = '{"foo":"matrix-current-invalid"}'
    follow_line = '{"jsonrpc":"2.0","method":"notify/matrix-idless-follow"}'
    stderr = ""
    try:
        current = instance.raw_line(invalid_line, timeout=0.5)
        follow = instance.raw_line(follow_line, timeout=TIMEOUT)
        process_alive = process.poll() is None
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()
    observed = json.loads(stderr.strip())
    failures = []
    if observed.get("kind") != kind or observed.get("candidate") != candidate:
        failures.append(f"candidato no acreditado: {observed!r}")
    if observed.get("invalid") != invalid_line or observed.get("follow") != follow_line:
        failures.append(f"líneas no acreditadas: {observed!r}")
    if not process_alive:
        failures.append("hijo/stdout no seguían vivos")
    return current, follow, observed.get("attributable"), failures


def check_current_invalid_correlates_only_strict_idless_32600_error() -> None:
    """Invalid actual acepta sólo error JSON-RPC estricto -32600, con id ausente o null."""
    attributable_cases = [
        (
            "absent",
            '{"jsonrpc":"2.0","error":{"code":-32600,"message":"valid absent"}}',
        ),
        (
            "null",
            '{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"valid null"}}',
        ),
    ]
    nonattributable_cases = [
        ("parse-absent", '{"jsonrpc":"2.0","error":{"code":-32700,"message":"parse"}}'),
        ("parse-null", '{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse"}}'),
        ("method-absent", '{"jsonrpc":"2.0","error":{"code":-32601,"message":"method"}}'),
        ("method-null", '{"jsonrpc":"2.0","id":null,"error":{"code":-32601,"message":"method"}}'),
        ("result-absent", '{"jsonrpc":"2.0","result":{"marker":"fresh result"}}'),
        ("result-null", '{"jsonrpc":"2.0","id":null,"result":null}'),
        ("code-float", '{"jsonrpc":"2.0","error":{"code":-32600.0,"message":"float"}}'),
        ("missing-message", '{"jsonrpc":"2.0","id":null,"error":{"code":-32600}}'),
        ("unparseable", '{not json'),
    ]
    failures: list[str] = []
    for label, candidate in attributable_cases:
        current, follow, extra, base = run_current_invalid_correlation_case(
            "attributable", candidate
        )
        failures.extend(f"{label}: {failure}" for failure in base)
        expected = json.loads(candidate)
        if current != expected:
            failures.append(f"{label}: -32600 estricto no correlacionó: {current!r}")
        if follow is not None or extra is not None:
            failures.append(f"{label}: control dejó backlog inesperado: {follow!r}")

    expected_attributable = {
        "jsonrpc": "2.0",
        "error": {
            "code": -32600,
            "message": "matrix attributable",
            "data": {"marker": "ATTRIBUTABLE_MATRIX"},
        },
    }
    for label, candidate in nonattributable_cases:
        current, follow, attributable_raw, base = run_current_invalid_correlation_case(
            "nonattributable", candidate
        )
        failures.extend(f"{label}: {failure}" for failure in base)
        if current != expected_attributable:
            failures.append(f"{label}: candidato fresco ocupó la respuesta actual: {current!r}")
        expected_follow = (
            {"unparseable_response": candidate}
            if label == "unparseable"
            else json.loads(candidate)
        )
        if follow != expected_follow:
            failures.append(f"{label}: candidato no quedó observable FIFO: {follow!r}")
        if attributable_raw is None or json.loads(attributable_raw) != expected_attributable:
            failures.append(f"{label}: -32600 atribuible no acreditado: {attributable_raw!r}")

    assert not failures, "invalid correlacionó frame idless no atribuible:\n- " + "\n- ".join(failures)


def run_idless_params_classifier_case(behavior: str, id_form: str, line: str):
    harness = load_harness()
    process = subprocess.Popen(
        [
            sys.executable,
            "-u",
            "-c",
            IDLESS_PARAMS_CLASSIFIER_CHILD,
            behavior,
            id_form,
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    stderr = ""
    try:
        classification = instance._raw_request_kind(line)
        response = instance.raw_line(line, timeout=TIMEOUT)
        pending_ids = list(getattr(instance, "_raw_pending_ids", []))
        pending_idless = getattr(instance, "_raw_pending_idless", 0)
        needs_resync = getattr(instance, "_raw_needs_resync", False)
        process_alive = process.poll() is None
    finally:
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()
    observed = json.loads(stderr.strip())
    failures = []
    if observed != {"behavior": behavior, "id_form": id_form, "line": line}:
        failures.append(f"hijo no acreditó input exacto: {observed!r}")
    if not process_alive:
        failures.append("hijo/stdout no seguían vivos")
    return (
        classification,
        response,
        pending_ids,
        pending_idless,
        needs_resync,
        failures,
    )


def check_absent_and_null_id_invalid_params_are_equivalent_invalid_requests() -> None:
    """rmcp trata params no-objeto con id ausente/null como invalid idless -32600."""
    params_cases = [
        ("number", "7"),
        ("string", '"bad"'),
        ("list", "[1,2]"),
    ]
    failures: list[str] = []
    for params_label, params_json in params_cases:
        for id_form in ("absent", "null"):
            id_field = "" if id_form == "absent" else '"id":null,'
            line = (
                '{"jsonrpc":"2.0",'
                + id_field
                + '"method":"ping","params":'
                + params_json
                + "}"
            )
            classification, response, pending_ids, pending_idless, resync, base = (
                run_idless_params_classifier_case("invalid-response", id_form, line)
            )
            prefix = f"{id_form}/{params_label}"
            failures.extend(f"{prefix}: {failure}" for failure in base)
            if classification != ("invalid", None):
                failures.append(f"{prefix}: clasificación divergente: {classification!r}")
            if not isinstance(response, dict) or response.get("error", {}).get("code") != -32600:
                failures.append(f"{prefix}: no observó -32600 estricto: {response!r}")
            if id_form == "absent" and "id" in response:
                failures.append(f"{prefix}: respuesta idless ausente ganó clave id: {response!r}")
            if id_form == "null" and response.get("id", "missing") is not None:
                failures.append(f"{prefix}: respuesta debía conservar id:null: {response!r}")
            if pending_ids or pending_idless or resync:
                failures.append(
                    f"{prefix}: respuesta inmediata dejó deuda: ids={pending_ids!r} "
                    f"idless={pending_idless!r} resync={resync!r}"
                )

    for id_form in ("absent", "null"):
        id_field = "" if id_form == "absent" else '"id":null,'
        line = (
            '{"jsonrpc":"2.0",'
            + id_field
            + '"method":"notify/normal","params":{"ok":true}}'
        )
        classification, response, pending_ids, pending_idless, resync, base = (
            run_idless_params_classifier_case("silence", id_form, line)
        )
        failures.extend(f"notification-{id_form}: {failure}" for failure in base)
        if classification != ("silence", None):
            failures.append(
                f"notification-{id_form}: idless normal no clasificó silencio: {classification!r}"
            )
        if response is not None or pending_ids or pending_idless or resync:
            failures.append(
                f"notification-{id_form}: silencio normal dejó salida/deuda: "
                f"response={response!r} ids={pending_ids!r} idless={pending_idless!r} "
                f"resync={resync!r}"
            )

    assert not failures, "id ausente/null divergen en params idless:\n- " + "\n- ".join(failures)


def run_resync_failure_case(operation: str, mode: str, exit_code: int):
    harness = load_harness()
    process = subprocess.Popen(
        [sys.executable, "-u", "-c", RESYNC_FAILURE_CHILD, mode, str(exit_code)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    if operation == "rpc":
        instance._next_id = 10
    first_line = '{"foo":"invalid-needs-resync"}'
    public_line = '{"jsonrpc":"2.0","method":"notify/must-not-send"}'
    outcome = None
    stderr = ""
    try:
        expired = instance.raw_line(first_line, timeout=TIMEOUT)
        try:
            if operation == "raw":
                value = instance.raw_line(public_line, timeout=TIMEOUT)
            elif operation == "rpc":
                value = instance.rpc("ping")
            else:
                raise AssertionError(f"operación desconocida: {operation}")
        except RuntimeError as error:
            outcome = ("error", str(error))
        else:
            outcome = ("return", value)
    finally:
        if process.stdin is not None:
            try:
                process.stdin.close()
            except (BrokenPipeError, OSError):
                pass
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if process.stderr is not None:
            stderr = process.stderr.read()
    observed = json.loads(stderr.strip())
    failures = []
    if expired is not None:
        failures.append(f"invalid inicial no venció: {expired!r}")
    if observed.get("first") != first_line or observed.get("mode") != mode:
        failures.append(f"hijo no acreditó invalid inicial: {observed!r}")
    try:
        barrier = json.loads(observed.get("barrier", "null"))
    except json.JSONDecodeError:
        barrier = None
    if (
        not isinstance(barrier, dict)
        or barrier.get("jsonrpc") != "2.0"
        or barrier.get("method") != "ping"
        or not isinstance(barrier.get("id"), str)
    ):
        failures.append(f"hijo no acreditó ping de resync correlacionable: {observed!r}")
    if observed.get("rest"):
        failures.append(
            f"se escribió operación pública tras fallar resync: {observed['rest']!r}"
        )
    return outcome, observed, failures


def check_resync_timeout_or_eof_raises_before_raw_or_rpc_public_send() -> None:
    """Timeout/EOF de barrera es RuntimeError de resync y aborta el envío público."""
    cases = [
        ("raw-timeout-alive", "raw", "timeout", 0),
        ("raw-eof", "raw", "eof", 23),
        ("rpc-eof", "rpc", "eof", 24),
    ]
    failures: list[str] = []
    for label, operation, mode, exit_code in cases:
        outcome, observed, base = run_resync_failure_case(operation, mode, exit_code)
        failures.extend(f"{label}: {failure}" for failure in base)
        if not outcome or outcome[0] != "error":
            failures.append(f"{label}: fallo de resync devolvió valor normal: {outcome!r}")
            continue
        message = outcome[1].lower()
        if "resync" not in message:
            failures.append(f"{label}: RuntimeError no nombra resync: {outcome[1]!r}")
        if mode == "timeout" and "timeout" not in message:
            failures.append(f"{label}: error no identifica timeout: {outcome[1]!r}")
        if mode == "eof" and "eof" not in message and "exit" not in message:
            failures.append(f"{label}: error no identifica EOF/exit: {outcome[1]!r}")
        if mode == "eof" and observed.get("rest") != []:
            failures.append(f"{label}: hijo EOF acreditó líneas públicas: {observed!r}")

    assert not failures, "fallo de resync no abortó operación pública:\n- " + "\n- ".join(failures)


def run_resync_write_error_matrix(operation: str) -> None:
    """Exige el mismo RuntimeError causal para todos los fallos de write de barrera."""
    harness = load_harness()
    error_cases = [
        ("value-error", ValueError),
        ("broken-pipe", BrokenPipeError),
        ("os-error", OSError),
    ]
    public_raw = '{"jsonrpc":"2.0","method":"notify/must-not-write"}'
    failures: list[str] = []

    for label, error_type in error_cases:
        marker = f"RESYNC_WRITE_{label.upper().replace('-', '_')}"
        failing_stdin = FailingBarrierStdin(error_type, marker)
        instance = object.__new__(harness.LodestarSession)
        instance.proc = WriteFailureProcess(failing_stdin)
        instance._raw_pending_idless = 1
        instance._raw_needs_resync = True
        instance._raw_backlog = []
        instance._raw_pending_ids = []
        instance._raw_reserved_ids = []
        instance._next_id = 10
        returned = None
        caught = None
        try:
            if operation == "raw":
                returned = instance.raw_line(public_raw, timeout=TIMEOUT)
            elif operation == "rpc":
                returned = instance.rpc("ping")
            else:
                raise AssertionError(f"operación desconocida: {operation}")
        except Exception as error:  # se inspeccionan tipo, mensaje y causa exactos abajo
            caught = error

        prefix = f"{operation}/{label}"
        if returned is not None or caught is None:
            failures.append(
                f"{prefix}: fallo de write devolvió valor/falso silencio: "
                f"returned={returned!r} error={caught!r}"
            )
        elif type(caught) is not RuntimeError:
            failures.append(
                f"{prefix}: esperaba RuntimeError de resync, obtuvo {type(caught).__name__}: "
                f"{caught!r}"
            )
        else:
            message = str(caught)
            normalized = message.lower()
            if "resync" not in normalized and "resincron" not in normalized:
                failures.append(f"{prefix}: mensaje no nombra resync: {message!r}")
            if marker not in message:
                failures.append(f"{prefix}: mensaje perdió la causa subyacente: {message!r}")
            cause = caught.__cause__
            if not isinstance(cause, error_type) or marker not in str(cause):
                failures.append(
                    f"{prefix}: causa no quedó encadenada como {error_type.__name__}: {cause!r}"
                )

        if len(failing_stdin.lines) != 1:
            failures.append(
                f"{prefix}: esperaba solo write de barrera, hubo {len(failing_stdin.lines)}: "
                f"{failing_stdin.lines!r}"
            )
        else:
            try:
                attempted = json.loads(failing_stdin.lines[0])
            except json.JSONDecodeError:
                failures.append(f"{prefix}: write de barrera no era JSON: {failing_stdin.lines[0]!r}")
            else:
                if (
                    attempted.get("jsonrpc") != "2.0"
                    or attempted.get("method") != "ping"
                    or not isinstance(attempted.get("id"), str)
                ):
                    failures.append(f"{prefix}: único write no era barrera correlacionable: {attempted!r}")
                if operation == "raw" and failing_stdin.lines[0].rstrip("\n") == public_raw:
                    failures.append(f"{prefix}: se intentó la raw pública en vez de la barrera")
                if operation == "rpc" and type(attempted.get("id")) is int:
                    failures.append(f"{prefix}: se intentó el rpc público en vez de la barrera")
        if failing_stdin.flush_calls != 0:
            failures.append(f"{prefix}: flush ocurrió pese al write fallido")
        if instance._raw_reserved_ids:
            failures.append(f"{prefix}: el id de barrera quedó reservado: {instance._raw_reserved_ids!r}")
        if not instance._raw_needs_resync:
            failures.append(f"{prefix}: el fallo de write limpió indebidamente el estado resync")

    assert not failures, f"{operation} no normalizó fallos de write de resync:\n- " + "\n- ".join(failures)


def check_raw_line_resync_write_errors_are_runtime_errors_with_cause() -> None:
    run_resync_write_error_matrix("raw")


def check_rpc_resync_write_errors_are_runtime_errors_with_cause() -> None:
    run_resync_write_error_matrix("rpc")


def initialize_resync_double(harness, process):
    instance = object.__new__(harness.LodestarSession)
    instance.proc = process
    instance._raw_pending_idless = 1
    instance._raw_needs_resync = True
    instance._raw_backlog = []
    instance._raw_pending_ids = []
    instance._raw_reserved_ids = []
    instance._next_id = 10
    return instance


def stop_coordinated_reader(instance) -> None:
    reader = getattr(instance, "_stdout_reader", None)
    read_ack = getattr(instance, "_stdout_read_ack", None)
    if read_ack is not None:
        read_ack.set()
    instance.proc.stdout.close()
    if reader is not None:
        reader.join(timeout=1)


def assert_prewrite_failure_does_not_create_pending_barrier(
    harness, operation: str, error_type, failures: list[str]
) -> None:
    """Control opuesto: si ``write`` falla, el id intentado nunca estuvo en vuelo."""
    marker = f"WRITE_NEVER_SENT_{operation.upper()}"
    failing_stdin = FailingBarrierStdin(error_type, marker)
    instance = initialize_resync_double(harness, WriteFailureProcess(failing_stdin))
    public_raw = '{"jsonrpc":"2.0","method":"notify/public-never-sent"}'
    caught = None
    try:
        if operation == "raw":
            instance.raw_line(public_raw, timeout=TIMEOUT)
        else:
            instance.rpc("public/never-sent")
    except Exception as error:
        caught = error

    prefix = f"{operation}/write-control"
    if type(caught) is not RuntimeError or marker not in str(caught):
        failures.append(f"{prefix}: fallo de write no fue RuntimeError causal: {caught!r}")
    if len(failing_stdin.lines) != 1:
        failures.append(f"{prefix}: esperaba un único write de barrera: {failing_stdin.lines!r}")
        return
    attempted = json.loads(failing_stdin.lines[0])
    attempted_id = attempted.get("id")
    if not isinstance(attempted_id, str) or attempted.get("method") != "ping":
        failures.append(f"{prefix}: write intentado no fue barrera: {attempted!r}")
    elif instance._raw_id_in_use(attempted_id):
        failures.append(
            f"{prefix}: write fallido creó deuda para barrera nunca enviada: {attempted_id!r}"
        )
    if failing_stdin.flush_calls != 0:
        failures.append(f"{prefix}: flush ocurrió pese al fallo en write")
    if instance._raw_reserved_ids:
        failures.append(f"{prefix}: reserva no se limpió: {instance._raw_reserved_ids!r}")


def run_partial_barrier_flush_failure_case(operation: str) -> None:
    """Un flush fallido conserva el id ya escrito hasta consumir su ACK tardío."""
    harness = load_harness()
    failures: list[str] = []
    public_raw = '{"jsonrpc":"2.0","method":"notify/public-after-resync"}'
    follow_raw = '{"jsonrpc":"2.0","method":"notify/backlog-must-be-empty"}'

    control_error = OSError if operation == "raw" else ValueError
    assert_prewrite_failure_does_not_create_pending_barrier(
        harness, operation, control_error, failures
    )

    for label, error_type in (("os-error", OSError), ("value-error", ValueError)):
        marker = f"PARTIAL_FLUSH_{operation.upper()}_{label.upper().replace('-', '_')}"
        process = CoordinatedFlushFailureProcess(error_type, marker)
        instance = initialize_resync_double(harness, process)
        first_error = None
        retry_value = None
        follow_value = None
        try:
            try:
                if operation == "raw":
                    instance.raw_line(public_raw, timeout=TIMEOUT)
                else:
                    instance.rpc("public/after-resync")
            except Exception as error:
                first_error = error

            prefix = f"{operation}/{label}"
            if type(first_error) is not RuntimeError:
                failures.append(
                    f"{prefix}: flush parcial no produjo RuntimeError de resync: {first_error!r}"
                )
            else:
                normalized = str(first_error).lower()
                if "resync" not in normalized and "resincron" not in normalized:
                    failures.append(f"{prefix}: error no nombra resync: {first_error!r}")
                cause = first_error.__cause__
                if not isinstance(cause, error_type) or marker not in str(cause):
                    failures.append(
                        f"{prefix}: causa de flush no quedó encadenada: {cause!r}"
                    )

            if len(process.stdin.lines) != 1 or len(process.stdin.barrier_ids) != 1:
                failures.append(
                    f"{prefix}: antes del retry sólo debía escribirse barrera1: "
                    f"lines={process.stdin.lines!r} barriers={process.stdin.barrier_ids!r}"
                )
                continue
            barrier1 = process.stdin.barrier_ids[0]
            first_line = json.loads(process.stdin.lines[0])
            if first_line.get("id") != barrier1 or first_line.get("method") != "ping":
                failures.append(f"{prefix}: primera línea no acredita barrera1: {first_line!r}")
            if not instance._raw_id_in_use(barrier1):
                failures.append(
                    f"{prefix}: write exitoso seguido de flush fallido no registró "
                    f"barrera1 pendiente/inflight: {barrier1!r}"
                )
            if instance._raw_reserved_ids:
                failures.append(
                    f"{prefix}: la reserva transitoria sobrevivió al error: "
                    f"{instance._raw_reserved_ids!r}"
                )
            if any(line.rstrip("\n") == public_raw for line in process.stdin.lines):
                failures.append(f"{prefix}: raw pública salió antes del retry")
            if any(type(json.loads(line).get("id")) is int for line in process.stdin.lines):
                failures.append(f"{prefix}: rpc público salió antes del retry")

            if operation == "raw":
                retry_value = instance.raw_line(public_raw, timeout=TIMEOUT)
            else:
                retry_value = instance.rpc("public/after-resync")
                follow_value = instance.raw_line(follow_raw, timeout=TIMEOUT)

            if len(process.stdin.barrier_ids) != 2:
                failures.append(
                    f"{prefix}: retry no envió exactamente barrera2: "
                    f"{process.stdin.barrier_ids!r}"
                )
                continue
            barrier2 = process.stdin.barrier_ids[1]
            if barrier2 == barrier1:
                failures.append(f"{prefix}: retry reutilizó el id de barrera1")
            written = [json.loads(line) for line in process.stdin.lines]
            if [item.get("id") for item in written[:2]] != [barrier1, barrier2]:
                failures.append(f"{prefix}: orden de barreras no acreditado: {written!r}")

            stale_ack = {
                "jsonrpc": "2.0",
                "id": barrier1,
                "result": {"marker": "BARRIER_ACK_1"},
            }
            if operation == "raw":
                if retry_value is not None:
                    failures.append(
                        f"{prefix}: operación pública devolvió ACK1 tardío en vez de silencio: "
                        f"{retry_value!r}; stale={stale_ack!r}"
                    )
                expected_lines = 3
            else:
                if not isinstance(retry_value, dict) or retry_value.get("result", {}).get(
                    "marker"
                ) != "PUBLIC_RPC":
                    failures.append(
                        f"{prefix}: rpc no alcanzó su respuesta fresca: {retry_value!r}"
                    )
                if follow_value is not None:
                    failures.append(
                        f"{prefix}: ACK1 quedó oculto y reapareció en la siguiente raw: "
                        f"{follow_value!r}; stale={stale_ack!r}"
                    )
                expected_lines = 4
            if len(written) != expected_lines:
                failures.append(
                    f"{prefix}: secuencia pública incompleta: esperaba {expected_lines} líneas, "
                    f"obtuvo {written!r}"
                )
            if instance._raw_id_in_use(barrier1):
                failures.append(f"{prefix}: barrera1 siguió pendiente tras consumir ACK1")
            if instance._raw_id_in_use(barrier2):
                failures.append(f"{prefix}: barrera2 siguió pendiente tras aceptar ACK2")
            expected_pending_ids = []
            if instance._raw_pending_ids != expected_pending_ids:
                failures.append(
                    f"{prefix}: estado pending terminal inexacto; "
                    f"esperaba {expected_pending_ids!r}, obtuvo {instance._raw_pending_ids!r}"
                )
            if instance._raw_pending_idless != 0:
                failures.append(
                    f"{prefix}: deuda idless sobrevivió al ACK2: "
                    f"{instance._raw_pending_idless!r}"
                )
            if instance._raw_reserved_ids:
                failures.append(
                    f"{prefix}: reservas no quedaron limpias al terminar: "
                    f"{instance._raw_reserved_ids!r}"
                )
            if instance._raw_needs_resync:
                failures.append(f"{prefix}: ACK2 no cerró el estado needs_resync")
            if instance._raw_backlog:
                failures.append(
                    f"{prefix}: backlog terminal no quedó vacío: {instance._raw_backlog!r}"
                )
        finally:
            stop_coordinated_reader(instance)

    assert not failures, f"{operation} perdió causalidad tras flush parcial:\n- " + "\n- ".join(
        failures
    )


def check_raw_line_partial_barrier_flush_failure_tracks_late_ack() -> None:
    run_partial_barrier_flush_failure_case("raw")


def check_rpc_partial_barrier_flush_failure_tracks_late_ack() -> None:
    run_partial_barrier_flush_failure_case("rpc")


def check_source_has_no_signal_global_state() -> None:
    harness_path = Path(__file__).resolve().parents[3] / "docs/qa/testbench/lodestar_harness.py"
    source = harness_path.read_text(encoding="utf-8")
    forbidden = ("SIGALRM", "setitimer", "ITIMER_REAL", "signal.signal(")
    found = [token for token in forbidden if token in source]
    assert not found, f"el timeout portable no puede depender de estado global de señales: {found}"


CHECKS = {
    "injected-id-domain": check_injected_rejected_id_domain_returns_observed_invalid_request,
    "fresh-idless-invalid-params": check_fresh_request_returns_observed_idless_invalid_params_error,
    "strict-integer-id": check_integer_id_rejects_bool_float_and_string_aliases,
    "null-params-correlates": check_null_params_is_absent_and_correlates_integer_id,
    "strict-read-response-id": check_read_response_integer_id_rejects_bool_float_and_string_aliases,
    "invalid-request": check_invalid_request_error_is_not_silenced,
    "non-string-method-observed-frame": check_non_string_method_returns_observed_invalid_request_instead_of_silence,
    "malformed-observed-frame": check_malformed_json_returns_observed_server_parse_error_instead_of_silence,
    "notification-observed-frame": check_notification_returns_observed_server_method_error_instead_of_silence,
    "prefetched": check_real_pipe_preserves_prefetched_second_frame,
    "blocking-return-after-deadline-fifo": check_raw_line_preserves_frame_returned_after_blocking_deadline_fifo,
    "late-idless-error": check_real_pipe_discards_late_idless_error_before_current_response,
    "silent-discards-expired-id": check_silent_input_discards_late_expired_request_response,
    "silent-discards-expired-idless-error": check_silent_input_discards_late_idless_error_from_expired_invalid_input,
    "rejected-bool-does-not-consume-fresh-idless": check_rejected_bool_id_does_not_consume_next_fresh_idless_error,
    "silence-returns-fresh-valid-id": check_silent_input_returns_fresh_valid_id_response_without_expired_request,
    "silence-preserves-expired-id-type-alias": check_silent_input_preserves_valid_type_alias_of_expired_id,
    "second-invalid-keeps-fresh-idless": check_second_invalid_observes_its_fresh_idless_error_after_silent_invalid,
    "notification-keeps-fresh-id-null": check_notification_observes_fresh_id_null_frame_after_silent_invalid,
    "rpc-avoids-expired-raw-id": check_rpc_does_not_reuse_expired_raw_request_id_or_accept_stale_response,
    "barrier-preserves-fresh-and-avoids-string-id": check_barrier_preserves_fresh_frame_and_avoids_pending_string_id,
    "raw-request-preserves-foreign-fifo": check_raw_request_preserves_foreign_frame_for_later_raw_fifo,
    "rpc-preserves-foreign-fifo": check_rpc_read_response_preserves_foreign_frame_for_later_raw_fifo,
    "barrier-preserves-fresh-id-null": check_barrier_preserves_fresh_id_null_error_for_next_public_raw,
    "barrier-discards-at-most-one-32600": check_barrier_discards_at_most_one_attributable_idless_32600,
    "pending-id-preserves-server-request": check_pending_id_does_not_consume_server_request_with_same_id,
    "pending-id-strict-response-shape": check_pending_id_consumption_requires_strict_jsonrpc_response_shape,
    "reused-raw-id-drains-stale": check_reused_pending_raw_id_barrier_drains_stale_before_fresh_second,
    "reused-raw-id-ack-clears-debt": check_reused_pending_raw_id_barrier_ack_extinguishes_unresolved_debt,
    "noncolliding-raw-id-no-barrier": check_noncolliding_raw_id_does_not_barrier_other_pending_id,
    "pending-id-preserves-error-null": check_pending_id_preserves_error_null_until_real_valid_response,
    "pending-id-validates-error-result": check_pending_id_validates_error_object_and_accepts_any_result_json,
    "rpc-preserves-nonfinite-as-unparseable": check_rpc_preserves_nonfinite_response_frames_as_unparseable_fifo,
    "raw-line-strict-json-nonfinite": check_raw_line_strict_json_rejects_nonfinite_responses_and_inputs,
    "current-invalid-preserves-fresh-idless-fifo": check_current_invalid_preserves_fresh_idless_before_attributable_32600_fifo,
    "current-invalid-strict-idless-32600": check_current_invalid_correlates_only_strict_idless_32600_error,
    "idless-null-absent-invalid-params": check_absent_and_null_id_invalid_params_are_equivalent_invalid_requests,
    "resync-failure-aborts-public-send": check_resync_timeout_or_eof_raises_before_raw_or_rpc_public_send,
    "raw-resync-write-errors": check_raw_line_resync_write_errors_are_runtime_errors_with_cause,
    "rpc-resync-write-errors": check_rpc_resync_write_errors_are_runtime_errors_with_cause,
    "raw-partial-barrier-flush": check_raw_line_partial_barrier_flush_failure_tracks_late_ack,
    "rpc-partial-barrier-flush": check_rpc_partial_barrier_flush_failure_tracks_late_ack,
    "portable": check_source_has_no_signal_global_state,
    "bounded-timeout": check_timeout_is_bounded_when_timer_delivery_is_unavailable,
    "read-response-eof": check_read_response_eof_reports_context_without_waiting_for_timeout,
    "raw-line-eof": check_raw_line_eof_reports_server_exit_instead_of_silence,
    "persistent-live-eof-raw-resync": check_raw_line_persists_live_transport_eof_across_calls_and_resync,
    "persistent-live-eof-rpc": check_rpc_persists_live_transport_eof_across_repeated_calls,
    "terminal-eof-raw-preflight": check_raw_line_terminal_eof_preflight_is_stable_and_never_rewrites,
    "terminal-eof-rpc-preflight": check_rpc_terminal_eof_preflight_caches_diagnostic_and_never_rewrites,
    "queued-response-before-terminal-raw": check_raw_line_drains_queued_response_before_terminal_preflight,
    "queued-response-before-terminal-rpc": check_rpc_drains_queued_response_before_terminal_preflight,
    "correlated-then-foreign-before-terminal-raw": check_raw_line_preserves_foreign_after_correlated_before_terminal,
    "correlated-then-foreign-before-terminal-rpc": check_rpc_preserves_foreign_after_correlated_before_terminal,
    "live-eof-preserves-foreign-raw": check_raw_line_live_eof_preserves_foreign_before_terminal,
    "live-eof-preserves-foreign-rpc": check_rpc_then_raw_live_eof_preserves_foreign_before_terminal,
    "terminal-drains-three-pre-eof-raw": check_raw_line_terminal_drains_three_pre_eof_frames,
    "terminal-drains-three-pre-eof-rpc": check_rpc_terminal_drains_three_pre_eof_frames,
    "queued-barrier-ack-before-terminal-raw": check_raw_line_resync_drains_queued_ack_before_terminal_public_preflight,
    "queued-barrier-ack-before-terminal-rpc": check_rpc_resync_drains_queued_ack_before_terminal_public_preflight,
    "ack-before-flush-error-raw": check_raw_line_acknowledged_barrier_beats_flush_error_and_terminal,
    "ack-before-flush-error-rpc": check_rpc_acknowledged_barrier_beats_flush_error_and_terminal,
    "past-deadline-ack-after-flush-raw": check_raw_line_past_deadline_still_drains_ack_after_flush_error,
    "past-deadline-ack-after-flush-rpc": check_rpc_past_deadline_still_drains_ack_after_flush_error,
    "past-deadline-foreign-before-ack-raw": check_raw_line_past_deadline_foreign_stops_before_queued_ack,
    "past-deadline-foreign-before-ack-rpc": check_rpc_past_deadline_foreign_stops_before_queued_ack,
    "post-deadline-probe-zero-timeout-raw": check_raw_line_post_deadline_probe_calls_stdout_item_with_zero_timeout,
    "post-deadline-probe-zero-timeout-rpc": check_rpc_post_deadline_probe_calls_stdout_item_with_zero_timeout,
    "close-lifecycle": check_close_terminates_real_process_and_stdout_reader_bounded,
}


def main() -> int:
    selectors = sys.argv[1:] or list(CHECKS)
    for selector in selectors:
        if selector not in CHECKS:
            raise AssertionError(f"selector desconocido: {selector}")
        CHECKS[selector]()
        print(f"PASS: {selector}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, OSError, subprocess.SubprocessError, RuntimeError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
