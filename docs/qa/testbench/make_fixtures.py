#!/usr/bin/env python3
"""Inyecta documentos patológicos en un worktree. Uso: make_fixtures.py SET DEST

Sets:
  mdi    — claves con punto literal y clave de primer nivel `frontmatter`
  chk_a  — patologías de contenido (frontmatter roto, enlaces rotos, wikilinks...)
  chk_b  — patologías binarias/estructurales (no-UTF8, symlink, BOM, >10MiB, tipos mezclados)
"""
import os
import sys


def w(dest, rel, content, mode="w"):
    path = os.path.join(dest, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, mode, encoding=None if "b" in mode else "utf-8") as f:
        f.write(content)


def mdi(dest):
    # MDI-10 / CHK-10: clave con punto literal, no anidada
    w(dest, "fixtures/dot-key.md",
      '---\ntitle: Clave con punto\n"sonar.projectKey": atlas-api\n---\n\n# Clave con punto\n')
    # MDI-11/12 / CHK-11: clave de primer nivel llamada frontmatter
    w(dest, "fixtures/frontmatter-key.md",
      "---\ntitle: Clave frontmatter\nfrontmatter:\n  x: 1\n---\n\n# Clave frontmatter\n")
    # apoyo para distinguir clave literal a.b vs mapa anidado a:{b:}
    w(dest, "fixtures/nested-env.md",
      "---\ntitle: Env anidado\nenv:\n  region: us-east\n---\n\n# Env anidado\n")
    w(dest, "fixtures/flat-env.md",
      '---\ntitle: Env plano\n"env.region": eu-west\n---\n\n# Env plano\n')


def chk_a(dest):
    # CHK-01 FM-UNCLOSED: frontmatter sin cerrar
    w(dest, "fixtures/roto-unclosed.md", "---\ntitle: Sin cerrar\ntype: equipo\n\n# Sin cerrar\n")
    # CHK-02 FM-YAML-INVALID
    w(dest, "fixtures/roto-yaml.md", "---\ntitle: x\n  bad: [unclosed\n---\n\n# Roto\n")
    # CHK-03 DOC-CONFLICT-MARKER
    w(dest, "fixtures/conflicto.md",
      "---\ntitle: Conflicto\n---\n\n<<<<<<< HEAD\nA\n=======\nB\n>>>>>>> feature\n")
    # CHK-04 LINK-TARGET-MISSING (destino .md -> Err)
    w(dest, "fixtures/enlace-roto.md",
      "---\ntitle: Enlace roto\n---\n\nVer [algo](../equipos/no-existe.md).\n")
    # CHK-05 LINK-TARGET-MISSING (destino no-.md -> Warn)
    w(dest, "fixtures/enlace-roto-warn.md",
      "---\ntitle: Enlace roto warn\n---\n\nVer [script](../scripts/no-existe.sh).\n")
    # CHK-06 LINK-ESCAPES-WORKSPACE
    w(dest, "fixtures/escapa.md",
      "---\ntitle: Escapa\n---\n\nVer [fuera](../../../fuera-del-workspace.md).\n")
    # CHK-07 LINK-CASE-MISMATCH (bastion.md existe, Bastion.md no)
    w(dest, "fixtures/case-mismatch.md",
      "---\ntitle: Case mismatch\n---\n\nVer [bastion](../equipos/Bastion.md).\n")
    # CHK-08 navegación pura, sin diagnóstico
    w(dest, "fixtures/navegacion-pura.md",
      "---\ntitle: Navegación\n---\n\n[volver](../)\n")
    # CHK-09 wikilink: debe parecer aislado
    w(dest, "fixtures/wikilink.md",
      "---\ntitle: Wikilink\n---\n\nVer [[bastion]] para la puerta de entrada.\n")


