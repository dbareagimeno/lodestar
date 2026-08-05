# Informe QA: pruebas extensivas de la superficie MCP sobre el workspace homelab

- **Fecha**: 2026-08-06 · **Binarios**: `lodestar-mcp` v0.5.0 (release, 2026-08-05) y CLI `lodestar` (debug) · **Host**: macOS/APFS (case-insensitive)
- **Campo de pruebas**: `~/repos/homelab` (36 documentos, 166 enlaces, 0 diagnósticos) — lecturas siempre bajo `--profile readonly`; toda mutación e inyección de fixtures en worktrees git efímeros. El repo real quedó byte-idéntico (misma `workspaceRevision` blake3 `4a7bba14…` antes y después).
- **Método**: matriz de casos con resultado esperado citando fuente (`contracts/mcp.yml`, `docs/user/*.md`, `ARCHITECTURE.md §19-§20`, `convenciones.md` del homelab); ejecución mecánica vía arnés JSON-RPC/stdio; **todo veredicto no-PASS pasó por verificación adversarial** (releer la fuente + reproducción aislada + intento de refutación) antes de contar como hallazgo. Tres rondas: matriz base (153 casos) → huecos (25) → huecos finales (11). La 4ª pasada de búsqueda no aportó casos genuinos: bucle cerrado.

## Resumen

**189 casos ejecutados. 176 conformes con el contrato tras verificación. 1 bug de motor confirmado, 2 discrepancias documentales confirmadas y 10 huecos contractuales registrados** (comportamientos reales que ninguna fuente fija, la mayoría "silencios" que el propio proyecto declara querer evitar). 12 esperados nuestros resultaron mal derivados y la verificación adversarial los reclasificó — en todos ellos el motor coincidía con la fuente leída completa.

El resultado global es muy favorable al motor: toda la mecánica central (lenguaje de consulta y sus type errors, proyecciones, grafo, ciclo plan/apply/revert con hash determinista y restauración byte a byte, concurrencia optimista, gate diferencial, writableRoots/referenceRoots, retención, catálogo de diagnósticos con severidades, descubrimiento, CLI y sus exit codes congelados, paginación cross-proceso) se comporta **exactamente** como declaran sus fuentes.

---

## 1. Bug de motor (1)

### M-01 · `change_revert` sobre un recibo `-revert` es un no-op silencioso que además destruye el redo

**Caso G1-18 · CONFIRMADO · gravedad alta (pérdida de datos de recuperación, sin error).**

Contrato: `mcp.yml` (change_revert, retorno) — «Es INVERSO al apply: `previousWorkspaceRevision == resultRevision` del apply revertido; `workspaceRevision == previousRevision` del apply» y semántica «como una NUEVA transacción inversa recuperable»; `safe-changes.md` L117-119 — "undoing is itself a transaction, with its own journal and its own recovery copies"; la intención de E25-H05 (`recovery.rs` L963-969) es justo que «deshacer el *undo*» no sea imposible.

Observado (reproducido en worktree limpio): `workspace_status` **lista** el recibo `X-revert` como revertible; `change_revert(receiptId: "X-revert")` responde `reverted: true` con `changedPaths` no vacío, pero devuelve el **mismo** `receiptId X-revert` (el sufijo no apila), `previousWorkspaceRevision == workspaceRevision` (no restaura el estado post-apply) y el fichero queda intacto — no-op sin error. Efecto destructivo adicional verificado: `backup_originals` **sobrescribe** `recovery/X-revert/` (que guardaba el estado redo) con el estado actual y reescribe `receipts/X-revert.json` como degenerado (`A→A`): el redo prometido queda destruido de forma permanente y silenciosa.

Causa raíz (diagnóstico): `lodestar-app/src/lib.rs` ~L2168 — `orig_txn_id = transaction_id(&receipt.change_set_id)`; el recibo `-revert` conserva el `changeSetId` **original**, así que revertirlo re-restaura `recovery/X/` (pre-apply, ya vigente) en vez de `recovery/X-revert/`, y `revert_txn_id = "{orig}-revert"` colisiona con el recibo que se está revirtiendo.

Repro: `scratchpad/batches/verify_G1-18b.json` del testbench (plan → apply → revert → revert del `-revert`).

---

## 2. Discrepancias documentales confirmadas (2)

### D-01 · Bajo `readonly`, `instructions` nombra las 10 tools mientras `tools/list` sirve 7

