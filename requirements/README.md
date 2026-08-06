# Requisitos de implementación de **lodestar**

> Este directorio descompone el contrato ratificado de [`ARCHITECTURE.md`](../ARCHITECTURE.md)
> en **épicas** e **historias** lo bastante granulares como para que un agente las implemente
> de una en una. **No reabre decisiones de diseño**: las traduce a unidades de trabajo
> verificables. Si una historia parece contradecir `ARCHITECTURE.md`, gana `ARCHITECTURE.md`
> (y se corrige la historia, no el diseño).

## Cómo leer estos documentos

- **`ARCHITECTURE.md` es la autoridad.** Cada historia cita la sección (`§N`) que implementa. Lee
  siempre la sección citada antes de implementar.
- **La spec de comportamiento es `docs/REFACTOR_PHASE_2.md` + `ARCHITECTURE.md §20`.** Las historias
  de E0–E14 citan además funciones del prototipo (`prototype/index.html`): desde `E15-H04` esas
  citas son **referencia histórica de v0.2.x** —explican el comportamiento portado, no lo
  arbitran— y el arnés diferencial JS-vs-Rust ya no existe.
- **Idioma**: español en código, comentarios, mensajes y commits (el usuario es hispanohablante),
  salvo identificadores técnicos congelados por el contrato (nombres de tipos, comandos, eventos,
  códigos de diagnóstico). **Ampliado por `ARCHITECTURE.md §21.1` (E27)**: la superficie
  **pública** del repo —README raíz, `docs/user/`, `examples/demo/`, CONTRIBUTING, SECURITY, CoC y
  templates de GitHub— se escribe en **inglés**; todo lo interno (este directorio incluido) sigue
  en español.

## Mapa de épicas (alineadas con `ARCHITECTURE.md §14`)

| Épica | Fase §14 | Crate / área | Doc |
|---|---|---|---|
| **E0** — Scaffolding del workspace | (previa) | Cargo workspace, frontend, CI, fixtures | [epica-00-scaffolding.md](epica-00-scaffolding.md) |
| **E1** — `lodestar-core` puro | 1 | modelo · conformidad · links · query · grafo · generación · export · diff | [epica-01-core.md](epica-01-core.md) |
| **E2** — `lodestar-cli` mínima | 2 | `init`/`check`/`index`/`tags`/`export`/`reindex`/`import` | [epica-02-cli.md](epica-02-cli.md) |
| **E3** — `lodestar-store` | 3 | SQLite/FTS5 + watcher + paridad | [epica-03-store.md](epica-03-store.md) |
| **E4** — `lodestar-vcs` | 4 | libgit2 local + binario `git` red + conformidad-por-commit | [epica-04-vcs.md](epica-04-vcs.md) |
| **E5** — `lodestar-workspace` | 5 | glue · único escritor · bus de eventos · checkpoint | [epica-05-workspace.md](epica-05-workspace.md) |
| **E6** — `src-tauri` + frontend Svelte | 6 | fachada desktop + UI portada verbatim + pill/overlay/Cambios | [epica-06-tauri-frontend.md](epica-06-tauri-frontend.md) |
| **E7** — `lodestar-mcp` | 7 | fachada agentes (rmcp/stdio) + golden cross-fachada | [epica-07-mcp.md](epica-07-mcp.md) |
| **E8** — Transversales de producto | 8 | migración · packaging · i18n · seguridad · config · first-run · errores · perf | [epica-08-transversales.md](epica-08-transversales.md) |

**Orden de construcción (E0–E8)**: estrictamente E0 → E1 → E2 → E3 → E4 → E5 → E6 → E7, con E8 entrelazada
(sus historias declaran de qué fase dependen). Cada fase se valida con el arnés de paridad antes de la
siguiente (`§14`). Una historia **no se puede empezar** hasta que sus dependencias (`Dependencias:`)
estén `Done`.

## Mapa de épicas del giro headless (alineadas con `ARCHITECTURE.md §19.8`)

