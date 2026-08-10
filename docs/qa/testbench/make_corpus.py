#!/usr/bin/env python3
"""Genera el CORPUS CANÓNICO del banco de conformidad (E33-H01, `ARCHITECTURE.md §22.2`).

Uso:
    make_corpus.py DEST [--seed N] [--no-patologicos]

El corpus es el campo de pruebas contra el que se escriben los esperados del banco
(`§22.3`), así que su única propiedad no negociable es el **determinismo**: la misma
semilla produce el mismo árbol, byte a byte, en cualquier máquina. De ahí las reglas
que este script se impone:

  * nada de `datetime.now()` ni de `uuid`: las fechas se derivan del índice del
    documento y las rutas son función pura de él;
  * nada de `os.listdir()`/`set`/`dict` recorridos por orden de inserción incidental:
    todo lo que se itera es una lista literal, en el orden en que está escrita;
  * PRNG **propio** (SplitMix64), no `random`: el módulo estándar no promete la misma
    secuencia entre versiones de Python, y el corpus tiene que sobrevivir a un upgrade
    del intérprete;
  * el symlink de `chk_b` se crea **relativo**, no absoluto: un destino absoluto
    metería la ruta de la máquina dentro del árbol.

Qué contiene (~60–90 documentos, según flags):

  1. Una red temática de documentos con **grafo real**: enlaces resueltos entre
     secciones, documentos huérfanos, dangling a `.md` (Err) y a ficheros del proyecto
     (Warn), y un case-mismatch.
  2. **Frontmatter heterogéneo y consultable**: tipos mezclados (`priority` número en
     unos, cadena en otros), fechas, listas, nulos explícitos y campos de referencia
     estilo `relacionadas:`.
  3. Los **sets patológicos de `make_fixtures.py`** integrados (`mdi`, `chk_a`,
     `chk_b`, `gaps`, `dirlinks`, `kinds`): la fauna de diagnósticos que el banco
     asevera. Se omiten con `--no-patologicos` cuando se quiere un corpus limpio.
  4. Las **semillas de los centinelas de H03** (`centinelas/`): una referencia de
     frontmatter rota, un par de paths que difieren solo en caja y un par NFC/NFD.
     Son los casos cuyo esperado es el comportamiento vigente de `decisiones §22`/`§24`.

Verificación del determinismo (el comando que documenta `README.md`):

    ./make_corpus.py /tmp/corpus-a && ./make_corpus.py /tmp/corpus-b
    diff -r /tmp/corpus-a /tmp/corpus-b && echo "idénticos"

Regla del repo: el corpus se genera en runtime y **nunca** se commitea.
"""
import argparse
import os
import sys
import unicodedata

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import make_fixtures  # noqa: E402  (los sets patológicos que se integran)

# Semilla por defecto: arbitraria pero FIJA. Lo que importa es que no cambie.
SEED = 0xE330001

# Secciones temáticas: (directorio, valor de `type:`). Lista literal = orden estable.
SECCIONES = [
    ("guias", "guia"),
    ("equipos", "equipo"),
    ("decisiones", "decision"),
    ("notas", "nota"),
]

ESTADOS = ["activo", "pendiente", "archivado"]
ETIQUETAS = ["infra", "producto", "seguridad", "datos", "onboarding"]

# Nº de documentos de la red temática. Con los sets patológicos y los centinelas, el
# total queda dentro del «~50–100 documentos» de §22.2.
N_TEMATICOS = 48

# Documentos finales de la red que quedan deliberadamente AISLADOS (ni entrantes ni
# salientes): `graph_query isolated` tiene que tener qué devolver.
N_AISLADOS = 6


class Aleatorio:
    """SplitMix64 — misma semilla, misma secuencia, en cualquier Python."""

    MASK = (1 << 64) - 1

    def __init__(self, semilla):
        self.estado = semilla & self.MASK

    def siguiente(self):
        self.estado = (self.estado + 0x9E3779B97F4A7C15) & self.MASK
        z = self.estado
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & self.MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & self.MASK
        return z ^ (z >> 31)

    def hasta(self, n):
        """Entero en [0, n)."""
        return self.siguiente() % n


def w(dest, rel, content, mode="w"):
    """Escribe `rel` bajo `dest` creando los directorios intermedios."""
    path = os.path.join(dest, rel)
    os.makedirs(os.path.dirname(path) or dest, exist_ok=True)
    with open(path, mode, encoding=None if "b" in mode else "utf-8") as f:
        f.write(content)


def ruta_tematica(i):
    """Ruta del documento `i` de la red. Función pura de `i` (no de la semilla): así los
    enlaces se calculan sin materializar el corpus entero."""
    directorio = SECCIONES[i % len(SECCIONES)][0]
    return "%s/doc-%02d.md" % (directorio, i)


