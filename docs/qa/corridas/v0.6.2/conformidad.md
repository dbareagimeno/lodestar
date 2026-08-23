# Corrida de conformidad — `v0.6.2`

El bruto JSON de esta corrida se conserva como artefacto comprimido externo. Su URL estable,
hash SHA-256, tamaño y esquema están en el [manifiesto de evidencia](manifest.json); este resumen
Markdown es la referencia versionada.

- **Fecha:** 2026-08-22
- **Veredicto:** **PASS — 103/103 casos, 0 FAIL, exit 0**
- **Corpus:** canónico de `make_corpus.py`, semilla `0xE330001`
- **Binario:** `lodestar-mcp` compilado en perfil `release`
- **Evidencia cruda:** artefacto comprimido externo inventariado en [`manifest.json`](manifest.json)

Esta es la primera corrida bajo la convención de release de E33-H07. El JSON conserva las
respuestas y veredictos caso a caso; las raíces efímeras y la ruta del checkout se representan
como `<ephemeral-root>` y `<repo>` para que el artefacto sea portable y no revele rutas privadas.

## Trazabilidad

- Estado del workspace: árbol integrado de E33 sobre
  `0c9d0ba7980a0d1059d918844ab3864f361b661b` (versión `0.6.2`), base descendiente del commit de
  release `73251af05fa2c4589447294719854801a23ae4aa`. La corrida incorpora los centinelas H03 del
  árbol de trabajo; no atribuye esos ficheros todavía no commiteados al commit base.
- Runner utilizado: `docs/qa/testbench/lodestar_harness.py`, commit `8976fb491ccd256fbfa3d970bfa94c3be0884d5e`.
- Modo: `--run-all`, los 18 lotes canónicos del gate; los casos `gate: false` no se incluyeron.
- La corrida fue contra un corpus recién generado; el corpus no se versiona.

El runner corregido de la trazabilidad anterior conserva como silencio observable las líneas que
rmcp descarta sin `id`; por eso el caso de protocolo de L12 queda aseverado contra el transporte
real y no contra un error inventado. Esto no altera el motor ni el contrato MCP.

## Reproducción

Desde la raíz del repositorio y en una máquina con los binarios release:

```bash
set -euo pipefail
cargo build --release --locked -p lodestar-cli -p lodestar-mcp
CORPUS="$(mktemp -d)/corpus"
./docs/qa/testbench/make_corpus.py "$CORPUS"
python3 docs/qa/testbench/lodestar_harness.py \
  --run-all \
  --root-corpus "$CORPUS" \
  --binary target/release/lodestar-mcp \
  --binary-cli target/release/lodestar \
  --out /ruta/temporal/conformidad.json
```

Un exit distinto de cero es un **stop-the-line**: no se abre ni se mergea el PR de release hasta
corregir la regresión y repetir la corrida. El gate no juzga conteos absolutos del inventario,
porque dependen del filesystem; sí exige el veredicto mecánico de todos los casos.

Para la convención completa por release, incluyendo la corrida de rendimiento y su `--gate`, ver
[`docs/qa/testbench/README.md`](../../testbench/README.md) y [`RELEASING.md`](../../../../RELEASING.md).

## Stdout del runner (corrida reproducida)

La siguiente es la salida stdout real de una ejecución reproducida con los binarios release y un
corpus generado con la misma semilla. Solo se saneó la ruta efímera que el propio runner imprime;
el veredicto y el listado de casos no se editaron.

~~~text

### lote gate_L1_consulta (corpus, perfil readonly)
PASS   L1-TYP-01
PASS   L1-TYP-04
PASS   L1-TYP-07
PASS   L1-TYP-08
PASS   L1-TYP-09
PASS   L1-TYP-ORDEN-MIXTO
PASS   L1-TYP-IGUALDAD-NO-TIPA
PASS   L1-TYP-HAS-MISSING

### lote gate_L2_proyeccion (corpus, perfil readonly)
PASS   L2-PRJ-02
PASS   L2-PRJ-03
PASS   L2-PRJ-05
PASS   L2-PRJ-04
PASS   L2-PRJ-08
PASS   L2-PRJ-BACKLINKS
PASS   L2-PRJ-SECTIONS

### lote gate_L3_metadata (corpus, perfil readonly)
PASS   L3-MDI-01
PASS   L3-MDI-03
PASS   L3-MDI-04
PASS   L3-MDI-05
PASS   L3-MDI-06-07
PASS   L3-MDI-10-LIMITE-1
PASS   L3-MDI-11-LIMITE-2
PASS   L3-MDI-10b-LIMITE-3

### lote gate_L5_grafo (corpus, perfil readonly)
PASS   L5-GRF-01
PASS   L5-GRF-02
PASS   L5-GRF-04-ISOLATED
PASS   L5-GRF-05-DANGLING
PASS   L5-GRF-06-PATH-BETWEEN
PASS   L5-GRF-OPERACION-INVALIDA
PASS   L5-IMP-AFFECTED