> Giro a **motor headless de integridad semántica** (`ARCHITECTURE.md §19`, ratificado 2026-07-22;
> supersede §13 en superficie). Git sale de las fachadas (crate `vcs` dormido); la UI queda congelada.

| Épica | Fase §19.8 | Área | Doc |
|---|---|---|---|
| **E9** — Reducción de alcance | 0 | Retirar git de superficie · congelar UI · `.lodestar/config.yaml` · canónico/runtime | [epica-09-reduccion-alcance.md](epica-09-reduccion-alcance.md) |
| **E10** — Esquemas y lectura headless | 1 | `core::schema` · revisiones · `lodestar-app` · envelope/errores · 5 tools READ/VERIFY | [epica-10-esquemas-lectura.md](epica-10-esquemas-lectura.md) |
| **E11** — Grafo e impacto | 2 | `graph_query` · relaciones tipadas · refs externas · `impact_analyze` | [epica-11-grafo-impacto.md](epica-11-grafo-impacto.md) |
| **E12** — Planificación de cambios | 3 | `ChangeSet` · 11 ops normalizadas · `change_plan` (sin escribir) | [epica-12-planificacion.md](epica-12-planificacion.md) |
| **E13** — Publicación recuperable | 4 | staging · journal · locks · recovery · receipts · `change_apply`/`change_revert` | [epica-13-publicacion-recuperable.md](epica-13-publicacion-recuperable.md) |
| **E14** — Integración software + evaluación | 5+6 | gate CI · convivencia · perfiles · benchmark §17 · métricas | [epica-14-integracion-evaluacion.md](epica-14-integracion-evaluacion.md) |

**Orden de construcción (E9–E14)**: estrictamente E9 → E10 → E11 → E12 → E13 → E14 (cada fase valida su
criterio de salida de `§19.8`/`REFACTOR §16` antes de la siguiente). Dentro de cada épica, el «Orden de
construcción» al final del documento fija el orden de sus historias. **E9 es prerrequisito de todo** (retira
git, define config/runtime); **E10** habilita 11–13 (schemas y revisiones son la base de impacto y
planificación); **E12** depende de **E11** (el impacto alimenta el riesgo del plan); **E13** aplica los
planes de **E12**; **E14** cierra. Ninguna historia del giro está **[BLOQUEADA]**: las decisiones de diseño
(D0–D6/D-CheckCode/D-check) se ratificaron en la puerta 1 (`decisiones §0`, `ARCHITECTURE.md §19`).

## Mapa de épicas de la migración a Markdown universal (alineadas con `ARCHITECTURE.md §20.14`)

> Migración de **OKF a workspaces Markdown universales** (`ARCHITECTURE.md §20`, ratificada
> 2026-07-23; fuente: `docs/REFACTOR_PHASE_2.md`). Lodestar deja de exigir un formato documental
> propio y opera sobre cualquier red de `.md` de un proyecto. **v0.3.0 es incompatible con v0.2.x.**

