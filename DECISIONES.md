# Decisiones pendientes (requieren tu criterio)

> Este documento recoge las decisiones que **no se pueden tomar por inercia** desde el código o
> `ARCHITECTURE.md` y que dependen de tu criterio de producto/entorno. Cada una lleva el estado
> actual, el porqué de que quede abierta y una **recomendación**. Nada aquí bloquea lo ya
> implementado (backend completo y testeado); son decisiones para cerrar el último tramo (sobre todo
> E6 desktop) y para afinar comportamiento.

---

## 0. Giro a motor headless de integridad semántica — ✅ RATIFICADO (2026-07-22)

- **Contexto**: `docs/REFACTOR.md` redefine Lodestar como **motor headless de integridad semántica**
  (busca/comprende/valida/modifica conocimiento vía cambios planificados y recuperables, sin editor,
  sin GUI y sin git). Propuesta de diseño en `docs/REFACTOR_DISENO_PROPUESTA.md`; diseño ratificado en
  **`ARCHITECTURE.md §19`** (supersede §13 en superficie de producto). Descomposición en
  `requirements/epica-09-*.md` … `epica-14-*.md`.
- **Sub-decisiones cerradas** (puerta 1 de `/planificar`):
  - **D0** — Adenda como **§19 nueva** + nota de cabecera en §13 ("superada en superficie; crate `vcs`
    y mecánica §13.2–§13.6 conservados como dormidos") + anotación en §10 (filas de git ciertas sobre
    el crate, exposición revertida).
  - **D1** — Capas nuevas: **Opción C (híbrido)** — mecánica transaccional en `lodestar-workspace`
    (único escritor); crate nuevo **`lodestar-app`** fino como servicios de caso de uso que comparten
    mcp/cli.
  - **D3** — Envelope en `lodestar-app`; **códigos de error** en `core::types`.
  - **D4** — Config migra a **`.lodestar/config.yaml`** YAML unificado
    (`workspace.{writableRoots,referenceRoots,ignored}` + `gate` + `transactions`; `identity` dormida).
  - **D5** — `.lodestar/{config,schema}.yaml` + `templates/` **versionados**; `.lodestar/runtime/` +
    `index.db` **gitignored**; `WorkspaceRevision` **excluye todo `.lodestar/`**.
  - **D6** — (a) generadores **solo CLI** + auto-regen dentro de `change_apply`; (b) transporte
    **stdio + `outputSchema` vía `schemars`**, `rmcp` **diferido**.
  - **D-CheckCode** — Familias estáticas acotadas de `CheckCode` (`SCHEMA-REQFIELD`, `SCHEMA-STATUS`,
    `REL-TARGET`, `REL-CARD`, `REL-TYPE`), i18n keyed por código.
  - **D-check** — `lodestar check` sigue como puerta de CI sobre el working tree;
    `--staged`/`--rev`/`--range` **diferidos** con el crate `vcs` dormido.
- **Confirmadas** (se declaran en §19, sin criterio adicional): `core::schema` en el core **puro**;
  modelo transaccional en `workspace`; reutilización de `OkfDiff`/`blast_radius`/`neighborhood`/
  `Mutation`/`RelPath`/blake3; seguridad §14 (simplificada al no haber git/red/exec en la superficie).
- **Cierres colaterales**: la parte de **git** de este documento queda **superada por §19** (§6 semántica
  de `merge` local, y la exposición de git en fachadas): el crate `vcs` se conserva dormido pero su
  superficie no se implementa en v2. §3 (rmcp) se reafina a "**stdio + `outputSchema`, `rmcp` diferido**".

---

## 1. Build de la fachada de escritorio Tauri (E6) — ✅ RESUELTO/IMPLEMENTADO

- **Estado**: `src-tauri` es ahora una **fachada Tauri v2 real y compilada**: tabla de comandos con
  los nombres congelados (`open_bundle`/`get_snapshot`/`read_concept`/`write_concept`/`create_concept`/
  `conformance`/`query`/`backlinks`/`graph_model`/… + `history`/`diff_working`/`commit`), estado del
  bundle abierto, y un **forwarder** que reemite el bus `IndexEvent` de la cache como `bundle:changed`
  (watcher + escrituras → UI en vivo). Compila en este entorno (webkit disponible) y produce el binario
  `lodestar-desktop`. El **CI de Rust** ya instala las libs de sistema (`libwebkit2gtk-4.1-dev`,
  `libsoup-3.0-dev`, …) y construye el `frontend/dist` antes del `cargo build` (Tauri lo embebe).
- **Empaquetado/release — PARCIALMENTE RESUELTO (v0.1.0)**:
  - **Plataformas objetivo cerradas**: **macOS Apple Silicon (arm64)**, **Windows** y **Linux**.
    Existe pipeline de release (`.github/workflows/release.yml`) que se dispara con el tag `vX.Y.Z`,
    compila las tres plataformas y crea un GitHub Release en **borrador** con los bundles (dmg/deb/
    appimage/nsis) + los binarios de CLI/MCP. `bundle.active = true` y los **iconos de marca** (la
    estrella dorada) ya están integrados. Runbook en [`RELEASING.md`](RELEASING.md).
  - **Firma/notarización — DIFERIDA (no cerrada)**: los bundles de v0.1.0 salen **SIN FIRMAR** para
    las tres plataformas (avisos de Gatekeeper/SmartScreen al instalar). Queda pendiente decidir e
    integrar certificados + notarización cuando se quiera distribución sin fricción (§12 packaging,
    E8-H06). **No es un no-go**; es trabajo de infraestructura + secretos.
  - **Updater**: sigue sin cablear (no bloquea; la distribución es por descarga manual del Release).
  - **crates.io — PREPARADO, SIN PUBLICAR**: el orden topológico y los `publish = false` (fixtures,
    tauri) están listos (ver [`RELEASING.md`](RELEASING.md)), pero **no se publica**: el repo es
    **privado** y `cargo publish` haría el código público y permanente. Queda a criterio del usuario.
- **Recomendación**: v0.1.0 ya se distribuye por Release multiplataforma sin firmar; abordar la
  firma/notarización (y opcionalmente el updater) en una iteración posterior, según necesidad real de
  distribución amplia.

## 2. Port de la UI del prototipo (E6) — ✅ IMPLEMENTADO (funcional)

