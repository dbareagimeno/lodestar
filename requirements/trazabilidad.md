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

> Ratificado 2026-07-22 (`ARCHITECTURE.md §19`, `decisiones §0`). Supersede §13 en superficie de
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

---

## E25 — Endurecimiento del camino de escritura → historias

> Auditoría del camino de escritura (2026-07-29), posterior a v0.3.1. Estas filas **no** salen de
> `§10`/`§12`: cada una es un **defecto** con su invariante incumplido y la historia que lo cierra.
> La columna «Defecto» conserva el identificador de la auditoría (S1–S9) para poder rastrear el
> hallazgo original.
>
> ✅ **Las 6 historias están CERRADAS** (rama `epic/e25-e26-endurecimiento`, 2026-08-01).

| Invariante / sección incumplida | Defecto | Historias |
|---|---|---|
| Invariantes #1 y #5 (`.md` única fuente de verdad; único escritor) · `§19.5` pasos 5–10 | **S1** — `publish_result` recomputa el conjunto afectado en T3 y escribe/borra fuera de lo que pasó por `assert_writable`, backup y journal (edición externa pisada, fichero nuevo borrado, `referenceRoot` sobrescribible) | ✅ E25-H01 |
| Invariante #1 · `§19.5` (copias de recuperación) · `REFACTOR §5.2` | **S2** — copias y `.absent` sin volcado, restauración verbatim de una copia rota, y un journal irrecuperable que cierra el workspace a la escritura para siempre | ✅ E25-H02 |
| Invariante #5 · `§19.5` (lock de publicación y retención) | **S3** — el GC corre fuera del lock y purga el plano de recuperación de una transacción viva de **otro** proceso, que publica entonces sin copias | ✅ E25-H03 |
| `§19.5` paso 11 · `REFACTOR §11.2` (recibo y reversibilidad) | **S5** — un fallo posterior a la publicación devuelve `Err` con el disco ya cambiado y **sin recibo**: `change_revert` responde `PLAN_EXPIRED` para siempre | ✅ E25-H04 |
| Invariante #5 (escritura atómica y durable) · `REFACTOR §11.3` (reversión) | **S6 + S7** — `io::delete` sin fsync de directorio y dir-fsync best-effort silencioso; `change_revert` compara la revisión **antes** del lock y no re-verifica dentro | ✅ E25-H05 |
| `§19.5` paso 11 · `REFACTOR §11.3` — **espejo de S5**, hallado por el juez de H04 (MAYOR-2) | La reversión no persiste registro durable antes de su punto de no retorno: `Err` sobre la inversa ya publicada, sin recibo, y el GC le purga las copias | ✅ E25-H05 |
| `§19.5` (un solo publicador) · `§20.13` (`.gitignore` como texto plano) | **S4 + S9** — lock sin prueba de propiedad (TTL wall-clock, `Drop` por ruta, pid sin host) y `.gitignore` versionado reescrito sin atomicidad y con CRLF normalizado a LF | ✅ E25-H06 |

## E26 — UX de errores de la superficie MCP → historias

> Auditoría de la superficie de errores (2026-07-29). Continúa `E24-H07`/`E24-H10`: el código estable
> ya viaja en todo error; lo que faltaba es el **mensaje**, la **honestidad** de la respuesta y la
> **cota**. Identificadores U1–U6 de la auditoría.
>
> ✅ **Las 5 historias están CERRADAS** (rama `epic/e25-e26-endurecimiento`, 2026-08-01).

| Invariante / sección incumplida | Defecto | Historias |
|---|---|---|
| `§19.3` (códigos estables) · `contracts/mcp.yml` `errores_ejecucion` | **U1 + U2** — 8 de 10 tools devuelven el código **pelado** (los productores de `lodestar-app` son `Result<_, ErrorCode>`); `graph_query` sin `ref` responde `DOCUMENT_NOT_FOUND`; `change_plan` descarta el `ParseError` que `knowledge_search` sí entrega | ✅ E26-H07 |
| `§20.8` (lenguaje tipado) · principio de `E24-H07` | **U3** — un `TypeError` de evaluación **excluye el documento en silencio** en `knowledge_search` y en la selección masiva de `change_plan`: respuesta recortada, decidida documento a documento | ✅ E26-H08 |
| `§20.8`/`§20.10` (un solo dialecto de dot-paths) · invariante #3 | **U4** — `metadata_inspect` normaliza con `FieldPath::parse` en vez de `build_field_path`: `graph.backlinks` significa dos cosas según la tool, el anclaje `frontmatter.` no funciona y el catálogo anuncia nombres no direccionables | ✅ E26-H09 |
| `§19.6` (presupuesto de payload) · `E24-H09` (validación de valores) | **U5** — `graph_query` sin default ni máximo (`None => total`, sirve el grafo completo) y `metadata_inspect` sin paginación ni tope en ninguno de sus dos modos | ✅ E26-H10 |
| `contracts/README.md` (el contrato describe el servidor real) | **U6** — `contracts/mcp.yml` describe el comportamiento pre-`E24-H10`, `E24-H07` declaró frontera sin tocar el contrato, y cuatro tools declaran sus errores como prosa suelta | ✅ E26-H11 |