def relativo(origen, destino):
    """Href relativo de `destino` visto desde `origen` (ambos relativos a la raíz)."""
    return "../" * origen.count("/") + destino


def red_tematica(dest, rng):
    """La red de documentos con grafo, huérfanos, frontmatter heterogéneo y fauna."""
    conectados = N_TEMATICOS - N_AISLADOS

    for i in range(N_TEMATICOS):
        tipo = SECCIONES[i % len(SECCIONES)][1]
        ruta = ruta_tematica(i)
        aislado = i >= conectados

        estado = ESTADOS[rng.hasta(len(ESTADOS))]
        etiqueta_a = ETIQUETAS[rng.hasta(len(ETIQUETAS))]
        etiqueta_b = ETIQUETAS[rng.hasta(len(ETIQUETAS))]
        # Tipos MEZCLADOS a propósito: `priority` es número en la mayoría y cadena en
        # uno de cada cuatro, así `metadata_inspect` ve un campo con dos tipos.
        if rng.hasta(4) == 0:
            prioridad = '"%d"' % (1 + rng.hasta(5))
        else:
            prioridad = "%d" % (1 + rng.hasta(5))
        # Fecha derivada del índice, jamás del reloj.
        fecha = "2026-%02d-%02d" % (1 + i % 12, 1 + i % 28)

        fm = [
            "---",
            "title: Documento %02d" % i,
            "type: %s" % tipo,
            "status: %s" % estado,
            "priority: %s" % prioridad,
            'updated: "%s"' % fecha,
            "tags: [%s, %s]" % (etiqueta_a, etiqueta_b),
        ]
        # Campo de REFERENCIA en frontmatter (estilo `relacionadas:`) y un null explícito
        # cada pocos documentos: material para las queries del banco.
        if not aislado and i % 3 == 0 and conectados > 1:
            otro = (i + 1 + rng.hasta(conectados - 1)) % conectados
            fm += ["relacionadas:", "  - %s" % ruta_tematica(otro)]
        if i % 5 == 0:
            fm.append("revisor: null")
        fm += ["---", ""]

        cuerpo = ["# Documento %02d" % i, "", "Documento canonico numero %d." % i, ""]
        for p in range(1 + rng.hasta(4)):
            cuerpo += ["Parrafo %d con texto buscable de relleno. " % p * (1 + rng.hasta(6)), ""]

        if not aislado:
            cuerpo += ["## Relacionados", ""]
            for _ in range(1 + rng.hasta(3)):
                destino_i = rng.hasta(conectados)
                if destino_i == i:
                    continue
                cuerpo.append("- [Documento %02d](%s)"
                              % (destino_i, relativo(ruta, ruta_tematica(destino_i))))
            cuerpo.append("")
            # Fauna de enlaces: dangling a documento (Err) y a fichero de proyecto (Warn).
            if i % 11 == 0:
                cuerpo += ["Pendiente: [borrador](%szz-inexistente/borrador-%02d.md)."
                           % ("../" * ruta.count("/"), i), ""]
            if i % 13 == 0:
                cuerpo += ["Apoyo: [script](%szz-inexistente/tarea-%02d.sh)."
                           % ("../" * ruta.count("/"), i), ""]

        w(dest, ruta, "\n".join(fm + cuerpo))

    # LINK-CASE-MISMATCH: `guias/doc-00.md` existe; `Guias/Doc-00.md` no, pero coincide
    # salvo capitalización → Warn (no portable entre sistemas de ficheros).
    w(dest, "guias/case-mismatch.md",
      '---\ntitle: Case mismatch\ntype: guia\nstatus: activo\npriority: 3\n'
      'updated: "2026-03-03"\ntags: [infra]\n---\n\n'
      "# Case mismatch\n\nVer [la guia](../Guias/Doc-00.md).\n")

    # LINK-ESCAPES-WORKSPACE: destino por encima de la raíz → Err no configurable.
    w(dest, "notas/escapa.md",
      '---\ntitle: Escapa\ntype: nota\nstatus: pendiente\npriority: "1"\n'
      'updated: "2026-04-04"\ntags: [seguridad]\n---\n\n'
      "# Escapa\n\nVer [fuera](../../../fuera-del-workspace.md).\n")

    # Navegación PURA (`../`, `./`): legítima desde E23-H11, NO produce diagnóstico. Es el
    # contraste que impide que un esperado del banco confunda «enlace raro» con «error».
    #
    # Ojo con el matiz vigente de `links::clasificar`: un href que NOMBRA algo (`../guias/`)
    # sigue siendo `Missing("guias")` aunque acabe en barra —no hay heurística de barra
    # final—, o sea un `LINK-TARGET-MISSING` de nivel `Warn`. Por eso aquí solo van los
    # hrefs que no nombran nada; el otro caso ya lo aporta `fixtures/raiz-dir.md` de
    # `dirlinks`, donde está declarado como tal.
    w(dest, "notas/navegacion.md",
      '---\ntitle: Navegacion\ntype: nota\nstatus: activo\npriority: 2\n'
      'updated: "2026-05-05"\ntags: [onboarding]\n---\n\n'
      "# Navegacion\n\n[volver](../) y [aqui mismo](./)\n")


