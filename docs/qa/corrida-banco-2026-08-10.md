# Corrida del banco de conformidad — 2026-08-10

> **Primera corrida del banco nuevo** (`ARCHITECTURE.md §22.3`, criterio **[BDD-3]** de
> `E33-H02`): el gate completo sobre un corpus canónico **recién generado**, con veredicto
> mecánico. Resultado: **97 casos, 0 FAIL, exit 0**.
>
> Esta corrida es evidencia datada, no un umbral: `§22` es instrumento interno y mientras
> `decisiones §14` siga abierta **nada de lo que el banco mida se promete en la superficie
> externa** (`§21.5`). El enganche a release —correr esto por cada versión y commitear la corrida—
> es `E33-H07`; aquí se establece el punto de partida. Esta evidencia completa **E33-H02**; H03–H08
> permanecen pendientes.

## Entorno

| | |
|---|---|
| Fecha | 2026-08-10 |
| Versión del workspace | `0.6.0` |
| Binario MCP | `target/release/lodestar-mcp` (release) |
| Binario CLI | `target/debug/lodestar` (lote `gate_H`) |
| Host | macOS 26.5.1, **APFS** (insensible a la caja, normalizante) |
| Python | 3.14.5 |
| Corpus | `make_corpus.py`, semilla por defecto `0xE330001`, **92 ficheros `.md`** en disco ⇒ **89 documentos** indexados |

**Por qué 92 ficheros dan 89 documentos**: el corpus planta a propósito dos pares de nombres que
colisionan en APFS y no en ext4 (un par que difiere solo en la caja y otro solo en la forma
Unicode NFC/NFD). En esta máquina cada par queda en un fichero. Es la razón por la que **ningún
esperado del banco asevera un conteo del inventario** (regla de oro de `FORMATO_EXPECT.md §6`): en
un runner Linux estos mismos 97 casos deben pasar igual, con otro número de documentos.

## Reproducción

```bash
cargo build --release -p lodestar-mcp && cargo build -p lodestar-cli
cd docs/qa/testbench
./make_corpus.py /tmp/corpus
LODESTAR_MCP_BIN=target/release/lodestar-mcp \
LODESTAR_CLI_BIN=target/debug/lodestar \
python3 lodestar_harness.py --run-all --root-corpus /tmp/corpus --out /tmp/corrida.json
```

El corpus se genera en runtime y **no se commitea** (regla del repo); el JSON de resultados crudos
son ~800 KB de respuestas del wire y tampoco se commitea — se regenera con el comando de arriba.
Lo que queda en el árbol es este resumen y los lotes, que son la definición del gate.

## Resultado

```
RESUMEN: gate 97 casos · PASSes 97 · FAILes 0 · exploratorios 0 · omitidos 0 · fuera de gate 0 (FAILes 0)
exit 0
```

| Lote | Casos | Veredicto | Qué cubre |
|---|---:|---|---|
| `gate_L1_consulta` | 8 | PASS | lenguaje de consulta, namespaces reservados, type errors del orden |
| `gate_L2_proyeccion` | 7 | PASS | `include` de search y get, `sections`, lo no pedido no viaja |
| `gate_L3_metadata` | 8 | PASS | `metadata_inspect` y las tres clases límite de dot-paths |
| `gate_L5_grafo` | 7 | PASS | las ocho operaciones de `graph_query`, `impact_analyze` |
| `gate_L6_plan` | 7 | PASS | forma del plan, hash determinista, guards de las operaciones |
| `gate_L7_apply` | 7 | PASS | round trip byte a byte y la familia de conflictos |
| `gate_L8_readonly` | 3 | PASS | el perfil oculta y rechaza; las lecturas no mutan |
| `gate_L9_check_a` | 9 | PASS | catálogo de diagnósticos de contenido y severidades |
| `gate_L10_check_b` | 5 | PASS | patologías binarias/estructurales; daño estructural vs semántico |
| `gate_L11_scopes` | 5 | PASS | los cuatro scopes de `knowledge_check` y `minimumSeverity` |
| `gate_L12_robustez` | 6 | PASS | cursores firmados, cotas de `limit`, errores de protocolo |
| `gate_G_descubrimiento` | 6 | PASS | ignore, writable/reference roots, config, `instructions` |
| `gate_H_cli_recuperacion` | 5 | PASS | exit codes congelados, gate de avisos, recuperación transparente |
| `gate_invariantes` | 6 | PASS | los transversales del informe §5 |
| `gate_verify_g1` | 5 | PASS | repros `verify_G1-*` no absorbidas por un lote temático |
| `gate_verify_g2` | 3 | PASS | repros `verify_G2-*` (move, retención, config viva) |
| **Total** | **97** | **0 FAIL** | |

