#!/usr/bin/env python3
"""Selftest del runner asertable del banco (E33-H02: BDD-1 y BDD-2).

Este script es el TEST de la historia: verifica, ejecutando el runner de verdad, que el
contrato de `FORMATO_EXPECT.md` se cumple. No implementa nada del runner — solo lo invoca
y juzga su stdout y su exit code. Se escribió en la fase roja: contra el arnés actual (que
ignora `expect`) debe FALLAR.

Uso:

    ./selftest_runner.py [--binary PATH] [--corpus DIR] [--keep]

Sin `--corpus`, genera uno con `make_corpus.py` en un tempdir y lo borra al terminar.
Sin `--binary` (ni `LODESTAR_MCP_BIN`), usa `target/release/lodestar-mcp` relativo a la
raíz del repo, derivada de la ubicación de este fichero — ni una ruta absoluta de máquina.

Exit: 0 si las doce comprobaciones pasan, 1 si alguna falla, 3 si no se pudo ni preparar
el entorno (binario ausente, corpus no generable).
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile

AQUI = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(AQUI, "..", "..", ".."))
ARNES = os.path.join(AQUI, "lodestar_harness.py")
LOTE = os.path.join(AQUI, "batches", "meta_runner.json")
BIN_POR_DEFECTO = os.path.join(REPO, "target", "release", "lodestar-mcp")

fallos = []


def comprueba(nombre, condicion, detalle=""):
    marca = "ok  " if condicion else "FALLO"
    print("[%s] %s" % (marca, nombre))
    if not condicion:
        if detalle:
            print("       %s" % detalle.replace("\n", "\n       ")[:2000])
        fallos.append(nombre)
    return condicion


def corre_runner(args, corpus, binario):
    """Invoca el arnés y devuelve (returncode, stdout+stderr)."""
    cmd = [sys.executable, ARNES, "--batch", LOTE, "--root-corpus", corpus,
           "--binary", binario] + args
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    return proc.returncode, proc.stdout + proc.stderr


def main():
    ap = argparse.ArgumentParser(description="Selftest del runner asertable del banco.")
    ap.add_argument("--binary", default=os.environ.get("LODESTAR_MCP_BIN", BIN_POR_DEFECTO))
    ap.add_argument("--corpus", help="corpus canónico ya generado (si no, se genera uno)")
    ap.add_argument("--keep", action="store_true", help="no borrar el corpus generado")
    args = ap.parse_args()

    if not os.path.exists(args.binary):
        print("ERROR: no existe el binario %s (compila con `cargo build --release -p "
              "lodestar-mcp` o pasa --binary)" % args.binary)
        return 3

    temporal = None
    corpus = args.corpus
    if corpus is None:
        temporal = tempfile.mkdtemp(prefix="banco-corpus-")
        corpus = os.path.join(temporal, "corpus")
        gen = subprocess.run([sys.executable, os.path.join(AQUI, "make_corpus.py"), corpus],
                             capture_output=True, text=True)
        if gen.returncode != 0:
            print("ERROR generando el corpus: %s" % (gen.stderr or gen.stdout))
            shutil.rmtree(temporal, ignore_errors=True)
            return 3
        print("corpus canónico en %s" % corpus)

    try:
        # ---- BDD-1: solo el gate. META-01 PASS, exit 0, META-02 ni se ejecuta. -------
        rc, salida = corre_runner([], corpus, args.binary)
        print("\n--- corrida de GATE (exit %d) ---\n%s" % (rc, salida[-4000:]))
        comprueba("BDD-1a · el gate sale con exit 0", rc == 0, "exit real: %d" % rc)
        comprueba("BDD-1b · META-01 es PASS",
                  re.search(r"\bPASS\b[^\n]*META-01|META-01[^\n]*\bPASS\b", salida) is not None,
                  "no hay veredicto PASS para META-01 en la salida")
        # El rótulo exacto lo elige el implementador (FORMATO_EXPECT.md §7): aquí solo se
        # exige que la línea agregada exista y declare CERO fallos, en singular o plural.
        comprueba("BDD-1c · el resumen agregado declara 0 FAIL en el gate",
                  re.search(r"^[^\n]*RESUMEN:[^\n]*\bFAILe?s?\b\D{0,3}0(?!\d)", salida,
                            re.MULTILINE) is not None,
                  "no hay línea «RESUMEN: … FAIL(es) 0 …»")
        comprueba("BDD-1d · META-03 se reconoce como exploratorio (no computa al veredicto)",
                  re.search(r"META-03", salida) is not None
                  and re.search(r"(EXPLOR\w*)[^\n]*META-03|META-03[^\n]*(EXPLOR\w*)",
                                salida, re.IGNORECASE) is not None,
                  "META-03 no aparece marcado como exploratorio")
        # Lo que se asevera es la INTENCIÓN («META-02 no se ejecutó y ningún caso falló»),
        # no la ausencia de la subcadena «FAIL» en toda la salida: el agregado la nombra por
        # fuerza al declarar su recuento, así que prohibirla en bloque ataría la redacción
        # del resumen (obligaría a rotularlo «FAILes» para no chocar con BDD-1c). Se exige
        # además un veredicto emitido, para que esto no pase de forma vacua cuando el runner
        # ni siquiera arranca: un argparse que aborta tampoco imprime «FAIL».
        veredictos_fail = re.findall(r"^[^\n]*\bFAIL\b[^\n]*$", salida, re.MULTILINE)
        veredictos_fail = [ln for ln in veredictos_fail if not ln.lstrip().startswith("RESUMEN:")]
        # META-02 debe aparecer OMITIDO (`SKIP`, FORMATO_EXPECT.md §5), no ejecutado: que se
        # anuncie la omisión es mejor que el silencio — así una demo que dejara de correr por
        # error se nota. Lo que no puede haber es un veredicto FAIL suyo.
        salta_meta02 = re.search(r"^[^\n]*\bSKIP\w*\b[^\n]*META-02|^[^\n]*META-02[^\n]*\bSKIP\w*\b",
                                 salida, re.MULTILINE | re.IGNORECASE) is not None
        comprueba("BDD-1e · sin --incluir-demos, META-02 se omite (SKIP) y ningún caso falla",
                  not veredictos_fail
                  and salta_meta02
                  and re.search(r"\bPASS\b", salida) is not None,
                  "líneas de FAIL fuera del resumen: %s; ¿META-02 marcado SKIP?: %s"
                  % (veredictos_fail, salta_meta02))

        # ---- BDD-2: con la demo. META-02 FAIL, exit != 0, subcampo nombrado. ---------
        rc2, salida2 = corre_runner(["--incluir-demos"], corpus, args.binary)
        print("\n--- corrida CON DEMOS (exit %d) ---\n%s" % (rc2, salida2[-6000:]))
        # El exit != 0 tiene que ser el del VEREDICTO (1 = «hay FAIL en el gate»), no el de
        # un uso incorrecto (2) ni el de un error de ejecución (3): si no se distinguiera,
        # un runner roto pasaría esta comprobación sin haber evaluado una sola aserción.
        comprueba("BDD-2a · con la demo incluida el runner sale con el exit de veredicto (1)",
                  rc2 == 1, "exit real: %d (2 = uso incorrecto, 3 = error de ejecución)" % rc2)
        comprueba("BDD-2b · META-02 es FAIL",
                  re.search(r"\bFAIL\b[^\n]*META-02|META-02[^\n]*\bFAIL\b", salida2) is not None,
                  "no hay veredicto FAIL para META-02")
        comprueba("BDD-2c · el resumen nombra el subcampo discrepante "
                  "«structured.capabilities.writes»",
                  "structured.capabilities.writes" in salida2,
                  "el detalle del FAIL no cita el path del subcampo")
        comprueba("BDD-2d · el detalle del FAIL muestra esperado y real del título",
                  "Titulo Inventado" in salida2 and "Documento 00" in salida2,
                  "el detalle no contrasta el valor esperado con el real")
        comprueba("BDD-2e · el invariante entre pasos también se reporta como fallo",
                  "structured.document.revision" in salida2,
                  "el invariante `same` fallido no aparece en el resumen")
        comprueba("BDD-2f · META-01 sigue siendo PASS con las demos incluidas",
                  re.search(r"\bPASS\b[^\n]*META-01|META-01[^\n]*\bPASS\b", salida2) is not None,
                  "META-01 dejó de pasar al incluir las demos")

        # ---- Estructural: ni una ruta absoluta de máquina en el directorio del banco --
        # El patrón se COMPONE en vez de escribirse literal: si esta línea llevara la
        # cadena tal cual, este mismo fichero haría fallar su propia comprobación.
        patron = "/" + "Users" + "/"
        grep = subprocess.run(["grep", "-rIl", patron, AQUI], capture_output=True, text=True)
        ficheros = [f for f in grep.stdout.split() if "/__pycache__/" not in f]
        comprueba("Estructural · ningún fichero de docs/qa/testbench/ lleva una ruta "
                  "absoluta de máquina (E33-H02, criterio estructural)",
                  not ficheros, "ficheros con ruta absoluta:\n" + "\n".join(ficheros))
    finally:
        if temporal and not args.keep:
            shutil.rmtree(temporal, ignore_errors=True)

    print("\n%s" % ("=" * 60))
    if fallos:
        print("SELFTEST EN ROJO: %d comprobación(es) fallidas: %s" % (len(fallos), ", ".join(fallos)))
        return 1
    print("SELFTEST EN VERDE: el runner cumple el contrato de FORMATO_EXPECT.md")
    return 0


if __name__ == "__main__":
    sys.exit(main())