| Épica | PRs `§20.14` | Área | Doc |
|---|---|---|---|
| **E15** — Workspace universal | 0 + 1 | Retirada de vcs/generadores/init-zip/prototipo · `cwd` como root · descubrimiento recursivo · config opcional | [epica-15-workspace-universal.md](epica-15-workspace-universal.md) |
| **E16** — Modelo documental genérico | 2 | `ParsedFrontmatter` YAML arbitrario · sin ficheros reservados · título derivado · patch quirúrgico · diagnósticos mínimos · `Concept`→`Document` | [epica-16-modelo-documental.md](epica-16-modelo-documental.md) |
| **E17** — Enlaces y grafo universal | 3 + 4 | Parser de enlaces · `LinkTarget` · diagnósticos de enlace · `Analysis` nueva · superficie de grafo | [epica-17-enlaces-grafo.md](epica-17-enlaces-grafo.md) |
| **E18** — Store v2 | 5 | DDL nuevo · metadata anidada · links genéricos · cold rebuild · paridad core/store | [epica-18-store-v2.md](epica-18-store-v2.md) |
| **E19** — Lenguaje de consulta | 6 | Parser · AST · type checking · namespaces · filtro JSON equivalente | [epica-19-lenguaje-consulta.md](epica-19-lenguaje-consulta.md) |
| **E20** — Inspección y validación genéricas | 7 + 8 | `metadata_inspect` (retira `core::schema`) · política `rejectNewErrors`/`allowExistingErrors` · diagnósticos de descubrimiento cableados | [epica-20-inspeccion-validacion.md](epica-20-inspeccion-validacion.md) |
| **E21** — Contrato MCP y transacciones genéricas | 9 + 10 | Contrato nuevo · 8 operaciones universales · selecciones masivas por consulta | [epica-21-transacciones-genericas.md](epica-21-transacciones-genericas.md) |
| **E22** — Migración y limpieza pública | 11 | `migrate-from-okf --dry-run` · docs · README · publicación incompatible · **e2e final** | [epica-22-migracion-publicacion.md](epica-22-migracion-publicacion.md) |
| **E24** — Cierre de defectos de la v0.3.0 | — | BOM que se tragaba el frontmatter · recuperación tras crash (WRITE_CONFLICT, workspace que parecía roto, fugas) · superficie de error honesta · seam real de failpoints · crash por señal | [epica-24-cierre-defectos-v031.md](epica-24-cierre-defectos-v031.md) |
| **E23** — Cierre de la migración | — | Defectos hallados en la revisión de la PR #17 · puerta de CI (failpoints) · e2e de sesión larga · schema de escritura · documentos de estado | [epica-23-cierre-migracion.md](epica-23-cierre-migracion.md) |

**E23 no fue una fase de `§20.14`**: fue la épica de **cierre**, abierta por la revisión de la PR #17
(2026-07-25) y **completada el 2026-07-26**, que saldó los defectos que la migración dejó vivos antes
de mergear. Su bloque A no se dedujo leyendo código: se reprodujo ejecutando los binarios — y esa
resultó ser su lección, porque **cinco de los defectos que cerró no estaban en la revisión inicial**:
aparecieron implementando, todos en código ya publicado y verde. Dos de ellos (la cobertura vacua de
un chokepoint y un GC que llevaba tres épicas juzgando con los valores por defecto) eran **tests que
pasaban por la razón equivocada**.

**Orden de construcción (E15–E22)**: estrictamente secuencial. **E15 es prerrequisito de todo** (sin
descubrimiento universal no hay nada que modelar); **E16** cambia el modelo documental y arrastra los
diagnósticos; **E17** depende de E16 (los enlaces se extraen de documentos ya genéricos); **E18** y
**E19** consumen el modelo y el grafo de E16/E17; **E20** retira `core::schema`; **E21** cierra la
frontera; **E22** publica.

**Hueco de cableado que E20-H04 cerró** (se conserva la nota porque el patrón se repite): los
diagnósticos de `§20.9` —`DOC-NOT-UTF8`, `DOC-TOO-LARGE`, `SYMLINK-UNSUPPORTED`, `PATH-NOT-UTF8` y
las colisiones de capitalización (`LINK-CASE-MISMATCH`)— los computaba `discovery::discover` de forma
determinista y su único llamador **los descartaba**, así que la mitad del catálogo era invisible.
E20-H04 los cableó a `knowledge_check` por un canal aparte (su target no es un documento) junto con
la política de severidad. Es la misma forma de hueco que E15-H07, que el cableado de `other_files`
de E17 y que la selección masiva de E22-H04: **capacidad computada que no llega al producto**. E23
encontró dos más de la misma familia (`recovery.pendingTransaction` cableado a `false` y el store
entero sin consumidor), y `lodestar check` se quedó **fuera** de este cableado hasta E23-H01 — de ahí
que la CLI y el MCP dieran veredictos distintos sobre el mismo workspace.

**Nota sobre el prototipo**: desde `E15-H04` el prototipo JS (`prototype/index.html`) **deja de ser la
spec de comportamiento** y el arnés diferencial se retira. La spec de la migración es
`docs/REFACTOR_PHASE_2.md`; `prototype/` queda como referencia histórica de v0.2.x.

