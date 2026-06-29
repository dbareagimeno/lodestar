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
