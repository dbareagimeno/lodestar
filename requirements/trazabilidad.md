# Matriz de trazabilidad

> Mapea cada **decisión ratificada** (`ARCHITECTURE.md §10`, filas 1–21) y cada **concern transversal con
> dueño** (`§12`) a las historias que la implementan. Sirve para auditar que **ninguna decisión se
> relitigó** y que **toda** quedó cubierta. Si una fila no tiene historia, es un hueco de cobertura.

## §10 — Decisiones ratificadas → historias

| # (§10) | Decisión | Historias |
|---|---|---|
| 1 | Core es la autoridad; SQLite acelerador verificado por paridad; trait `ConceptStore` a escala | E1-H07, E3-H07, E3-H08, E2-H02 |
| 2 | `Workspace` vive en `lodestar-workspace`; `rusqlite`/`notify` solo en `store` | E0-H01, E5-H01, E5-H02 |
| 3 | `Check`/`Severity`/`CheckCode` una sola definición en `core::types` | E1-H03 |
| 4 | Bug del gate: `hard_fail` = #ficheros con Err (no `.max()` mal) | E1-H03 (test `.max()`), E1-H07 |
| 5 | `Analysis` congelado: `out` strings, `inn`, `per_file`, camelCase | E1-H07 |
| 6 | Sin capa DTO paralela; `.d.ts` generado desde Rust | E0-H04, E6-H03, E1-H19 |
| 7 | Nombres de evento/comando congelados + `ipc.ts` generado + smoke | E6-H01, E6-H02, E6-H03, E6-H14 |
| 8 | Un watcher = único escritor; comandos solo escriben el `.md` | E3-H04, E5-H01, E5-H02 |
| 9 | `RelPath` newtype validado (chokepoint path-traversal) | E1-H01 |
| 10 | `store` dueño único del DDL; ORPHAN/LINK-STUB sintetizados; columnas = nombres del Check | E3-H01, E3-H05 |
| 11 | `body:` subcadena (no FTS MATCH); un solo `match_token`; FTS superset | E1-H11, E3-H02 |
| 12 | Generadores puros (devuelven `Mutation`); workspace aplica y diffea | E1-H14, E2-H03, E5-H04 |
| 13 | `merge_frontmatter` (patch null-borra) vive en el core | E1-H13, E7-H03 |
| 14 | Feature `schemars` para outputSchema del MCP | E1-H20, E7-H02 |
| 15 | git en `lodestar-vcs`; transporte híbrido libgit2 local + binario `git` red | E0-H01, E4-H01, E4-H05, E4-H07 |
| 16 | Restore/switch/merge no pierden trabajo → checkpoint; regeneran index/tags | E4-H06, E5-H05 |
| 17 | `OKF-CONFLICT` hard-fail por marcadores de merge | E1-H06, E4-H06 |
| 18 | `RepoState` detecta merge/rebase en curso; niega commit | E1-H19, E4-H03, E5-H05 |
| 19 | Pill nunca obsoleto: ref-watch + update optimista + reconcile al enfocar | E4-H08, E5-H07, E6-H09 |
| 20 | Tipos commit/diff/cache una familia; cache de conformidad por tree-oid; golden | E1-H17, E1-H19, E4-H09, E7-H06 |
| 21 | Contador "sin commitear" por hash por path; `OkfDiff` perezoso; LCS dos-filas/Hirschberg | E1-H17, E4-H03, E6-H11 |

## §12 — Concerns transversales (con dueño) → historias

| Tema (§12) | Historias |
|---|---|
| Migración del prototipo (localStorage + replay de historial) | E2-H06, E8-H02 |
| Versionado OKF (`okf_version`, warn-and-degrade, aditivo-solo) | E1-H07, E8-H05 |
| i18n (conformidad keyed por código; cabeceras canónicas fijas) | E1-H06, E8-H03 |
| Packaging (updater, firma/notarización, 3 binarios, release CI, compat) | E8-H06 |
| Testing/paridad (fixtures, diferencial, golden cross-fachada, property, e2e) | E0-H03, E1-H18, E3-H07, E6-H14, E7-H06 |
| Seguridad (DOMPurify, escapar FTS5, subproceso git confinado, threat model) | E3-H02, E4-H07, E6-H07, E8-H04 |
| Errores (taxonomía, código estable, supervisar watcher) | E1-H02, E5-H06, E8-H08 |
| Config (app-global + por-bundle `lodestar.toml`) | E8-H01 |
| Un bundle por proceso (lockfile) | E5-H01, E7-H01 |
| First-run (`init`/crear bundle, `git init`, `.lodestar/` ignorada) | E2-H05, E4-H02, E6-H13, E8-H10 |
| Sincronización / remoto (push/pull in-app; clone/remotos no-goal) | E4-H07, E6-H09 |
| Paridad con `git` CLI (commits libgit2 sin hooks/firma; red por binario) | E4-H05, E4-H10, E8-H12 |
| Identidad / atribución (autor+committer, override, agente distinguible) | E4-H05, E7-H05, E8-H07, E8-H01 |
| CRDT (futuro): core sin I/O para server `axum` | E8-H11 |

## §11 — Presupuesto de rendimiento → historias

| Objetivo (§11) | Historias |
|---|---|
| Cold open 10k < ~2s | E3-H03, E8-H09 |
| edit → UI < 150 ms | E3-H04, E8-H09 |
| grafo 60 fps (Barnes-Hut, cap/cluster, virtualización) | E6-H05, E6-H08, E8-H09 |
| Proyecciones SQL / eventos delta a escala | E3-H08, E5-H03 |

## §13.8 — Scope ratificado de git (v1) → historias

