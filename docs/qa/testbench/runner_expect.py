#!/usr/bin/env python3
"""Evaluador del formato `expect` del banco de conformidad (E33-H02).

Este módulo NO habla JSON-RPC ni sabe de corpus: recibe el resultado que el arnés ya
produjo para cada paso y decide si el caso PASA o FALLA. La semántica exacta de cada
clave está fijada en `FORMATO_EXPECT.md`, que es el contrato; aquí solo se implementa,
con una función por familia de aserción para que la correspondencia sea legible.

Piezas públicas:

  * `resuelve_path(raiz, path)` — el selector de §3.1 (el mismo dialecto de `@stepN`).
  * `evalua_paso(resultado, expect)` — las aserciones de §3 sobre UN paso.
  * `evalua_invariante(resultados, inv)` — los invariantes de §4 ENTRE pasos.
  * `evalua_caso(caso, resultados)` — el veredicto del caso completo.
  * `es_asertable(caso)` — si el caso computa al veredicto o es exploratorio (§1).
  * `entra_al_gate(caso, spec)` — la clave `gate` de §5, con la herencia del lote.
"""

import json
import re

# Veredictos (§7, la clave `verdict` del JSON de resultados).
PASS = "PASS"
FAIL = "FAIL"
SKIP = "SKIP"
EXPLORATORY = "EXPLORATORY"

# Sentinela de «el path no resuelve». No es `None`, porque `None` es un valor legítimo
# (un `nextCursor: null` resuelve y vale null); confundirlos rompería `absent` y `present`.
NO_RESUELVE = object()

# Las claves que `step.expect` reconoce. Una clave desconocida es un error del LOTE, no
# una aserción que se cumple sola: se reporta como fallo para que un typo en un esperado
# no se convierta en un PASS silencioso.
CLAVES_DE_PASO = {
    "is_error", "error_code", "protocol_error_code", "equals", "present", "absent",
    "matches", "contains", "not_contains", "length", "min_length", "type", "rc",
    "describe",
}
CLAVES_DE_INVARIANTE = {"invariant", "steps", "path", "describe"}

# `describe` es prosa, no aserción: no convierte un `expect` en asertable (§3).
CLAVES_NO_ASERTIVAS = {"describe"}

# Nombres de tipo JSON admitidos por la clave `type` (§3).
TIPOS_JSON = {
    "object": dict,
    "array": list,
    "string": str,
    "number": (int, float),
    "boolean": bool,
    "null": type(None),
}


def _breve(valor, tope=300):
    """Representación corta y estable de un valor, para el detalle de un FAIL."""
    if valor is NO_RESUELVE:
        return "<el path no resuelve>"
    try:
        texto = json.dumps(valor, ensure_ascii=False, sort_keys=True)
    except (TypeError, ValueError):
        texto = repr(valor)
    return texto if len(texto) <= tope else texto[:tope] + "…"


def resuelve_path(raiz, path):
    """Aplica el selector de `FORMATO_EXPECT.md §3.1` sobre el resultado de un paso.

    Segmentos separados por `.`; un segmento entero indexa una lista. Devuelve
    `NO_RESUELVE` si la clave no existe, el índice se sale de rango o se desciende sobre
    un escalar — nunca lanza excepción: un path que no resuelve es un FAIL con motivo, no
    una caída del runner.
    """
    actual = raiz
    for parte in path.split("."):
        if isinstance(actual, list):
            if not re.fullmatch(r"\d+", parte):
                return NO_RESUELVE
            indice = int(parte)
            if indice >= len(actual):
                return NO_RESUELVE
            actual = actual[indice]
        elif isinstance(actual, dict):
            if parte not in actual:
                return NO_RESUELVE
            actual = actual[parte]
        else:
            return NO_RESUELVE
    return actual