### lote gate_L6_plan (corpus, perfil standard)
PASS   L6-PLN-01
PASS   L6-PLN-02-HASH-DETERMINISTA
PASS   L6-PLN-DEFAULTS-POLICY
PASS   L6-PLN-DELETE-INBOUND
PASS   L6-PLN-OCUPADO
PASS   L6-PLN-REVISION-CONFLICT
PASS   L6-PLN-SELECCION-MASIVA
PASS   L6-PLN-SECUENCIAL-MISMO-DOCUMENTO

### lote gate_L7_apply (corpus, perfil standard)
PASS   L7-APL-01-ROUND-TRIP
PASS   L7-APL-07-PLAN-STALE
PASS   L7-APL-PLAN-INEXISTENTE
PASS   L7-APL-INVALID-RESULT
PASS   L7-APL-WRITE-CONFLICT
PASS   L7-APL-PERMISSION-DENIED
PASS   L7-APL-REVERT-DEL-REVERT
PASS   L7-APL-SECUENCIAL-MISMO-DOCUMENTO

### lote gate_L8_readonly (corpus, perfil readonly)
PASS   L8-ROB-17
PASS   L8-ROB-18-CAPABILITIES
PASS   L8-READONLY-NO-MUTA

### lote gate_L9_check_a (corpus, perfil readonly)
PASS   L9-CHK-01-FM-UNCLOSED
PASS   L9-CHK-02-FM-YAML-INVALID
PASS   L9-CHK-03-CONFLICT-MARKER
PASS   L9-CHK-04-05-MISMO-CODIGO-DOS-NIVELES
PASS   L9-CHK-06-ESCAPES
PASS   L9-CHK-07-CASE-MISMATCH
PASS   L9-CHK-08-NAVEGACION-PURA
PASS   L9-CHK-DIRLINK
PASS   L9-CHK-09-WIKILINK

### lote gate_L10_check_b (corpus, perfil readonly)
PASS   L10-CHK-17-BOM
PASS   L10-CHK-18-BOM-MAS-ROTO
PASS   L10-CHK-12-13-DANO-SEMANTICO
PASS   L10-CHK-14-15-16-DESCUBRIMIENTO
PASS   L10-CHK-TOO-LARGE-Y-SYMLINK

### lote gate_L11_scopes (corpus, perfil readonly)
PASS   L11-CHK-20-21-SCOPE-ACOTA
PASS   L11-CHK-23-UMBRAL
PASS   L11-CHK-24-DOCUMENT-FANTASMA
PASS   L11-CHK-22-AFFECTED
PASS   L11-CHK-27-COUNTS-VS-SUMMARY

### lote gate_L12_robustez (corpus, perfil readonly)
PASS   L12-ROB-07-08-PAGINACION
PASS   L12-ROB-05-CURSOR-MALFORMADO
PASS   L12-ROB-06-CURSOR-AJENO
PASS   L12-ROB-09-11-COTAS-LIMIT
PASS   L12-ROB-13-14-PARAMS-DESCONOCIDOS
PASS   L12-ROB-15-PROTOCOLO

### lote gate_G_descubrimiento (corpus, perfil standard)
PASS   G-IGNORE-PODA
PASS   G-REFROOTS
PASS   G-REFROOT-INMUTABLE
PASS   G-CONFIG-ROTA
PASS   G-CONFIG-VALIDATION-FAMILIAS
PASS   G-INICIALIZACION-PROTOCOLO

### lote gate_H_cli_recuperacion (corpus, perfil standard)
PASS   H-CLI-EXIT-CODES
PASS   H-MCP-ARRANQUE
PASS   H-WARNGATE
PASS   H-RECEIPTS-RETENCION
PASS   H-RECUPERACION-TRANSPARENTE

### lote gate_invariantes (corpus, perfil readonly)
PASS   INV-WHERE-EQUIV-FILTER
PASS   INV-LECTURAS-NO-MUTAN
PASS   INV-REVISION-ES-DEL-CONTENIDO
PASS   INV-ORDEN-DETERMINISTA
PASS   INV-ERRORES-TIENEN-CODIGO
PASS   INV-DOT-PATH-TRES-CLASES

### lote gate_verify_g1 (corpus, perfil standard)
PASS   V-G1-07-TITULO-DERIVADO
PASS   V-G1-22-DOT-PATH-INCLUDE
PASS   V-G1-13-REPLACE-TEXT-NOOP
PASS   V-G1-14-PATCH-RFC-7386
PASS   V-G2-03-KINDS-DE-ENLACE

### lote gate_verify_g2 (corpus, perfil standard)
PASS   V-G2-10-MOVE-Y-NAVEGACION
PASS   V-G2-04-RETENCION-GC
PASS   V-G1-04-VALIDATION-CONFIG-VIVA

### lote sentinela_s22 (corpus, perfil readonly)
PASS   S22-01
PASS   S22-02

### lote sentinela_s24 (corpus, perfil standard)
PASS   S24-01
PASS   S24-02

RESUMEN: gate 103 casos · PASSes 103 · FAILes 0 · exploratorios 0 · omitidos 0 · fuera de gate 0 (FAILes 0)
resultados en <ephemeral-root>/conformidad.raw.json
~~~