def centinelas(dest):
    """Las semillas de los centinelas de E33-H03 (`ARCHITECTURE.md §22.3`).

    Su esperado es el **comportamiento vigente** de las decisiones abiertas
    `decisiones §22` y `§24`: si el comportamiento cambia, el centinela falla y obliga a
    actualizar esperado y ficha a la vez. Aquí solo se planta el material; el esperado lo
    escribe H03.
    """
    # (a) Referencia de FRONTMATTER rota: `relacionadas:` apunta a un documento que no
    # existe. Vigente: el grafo solo mira enlaces del CUERPO, así que esto NO es un
    # diagnóstico de enlace ni una arista — el centinela fija justo eso.
    w(dest, "centinelas/ref-frontmatter-rota.md",
      '---\ntitle: Referencia de frontmatter rota\ntype: nota\nstatus: pendiente\n'
      'priority: 4\nupdated: "2026-06-06"\ntags: [datos]\n'
      "relacionadas:\n  - centinelas/no-existe-jamas.md\n---\n\n"
      "# Referencia de frontmatter rota\n\n"
      "El destino de `relacionadas:` no existe; el cuerpo no enlaza a nadie.\n")

    # (b) Par de paths que difieren SOLO en caja. OJO, y esto es el dato que el centinela
    # de H03 tiene que fijar: en APFS/HFS+ (macOS, insensible a la caja por defecto) el
    # segundo `w()` **pisa** al primero y en disco queda UN fichero (`informe.md` con el
    # contenido de `Informe.md`); en un ext4 quedan dos. O sea que el nº de documentos del
    # corpus depende del sistema de ficheros, y el esperado del banco debe expresarse en
    # términos del comportamiento observado, no de un conteo absoluto.
    w(dest, "centinelas/caja/informe.md",
      '---\ntitle: Informe minusculas\ntype: nota\nstatus: activo\npriority: 1\n'
      'updated: "2026-07-07"\ntags: [datos]\n---\n\n# Informe\n\nRuta en minusculas.\n')
    w(dest, "centinelas/caja/Informe.md",
      '---\ntitle: Informe mayuscula\ntype: nota\nstatus: activo\npriority: 1\n'
      'updated: "2026-07-08"\ntags: [datos]\n---\n\n# Informe\n\nRuta con mayuscula.\n')

    # (c) Par NFC/NFD DE VERDAD: el mismo nombre lógico («canción.md»), con la ÚNICA
    # diferencia de la forma Unicode. Nada de sufijos que los distingan — un `-nfc`/`-nfd`
    # los volvería dos nombres distintos que no pueden colisionar en ningún filesystem, y
    # el centinela no probaría nada.
    #
    # Comportamiento OBSERVADO (medido en esta máquina el 2026-08-10, no recordado):
    #   · APFS (macOS, normalizante): el segundo `w()` **pisa** al primero. En disco queda
    #     UN fichero, `canción.md`, con el contenido del que escribió en forma NFD, y
    #     `os.path.exists()` responde True para las DOS formas — el filesystem las trata
    #     como el mismo nombre.
    #   · Linux/ext4 (no normalizante): son dos nombres distintos ⇒ DOS ficheros.
    # O sea que el nº de documentos del corpus depende del filesystem, igual que en (b).
    nfc = unicodedata.normalize("NFC", "canción")
    nfd = unicodedata.normalize("NFD", "canción")
    assert nfc != nfd, "el par debe diferir en la forma Unicode y en nada más"
    w(dest, "centinelas/unicode/%s.md" % nfc,
      '---\ntitle: Forma NFC\ntype: nota\nstatus: activo\npriority: 2\n'
      'updated: "2026-08-01"\ntags: [datos]\n---\n\n# Forma NFC\n\n'
      "Escrito con el nombre de fichero en forma COMPUESTA (NFC).\n")
    w(dest, "centinelas/unicode/%s.md" % nfd,
      '---\ntitle: Forma NFD\ntype: nota\nstatus: activo\npriority: 2\n'
      'updated: "2026-08-02"\ntags: [datos]\n---\n\n# Forma NFD\n\n'
      "Escrito con el nombre de fichero en forma DESCOMPUESTA (NFD).\n")
    # Control ASCII: mismo patrón de nombre sin un solo carácter compuesto, para poder
    # distinguir «esto lo hace la normalización Unicode» de «esto lo hace el resolutor de
    # enlaces».
    w(dest, "centinelas/unicode/cancion.md",
      '---\ntitle: Control ASCII\ntype: nota\nstatus: activo\npriority: 2\n'
      'updated: "2026-08-04"\ntags: [datos]\n---\n\n# Control ASCII\n\n'
      "Sin caracteres compuestos.\n")
    # Enlazador: apunta al par por sus DOS formas y al control ASCII. En APFS ambos hrefs
    # resuelven al mismo documento; en ext4, uno resuelve y el otro es un dangling. Ese
    # contraste es justo lo que el centinela de H03 tiene que fijar.
    w(dest, "centinelas/unicode/enlazador.md",
      '---\ntitle: Enlazador unicode\ntype: nota\nstatus: activo\npriority: 3\n'
      'updated: "2026-08-03"\ntags: [datos]\n---\n\n# Enlazador\n\n'
      "[forma NFC](%s.md), [forma NFD](%s.md) y el [control ASCII](cancion.md).\n"
      % (nfc, nfd))