def chk_b(dest):
    # CHK-12 fecha sin comillas/sin ceros
    w(dest, "fixtures/fecha-mala.md",
      "---\ntitle: Fecha mala\ntype: pendiente\nupdated: 2026-8-5\n---\n\n# Fecha\n")
    # CHK-13 tipo mezclado: priority string donde el resto es número
    w(dest, "fixtures/priority-string.md",
      '---\ntitle: Priority string\ntype: pendiente\npriority: "2"\n---\n\n# Mixto\n')
    # CHK-14 DOC-NOT-UTF8
    w(dest, "fixtures/binario.md", b"---\ntitle: x\n---\n\n\xff\xfe basura\n", mode="wb")
    # CHK-15 SYMLINK-UNSUPPORTED
    link = os.path.join(dest, "fixtures/enlace-simbolico.md")
    os.makedirs(os.path.dirname(link), exist_ok=True)
    if not os.path.lexists(link):
        os.symlink(os.path.join(dest, "README.md"), link)
    # CHK-16 DOC-TOO-LARGE (>10 MiB)
    big = "---\ntitle: Gigante\n---\n\n" + ("relleno " * 10 + "\n") * 150000
    assert len(big.encode()) > 10 * 1024 * 1024
    w(dest, "fixtures/gigante.md", big)
    # CHK-17 DOC-BOM
    w(dest, "fixtures/con-bom.md", b"\xef\xbb\xbf---\ntitle: Con BOM\n---\n\n# BOM\n", mode="wb")
    # CHK-18 BOM + YAML inválido
    w(dest, "fixtures/bom-mas-roto.md",
      b"\xef\xbb\xbf---\ntitle: x\n  bad: [unclosed\n---\n\n# BOM roto\n", mode="wb")


def gaps(dest):
    # G1-06 null explícito y clave vacía
    w(dest, "fixtures/nulo.md",
      '---\ntitle: Nulo explicito\ntype: prueba\ncampo_nulo: null\nvacio:\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\nCuerpo.\n')
    # G1-07 título desde H1
    w(dest, "fixtures/sin-title-fm.md",
      '---\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\n# Titulo Desde El H1\n\nCuerpo.\n')
    # G1-07/08 sin frontmatter ni H1
    w(dest, "fixtures/pelado.md", "Solo un parrafo sin heading.\n")
    # G1-09 clasificación completa de enlaces
    w(dest, "fixtures/enlaces-variados.md",
      '---\ntitle: Enlaces variados\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\n'
      "# Enlaces variados\n\n"
      "Inline con fragmento: [diag](../equipos/bastion.md#diagnostico)\n"
      "Referencia: [la guia][tmux]\n"
      "Ancla propia: [arriba](#enlaces-variados)\n"
      "Externa: [web](https://example.com/x)\n"
      "Fichero de proyecto: [script](script.sh)\n\n"
      "[tmux]: ../guias/tmux.md\n")
    w(dest, "fixtures/script.sh", "#!/bin/sh\necho ok\n")
    # G1-10 href raíz-absoluto
    w(dest, "fixtures/raiz-absoluta.md",
      '---\ntitle: Raiz absoluta\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\n[bastion](/equipos/bastion.md)\n')
    # G1-12 delete sin entrantes
    w(dest, "fixtures/sin-entrantes.md",
      '---\ntitle: Sin entrantes\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\nEnlaza fuera: [bastion](../equipos/bastion.md)\n')
    # G1-13 replace_text con contador
    w(dest, "fixtures/contador.md",
      '---\ntitle: Contador\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\nfoo uno. foo dos. foo tres.\n')
    # G1-22 valores no representables en JSON
    w(dest, "fixtures/raros.md",
      '---\ntitle: Valores raros\ntype: prueba\nflotante: .inf\nsubmapa:\n  1: uno\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\nCuerpo.\n')


def cfg_writable(dest):
    w(dest, ".lodestar/config.yaml", "workspace:\n  writableRoots: [pendientes]\n")


def cfg_receipts(dest):
    w(dest, ".lodestar/config.yaml",
      'transactions:\n  maximumReceipts: 1\n  retainReceiptsFor: "24h"\n')


