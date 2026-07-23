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
- **Idioma**: español en código, comentarios, UI, mensajes y commits (el usuario es hispanohablante),
  salvo identificadores técnicos congelados por el contrato (nombres de tipos, comandos, eventos,
  códigos `OKF-*`).

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
(D0–D6/D-CheckCode/D-check) se ratificaron en la puerta 1 (`DECISIONES.md §0`, `ARCHITECTURE.md §19`).

## Mapa de épicas de la migración a Markdown universal (alineadas con `ARCHITECTURE.md §20.14`)

> Migración de **OKF a workspaces Markdown universales** (`ARCHITECTURE.md §20`, ratificada
> 2026-07-23; fuente: `docs/REFACTOR_PHASE_2.md`). Lodestar deja de exigir un formato documental
> propio y opera sobre cualquier red de `.md` de un proyecto. **v0.3.0 es incompatible con v0.2.x.**

| Épica | PRs `§20.14` | Área | Doc |
|---|---|---|---|
| **E15** — Workspace universal | 0 + 1 | Retirada de vcs/generadores/init-zip/prototipo · `cwd` como root · descubrimiento recursivo · config opcional | [epica-15-workspace-universal.md](epica-15-workspace-universal.md) |
| **E16** — Modelo documental genérico | 2 | `ParsedFrontmatter` YAML arbitrario · sin ficheros reservados · título derivado · patch quirúrgico · diagnósticos mínimos · `Concept`→`Document` | [epica-16-modelo-documental.md](epica-16-modelo-documental.md) |
| **E17** — Enlaces y grafo universal | 3 + 4 | Parser de enlaces · `LinkTarget` · diagnósticos de enlace · `Analysis` nueva · superficie de grafo | [epica-17-enlaces-grafo.md](epica-17-enlaces-grafo.md) |
| **E18** — Store v2 | 5 | DDL nuevo · metadata anidada · links genéricos · cold rebuild · paridad core/store | *(pendiente)* |
| **E19** — Lenguaje de consulta | 6 | Parser · AST · type checking · namespaces · filtro JSON equivalente | *(pendiente)* |
| **E20** — Inspección y validación genéricas | 7 + 8 | `metadata_inspect` (retira `core::schema`) · política `rejectNewErrors`/`allowExistingErrors` · **cablear los diagnósticos de descubrimiento** (ver abajo) | *(pendiente)* |
| **E21** — Contrato MCP y transacciones genéricas | 9 + 10 | Contrato nuevo · 8 operaciones universales · selecciones masivas por consulta | *(pendiente)* |
| **E22** — Migración y limpieza pública | 11 | `migrate-from-okf --dry-run` · docs · README · publicación incompatible | *(pendiente)* |

**Orden de construcción (E15–E22)**: estrictamente secuencial. **E15 es prerrequisito de todo** (sin
descubrimiento universal no hay nada que modelar); **E16** cambia el modelo documental y arrastra los
diagnósticos; **E17** depende de E16 (los enlaces se extraen de documentos ya genéricos); **E18** y
**E19** consumen el modelo y el grafo de E16/E17; **E20** retira `core::schema`; **E21** cierra la
frontera; **E22** publica.

**Hueco de cableado pendiente, con dueño (E20)**: `discovery::discover` computa los diagnósticos de
`§20.9` —`DOC-NOT-UTF8`, `DOC-TOO-LARGE`, `SYMLINK-UNSUPPORTED`, `PATH-NOT-UTF8` y las colisiones de
capitalización (`LINK-CASE-MISMATCH`)— de forma determinista, y su único llamador
(`Workspace::discover_files`) **los descarta**. Ni `knowledge_check` ni `lodestar check` los ven: hoy
la mitad del catálogo de `§20.9` es invisible. Está documentado como diferido en el propio
`discover_files`, no es un descuido silencioso, pero es la misma forma de hueco que E15-H07 y el
cableado de `other_files` de E17 (capacidad computada que no llega al producto). El call-site es el
mismo que ya se toca, así que es barato — lo que falta es el criterio de **política de severidad**,
que es precisamente lo que E20 aporta.

**Nota sobre el prototipo**: desde `E15-H04` el prototipo JS (`prototype/index.html`) **deja de ser la
spec de comportamiento** y el arnés diferencial se retira. La spec de la migración es
`docs/REFACTOR_PHASE_2.md`; `prototype/` queda como referencia histórica de v0.2.x.

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

1. **Compila** en el workspace sin warnings nuevos (`cargo build`/`cargo clippy -- -D warnings`
   para Rust; `svelte-check`/`tsc --noEmit` para el frontend).
2. **Tiene tests** que cubren su comportamiento (unit + el arnés de paridad/golden que aplique) y
   **pasan** (`cargo test`, `vitest`, etc.).
3. **Respeta los invariantes no negociables** de `CLAUDE.md` / `ARCHITECTURE.md §2,§10`:
   core puro, único escritor, una sola verdad computada, un solo contrato de tipos, `RelPath`
   newtype, vocabulario git directo.
4. **No reintroduce duplicación de tipos** ni capa DTO paralela (principio #4).
5. **Documenta** la superficie pública nueva (`///` en Rust) en español.
6. El **arnés de paridad** de su fase sigue verde (cuando exista).

## Invariantes que toda historia debe preservar (recordatorio)

1. Los `.md` en disco son la **única fuente de verdad**; lo demás se deriva.
2. `lodestar-core` es **puro** (`#![forbid(unsafe_code)]`, sin `tauri`/`rusqlite`/`notify`/`tokio`/`git2`).
3. **Una sola verdad computada**: cuando SQL y core podrían discrepar, gana el core.
4. **Un solo contrato de tipos** en `lodestar-core::types`; el `.d.ts` se genera desde Rust.
5. **Un watcher = único escritor**: los comandos escriben el `.md` (atómico temp+rename); el watcher reconcilia.
6. `RelPath` newtype validado (rechaza absolutas/`..`): único chokepoint de path-traversal.
7. git con **vocabulario directo**; transporte híbrido (libgit2 local + binario `git` solo para red).

## Trazabilidad

Cada historia mapea a una o más decisiones ratificadas (`§10`, filas 1–21) y/o concerns transversales
(`§12`). El campo **Referencias** las nombra para que el revisor pueda auditar que la decisión no se
relitigó. La matriz de cobertura está en [trazabilidad.md](trazabilidad.md).