- **Estado**: el frontend Svelte 5 es una app funcional completa sobre el `BundleSnapshot`:
  layout de **tres columnas** (páginas · centro · enlaces) con paneles colapsables, **árbol** filtrable
  con estados (orphan/invalid), **tabs** editor · grafo · cambios, **editor multi-escritor** que guarda
  por el único escritor con validación y diagnósticos localizados, **panel de enlaces** (entrantes/
  salientes/índice), **isla imperativa del grafo** (`createStarMap`: posee el SVG + loop rAF, recibe
  nodos/aristas por `$effect`, nunca `{#each}`), y **modo «Cambios»** (diff semántico `OkfDiff` + commit
  con mensaje sugerido). Aspecto con las variables CSS portadas del prototipo. `npm run check`/`build`
  en verde.
- **Qué queda (pulido, no bloquea)**: rails **redimensionables por arrastre** (hoy son colapsables),
  overlay de grafo a pantalla completa, resaltado de query en el grafo con la **semántica del core**
  (hoy es subcadena sobre el id), y detalles de micro-interacción del prototipo.
- **Recomendación**: iterar el pulido visual según uso real; la funcionalidad completa ya está.

## 3. Transporte MCP: stdio propio vs `rmcp` oficial (E7)

> **Reafinada por §0/§19 (2026-07-22)**: se mantiene **stdio** y se activa **`outputSchema` vía
> `schemars`** (lo exige el contrato de la superficie 13→10, `REFACTOR §13`); **`rmcp` sigue diferido**
> hasta tener un cliente que lo requiera.


- **Estado**: el MCP funciona como servidor **JSON-RPC por stdio** (stdout puro), con 13 tools y
  test golden cross-fachada (salida de cada tool == `Workspace` directo). Falta el transporte oficial
  `rmcp` + `resources` + `outputSchema` (feature `schemars` ya preparada en el core).
- **Qué decidir**: ¿adoptamos `rmcp` ahora (transporte oficial, resources, negociación de capacidades)
  o mantenemos el stdio propio hasta tener un consumidor que exija `rmcp`?
- **Recomendación**: mantener stdio hasta tener un cliente MCP real que lo requiera; el contrato de
  tools ya está congelado, migrar el transporte después es mecánico.

## 4. Generación del `.d.ts` desde Rust (ts-rs/specta) — E0-H04/E6-H03 — ⚪ OBSOLETA (UI retirada de `main`)

> **Obsoleta para el espejo TS** desde el giro headless: `frontend/src/lib/ipc/types.ts` desapareció
> al retirar la UI de escritorio de `main` a la rama `experimental/ui-desktop`. Los tipos de
> `core::types` los consumen ya directamente `lodestar-cli`/`lodestar-mcp` (Rust), sin espejo TS que
> generar. Se conserva el registro histórico abajo; si la UI vuelve a evolucionar en esa rama, la
> decisión de ts-rs se retomaría allí.

- **Estado**: `frontend/src/lib/ipc/types.ts` era un **espejo a mano** del contrato de `core::types`,
  marcado como «a generar». Los nombres/orden coincidían con Rust.