## Deuda declarada por la auditoría de E25/E26 → `decisiones §16`

> Lo que quedó **explícitamente fuera** de esta tanda, con su origen. No son filas de cobertura
> pendiente: son decisiones de no-hacer, registradas para que la próxima auditoría no las
> redescubra. El detalle, con opciones y recomendación, está en [`decisiones §16`](../decisiones/16-deuda-auditoria-e25-e26.md).

| Punto (§16) | Origen |
|---|---|
| (a) *Quoting* en el lenguaje: clave con punto literal, clave `frontmatter` literal, fusión de nombres | E26-H09 |
| (b) `Envelope`/`ErrorEnvelope` sin llamantes | auditoría UX |
| (c) Cache SQLite y watcher sin uso en producción | auditoría UX (va con `§14`) |
| (d) Servidor MCP monohilo, sin timeout ni cancelación | auditoría UX (va con `§3`) |
| (e) Config sin `deny_unknown_fields`; config ilegible → defaults silenciosos | auditoría de escritura (va con `§15`) |
| (f) Workspace vacío indistinguible de directorio equivocado | auditoría UX |
| (g) API pública no transaccional de `Workspace` (**S8**) | auditoría de escritura |
| (h) Escritores de runtime sin lock (`persist_plan`/`write_receipt`) | juez ciego de E25-H03 |
| (i) Secuencia de sellado duplicada `apply`/`revert` | juez ciego de E25-H05 |
| (j) Cursor basura reinicia la paginación en silencio | juez ciego de E26-H10 |
| (k) Trazabilidad sin filas de E15–E24 | cierre de E24-H18, verificado aquí |

## Consecuencias observables declaradas (E25–E26) → historia que las declara

> Cambios de resultado sobre bases existentes que la rama introduce. Se recogen aquí para que la nota
> de release no dependa de releer las épicas; `E26-H11` es quien las consolida.

| Consecuencia | Historia |
|---|---|
| La convergencia «a uno de los dos bordes» pasa a estar **condicionada** a que las copias de recuperación verifiquen; lo que no verifica va a cuarentena y se reporta con `RECOVERY_FAILED` | E25-H02 |
| `graph_query` sin `ref` cambia de `DOCUMENT_NOT_FOUND` a `INVALID_SCHEMA` | E26-H07 |
| Una consulta con un error de **tipo** pasa de devolver una lista recortada a **fallar** | E26-H08 |
| `metadata_inspect{field:"graph.backlinks"}` pasa a fallar; `field:"frontmatter.status"` pasa a funcionar; el `name` del catálogo cambia para las claves que colisionan con un namespace | E26-H09 |
| `graph_query` sin `limit` deja de devolver el grafo completo (100 nodos + `nextCursor`) | E26-H10 |

## Catálogo de `ErrorCode` (16 filas) — movimientos de esta rama

| Código | Movimiento | Historia |
|---|---|---|
| `RECOVERY_FAILED` | gana su **primer emisor real** (cuarentena de un journal irrecuperable) y sale de `codigos_sin_emisor`, que baja de 5 a 4 filas | E25-H02 |