## Mapa de épicas de endurecimiento posterior a v0.3.1

> Auditoría del **camino de escritura** y de la **superficie de errores** (2026-07-29), posterior a la
> publicación de v0.3.1 y al cierre del bloque C de E24. Ninguna de las dos es una fase de `§20.14`
> ni de `§19.8`: son épicas de **endurecimiento** que cierran defectos que la suite no mira, todos de
> concurrencia, durabilidad o superficie. **Ratificadas el 2026-07-29** y **COMPLETAS el 2026-08-01**
> (rama `epic/e25-e26-endurecimiento`, hasta `7ebe764`). Detalle por historia, veredictos de los
> jueces ciegos e invariantes verificados en
> [`IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md); la deuda que quedó **fuera** por decisión
> está en [`decisiones §16`](../decisiones/16-deuda-auditoria-e25-e26.md).

| Épica | Estado | Área | Doc |
|---|---|---|---|
| **E25** — Endurecimiento del camino de escritura | ✅ **completa · 6/6 historias** | TOCTOU entre planificar y publicar · durabilidad y verificación de las copias de recuperación · GC vs transacción viva de otro proceso · recibo antes del punto de no retorno · fsync de borrado y revert bajo el lock · propiedad del lock y `.gitignore` del usuario | [epica-25-endurecimiento-escritura.md](epica-25-endurecimiento-escritura.md) |
| **E26** — UX de errores de la superficie MCP | ✅ **completa · 5/5 historias** | código **y** mensaje en las 10 tools · `TypeError` de consulta visible · un solo dialecto de dot-paths · cotas y paginación en toda la superficie · contrato sincronizado | [epica-26-ux-errores.md](epica-26-ux-errores.md) |

> **Las dos épicas se enmendaron durante la implementación**, las dos por un hallazgo del ciclo
> anterior y con commit de spec propio antes de la fase roja: `E25-H02` ganó el sellado del aborto de
> ventana (lo destapó implementar `E25-H01`) y `E25-H05` ganó «revertir también deja recibo»
> (**MAYOR-2** del juez ciego de `E25-H04`). El texto de las épicas incluye ambas enmiendas, así que
> describe lo que se construyó, no solo lo que se planificó.

**Orden de construcción (E25–E26)**: **E25 → E26** (misma rama). Dentro de E25 el orden es
estrictamente `H01 → H02 → H03 → H04 → H05 → H06`: las cuatro primeras tocan el mismo camino y los
mismos ficheros (`transaction.rs`/`recovery.rs`/`receipts.rs`), y cada una apoya el contrato de la
siguiente. Dentro de E26, `H07 → H08 → H09 → H10 → H11`: **H07** abre el canal de código+mensaje del
que dependen las tres siguientes, y **H11** es la pasada final de coherencia del contrato — depende
de H07–H10 **y** de **E25-H02**, cuyo delta entra en la misma pasada. E26 no depende
funcionalmente de E25 salvo en esa fila: si se paralelizaran, su único punto de encuentro es
`contracts/mcp.yml`. Ninguna historia está **[BLOQUEADA]**: la única decisión abierta que las roza
—rechazar parámetros **no** declarados, registrada por `E24-H18` en `decisiones/`— está
explícitamente fuera de alcance en las dos épicas.

**Principio rector de cada una** (la regla que desempata dudas durante toda la épica): en **E25**,
*una salvaguarda vale por el estado sobre el que se ejerce, no por el estado sobre el que se
computó* — ante la duda, re-mirar bajo el lock y abortar si cambió. En **E26**, el de `E24-H07`:
*una respuesta silenciosamente equivocada es peor que un error*.

## Mapa de épicas de producto y apertura OSS

> Primera épica de **superficie externa** (`ARCHITECTURE.md §21`, ratificada 2026-08-01;
> `decisiones §17`). Origen: review OSS externa que concluyó que el repo está técnicamente por
> delante de su adopción. No toca el motor ni la frontera MCP: release con guardarraíles, README en
> inglés, demo ejecutable con smoke en CI, `docs/user/`, reorganización de `docs/` y embudo de
> contribución.

| Épica | Estado | Área | Doc |
|---|---|---|---|
| **E27** — Producto, distribución y apertura OSS | ✅ completa (2026-08-02; H10 bloqueada por `decisiones §17-DA`) | guardarraíles de release · README EN · `examples/demo/` + smoke CI · `docs/user/` · `docs/history/` · CONTRIBUTING/SECURITY/CoC · templates | [epica-27-producto-distribucion-oss.md](epica-27-producto-distribucion-oss.md) |

**Orden de construcción (E27)**: `H01 → H03 → H02 → H04 → H06 → H05 → H11 → H07 → H08 → H09 →
[H10]`. **H10 (crates.io) está `[BLOQUEADA por decisiones §17]`** hasta que esa decisión se reabra.
**Principio rector**: *la superficie externa solo promete lo que el motor ejecuta hoy* — mientras
`decisiones §14` siga abierta, ningún documento público presenta `reindex`/la cache SQLite como
camino de lectura ni promete rendimiento a escala (`§21.5`). **Regla de idioma** (`§21.1`): la
superficie pública que esta épica produce va en **inglés**; los documentos internos siguen en
español.

## Mapa de épicas de la campaña de bugfixes del testbench homelab

> Origen: `decisiones/23-hallazgos-testbench-homelab.md` (dogfooding sistemático, 189 casos sobre
> el workspace real del homelab, 2026-08-06). Trece hallazgos de naturalezas incompatibles se
> repartieron por prioridad y dueño (la misma lección de `decisiones §16`); **E28 es su Fase 0**:
> las dos filas con **riesgo real de pérdida de conocimiento** (prioridades 5 y 4), ejecutadas antes
> que cualquier otro hallazgo de la tabla. **E29 es la Fase 1**: la épica de honestidad de superficie
> que el orden de trabajo de `decisiones/README.md` (L110-132) fija como lo siguiente que entra. El
> tablero de la campaña está en [`docs/qa/campana-bugfixes-2026-08.md`](../docs/qa/campana-bugfixes-2026-08.md).

| Épica | Estado | Área | Doc |
|---|---|---|---|
| **E28** — Fase 0: defectos destructivos del testbench homelab | ✅ completa (2026-08-06) | `change_revert` de un recibo `-revert` restaura de verdad (+ unifica la coreografía de sellado de `decisiones §16(i)`) · guard de colisión en `create`/`move` (`DOCUMENT_ALREADY_EXISTS`, catálogo 16→17) | [epica-28-defectos-destructivos-testbench.md](epica-28-defectos-destructivos-testbench.md) |
| **E29** — Fase 1: honestidad de superficie | propuesta (pendiente de ratificación) | config estricta (`§16e`+A-08) · `policy` parcial (`§19b`) · `has(frontmatter)` (`§19a`) · type error de afijo (A-04) · scope `paths` (A-07) · aviso de workspace vacío (`§16f`) · `canApply` vinculante (`§18`) · wire estricto (`§15`) · `instructions` por perfil + `protocolVersion` (D-01) · repliegue de la API no transaccional (`§16g`) · retirada del `Envelope` (`§16b`) | [epica-29-honestidad-superficie.md](epica-29-honestidad-superficie.md) |

**Orden de construcción (E28)**: `H01` y `H02` son independientes y paralelizables (ficheros y
garantías distintas); `H01` va primero en la secuencia por gravedad (pérdida de datos activa) y por
ser la prioridad más alta de `decisiones §23`. Los demás hallazgos de esa ficha (D-01/D-02,
A-01…A-10) quedan **fuera de E28** — tienen su propio dueño (épica de honestidad de superficie,
ciclo de higiene de `decisiones §16(j)`, o `§19`) y no implican escritura destructiva.

**Orden de construcción (E29)**: `H01 → H02 → H03 → H04 → H05 → H06 → H09 → H07 → H08 → H11 → H10`.
`H01` primero por decisión explícita de `decisiones §16(e)` (fija el criterio «lo desconocido se
rechaza» que `H08` aplica al wire); `H08` la última de las de comportamiento porque `§15` la declara
la mayor de la épica y pide que su complicación no arrastre a las demás; `H10` **después de que
E28-H01 esté integrada**, por compartir `crates/lodestar-workspace/tests/transactions.rs`. **Principio
rector**: *la superficie solo promete lo que el motor ejecuta hoy* (`§21.5` generalizado del exterior
al wire), con el corolario de E26: *una respuesta silenciosamente equivocada es peor que un error*.

## Formato de una historia

Cada historia tiene un identificador estable `E<épica>-H<nn>` y esta plantilla:

```
### E1-H07 — Título corto y accionable
- **Objetivo**: una frase: qué capacidad entrega.
- **Referencias**: ARCHITECTURE §X.Y · prototipo `funcA`/`funcB` · historias relacionadas.
- **Alcance**: el trabajo concreto, en viñetas. Incluye señales de API (firmas Rust) cuando el
  contrato las fija.
- **Fuera de alcance**: lo que NO entra (para evitar scope creep).
- **Criterios de aceptación**: checklist binario y verificable (lo que un revisor comprueba).
- **Dependencias**: IDs de historias que deben estar `Done` antes.
- **Pruebas**: qué tests/fixtures demuestran la historia.
```

## Definición de **Done** (aplica a TODA historia)

Una historia está `Done` cuando:

1. **Compila** en el workspace sin warnings nuevos (`cargo build`/`cargo clippy -- -D warnings`).
   Los gates del frontend (`svelte-check`/`tsc`) se retiraron de `main` con la UI
   (`experimental/ui-desktop`).
2. **Tiene tests** que cubren su comportamiento (unit + el arnés de paridad/golden que aplique) y
   **pasan** (`cargo test --workspace`, más `--features test-failpoints` en los crates que lo
   gatean).
3. **Respeta los invariantes no negociables** de `CLAUDE.md` / `ARCHITECTURE.md §2,§10`:
   core puro, único escritor, una sola verdad computada, un solo contrato de tipos, `RelPath`
   newtype.
4. **No reintroduce duplicación de tipos** ni capa DTO paralela (principio #4).
5. **Documenta** la superficie pública nueva (`///` en Rust) en español.
6. El **arnés de paridad** de su fase sigue verde (cuando exista).

## Invariantes que toda historia debe preservar (recordatorio)

1. Los `.md` en disco son la **única fuente de verdad**; lo demás se deriva.
2. `lodestar-core` es **puro** (`#![forbid(unsafe_code)]`, sin `tauri`/`rusqlite`/`notify`/`tokio`/`git2`).
3. **Una sola verdad computada**: cuando SQL y core podrían discrepar, gana el core.
4. **Un solo contrato de tipos** en `lodestar-core::types`, sin capa DTO paralela. Lo derivado es
   el JSON Schema de `outputSchema` (vía `schemars`); el espejo `.d.ts` **ya no existe** — se fue
   con la UI a `experimental/ui-desktop` (E27-H07 corrigió esta fila, que aún lo afirmaba).
5. **Un watcher = único escritor**: los comandos escriben el `.md` (atómico temp+rename); el watcher reconcilia.
6. `RelPath` newtype validado (rechaza absolutas/`..`): único chokepoint de path-traversal.
7. ~~git con **vocabulario directo**; transporte híbrido~~ — **RETIRADO** (`E15-H01`,
   `ARCHITECTURE.md §20.13`): git salió de la superficie y del repo; no queda crate `vcs` ni
   `git2`. Se conserva tachado porque las épicas históricas E0–E8 lo citan.

## Trazabilidad

Cada historia mapea a una o más decisiones ratificadas (`§10`, filas 1–21) y/o concerns transversales
(`§12`). El campo **Referencias** las nombra para que el revisor pueda auditar que la decisión no se
relitigó. La matriz de cobertura está en [trazabilidad.md](trazabilidad.md).