**Caso G1-24 · CONFIRMADO.** `mcp.yml` (meta.protocolo.instructions) garantiza que el texto «nombre EXACTAMENTE las tools que sirve `tools/list`» — y el test interno lo endurece («ni una de menos… ni una de más: una tool que no existe manda al agente a un `-32602`»). Reproducido con `--profile readonly`: `tools/list` sirve 7 tools, pero `instructions` describe el flujo completo de 10 (incluye `change_plan`/`change_apply`/`change_revert`) con solo una nota final de que en readonly «no están disponibles». Un agente que siga las instructions al pie de la letra acaba en `-32602`. Además: `initialize` **acepta cualquier `protocolVersion`** (p. ej. `"1990-01-01"`) y la ecoa tal cual — el contrato lista versiones aceptadas pero el rechazo no existe.

### D-02 · `patch_frontmatter`: ARCHITECTURE §20.4 promete distinguir asignar-`null` de eliminar-clave; el wire no puede expresarlo

**Caso G1-14 · CONFIRMADO.** ARCHITECTURE §20.4 (L1092-1095): patch_frontmatter «(`set` + `remove`) … **distingue explícitamente asignar `null` de eliminar una clave**». Pero `mcp.yml` L787 declara un único parámetro `patch` sin sintaxis de remove, y el normalizador aplica merge-patch RFC 7386 (su propio mensaje: «un valor escribe la clave, “null” la borra»): serde mapea JSON `null` a remove, y el brazo del core que sí asigna null explícito (`FrontmatterPatch` con `Some(Value::Null)`) es **inalcanzable desde MCP**. ARCHITECTURE se contradice a sí misma (L850/L227/L400/L515 dicen «null-borra»). Reproducido: `patch: {optional: null}` borra la línea `optional: true` del fichero. Arreglo propuesto: corregir §20.4 (declarar RFC 7386) o añadir sintaxis de remove al wire.

---

## 3. Huecos contractuales registrados (10)

Comportamientos reales, reproducidos y estables, que ninguna fuente de usuario fija. Varios reproducen la clase de «resultado recortado indistinguible del correcto» que E24/E26 declaran haber cerrado en otros puntos.

| id | Qué hace hoy el motor | Por qué merece fijarse |
|---|---|---|
| A-01 (PRJ-07) | `knowledge_get.sections` con un headingPath sin match lo **omite en silencio**; si ninguno casa, `body: ""` | Solo documentado en un doc-comment de `core::model::extract_sections`; un body acotado es indistinguible de «todas las secciones existían» |
| A-02 (ROB-05) | Cursor malformado (`"zzz-no-hex"`) cae a **offset 0 en silencio** (`decode_cursor` `unwrap_or(0)`) | Choca con el principio declarado en `mcp.yml` §validacion_de_argumentos: «se RECHAZA cuando hay un despachador que de todos modos tiene que interpretar el valor» |
| A-03 (ROB-06) | Un `nextCursor` de `graph_query` es **aceptado** por `knowledge_search` y reinterpretado como offset propio: página válida pero semánticamente equivocada | El esquema offset-hex compartido hace el origen indistinguible por construcción; el contrato llama al cursor «opaco» mientras publica su codificación |
| A-04 (G1-20) | `starts_with`/`ends_with` sobre campo no-string → **false silencioso** (los 7 docs con `priority: 3` desaparecen de `priority starts_with "3"` sin error) | El comentario de `eval.rs` L340-342 reconoce el hueco y que ningún test lo cubre; contrasta con el type error ruidoso del orden (E26-H08) |
| A-05 (G1-11) | `create` sobre path **existente** y `move` a destino **ocupado** producen planes `canApply: true` sin fricción — aplicados, pisarían conocimiento | Ningún código de colisión declarado; el caso inverso (`DOCUMENT_NOT_FOUND`) sí existe. Es el hueco con más riesgo práctico para un agente |
| A-06 (G1-13) | `replace_text` con `find` sin ocurrencias y sin aserción → plan **no-op silencioso** (`canApply: true`, diff vacío) | `safe-changes.md` solo documenta el vacío-sin-error para selecciones masivas; en forma-array no está fijado |
| A-07 (G1-23) | `knowledge_check` scope `paths` **traga** los paths inexistentes (incluso `paths: ["no-existe.md"]` a solas → 0 diagnósticos, sin error) | La enumeración de errores excluye `paths` de `DOCUMENT_NOT_FOUND`; un typo desaparece en silencio |
| A-08 (G1-04) | La sintaxis de `validation` usa **familias** (`danglingDocumentLinks`, `malformedFrontmatter`…), no códigos de diagnóstico; una clave desconocida (p. ej. `LINK-TARGET-MISSING: ignore`) es **silenciosamente inerte** | Las familias solo existen en `config.rs`; ninguna fuente de usuario las lista. Contrasta con la config rota, que sí es exit 3 ruidoso |
| A-09 (G2-04) | La config se lee **una vez por sesión** (`Workspace::open`); un `config.yaml` escrito con el servidor vivo no se aplica (el GC de recibos siguió usando `maximumReceipts: 20` cacheado) | `mcp.yml` lista «INTERNAL_IO_ERROR (carga de config)» por llamada, sugiriendo relectura; el comportamiento real solo está en un comentario de `lib.rs:116` |
| A-10 (G2-10, nit) | `mcp.yml` L149-151: «un path que normaliza a un directorio (`../docs/..`)» da `workspaceDirectory` — impreciso: solo cuando normaliza a la **raíz**; un path que normaliza a un directorio con nombre es `missing` | Menor; detectado al verificar el recálculo de navegación pura en `move` (que funciona como declara E23-H11) |