def patologicos(dest):
    """Integra los sets de `make_fixtures.py` que aportan fauna al corpus canónico.

    Se reusa el script existente en vez de copiar sus documentos: es la misma fauna que
    ya cubren los lotes históricos, y así no puede divergir.

    Quedan fuera exactamente los sets que **escriben `.lodestar/config.yaml`** —`cfg_*`,
    `refroots` y `warngate_b`—, porque esa config cambiaría la política de validación o el
    gate de TODO el corpus, no solo de sus documentos; y `ignore`, que planta un
    `.lodestarignore` en la raíz y podaría documentos del descubrimiento.

    `warngate_a` **sí** entra: al contrario que `warngate_b`, no escribe config alguna —
    son dos documentos que se enlazan entre sí más un enlace a un `.png` inexistente
    (un `LINK-TARGET-MISSING` de nivel `Warn`), que es fauna legítima del corpus.
    """
    for fn in (make_fixtures.mdi, make_fixtures.chk_a, make_fixtures.chk_b,
               make_fixtures.gaps, make_fixtures.dirlinks, make_fixtures.kinds,
               make_fixtures.warngate_a):
        fn(dest)

    # `chk_b` crea el symlink apuntando a un destino ABSOLUTO, que metería la ruta de la
    # máquina dentro del árbol y rompería el determinismo entre máquinas. Se rehace
    # relativo (el diagnóstico `SYMLINK-UNSUPPORTED` es el mismo).
    link = os.path.join(dest, "fixtures/enlace-simbolico.md")
    if os.path.lexists(link):
        os.remove(link)
    os.symlink("../README.md", link)


def readme(dest):
    """Un README del propio corpus: lo primero que ve quien lo inspecciona a mano."""
    w(dest, "README.md",
      "# Corpus canonico del banco de conformidad\n\n"
      "Generado por `docs/qa/testbench/make_corpus.py` (E33-H01, `ARCHITECTURE.md "
      "§22.2`). No se commitea: se regenera con la misma semilla cuando hace falta.\n\n"
      "Secciones: `guias/`, `equipos/`, `decisiones/`, `notas/`, `centinelas/` y "
      "`fixtures/` (los sets patologicos).\n\n"
      "Entradas: [primera guia](guias/doc-00.md), "
      "[primer equipo](equipos/doc-01.md), "
      "[centinela de frontmatter](centinelas/ref-frontmatter-rota.md).\n")


def main():
    p = argparse.ArgumentParser(description="Genera el corpus canonico del banco.")
    p.add_argument("dest", help="directorio destino (se crea si no existe)")
    p.add_argument("--seed", type=lambda s: int(s, 0), default=SEED,
                   help="semilla del PRNG (por defecto 0x%X)" % SEED)
    p.add_argument("--no-patologicos", action="store_true",
                   help="omite los sets de make_fixtures.py (corpus limpio)")
    args = p.parse_args()

    os.makedirs(args.dest, exist_ok=True)
    rng = Aleatorio(args.seed)

    readme(args.dest)
    red_tematica(args.dest, rng)
    centinelas(args.dest)
    if not args.no_patologicos:
        patologicos(args.dest)

    documentos = sum(len([f for f in files if f.endswith(".md")])
                     for _, _, files in os.walk(args.dest))
    print("corpus canonico generado en %s (semilla 0x%X, %d documentos .md)"
          % (args.dest, args.seed, documentos))


if __name__ == "__main__":
    main()