def _iguales(a, b):
    """Igualdad JSON estructural: mismo tipo, y en objetos/listas mismos elementos.

    `True == 1` en Python y eso convertiría `equals: {…: true}` en un esperado que casa
    con un `1` del wire; el banco necesita distinguirlos, así que los booleanos solo
    igualan a booleanos.
    """
    if isinstance(a, bool) != isinstance(b, bool):
        return False
    if isinstance(a, dict) and isinstance(b, dict):
        # JSON object member order is part of the bank's structural equality contract.
        # ``dict_keys`` equality is set-like in Python and would erase that distinction.
        return (list(a.keys()) == list(b.keys())
                and all(_iguales(a[k], b[k]) for k in a))
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(_iguales(x, y) for x, y in zip(a, b))
    return a == b


def _falta(step, path, esperado, real, motivo):
    """Un fallo de aserción, en la forma que §7 exige del JSON de resultados."""
    return {"step": step, "path": path, "expected": esperado, "actual": real,
            "reason": motivo}


def _tiene_aserciones(expect):
    """¿Este `expect` asevera algo? Un `{}` (o un `{describe: …}` a secas) no (§3)."""
    if not isinstance(expect, dict):
        return False
    return any(clave not in CLAVES_NO_ASERTIVAS for clave in expect)


def es_asertable(caso):
    """Un caso es asertable si algún paso —o el propio caso— asevera algo (§1)."""
    invariantes = caso.get("expect")
    if isinstance(invariantes, list) and invariantes:
        return True
    if isinstance(invariantes, dict) and _tiene_aserciones(invariantes):
        return True
    for paso in caso.get("steps") or []:
        if _tiene_aserciones(paso.get("expect")):
            return True
    return False


def entra_al_gate(caso, spec):
    """La clave `gate` de §5: el valor del caso gana al del lote; por defecto, `true`."""
    if "gate" in caso:
        return bool(caso["gate"])
    return bool(spec.get("gate", True))


def expect_de_paso(caso, indice):
    """El `expect` que aplica al paso `indice`, con la regla de la forma corta (§2).

    En forma corta (sin `steps`) el `expect` del caso, si es un OBJETO, es el del paso
    único; si es lista, son invariantes entre pasos y no asevera nada sobre el paso.
    """
    pasos = caso.get("steps")
    if pasos:
        return pasos[indice].get("expect")
    del_caso = caso.get("expect")
    return del_caso if isinstance(del_caso, dict) else None


def invariantes_de_caso(caso):
    """Los invariantes entre pasos declarados a nivel de caso (§4).

    En forma corta con `expect` objeto, ese objeto es del paso único (§2), no un
    invariante: aquí no cuenta.
    """
    del_caso = caso.get("expect")
    if isinstance(del_caso, list):
        return del_caso
    if isinstance(del_caso, dict) and not caso.get("steps"):
        return []
    if isinstance(del_caso, dict):
        # `case.expect` es, por contrato, una LISTA de invariantes. Un objeto en la forma
        # larga es un lote mal escrito: se reporta en `evalua_caso`, no se ignora.
        return [del_caso]
    return []


# --------------------------------------------------------------------------- §3
# Las aserciones de un paso. Cada una devuelve la lista de fallos que produce (vacía si
# la aserción se cumple), y todas reciben ya resuelto el resultado del paso.

def _err_de_protocolo(resultado):
    return resultado.get("protocol_error") if isinstance(resultado, dict) else None


def _asevera_is_error(resultado, esperado, indice):
    real = resultado.get("is_error")
    protocolo = _err_de_protocolo(resultado)
    excepcion = resultado.get("harness_exception")
    if esperado is False:
        # `false` exige además que no haya error de protocolo ni excepción del arnés (§3):
        # una tool que ni se ejecutó no es «una tool que respondió sin error».
        if protocolo is not None:
            return [_falta(indice, "protocol_error", "sin error de protocolo",
                           _breve(protocolo), "el paso falló en el protocolo JSON-RPC")]
        if excepcion is not None:
            return [_falta(indice, "harness_exception", "sin excepción del arnés",
                           _breve(excepcion), "el arnés no pudo ejecutar el paso")]
    if real is None:
        return [_falta(indice, "is_error", esperado, "<ausente>",
                       "el resultado no tiene `is_error` (¿error de protocolo o paso "
                       "no-call?)")]
    if not isinstance(real, bool) or real is not esperado:
        return [_falta(indice, "is_error", esperado, real, "`is_error` no coincide")]
    return []