> **El catálogo sigue teniendo 16 filas**: ninguna historia de E25/E26 añade, borra ni renombra un
> código (invariante #4; el grep de CI de `E24-H17` lo hace cumplir). Lo único que cambia es qué
> caminos emiten qué.

---

## E27 — Producto, distribución y apertura OSS → historias

> Diseño ratificado el 2026-08-01: `ARCHITECTURE.md §21` (adenda aditiva) y `decisiones §17`
> (cierre de la puerta 1). Estas filas no salen de `§10`/`§12`: mapean cada sub-decisión de
> `§21`/`§17` y cada hallazgo confirmado de la review OSS externa a la historia que lo cubre.
> Ninguna historia toca la frontera MCP.

| Decisión / hallazgo | Fuente | Historias |
|---|---|---|
| Guardarraíles del pipeline: tag ≡ `v`+`workspace.package.version` (step que falla) + `SHA256SUMS-<target>.txt` como asset | `§21.2` · hallazgo del tag `0.5.0` sin prefijo | E27-H01 |
| Demo ejecutable: workspace con defectos deliberados como terreno del quickstart y las docs | `§21.3` · hallazgo «sin demo end-to-end» | E27-H03 |
| README público en inglés, instalación por binarios, snippet MCP, quickstart contra la demo | `§21.1`/`§21.5` · hallazgo «instalación solo `cargo install --path`» | E27-H02 |
| README y demo protegidos por CI (smoke que ejecuta el guion y aserta salidas clave) | `§21.3` · lección de E23 («ejecútalo») | E27-H04 |
| Taxonomía documental vigente/superseded: `docs/history/` para los 4 superseded; `REFACTOR_PHASE_2.md` NO se mueve (~51 citas) | `§21.3` · `§17`-DC | E27-H06 |
| Docs de usuario operativas en inglés: quickstart, mcp-clients, ci | `§21.1` · hallazgo «docs = arqueología, cero docs de usuario» | E27-H05 |
| Docs de usuario de referencia en inglés: query-language (con límites de `§12`/`§16.a` declarados), safe-changes | `§21.1`/`§21.5` | E27-H11 |
| Épicas E0–E8 marcadas históricas en fichero; invariantes #4 (`.d.ts`) y #7 (git) corregidos en `requirements/README.md`; regla de idioma registrada | `§21.1` · hallazgo «2 invariantes retirados listados como vigentes» | E27-H07 |
| Embudo de contribución: CONTRIBUTING issues-first + SECURITY (Private Vulnerability Reporting + email) + Contributor Covenant 2.1 | `§21.4` · `§17`-DB/DD · hallazgo «embudo OSS cerrado» | E27-H08 |
| Templates de issue/PR + roadmap que apunta a `decisiones/` (sin documento paralelo) | `§21.4` | E27-H09 |
| Publicación en crates.io — **diferida** | `§17`-DA (reabrible) | E27-H10 **[BLOQUEADA por decisiones §17]** |
| Regla transversal: la superficie externa no presenta la cache como camino de lectura ni promete escala mientras `§14` siga abierta | `§21.5` · `decisiones §14` | E27-H02, E27-H03, E27-H05, E27-H11 (criterio de aceptación en las cuatro) |

---

## E28 — Fase 0 de la campaña de bugfixes del testbench homelab → historias

> Origen: `decisiones §23` (hallazgos del testbench MCP sobre el homelab, 2026-08-06), filas M-01 y
> A-05 — las dos marcadas «historia propia, inmediata»/«historia propia» por gravedad y prioridad.
> Ninguna historia toca una decisión de `§10`/`§12`; mapean hallazgos verificados del testbench
> (`docs/qa/informe-homelab-2026-08-06.md`) a la historia que los cierra.

| Hallazgo | Fuente | Historia |
|---|---|---|
| **M-01** — `change_revert` de un recibo `-revert` es un no-op silencioso que sobrescribe `recovery/`/`receipts/` del redo | `decisiones §23` fila 1 · informe §1, caso G1-18 | E28-H01 (además salda `decisiones §16(i)`, secuencia de sellado duplicada `apply`/`revert`) |
| **A-05** — `create`/`move` sobre un `path`/`to` ya ocupado producen `canApply: true` sin fricción | `decisiones §23` fila 2 · informe §3, caso G1-11 | E28-H02 |

**Adenda correctiva (2026-08-06)**: los jueces ciegos que verificaron H01/H02 ejecutando el binario
real encontraron un bloqueante en cada una. Ninguna de las dos historias nuevas toca una decisión
de `§10`/`§12`; corrigen bloqueantes de historias ya integradas.

| Hallazgo | Fuente | Historia |
|---|---|---|
| `changeSetId` determinista reutiliza el mismo `txnId` en un re-apply idéntico: `change_apply` sobrescribe `recovery/`/`receipts/` de la transacción previa (guard anti-sobrescritura de H01 solo vivía en `change_revert`), y el `revert` posterior queda sin salida (`WRITE_CONFLICT`) | veredicto de juez ciego sobre `E28-H01`, reproducido por JSON-RPC | E28-H03 |
| `change_plan` normaliza cada operación contra el `DocumentSet` inicial, no el acumulado del propio plan: falsos negativos destructivos (`[move a→final, move b→final]`, `[create X, move b→X]`, `[create X, create X]`) y regresión de dos idiomas legítimos (`[delete X, create X]`, `[move A→B, create A]`) | veredicto de juez ciego sobre `E28-H02`, reproducido por JSON-RPC | E28-H04 (abre `decisiones §24`, equivalencia de caja/Unicode, fuera de su alcance) |

---

## E32 — Gaps de suite medidos por mutantes → historias

> Origen: `decisiones §27` (pasada de `/mutantes` que cerró E31, 2026-08-08, acotada a
> `crates/lodestar-core/src/model.rs` y `plan.rs`). Ninguna fila toca una decisión de `§10`/`§12`:
> son **agujeros de suite** sobre comportamiento correcto, cada uno verificado aplicando la mutación
> al árbol y viendo la suite en verde. Trabajo tests-only; la evidencia de cada test es su mutación
> (rojo con ella, verde sin ella — lección de E30/E31).

| Gap (§27) | Función | Historia |
|---|---|---|
| (a) CRLF en `split_front` sin un solo test (red bajo `E31-H02`/`§26`) | `model::split_front` | E32-H01 |
| (b) El no-op byte a byte de `patch_frontmatter` sin quien lo sujete | `model::patch_frontmatter` | E32-H01 |
| (c) `relation_changes` jamás aseverado, y viaja al wire en las 3 tools de cambio | `plan::semantic_diff` | E32-H01 |
| (d) `ensure_exists` puede devolver siempre `Ok` sin que nada se ponga rojo | `plan::ensure_exists` | E32-H01 |
| (e) `sort_paths_cmp` es contractual (ordena `semanticDiff.*`) y lo cubre un test de 2 paths | `model::sort_paths_cmp` | E32-H01 |
| (f) `locate_section` puede editar la sección hermana homónima | `model::locate_section` | E32-H01 |