- **Decidido (2026-07-10)**: **sí a ts-rs** — el `.d.ts` se generará desde Rust. Además, la
  frontera front↔back queda descrita por **contratos YAML de superficie** (`contracts/ipc.yml`,
  `contracts/mcp.yml` + `contracts/README.md`): el YAML documenta comandos/eventos/tools y su
  semántica; los **tipos** siguen viviendo solo en `core::types` (invariante #4). El drift se
  vigila con el skill `/contrato --check` (agente `guardian-contrato`).
- **Pendiente**: la implementación de ts-rs (deps + paso de build + marcar `types.ts` como
  generado/«NO EDITAR»). Acordado ejecutarla como **primera historia del nuevo flujo `/ciclo`**
  (dogfooding de `.claude/README.md`). Esta sección se cierra en ese PR.

## 5. i18n multi-idioma

- **Estado**: la app es **español-only** en v1 (decisión ya tomada en `CLAUDE.md`). El catálogo de
  conformidad está **keyed por `CheckCode`** (`frontend/src/lib/i18n.ts`) y el core emite `code`+
  `targets`, así que añadir un locale = añadir un objeto con las mismas claves.
- **Qué decidir**: ¿hay que soportar inglés u otro idioma en v1? Si no, esto queda cerrado.
- **Recomendación**: mantener español-only en v1; la arquitectura ya no lo impide en el futuro.

## 6. Semántica de `merge` local — ⚫ CERRADA/OBSOLETA (crate `vcs` borrado, §20)

> **Cerrada por §20 (2026-07-23)**: la migración a workspaces Markdown universales **borra** el crate
> `lodestar-vcs` (E15-H01), no lo deja dormido. Ya no hay `merge` que decidir: si git volviera algún
> día a la superficie, se rediseñaría desde cero. Se conserva el registro histórico.
>
> **Superada antes por §0/§19 (2026-07-22)**: git sale de la superficie de producto; el crate `vcs` (con su
> `merge` a nivel de árbol) se conserva **dormido**, sin fachadas que lo expongan. Esta decisión queda
> como diseño de referencia por si git vuelve.


- **Estado**: `merge` se implementa a **nivel de árbol** (`merge_trees` de libgit2): el vcs **no
  escribe el working tree**; devuelve el `FileMap` resultante para que la workspace lo aplique por el
  único escritor. En conflicto, los ficheros llevan marcadores `<<<<<<< / ======= / >>>>>>>` (los
  detecta `OKF-CONFLICT`) y se deja `MERGE_HEAD` → `repo_state() = Merging` bloquea el commit hasta
  resolver. Fast-forward y up-to-date resueltos aparte.
- **Por qué está abierta**: es una elección de UX. La alternativa sería delegar el merge al binario
  `git` (con su resolución/hooks), lo que rompería el invariante «vcs no escribe el working tree en
  local» y el modelo de único escritor.
- **Qué decidir**: ¿confirmas el merge a nivel de árbol por el único escritor (recomendado, coherente
  con §16) o prefieres delegar en el binario `git`?
- **Recomendación**: confirmar el enfoque actual.

## 7. `lodestar check --range a..b` — ⚫ CERRADA/OBSOLETA (sin git, §20)

> **Cerrada por §20 (2026-07-23)**: `--staged`/`--rev`/`--range` se retiraron de la superficie en
> E9-H02 quedando diferidos con el crate `vcs` dormido; al borrarse el crate en E15-H01 dejan de
> tener implementación posible. `check` juzga el working tree y nada más. Registro histórico abajo.

- **Estado**: `--range` juzga **la punta** del rango (equivale a `--rev b`).
- **Qué decidir**: ¿basta con la punta o quieres verificar que **cada commit** del rango es conforme
  (útil para bisect/PR gates)? Lo segundo es más caro pero más estricto.
- **Recomendación**: dejar la punta por defecto y añadir `--each` si en algún momento hace falta el
  barrido por-commit.

## 8. Esquema de `lodestar.toml` — ⚫ CERRADA/OBSOLETA (fichero retirado, §20)

> **Cerrada por §20 (2026-07-23)**: `lodestar.toml` se **borra** en E15-H08. Su `[identity]` murió
> con git (E15-H01) y su `[gate]` se absorbe en `.lodestar/config.yaml`, el único fichero de
> configuración (`§20.5`). Lo que la pregunta abierta pedía —override de severidad por código y
> exclusión de rutas— **se concede** en el formato nuevo: `discovery.exclude` y la sección
> `validation:` de `§20.9`, que fija la severidad por familia de diagnóstico. Registro histórico:

- **Estado**: soporta `[gate] block_warnings` (strictness) e `[identity] name/email` (override de
  autor/committer). Defaults seguros (solo `Err` bloquea; identidad por defecto).
- **Qué decidir**: ¿quieres más granularidad, p. ej. **override de severidad por código** (subir/bajar
  un `CheckCode` concreto) o listas de exclusión de rutas?
- **Recomendación**: mantener el esquema mínimo actual hasta tener una necesidad real; es aditivo.

## 9. Transversales diferidas de producto (E8)

Pendientes de priorización (no bloquean el núcleo):
- **Gate de rendimiento (§11)**: bench de cold-open 10k < ~2s y edit→UI < 150 ms como test de CI.
  El motor incremental ya existe (store); falta el arnés de bench con umbrales.
- **Packaging/release CI + updater + firma** (ligado al punto 1): **CI de release ya existe**
  (`release.yml`, tres plataformas, bundles sin firmar); **queda la firma/notarización + updater**.
- **Threat model** documentado (§12 seguridad); las piezas ya están (RelPath anti path/zip-slip,
  FTS5 escapado, git de red confinado al binario, libgit2 local sin hooks).
- ~~Arnés diferencial JS-vs-Rust (E1-H18)~~ — **hecho y luego RETIRADO en `E15-H04`** (el prototipo
  dejó de ser spec con la migración a Markdown universal, `ARCHITECTURE.md §20.13`). Histórico:
  `prototype/harness/` ejecutaba las funciones
  puras del prototipo en node como oráculo y `tests/differential.rs` compara con el core (6 fixtures);
  cazó y cerró 6 divergencias de paridad.

## 10. Ghosts como primitiva de planificación + templates (siguiente feature, no iniciada)

> **Parcialmente superada por §20 (2026-07-23)**: la primitiva **sobrevive y de hecho mejora** — un
> ghost es un enlace a un `.md` inexistente, que en el modelo nuevo es un `LinkTarget::Missing`
> (`§20.6`) con su `dangling` identificando origen y href crudo (`§20.7`), más informativo que el
> `LINK-STUB` de antes. Lo que **muere** son las piezas OKF de la propuesta: el gesto de UI (la UI se
> retiró de `main`) y los *templates por `type`* con `.lodestar/templates/` (`core::schema` se borra
> en E20; `§20` no tiene tipos documentales). Si se retoma, el backlog de ghosts se lee hoy con
> `graph_query(dangling)`.

- **Contexto**: los *ghosts* («por escribir») ya existen y están portados: nodo con `ghost: bool` en
  `GraphModel` (`core/graph.rs`) derivado de enlaces a `.md` inexistentes, check `LINK-STUB` con
  severidad **info** (no rompe `check`). Dan un modelo de estados gratis y no falseable:
  ghost = planificado · existe-pero-no-conforme = en curso · conforme = hecho. Todo derivado de los
  `.md` en disco (invariante #1), sin campo `status:` que mantener.
- **Qué se quiere** (acordado como dirección, pendiente de diseño):
  1. **Crear ghosts desde la UI**: gesto de «esto habrá que crearlo». Para no introducir estado
     nuevo, «crear un ghost» debe materializarse como **insertar un enlace** en una página existente
     (la actual, o una página-plan por convención) — el ghost sigue siendo 100% derivado.
  2. **Tool MCP para leer ghosts** (`list_ghosts` o similar): ghosts con sus backlinks e in-degree
     (cuántas páginas lo reclaman = prioridad), para que un agente consuma el backlog y vaya creando
     páginas conformes siguiendo el plan. El contexto/spec de cada ghost es la prosa alrededor de
     los enlaces que le apuntan.
  3. **Templates**: plantillas tanto de **archivos sueltos** (esqueleto de frontmatter/cuerpo por
     `type`) como de **directorios** (estructura de páginas planificadas — posiblemente expresable
     como una página-plan que genera los ghosts de toda la estructura).
- **Qué decidir cuando se aborde**: UX del gesto en la UI (¿desde el grafo?, ¿desde autocompletado
  de enlaces?), dónde viven los templates (¿`.lodestar/templates/`?, ¿páginas especiales?), si el
  template de directorio crea ghosts (solo plan) o stubs (archivos reales), y la firma exacta de la
  tool MCP.
- **Recomendación**: mantener el principio «ghost = derivado de enlaces»; cualquier variante que
  requiera una lista de ghosts persistida aparte contradice el invariante #1.

## 11. `pulldown-cmark` en `lodestar-core` (E17) — 🟡 TOMADA, revisable

- **Contexto**: la migración exige enlaces Markdown **de referencia** (`[t][id]` con su definición
  `[id]: ../p.md` en otro punto del documento) y **offsets fiables** del destino dentro del cuerpo,
  para reescribirlo en `move_document` (`§20.6`, `§20.11`). Hoy el parser son dos regex
  (`crates/lodestar-core/src/model.rs:16-17,257-258`) que solo ven `[texto](href)`.
- **Decidido (2026-07-23, al escribir la épica E17)**: adoptar `pulldown-cmark` como dependencia de
  `lodestar-core`. Es **pura** (sin I/O, sin runtime, sin C), así que no viola el invariante #2 ni el
  job `core-purity` del CI, que prohíbe `tokio`/`rusqlite`/`git2`/`notify`/`tauri`. Aporta
  resolución nativa de referencias, `link_type` (que es exactamente la clasificación de `§20.6`) y
  `OffsetIter`.
- **Por qué queda anotada aquí**: es la **primera dependencia de parsing** que entra en el core, que
  hasta ahora se autoabastecía con regex. Si prefieres no ampliar la superficie de dependencias del
  core, la alternativa es extender la regex — pero no cubre enlaces de referencia sin reimplementar
  buena parte de un parser Markdown, y los offsets serían menos fiables.
- **Reversible**: solo afecta a `crates/lodestar-core/src/links.rs` (E17-H01). Dilo antes de que
  E17 empiece y se replantea.

## 12. Comparación de fechas en el lenguaje de consulta (E19) — ✅ CERRADA en (a) (E23-H14)

- **Contexto** (detectado en la fase roja de E16-H01): `REFACTOR_PHASE_2 §Fase 4` exige soportar
  *"fechas interpretadas como valores YAML"*, y `§Fase 5` pide comparaciones tipadas sin coerción
  implícita (`priority >= 2` funciona, `priority >= "high"` es error de tipo). Pero **`serde_yaml`
  0.9.34 no tiene tipo timestamp**: un `2026-07-23` sin comillas se deserializa como `String`.
- **Consecuencia**: hoy `reviewed_at > "2026-01-01"` sería una comparación de **strings**. Para
  fechas ISO-8601 bien formadas el orden lexicográfico coincide con el cronológico, así que
  «funciona» — pero silenciosamente, y deja de funcionar con formatos mixtos (`2026-7-3`), con
  offsets de zona horaria distintos, o al comparar una fecha con un datetime.
- **Qué decidir**: (a) declarar explícitamente que las fechas son strings y su comparación es
  lexicográfica, documentándolo como limitación; (b) introducir un tipo fecha propio en el core que
  reconozca ISO-8601 al indexar (`§20.12` guarda `value_type` en el store, así que hay sitio);
  (c) cambiar de librería YAML por una que tipe timestamps.
- **Recomendación**: **(a) para E19** —es lo barato y cubre el caso real, que son fechas ISO— y
  reevaluar en E20, cuando `metadata_inspect` tenga que **comunicar** el tipo inferido de cada
  propiedad y la ficción de "todo es string" se note. No bloquea: se puede empezar por (a) y migrar
  a (b) sin romper el wire, porque el tipo viaja en `value_type`.
- **Resolución (E23-H14, 2026-07-25): (a)**, declarado por escrito. E19 y E20 cerraron sin tocar
  esta decisión y la limitación **no estaba documentada en ninguna superficie de usuario** — ni en el
  README ni en el contrato—, que era la mitad peor: un motor que presume de *no coercionar tipos*
  tenía una coerción implícita de facto, sin avisar. Ahora está declarada en `contracts/mcp.yml`
  (semántica de `where`) y en el README.
  **Lo que se declara**: no hay tipo fecha. Un `2026-07-23` sin comillas en el frontmatter es un
  **string** para `serde_yaml` 0.9, y las comparaciones de orden entre strings son **lexicográficas**.
  Para fechas ISO-8601 bien formadas y de la misma longitud eso **coincide** con el orden
  cronológico, así que el caso real funciona; deja de funcionar con formatos mixtos (`2026-7-3`),
  con offsets de zona horaria distintos, o al comparar una fecha con un datetime.
  **Migrar a (b)** —tipo fecha propio en el core, reconocido al indexar— sigue siendo posible sin
  romper el wire, porque el tipo viaja en `value_type` (`§20.12`). Se hará si aparece un caso real
  con formatos mixtos, no antes.

## 13. `Conformant → Valid`: la mitad que falta de `§20.3` — ✅ CERRADA en (a) (E23-H14)

- **Contexto** (detectado al implementar E16-H06): la tabla de terminología de `§20.3` manda dos
  sustituciones emparejadas, `Conformance → Validation` y `Conformant → Valid`. La primera está
  hecha (`ApplyConformance` → `ApplyValidation`, `Store::conformance_counts` →
  `validation_counts`…). La segunda **no**, y el resultado es una asimetría visible en el wire:
  `ApplyValidation { conformant, errors, warnings }` — el contenedor habla de *validación* y el
  veredicto sigue hablando de *conformidad*.
- **Por qué no se cerró**: `conformant` no aparece solo como campo. Está en
  `ErrorCode::NonconformantResult` (wire `NONCONFORMANT_RESULT`), que es **una de las 16 filas
  congeladas** del catálogo de errores de `§19.3`, y en `PlanPolicy::requireConformantResult` y
  `allowNonconformant`, que son superficie de `change_plan`. No es un renombre léxico: toca el
  contrato de errores.
- **Qué decidir**: (a) completar la pareja y aceptar el cambio de wire
  (`NONCONFORMANT_RESULT` → `INVALID_RESULT`, `requireConformantResult` → `requireValidResult`),
  aprovechando que v0.3 ya es incompatible; (b) dejar `conformant` como término del **veredicto**
  y documentar en `§20.3` que la sustitución solo aplica al *sustantivo*, no al *adjetivo*.
- **Recomendación**: **(a)**, y hacerlo en **E21**, que es cuando se toca el motor transaccional y
  su contrato de errores. Hacerlo ahora significaría abrir el catálogo congelado dos veces.
  Mientras tanto la asimetría es fea pero inocua: `core::types::ValidationReport` ya tenía esa
  misma forma antes de la migración, así que no se ha introducido una discrepancia nueva.
- **Estado (E21-H04, cierre de E21)**: **APLAZADA, no cerrada**. E21-H04 hizo el renombre neutro
  que sí era léxico y sin riesgo (`core::diff::OkfDiff` → `SnapshotDiff`) y repasó el contrato: en
  la superficie viva de `mcp.yml` no queda vocabulario OKF salvo `conformant`/
  `requireConformantResult`/`NONCONFORMANT_RESULT`, que son exactamente esta decisión. Completar la
  opción (a) exige tocar `ErrorCode::NonconformantResult` —**una de las 16 filas congeladas** del
  catálogo de errores (`§19.3`)— y la superficie de `change_plan`; eso **no** es un renombre léxico
  y queda **fuera** del alcance de E21-H04 (la historia acota su repaso a lo que no abre el catálogo
  congelado). La asimetría sigue siendo fea pero inocua. Se retoma en la historia que decida abrir
  el catálogo de errores; hasta entonces `conformant`/`NONCONFORMANT_RESULT` se conservan tal cual y
  su presencia en la superficie activa de `mcp.yml` es **deuda documentada**, no una discrepancia de
  contrato.
- **Resolución (E23-H14, 2026-07-25): (a)**, decidida por el usuario. Se abre el catálogo de 16
  códigos —**la única vez**— porque el momento es ahora o nunca: v0.3.0 ya es incompatible con
  v0.2.x, así que romper el wire cuesta **cero** hoy y deja de costarlo en cuanto se publique. Y
  porque era el **único** de los 29 criterios de aceptación de `REFACTOR_PHASE_2` demostrablemente
  incumplido: *«no existe terminología OKF en la API pública»*.
  Renombres aplicados (69 apariciones en 12 ficheros de producción + 16 de test):
  `conformant` → `valid` · `requireConformantResult` → `requireValidResult` · `allowNonconformant`
  → `allowInvalid` · `NONCONFORMANT_RESULT` → `INVALID_RESULT` ·
  `ErrorCode::NonconformantResult` → `ErrorCode::InvalidResult` ·
  `WorkspaceError::NonconformantResult` → `WorkspaceError::InvalidResult`. El catálogo **sigue
  teniendo 16 filas**: se sustituye una, no se añade ninguna.
  También la salida humana de `lodestar check`: `CONFORME`/`NO CONFORME` → `VÁLIDO`/`NO VÁLIDO`. No
  lo exigía la tabla de `§20.3` —que habla de la API—, pero es la aparición más visible del término
  retirado: la línea que un humano lee en cada ejecución de CI.
  **No se tocaron** los documentos históricos (`docs/REFACTOR.md`,
  `docs/REFACTOR_DISENO_PROPUESTA.md`) ni las tablas de terminología que **documentan el propio
  renombre** (`§20.3`, `REFACTOR_PHASE_2 §Terminología`): ahí `Conformant` es el término de partida
  y sustituirlo las dejaría diciendo «Valid → Valid».

## 14. El store (épica E18) no tiene ningún consumidor — 🟠 ABIERTA (E23-H16)

- **Contexto** (detectado en la revisión de la PR #17, 2026-07-25): la épica **E18 entera** —DDL v2
  `documents`/`metadata`/`links`/`diagnostics`, metadata indexada recursivamente por field path con
  su tipo, FTS sin campos privilegiados, cold rebuild, tests de paridad core↔store— está construida,
  verificada… y **ningún consumidor la usa**.
- **Hallazgo verificado**: el único `enable_cache()` del producto está en
  `crates/lodestar-cli/src/commands.rs` (`lodestar reindex`), y solo la **construye**. `App::open` abre
  el `Workspace` sin cache; ninguna de las 10 tools MCP lee de SQLite. `knowledge_search` resuelve por
  `core::text::loose_text_match` sobre el `DocumentSet` en RAM, y `Workspace::document_set()` llama a
  `discovery::discover` en **cada invocación**: relee y reparsea la base entera desde disco en cada
  llamada. Las mediciones de escala de E14-H05 (~10k documentos) son, por tanto, el rendimiento real
  del producto, no el de la cache.
- **Agravante**: el walker del store (`crates/lodestar-store/src/lib.rs`) construye su **propio**
  `ignore::WalkBuilder` y **no aplica la `DiscoveryPolicy`** de `§20.5`: ni `.lodestarignore`, ni
  `discovery.include`/`exclude`, ni `maxDocumentBytes`, ni el endurecimiento de determinismo
  (`parents(false)`, `git_global(false)`, `git_exclude(false)`). E15-H07 declaró explícitamente que
  «la reconfiguración del watcher a la política nueva es parte de E18»; E18 tiene cuatro historias y
  ninguna es esa. Consecuencia: la **paridad core↔store** —invariante 13 de `REFACTOR_PHASE_2` y
  criterio de aceptación— solo se sostiene en workspaces con política por defecto, que es justo el
  caso que ejercitan los tests de paridad. Hoy es inocua porque nadie lee el store; se convierte en
  un bug real el día que se conecte.
- **Qué decidir**: (a) **conectarlo** — `document_set()` lee de SQLite con invalidación por hash y el
  walker del store se alinea con `DiscoveryPolicy`; es la opción que rentabiliza E18 y arregla el
  reparseo por llamada, y la que más código toca; (b) **acotarlo** — declarar por escrito que la
  cache solo sirve a `reindex` y a consumidores externos, y documentar que el motor lee de disco;
  (c) **retirarla** — borrar `lodestar-store` como se borró `lodestar-vcs` en E15-H01, asumiendo que
  el modelo en RAM basta para el tamaño objetivo.
- **Recomendación**: **(a)**, pero **no dentro de E23**. Conectar el store cambia el camino de lectura
  de las 10 tools y toca el invariante #3 («SQLite es cache derivada… cuando podrían discrepar, gana
  el core»), así que merece su propia épica con puerta de diseño, no un apéndice del cierre de la
  migración. Mientras tanto, lo honesto es que esté **registrado**: hoy no es un bug —nada discrepa
  porque nada lee—, pero sí es la mayor cantidad de capacidad construida sin consumidor del repo, y
  la razón por la que cada llamada MCP reparsea la base completa.
- **No bloquea** el merge de la PR #17: el producto funciona, solo que sin cache.

---

## 15. ¿Debe el servidor RECHAZAR los parámetros que no declara? — 🟠 ABIERTA (E24-H09)

- **Contexto**: `contracts/mcp.yml` enuncia la **regla de la casa** de la superficie MCP: *«el
  servidor valida los VALORES de los parámetros que declara, e IGNORA lo que no declara»*. La
  primera mitad **ya se cumple** desde `E24-H09`, que era donde estaba el defecto real (`limit: 0`
  devolvía 0 resultados en silencio pese a que el schema declara `minimum: 1`).
- **Lo que queda abierto es la segunda mitad**, y es una **deuda que el propio contrato declara**:
  todos los `inputSchema` anuncian `additionalProperties: false`, y **el servidor no lo ejecuta**.
  O sea, la superficie afirma algo que no cumple — exactamente el defecto que E23 vino a saldar,
  aquí en su forma más pequeña.
- **Medido** (revisión de la v0.3.0, sonda 4): 15 casos aceptados en silencio, entre ellos un
  `sort` retirado en E23-H11, un `offset` que no existe y typos como `wheres`/`filters`. Un agente
  que se equivoca de nombre de parámetro no se entera: recibe la respuesta por defecto.
- **Por qué NO se cerró en E24**: no es un bugfix, es **revisar un criterio ratificado**. La
  política vigente está escrita en tres sitios (`contracts/mcp.yml` `validacion_de_argumentos`, la
  cabecera de `tests/descubribilidad.rs`, y la justificación del schema plano en `tools.rs`), y su
  razonamiento no es trivial: `operacion_item_schema()` declara **18 propiedades planas a
  propósito** —sin `oneOf` por operación— porque un `oneOf` mal escrito rechazaría entradas
  válidas. Activar `additionalProperties` en ejecución sin resolver eso primero rompería `create`
  con campos de otra op.
- **Qué decidir**: (a) **ejecutar** lo que el schema declara, resolviendo antes el `oneOf` por
  operación; (b) **dejar de declararlo** — quitar `additionalProperties: false` de los schemas, de
  modo que la superficie deje de afirmar lo que no cumple, a costa de que el cliente ya no valide;
  (c) **declararlo como tolerancia deliberada** y documentarlo en las `instructions` del servidor,
  para que un agente sepa que un parámetro inventado se descarta.
- **Recomendación**: **(a)**, en la misma épica que E24-H07/H08 (v0.4.0), porque las tres tocan la
  misma superficie de entrada y comparten el criterio de fondo: *lo que el motor no entiende, lo
  dice*. Hoy no es un bug de datos —nada se corrompe—, pero sí una respuesta silenciosamente
  equivocada, que es la clase de defecto que esta épica ha estado cerrando.

---

## 16. Deuda declarada por la auditoría de E25/E26 — 🟠 ABIERTA (fuera de esa tanda, por decisión)

> **Qué es esta sección**: lo que la auditoría del camino de escritura y de la superficie de errores
> (2026-07-29) y los **jueces ciegos de las 11 historias** dejaron **explícitamente fuera** de
> E25/E26. No es una lista de bugs pendientes: es lo que se decidió **no** arreglar ahí, con el
> motivo. Cada punto lleva su **origen**, para que la próxima auditoría no lo redescubra como
> hallazgo nuevo — que es exactamente lo que E23 y E24 pagaron dos veces.
>
> **Ninguno bloquea el merge de E25/E26.** Varios (b, c, g) son *capacidad construida sin consumidor*,
> la misma familia que `§14`; otros (a, j) son límites de sintaxis que hoy son **ruidosos, no
> silenciosos**, que es la propiedad que estas épicas vinieron a garantizar.

### (a) *Quoting* en el lenguaje de consulta: tres límites latentes

- **Origen**: **E26-H09** (un solo dialecto de dot-paths). Al unificar `metadata_inspect` con
  `build_field_path` quedaron a la vista tres casos que el dialecto único **no puede expresar**:
  1. una clave de frontmatter que **contiene un punto literal** (`sonar.projectKey`) — direccionable
     con `FieldPath::from_segments` desde Rust, pero no desde la sintaxis textual, que siempre parte
     por puntos;
  2. una clave del usuario llamada literalmente **`frontmatter`** — el prefijo se interpreta como
     anclaje (E24-H08), así que la clave homónima queda tapada;
  3. la **fusión de nombres**: `a.b` como clave literal y `a` → `b` anidado producen el mismo
     `FieldPath`, así que el catálogo no los distingue.
- **Por qué no se cerró**: los tres piden **sintaxis nueva** (comillas o *escaping* en el lenguaje),
  o sea abrir `§20.8`, que es una decisión de diseño con puerta propia — no un apéndice de una épica
  de endurecimiento. Y hoy **no son silenciosos**: (1) y (3) son casos raros y documentados, y (2)
  produce un resultado explicable, no una respuesta equivocada disfrazada de correcta.
- **Qué decidir**: si el lenguaje gana *quoting* (`frontmatter."sonar.projectKey"`) o si estos tres
  casos se declaran **fuera de alcance por escrito** en `§20.8`.
- **Recomendación**: declararlos por escrito ahora y abrir el *quoting* solo si aparece un caso real.
  Sintaxis nueva sin demanda es superficie que hay que mantener para siempre.

### (b) `Envelope`/`ErrorEnvelope` no tienen llamantes

- **Origen**: auditoría de la superficie (UX de errores).
- **Qué es**: el envelope de `lodestar-app` (E10-H01, decisión **D3** de `§0`) existe, compila y está
  testeado, pero **ninguna fachada lo usa**: MCP devuelve `structuredContent` + texto con el código, y
  la CLI sus exit codes. Es capacidad construida sin consumidor, como el store de `§14`.
- **Por qué no se cerró en E25/E26**: E26 trabajó sobre la superficie **real** (código + mensaje en
  las 10 tools). Meter el envelope habría sido cambiar la forma del wire en la misma tanda que
  arreglaba su contenido.
- **Qué decidir**: (a) **cablearlo** como forma única de respuesta de las dos fachadas; (b)
  **retirarlo** como se retiró `lodestar-vcs`; (c) **acotarlo** por escrito a consumidores futuros.
- **Recomendación**: **(b) o (c)**. Tras E26-H07 el wire ya es honesto sin envelope; mantener dos
  formas de respuesta es la clase de duplicación que el invariante #4 existe para evitar.

### (c) La cache SQLite y el watcher siguen sin uso en producción

- **Origen**: auditoría de la superficie; es la **misma deuda de `§14`**, vista desde el otro lado.
- **Qué añade a `§14`**: no solo el store no tiene consumidor — el **watcher** (E3-H04, el «único
  escritor reconcilia» del invariante #5) tampoco corre en el motor headless: sin `enable_cache` no
  hay nada que reconciliar. El invariante #5 se sostiene hoy por el **protocolo de escritura**
  (temp+fsync+rename por el único camino), no por el watcher.
- **Recomendación**: tratarlo **con `§14`**, en la misma decisión. No merece épica propia separada.

### (d) Servidor MCP monohilo, sin *timeout* ni cancelación

- **Origen**: auditoría de la superficie.
- **Qué es**: el bucle JSON-RPC atiende **una petición a la vez** y no hay forma de cancelar ni de
  acotar en el tiempo una llamada larga (`knowledge_check` sobre una base grande, un `change_plan`
  con selección masiva). Un cliente que se impaciente no tiene protocolo para decirlo.
- **Por qué no se cerró**: es **diseño de transporte**, y su decisión natural va con `§3` (rmcp
  oficial), que ya contempla el problema desde fuera.
- **Recomendación**: resolverlo **dentro de `§3`**. Escribir cancelación a mano sobre el stdio propio
  para luego migrar a rmcp sería trabajo tirado.

### (e) La config no rechaza claves desconocidas, y una config ilegible cae a *defaults* en silencio

- **Origen**: auditoría del camino de escritura.
- **Qué es**: `WorkspaceConfig` no lleva `#[serde(deny_unknown_fields)]`, así que un
  `writableRoots` mal escrito (`writable_roots`, `writeableRoots`) se **ignora sin avisar** y el
  workspace queda con la política por defecto — es decir, **más permisivo** de lo que el usuario
  cree. Y un `.lodestar/config.yaml` ilegible degrada a *defaults* en silencio, cuando la CLI ya fija
  el precedente contrario: un `lodestar.toml` inválido era exit 3, no *defaults* (revisión 2026-07).
- **Por qué no se cerró**: es la **misma pregunta que `§15`** —¿rechazar lo que no se declara?— en el
  fichero de config en vez de en el wire, y merece la misma respuesta para no dejar el repo con dos
  criterios opuestos.
- **Recomendación**: **decidirlo junto con `§15`**, y en la dirección estricta: una raíz de escritura
  que el motor no aplica porque el usuario escribió mal la clave es un fallo de seguridad silencioso,
  no una tolerancia amable.

### (f) Un workspace vacío es indistinguible de un directorio equivocado

- **Origen**: auditoría de la superficie.
- **Qué es**: `cd` a un directorio que no es el que se creía (o donde la `DiscoveryPolicy` excluye
  todo) da `workspace_status` con 0 documentos y `lodestar check` **exit 0 · VÁLIDO**. La respuesta es
  literalmente correcta —no hay nada mal— y prácticamente engañosa: es el «respondió que sí a algo que
  no entendió» del principio rector de E26, en la puerta de entrada.
- **Por qué no se cerró**: cambiar el veredicto de `check` sobre un workspace vacío es un **cambio de
  contrato de la puerta de CI** (un repo legítimamente vacío pasaría a fallar), y eso pide decisión,
  no arreglo.
- **Qué decidir**: (a) **avisar** sin cambiar el exit code (un diagnóstico de nivel `warn` «0
  documentos descubiertos bajo esta raíz»); (b) **exit distinto**; (c) dejarlo como está.
- **Recomendación**: **(a)**. Conserva el contrato de exit codes y cierra el engaño.

### (g) API pública de `Workspace` no transaccional (defecto **S8** de la auditoría)

- **Origen**: auditoría del camino de escritura (S8), confirmado por los jueces de E25.
- **Qué es**: `create_document`, `write_document`, `merge_frontmatter` y `publish` son **públicos** y
  escriben el canónico **sin lock, sin journal y sin copias de recuperación** — o sea, esquivan las
  seis garantías que E25 acaba de reforzar. Hoy son inofensivos porque **no tienen llamadores de
  producción** (solo tests), exactamente el mismo caso que `materialize_staging` en E23-H12.
- **Por qué no se cerró en E25**: retirarlos o replegarlos a `pub(crate)` toca la API pública del
  crate y la suite que los usa; hacerlo dentro de una épica de endurecimiento habría mezclado un
  cambio de superficie con seis arreglos de concurrencia.
- **Qué decidir**: (a) **replegar a `pub(crate)`** o marcarlos `#[doc(hidden)]` como primitivas de
  test; (b) **hacerlos transaccionales**; (c) documentarlos como «solo test» y dejarlos.
- **Recomendación**: **(a)**. Una API pública que rompe el invariante nuclear del crate es una trampa
  con fecha de caducidad: funciona hasta que alguien la llama.

### (h) Los escritores de runtime no toman el lock

- **Origen**: **reserva del juez ciego de E25-H03**.
- **Qué es**: `persist_plan` y `write_receipt` escriben bajo `.lodestar/runtime/` **sin** el lock de
  publicación, mientras el barrido de temporales del GC (E24-H06) puede correr **desde otro proceso**.
  La ventana es estrecha y el daño acotado (un plan o un recibo que hay que reescribir, no un `.md`),
  y por eso E25-H03 se limitó a proteger el plano de **recuperación**, que es el que sí sostiene el
  invariante nuclear.
- **Recomendación**: cerrarlo si aparece un caso real, o cuando se toque el GC por otro motivo. Está
  registrado para que no se confunda con un olvido.

### (i) La secuencia de sellado está duplicada entre `apply` y `revert`

- **Origen**: **reserva del juez ciego de E25-H05**.
- **Qué es**: tras E25-H04/H05, publicar y revertir comparten la **misma coreografía** —promover el
  recibo pendiente, limpiar staging, borrar el journal, fsync del directorio— escrita **dos veces**.
  No es duplicación de *lógica de dominio* (invariante #3 no se incumple: la mecánica de recibo sí se
  reusa), pero sí de **secuencia**, que es donde un arreglo futuro se aplicará a una mitad y no a la
  otra. Es la forma exacta del defecto que E25-H05 vino a cerrar.
- **Recomendación**: extraer un `sellar_publicado(txn_id, journal_path)` compartido, en un ciclo
  corto y sin cambio de comportamiento, **con la suite actual como red**. Candidata clara a `/ciclo`.

### (j) Un cursor basura reinicia la paginación en silencio

- **Origen**: **reserva del juez ciego de E26-H10**.
- **Qué es**: `decode_cursor` interpreta un cursor ilegible como **offset 0**, así que un cursor
  corrupto o de otra tool devuelve **la primera página** en vez de un error. Un agente que pagina en
  bucle con un cursor mal propagado no termina nunca y no se entera. Es —en pequeño— el mismo patrón
  que E26-H08 acaba de retirar de la evaluación de consultas.
- **Por qué no se cerró en H10**: H10 tenía que introducir cotas **sin** cambiar el resultado de las
  llamadas correctas (`paginar_no_pierde_ni_duplica`); rechazar cursores inválidos es un caso de error
  **nuevo** en cuatro tools, o sea otro delta de contrato, en una historia que ya llevaba el suyo.
- **Recomendación**: `INVALID_SCHEMA` con mensaje, en el mismo ciclo que cualquier retoque futuro de
  paginación. Es barato y coherente con el principio rector de E26.

### (k) La matriz de trazabilidad no tiene filas de E15–E24

- **Origen**: **observación del cierre de E24-H18**, verificada al cerrar E25/E26.
- **Qué es**: `requirements/trazabilidad.md` se quedó en el giro headless (E9–E14). La migración a
  Markdown universal (E15–E22), el cierre de la PR #17 (E23) y el de la v0.3.0 (E24) **nunca se
  trazaron**, pese a que el alcance de E24-H18 lo declaraba («`requirements/README.md` y
  `requirements/trazabilidad.md` incorporan la épica»): el README **sí** se actualizó, la matriz no.
  Diez épicas sin fila. E25/E26 sí están trazadas, con lo que el hueco queda en medio y a la vista.
- **Por qué no se cerró aquí**: reconstruir la trazabilidad de diez épicas a posteriori es un trabajo
  de documentación con su propio alcance, y hacerlo a la carrera produciría filas plausibles en vez
  de filas verificadas — el defecto que el documento existe para impedir.
- **Recomendación**: una historia propia, con el criterio de que **cada fila se verifique contra la
  épica**, no contra el recuerdo.

### (l) Deuda de fuerza de suite y flecos menores registrados por los jueces ciegos

- **Origen**: las **reservas MENORES** de los veredictos de E25/E26. A diferencia de las mayores
  —que se cerraron en el mismo ciclo— y de (h), (i), (j), que son deuda de diseño, estas son de otra
  clase: **la suite no muerde ahí**. Casi todas salieron de *mutation testing*, no de un fallo
  observado, y se registran juntas porque comparten remedio.
- **Qué es**, por historia y con el mutante que lo destapó:
  - **E25-H01** — mutación **(g)**: el cálculo de `paths_divergentes` y el mensaje del conflicto de
    ventana pueden **vaciarse** sin que ningún test muerda. El aborto sigue ocurriendo (eso sí está
    cubierto), pero el diagnóstico que dice **qué** divergió no lo fija nadie.
  - **E25-H02** — mutación **S**: el sidecar de huellas movido a cuarentena se puede **borrar** sin
    que falle un test, pese a que la cuarentena existe precisamente para no perder material forense.
    Mutación **N**: la **numeración** de cuarentenas repetidas (`.2`, `.3`) tampoco está fijada, así
    que dos irrecuperables del mismo `txnId` podrían pisarse sin que se note.
  - **E25-H03** — mutación **c**: el **no-op silencioso** del GC dentro de `recover_if_pending` no
    tiene arnés. Está **mitigado** por el testigo tipado (`&WorkspaceLock` como prueba de que el lock
    se posee), que hace el error difícil de cometer de nuevo, pero mitigado no es cubierto.
  - **E25-H04** — mutante **k**: el guard `recibo_a_salvo` no tiene test que **inyecte un fallo de
    promoción**, así que la rama que decide qué hacer cuando el recibo no se pudo promover no se
    ejerce. Tiene **espejo** en el sellado del revert de H05.
  - **E25-H05** — dos: el wrapper `Workspace::revert_transaction` quedó **sin llamador ni test**
    propio; y la **re-verificación es única** en el revert mientras el apply comprueba **dos veces**,
    lo que deja declarada una ventana `[paso 2b, primer rename]` más ancha en el revert que en el
    apply. Es estrechamiento posible, no un agujero: la comprobación que importa —bajo el lock— sí
    está, y es la que E25-H05 introdujo.
  - **E26-H09** — **divergencia latente core↔store**: el catálogo publica ahora nombres **anclados**
    (`frontmatter.graph.backlinks`), mientras el store sigue indexando `metadata.field_path` con los
    nombres crudos de `walk`. **Hoy esa columna no la lee nadie**, así que no hay discrepancia
    observable — es la misma situación exacta de (c)/`§14`, y se resolverá con ella.
  - **E26-H10** — la **aritmética de paginación está en 4 copias** y los límites se aplican en **3
    sitios por tool**. Es el vector del mutante **M10**, que se cerró clavando el default con un
    test; la duplicación sigue ahí, y es donde un arreglo futuro se aplicará a unas copias y no a
    otras — la misma forma que (i).
- **Por qué no se cerró**: ninguno es un defecto observable hoy. Cerrarlos uno a uno al vuelo, al
  final de once historias, habría añadido tests escritos para matar un mutante concreto en vez de
  para describir comportamiento — que es como se acumulan suites grandes y flojas. Y dos de ellos
  (H09, H10) no se arreglan con un test sino con el refactor que ya recomiendan (c) e (i).
- **Recomendación**: **una pasada de `/mutantes` acotada** a los ficheros que E25/E26 tocaron, con
  presupuesto cerrado, que convierta en test los supervivientes que describan comportamiento real
  —(g), S, N, k son los candidatos claros—; y, cuando se toque ese código por otro motivo, los dos
  refactores compartidos que ya están recomendados: **`sellar_publicado`** (i) y un **helper único de
  paginación** para la aritmética y los límites. La divergencia core↔store de H09 **no se toca aquí**:
  va con la decisión de `§14`.

---

### Resumen de la recomendación

Los puntos **1** (build de Tauri) y **2** (port de la UI) están **implementados**: la app de escritorio
compila, corre y es funcional de extremo a extremo. Lo que queda son decisiones de **producto/pulido**,
no de arquitectura: firma/notarización + updater (1) —el empaquetado y las plataformas ya salen en
`release.yml` (v0.1.0, sin firmar)—, pulido visual (2), y los puntos **3–9** (rmcp,
`.d.ts` generado, i18n, semántica de merge/`--range`, esquema de `lodestar.toml`, benches/threat model),
que solo necesitan tu criterio o pueden esperar sin deuda. El punto **10** (ghosts como primitiva
de planificación + templates) es la **siguiente feature acordada**, pendiente de diseño.