def _asevera_error_code(resultado, esperado, indice):
    protocolo = _err_de_protocolo(resultado)
    if protocolo is not None:
        return [_falta(indice, "error_code", esperado, _breve(protocolo),
                       "se esperaba un error de TOOL y llegó uno de PROTOCOLO")]
    if resultado.get("is_error") is not True:
        return [_falta(indice, "error_code", esperado, resultado.get("error_code"),
                       "el paso no es un error de tool (`is_error` no es true)")]
    real = resultado.get("error_code")
    if real != esperado:
        return [_falta(indice, "error_code", esperado, real,
                       "el código de error no coincide")]
    return []


def _asevera_protocol_error_code(resultado, esperado, indice):
    protocolo = _err_de_protocolo(resultado)
    if protocolo is None:
        return [_falta(indice, "protocol_error.code", esperado, "<sin error de protocolo>",
                       "se esperaba un error de PROTOCOLO JSON-RPC y no lo hubo")]
    real = protocolo.get("code") if isinstance(protocolo, dict) else None
    if (not isinstance(esperado, int) or isinstance(esperado, bool)
            or not isinstance(real, int) or isinstance(real, bool) or real != esperado):
        return [_falta(indice, "protocol_error.code", esperado, real,
                       "el código de protocolo no coincide")]
    return []


def _asevera_equals(resultado, mapa, indice):
    fallos = []
    for path, esperado in mapa.items():
        real = resuelve_path(resultado, path)
        if real is NO_RESUELVE:
            fallos.append(_falta(indice, path, esperado, "<el path no resuelve>",
                                 "el path no resuelve"))
        elif not _iguales(real, esperado):
            fallos.append(_falta(indice, path, esperado, real, "valor distinto"))
    return fallos


def _asevera_present(resultado, paths, indice):
    fallos = []
    for path in paths:
        if resuelve_path(resultado, path) is NO_RESUELVE:
            fallos.append(_falta(indice, path, "<presente>", "<el path no resuelve>",
                                 "el path no resuelve"))
    return fallos


def _asevera_absent(resultado, paths, indice):
    fallos = []
    for path in paths:
        real = resuelve_path(resultado, path)
        # `absent` se cumple si el path no resuelve O resuelve a null (§3): en el wire,
        # «campo ausente» y «campo null» son la misma afirmación para el banco.
        if real is not NO_RESUELVE and real is not None:
            fallos.append(_falta(indice, path, "<ausente o null>", real,
                                 "el path resuelve a un valor no nulo"))
    return fallos


def _asevera_matches(resultado, mapa, indice):
    fallos = []
    for path, patron in mapa.items():
        real = resuelve_path(resultado, path)
        if real is NO_RESUELVE:
            fallos.append(_falta(indice, path, patron, "<el path no resuelve>",
                                 "el path no resuelve"))
        elif not isinstance(real, str):
            fallos.append(_falta(indice, path, patron, real,
                                 "el valor no es una cadena"))
        elif re.search(patron, real) is None:
            fallos.append(_falta(indice, path, patron, real,
                                 "la regex no casa (semántica re.search)"))
    return fallos


def _asevera_contains(resultado, mapa, indice, negado=False):
    fallos = []
    for path, buscado in mapa.items():
        real = resuelve_path(resultado, path)
        if real is NO_RESUELVE:
            fallos.append(_falta(indice, path, buscado, "<el path no resuelve>",
                                 "el path no resuelve"))
            continue
        if isinstance(real, list):
            contiene = any(_iguales(elem, buscado) for elem in real)
        elif isinstance(real, str) and isinstance(buscado, str):
            contiene = buscado in real
        else:
            fallos.append(_falta(indice, path, buscado, real,
                                 "el valor no es lista, ni cadena con subcadena buscada"))
            continue
        if contiene == negado:
            motivo = "contiene lo que no debía" if negado else "no contiene lo esperado"
            fallos.append(_falta(indice, path, buscado, real, motivo))
    return fallos