---

## 4. Esperados nuestros refutados por la verificación (12)

Registrados porque documentan lecturas erróneas fáciles de repetir; en todos, el motor coincide con la fuente completa:

- `../` en argumentos de tool se rechaza SIEMPRE aunque no escape (`RelPath` rechaza «las que contienen `..`», no «las que escapan») — ROB-04.
- `directlyAffected` cuenta **enlaces**, `transitivelyAffected` cuenta **documentos**: no son comparables (IMP-05).
- `workspace_status.counts` deriva de `Analysis` (inventario); el check en scope workspace **añade** descubrimiento — no son iguales por contrato (CHK-27).
- `--profile` inválido es exit 2 documentado, no ambigüedad (ROB-21).
- Un doc excluido por `.lodestarignore` cae en el lado **podado** (Missing), no en «excluido-pero-visitado» (WorkspaceFile) — el propio §20.6 anticipa la confusión (G2-01).
- `graph_query.backlinks` deduplica por vecino (grafo); una-entrada-por-enlace es de `knowledge_get.backlinks` (G2-03).
- `blocked ≠ invalid`: con `gate.blockWarnings: true`, `valid` sigue `true` (0 err) y el CLI da exit 1 — ambos correctos por contrato (G2-06b).
- Un journal fantasma no bloquea: `change_apply` corre la recuperación transparente primero y la resuelve (G2-09).
- El move que degrada `workspaceDirectory('a')` a `missing('a')` es composición inevitable de dos reglas del mismo párrafo de §20.6, y el plan lo **predice** en `diagnosticsIntroduced` (G2-10).
- El plan de delete normaliza `inboundLinksPolicy: "reject"` explícito aunque el caller lo omitiera sin backlinks — contrato de entrada, no de forma del plan (G1-12).
- `include: ["frontmatter.submapa.1"]` devuelve la clave plana `"submapa.1"` — «sufijo TAL CUAL se pidió, sin re-anidar» (G1-22).
- El título derivado de un fichero pelado es el stem en minúsculas («sin Title Case») (G1-07).

## 5. Estadísticas

| Ronda | Casos | Conformes | Bug motor | Doc confirmada | Contrato-ambiguo | Error de test |
|---|---|---|---|---|---|---|
| 1 (matriz base, 12 lotes) | 153 | 150 | 0 | 0 | 3 | 4 |
| 2 (huecos: config, gate, enlaces, null) | 25 | 17 | 1 | 2 | 5 | 3 |
| 3 (descubrimiento, CLI, refroots, recovery) | 11 | 9 | 0 | 0 | 2* | 5 |
| **Total** | **189** | **176** | **1** | **2** | **10** | **12** |

\* A-09 y el nit A-10. Áreas verificadas conformes destacables: hash determinista de planes, revert byte a byte, `PLAN_STALE`/`REVISION_CONFLICT`/`WRITE_CONFLICT`/`INVALID_RESULT`/`PERMISSION_DENIED`/`INBOUND_LINKS_EXIST`/`PLAN_EXPIRED`, todo el catálogo CheckCode con severidades (incl. BOM preservado byte a byte tras apply y patch rechazado sobre frontmatter ilegible), wikilinks aislados por diseño, las 3 clases límite del dialecto de dot-paths, cursor autosuficiente entre procesos, `referenceRoots` (visibles, no escribibles, fuera de `workspace_revision`), `.lodestarignore`, CLI `check`/`--json`/`--sarif` con exit codes congelados, y la equivalencia `where ≡ filter` también en los errores.

## 6. Reproducibilidad

Arnés y matriz completa (specs con esperado+cita, resultados crudos por lote, mini-lotes de verificación) en el scratchpad de la sesión de pruebas: `lodestar_harness.py` (JSON-RPC/stdio, gestiona worktrees y fixtures), `make_fixtures.py` (15 sets de documentos patológicos y configs), `batches/*.json`, `results/*.json`. Cada hallazgo de las secciones 1-3 tiene reproducción aislada en un `batches/verify_*.json`.