def cfg_validation(dest):
    w(dest, ".lodestar/config.yaml", "validation:\n  LINK-TARGET-MISSING: ignore\n")


def cfg_validation_familias(dest):
    # verify_G1-04: sintaxis por FAMILIA (config.rs §20.9), no por codigo de diagnostico
    w(dest, ".lodestar/config.yaml",
      "validation:\n  danglingDocumentLinks: ignore\n  missingWorkspaceFiles: ignore\n")


def cfg_broken(dest):
    w(dest, ".lodestar/config.yaml", 'gate:\n  blockWarnings: "yes please"\n')


def ignore(dest):
    # G2-01: .lodestarignore excluye y poda
    w(dest, ".lodestarignore", "borrador.md\nignorados/\n")
    w(dest, "borrador.md",
      '---\ntitle: Borrador excluido\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\n# Borrador\n')
    w(dest, "ignorados/oculto.md",
      '---\ntitle: Oculto podado\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\n# Oculto\n')
    w(dest, "enlazador.md",
      '---\ntitle: Enlazador\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\n[b](borrador.md) y [o](ignorados/oculto.md)\n')


def dirlinks(dest):
    # G2-02/G2-10: enlaces a directorios y navegación pura a distintas profundidades
    w(dest, "fixtures/raiz-dir.md",
      '---\ntitle: Enlace a directorio\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\n[g](guias/)\n')
    w(dest, "a/b/doc.md",
      '---\ntitle: Doc profundo\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\n[volver](../) y [raiz](../../)\n')
    w(dest, "a/otro.md",
      '---\ntitle: Otro\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\nVer [doc](b/doc.md).\n')


def kinds(dest):
    # G2-03: agotar el enum de kind (reference/collapsed/shortcut/autolink)
    w(dest, "fixtures/destino.md",
      '---\ntitle: Destino\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\n# Destino\n')
    w(dest, "fixtures/kinds.md",
      '---\ntitle: Kinds\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\n'
      "[full][ref1]\n[ref2][]\n[ref3]\n<https://example.com/auto>\n\n"
      "[ref1]: destino.md\n[ref2]: destino.md\n[ref3]: destino.md\n")


def warngate_a(dest):
    # G2-06 caso A: 1 warn (enlace a png inexistente), 0 err, sin config
    w(dest, "fixtures/wg-index.md",
      '---\ntitle: WG index\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\n[on](wg-onboarding.md) y [m](mapa.png)\n')
    w(dest, "fixtures/wg-onboarding.md",
      '---\ntitle: WG onboarding\ntype: prueba\ntags: [prueba]\nupdated: "2026-08-05"\n---\n\n[idx](wg-index.md)\n')


def warngate_b(dest):
    warngate_a(dest)
    w(dest, ".lodestar/config.yaml", "gate:\n  blockWarnings: true\n")


def refroots(dest):
    # G2-08: referenceRoots visibles pero nunca escribibles
    w(dest, ".lodestar/config.yaml",
      "workspace:\n  writableRoots: [pendientes]\n  referenceRoots: [equipos]\n")


SETS = {"mdi": [mdi], "chk_a": [chk_a], "chk_b": [chk_b], "gaps": [gaps],
        "cfg_writable": [cfg_writable], "cfg_receipts": [cfg_receipts],
        "cfg_validation": [cfg_validation], "cfg_broken": [cfg_broken],
        "cfg_validation_familias": [cfg_validation_familias],
        "ignore": [ignore], "dirlinks": [dirlinks], "kinds": [kinds],
        "warngate_a": [warngate_a], "warngate_b": [warngate_b],
        "refroots": [refroots]}

if __name__ == "__main__":
    fixture_set, dest = sys.argv[1], sys.argv[2]
    for fn in SETS[fixture_set]:
        fn(dest)
    print(f"fixtures '{fixture_set}' inyectados en {dest}")