def _asevera_longitud(resultado, mapa, indice, minima=False):
    fallos = []
    for path, n in mapa.items():
        real = resuelve_path(resultado, path)
        if real is NO_RESUELVE:
            fallos.append(_falta(indice, path, n, "<el path no resuelve>",
                                 "el path no resuelve"))
            continue
        if not isinstance(real, (list, dict, str)):
            fallos.append(_falta(indice, path, n, real,
                                 "el valor no es lista, objeto ni cadena"))
            continue
        largo = len(real)
        if (largo < n) if minima else (largo != n):
            fallos.append(_falta(indice, path, ("≥ %d" % n) if minima else n, largo,
                                 "longitud fuera de lo aseverado"))
    return fallos


def _asevera_tipo(resultado, mapa, indice):
    fallos = []
    for path, nombre in mapa.items():
        real = resuelve_path(resultado, path)
        if real is NO_RESUELVE:
            fallos.append(_falta(indice, path, nombre, "<el path no resuelve>",
                                 "el path no resuelve"))
            continue
        if nombre not in TIPOS_JSON:
            fallos.append(_falta(indice, path, nombre, real,
                                 "tipo declarado desconocido (usa object/array/string/"
                                 "number/boolean/null)"))
            continue
        # En JSON `true` no es un número aunque Python lo herede de `int`.
        if nombre == "number" and isinstance(real, bool):
            fallos.append(_falta(indice, path, nombre, real, "es booleano, no número"))
            continue
        if not isinstance(real, TIPOS_JSON[nombre]):
            fallos.append(_falta(indice, path, nombre, real, "el tipo no coincide"))
    return fallos


def _asevera_rc(resultado, esperado, indice):
    real = resuelve_path(resultado, "rc")
    if real is NO_RESUELVE:
        return [_falta(indice, "rc", esperado, "<el path no resuelve>",
                       "el paso no tiene `rc` (¿no es shell/spawn?)")]
    if (not isinstance(esperado, int) or isinstance(esperado, bool)
            or not isinstance(real, int) or isinstance(real, bool) or real != esperado):
        return [_falta(indice, "rc", esperado, real, "el exit code no coincide")]
    return []


def evalua_paso(resultado, expect, indice):
    """Todas las aserciones de §3 sobre un paso. Devuelve la lista de fallos."""
    if not isinstance(expect, dict):
        return []
    fallos = []
    desconocidas = sorted(set(expect) - CLAVES_DE_PASO)
    if desconocidas:
        fallos.append(_falta(indice, ", ".join(desconocidas), "<clave de expect válida>",
                             "<desconocida>",
                             "clave de `expect` no reconocida: el lote está mal escrito"))
    if not isinstance(resultado, dict):
        return fallos + [_falta(indice, "", "<un resultado de paso>", _breve(resultado),
                                "el paso no produjo un resultado inspeccionable")]
    if "harness_exception" in resultado and "is_error" not in expect:
        fallos.append(_falta(indice, "harness_exception", "<sin excepción>",
                             _breve(resultado["harness_exception"]),
                             "el arnés no pudo ejecutar el paso"))

    if "is_error" in expect:
        fallos += _asevera_is_error(resultado, expect["is_error"], indice)
    if "error_code" in expect:
        fallos += _asevera_error_code(resultado, expect["error_code"], indice)
    if "protocol_error_code" in expect:
        fallos += _asevera_protocol_error_code(resultado, expect["protocol_error_code"],
                                               indice)
    if "equals" in expect:
        fallos += _asevera_equals(resultado, expect["equals"], indice)
    if "present" in expect:
        fallos += _asevera_present(resultado, expect["present"], indice)
    if "absent" in expect:
        fallos += _asevera_absent(resultado, expect["absent"], indice)
    if "matches" in expect:
        fallos += _asevera_matches(resultado, expect["matches"], indice)
    if "contains" in expect:
        fallos += _asevera_contains(resultado, expect["contains"], indice)
    if "not_contains" in expect:
        fallos += _asevera_contains(resultado, expect["not_contains"], indice, negado=True)
    if "length" in expect:
        fallos += _asevera_longitud(resultado, expect["length"], indice)
    if "min_length" in expect:
        fallos += _asevera_longitud(resultado, expect["min_length"], indice, minima=True)
    if "type" in expect:
        fallos += _asevera_tipo(resultado, expect["type"], indice)
    if "rc" in expect:
        fallos += _asevera_rc(resultado, expect["rc"], indice)

    descripcion = expect.get("describe")
    if descripcion:
        for fallo in fallos:
            fallo.setdefault("describe", descripcion)
    return fallos