El lote `meta_runner` (el autotest del runner) queda **fuera de `--run-all`** a propósito: se
ejecuta desde `selftest_runner.py`, que verificó las 12 comprobaciones del contrato en verde en
esta misma sesión.

## Qué prueba esta corrida (y qué no)

**Prueba** que el motor v0.6.0 se comporta como declaran sus fuentes en 97 puntos verificables
mecánicamente, entre ellos los invariantes que sostienen el resto: hash de plan determinista,
restauración byte a byte tras revert, `where ≡ filter` incluso en los errores, orden total estable
en la paginación, y que una tanda de lecturas no altera un byte.

**Prueba también** que los hallazgos de `decisiones §23` saldados por E28–E31 siguen saldados: el
banco los asevera con su comportamiento **corregido**, así que una regresión los devolvería a rojo.
Los que más importan: `M-01` (revert de un `-revert`, que era un no-op destructivo con pérdida del
redo) ahora rehace y encadena; `A-05` (`create`/`move` sobre path ocupado) es
`DOCUMENT_ALREADY_EXISTS` en la planificación; `A-02`/`A-03` (cursor malformado o ajeno) es
`INVALID_SCHEMA` en vez de servir la página 1 en silencio; `A-04` (`starts_with` sobre no-string) es
un type error ruidoso; `A-07` (scope `paths` con un typo) es `DOCUMENT_NOT_FOUND`.

**Un dato que la corrida destapó y que conviene registrar**: la regla de la casa sobre parámetros no
declarados se **invirtió** respecto de la campaña de agosto. Los casos históricos `ROB-13`/`ROB-14`
fijaban que una clave desconocida se ignoraba en silencio; hoy es `INVALID_SCHEMA` nombrándola y
listando las legales. El esperado histórico habría dado FAIL, y el portado lo asevera corregido
(`gate_L12` · `L12-ROB-13-14-PARAMS-DESCONOCIDOS`). Es exactamente la clase de deriva que el banco
existe para detectar.

**No prueba** rendimiento (es `E33-H04`/`H05`), ni cierra ninguna decisión: los centinelas de
`decisiones §22` (integridad referencial de frontmatter) y `§24` (caja/Unicode) son `E33-H03` y, cuando existan, el banco **detectará** un cambio de
comportamiento sin juzgarlo. Tampoco prueba nada sobre la cache SQLite: el producto no la lee
(`decisiones §14`) y el banco no la toca.

## Portabilidad

Ni un fichero de `docs/qa/testbench/` contiene una ruta absoluta de máquina (criterio estructural de
`E33-H02`, verificado por `selftest_runner.py`). La corrida histórica usó explícitamente el CLI
debug mediante `LODESTAR_CLI_BIN=target/debug/lodestar`; sin override, el runner usa su fallback
release relativo al repo. El binario sale de `--binary`/`LODESTAR_MCP_BIN` o de ese fallback; el
root es un argumento; los lotes usan tokens (`@root`, `@bin.mcp`, `@bin.cli`, `@repo`, `@testbench`). Los 97 casos deberían pasar en un runner Linux sin cambio
alguno — lo que variará es el número de documentos del corpus, que ningún esperado asevera.