| Tema (§13.8) | Decisión v1 | Historias |
|---|---|---|
| Sincronización / remoto | push/pull/fetch in-app; clone/remotos no-goal | E4-H07, E6-H09 |
| Firma de commits | sin firmar; avisar si se exige | E4-H05, E8-H12 |
| LFS / `.gitattributes` | commit detecta y avisa; push/pull respetan | E4-H05, E8-H12 |
| Ramas | crear/cambiar/merge locales; rebase diferido | E4-H06, E5-H05 |
| Propuestas | `status: review`, no ramas/PR | E6-H10 |
| Tags/submódulos/worktrees/bare | diferidos; degradan sin crashear | E8-H12 |

## Cobertura de los 15 `CheckCode` (§4.1) → historia productora

Todos producidos por **E1-H06** (conformidad) y agregados por **E1-H07** (analyze):
`OKF-FM01` · `OKF-FM02` · `OKF-FM03` · `OKF-TYPE` · `REC-TITLE` · `REC-DESC` · `FMT-TAGS` · `FMT-TS` ·
`LINK-STUB` · `LINK-REL` · `ORPHAN` · `BODY-STRUCT` · `OKF-IDX` · `OKF-LOG` · `OKF-CONFLICT`.

> `LINK-STUB` y `ORPHAN` se **sintetizan** en el store (E3-H05), no se materializan (`§10` fila 10),
> pero su definición canónica vive en el core (E1-H06) y la paridad lo verifica (E3-H07).

---

## §19 — Giro headless (decisiones D0–D6/D-CheckCode/D-check) → historias

> Ratificado 2026-07-22 (`ARCHITECTURE.md §19`, `DECISIONES.md §0`). Supersede §13 en superficie de
> producto. Cada sub-decisión mapea a las historias que la implementan (épicas E9–E14).

| Sub-decisión (§0/§19) | Historias |
|---|---|
| D0 — §19 nueva + nota en §13/§10 (git dormido) | E9-H01, E9-H02, E9-H03 |
| D1 — Opción C: mecánica en `workspace`, `lodestar-app` fino | E10-H01, E12-H08, E13-H08 |
| D3 — Envelope en `lodestar-app`; códigos de error en `core::types` | E10-H01, E10-H02 |
| D4 — Config a `.lodestar/config.yaml` (writable/reference/ignored + gate + transactions) | E9-H05 |
| D5 — Canónico vs runtime; `WorkspaceRevision` excluye `.lodestar/` | E9-H06, E10-H03 |
| D6a — Generadores solo CLI + auto-regen en `change_apply` | E13-H11, E14-H01 |
| D6b — stdio + `outputSchema` (schemars); rmcp diferido | E10-H13 |
| D-CheckCode — familias estáticas `SCHEMA-*`/`REL-*`; i18n por código | E10-H06, E10-H07, E11-H03 |
| D-check — `check` sobre working tree; `--staged/--rev/--range` diferidos con vcs | E9-H02, E14-H01 |

## §19 — Capacidades del motor headless (`REFACTOR §8`) → historias

| Capacidad (tool / pieza) | Historias |
|---|---|
| `core::schema` (DocType/relations/lifecycle/templates) puro | E10-H05, E10-H07, E11-H03 |
| `ConceptRevision` / `WorkspaceRevision` (identidad determinista) | E10-H03 |
| Envelope + códigos de error (`lodestar-app`) | E10-H01, E10-H02 |
| `workspace_status` | E10-H08 |
| `knowledge_search` (sustituye `query`) | E10-H09 |
| `knowledge_get` | E10-H10 |
| `schema_inspect` | E10-H11 |
| `knowledge_check` (sustituye `conformance_check`) | E10-H12, E14-H01 |
| `graph_query` (consolida backlinks/orphans/dangling/neighborhood) | E11-H01, E11-H02 |
| `impact_analyze` (reusa blast-radius) | E11-H05 |
| `change_plan` (normaliza+simula+valida, sin escribir) | E12-H05, E12-H06, E12-H07, E12-H08, E12-H09 |
| Modelo transaccional (staging/journal/locks/recovery/receipts) | E13-H01…E13-H07, E13-H10 |
| `change_apply` / `change_revert` | E13-H08, E13-H09 |
| Perfiles `readonly`/`standard` + instrucciones | E14-H03 |
| Seguridad §14 (RelPath + writableRoots + symlink; sin red/exec/git) | E9-H05, E11-H04, E13-H08 |

## Benchmark funcional (`REFACTOR §17`) → historias que lo cubren

| Escenario §17 | Historia(s) |
|---|---|
| Encontrar una decisión por significado | E10-H09 |
| Crear un concepto válido | E13-H08 |
| Crear un concepto sin campo obligatorio → rechazado | E10-H07, E12-H04 |
| Mover un concepto con 30 backlinks | E11-H05, E12-H06 |
| Borrar un concepto referenciado → rechazo con blockers | E11-H05, E12-H06 |
| Modificar un concepto cambiado externamente → `REVISION_CONFLICT` | E12-H08 |
| Cambiar cinco conceptos relacionados → un change set | E12-H08 |
| Introducir una relación inválida → error antes de escribir | E11-H03, E12-H07 |
| Corregir safe fixes → `apply_fix` | E10-H12, E12-H07 |
| Revisar un refactor → diff semántico | E12-H03 |
| Recuperar un cambio reciente → `change_revert` | E13-H09 |
| Cerrar Lodestar durante publicación → recuperación determinista | E13-H06 |
| Intentar escribir fuera de `writableRoots` → rechazo | E13-H08 |
| Referenciar un archivo de código inexistente → diagnóstico | E11-H04 |
| Editar directamente un Markdown inválido → detectado | E10-H12, E14-H01 |