# --------------------------------------------------------------------------- §4

def evalua_invariante(resultados, inv):
    """Un invariante `same`/`differs` entre pasos (§4). Devuelve la lista de fallos."""
    if not isinstance(inv, dict):
        return [_falta(None, "", "<un invariante>", _breve(inv),
                       "el invariante de caso no es un objeto")]
    desconocidas = sorted(set(inv) - CLAVES_DE_INVARIANTE)
    fallos = []
    if desconocidas:
        fallos.append(_falta(None, ", ".join(desconocidas), "<clave de invariante válida>",
                             "<desconocida>", "clave de invariante no reconocida"))
    nombre = inv.get("invariant")
    indices = inv.get("steps")
    path = inv.get("path")
    descripcion = inv.get("describe")
    if nombre not in ("same", "differs"):
        fallos.append(_falta(None, path, "same | differs", nombre,
                             "invariante desconocido"))
        return _con_describe(fallos, descripcion)
    if not isinstance(indices, list) or len(indices) < 2 or not isinstance(path, str):
        fallos.append(_falta(None, path, "steps (≥2 índices) y path",
                             _breve({"steps": indices, "path": path}),
                             "invariante mal declarado"))
        return _con_describe(fallos, descripcion)

    valores = []
    for indice in indices:
        if not isinstance(indice, int) or indice < 0 or indice >= len(resultados):
            fallos.append(_falta(indice, path, "<un paso existente>", indice,
                                 "el índice de paso no existe en el caso"))
            continue
        valor = resuelve_path(resultados[indice], path)
        if valor is NO_RESUELVE:
            fallos.append(_falta(indice, path, "<presente en todos los pasos>",
                                 "<el path no resuelve>",
                                 "el path no resuelve en el paso %d" % indice))
            continue
        valores.append((indice, valor))

    if fallos:
        return _con_describe(fallos, descripcion)

    primero_i, primero = valores[0]
    if nombre == "same":
        for indice, valor in valores[1:]:
            if not _iguales(valor, primero):
                fallos.append(_falta(indice, path, primero, valor,
                                     "invariante `same`: el paso %d difiere del %d"
                                     % (indice, primero_i)))
    else:  # differs
        if all(_iguales(valor, primero) for _, valor in valores[1:]):
            fallos.append(_falta(indices[-1], path, "<algún valor distinto>", primero,
                                 "invariante `differs`: todos los pasos coinciden"))
    return _con_describe(fallos, descripcion)


def _con_describe(fallos, descripcion):
    if descripcion:
        for fallo in fallos:
            fallo.setdefault("describe", descripcion)
    return fallos


# --------------------------------------------------------------------------- veredicto

def evalua_caso(caso, resultados):
    """Veredicto del caso: `(verdict, failures)` según §3–§5.

    `resultados` es la lista de resultados de paso tal como los produjo el arnés, en
    orden. Un caso sin aserciones es EXPLORATORIO y no computa.
    """
    if not es_asertable(caso):
        return EXPLORATORY, []

    fallos = []
    if caso.get("session_error"):
        fallos.append(_falta(None, "session_error", "<sesión abierta>",
                             _breve(caso["session_error"]),
                             "no se pudo abrir la sesión del caso"))
    pasos = caso.get("steps")
    total = len(pasos) if pasos else 1
    for indice in range(total):
        expect = expect_de_paso(caso, indice)
        if not isinstance(expect, dict):
            continue
        if indice >= len(resultados):
            fallos.append(_falta(indice, "", "<un paso ejecutado>", "<no ejecutado>",
                                 "el paso no llegó a ejecutarse"))
            continue
        fallos += evalua_paso(resultados[indice], expect, indice)

    for inv in invariantes_de_caso(caso):
        fallos += evalua_invariante(resultados, inv)

    return (FAIL if fallos else PASS), fallos
