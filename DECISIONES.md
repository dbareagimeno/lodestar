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

## 6. Semántica de `merge` local

> **Superada por §0/§19 (2026-07-22)**: git sale de la superficie de producto; el crate `vcs` (con su
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

## 7. `lodestar check --range a..b`

- **Estado**: `--range` juzga **la punta** del rango (equivale a `--rev b`).
- **Qué decidir**: ¿basta con la punta o quieres verificar que **cada commit** del rango es conforme
  (útil para bisect/PR gates)? Lo segundo es más caro pero más estricto.
- **Recomendación**: dejar la punta por defecto y añadir `--each` si en algún momento hace falta el
  barrido por-commit.

## 8. Esquema de `lodestar.toml`

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
- ~~Arnés diferencial JS-vs-Rust (E1-H18)~~ — **hecho**: `prototype/harness/` ejecuta las funciones
  puras del prototipo en node como oráculo y `tests/differential.rs` compara con el core (6 fixtures);
  cazó y cerró 6 divergencias de paridad.

## 10. Ghosts como primitiva de planificación + templates (siguiente feature, no iniciada)

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

---

### Resumen de la recomendación

Los puntos **1** (build de Tauri) y **2** (port de la UI) están **implementados**: la app de escritorio
compila, corre y es funcional de extremo a extremo. Lo que queda son decisiones de **producto/pulido**,
no de arquitectura: firma/notarización + updater (1) —el empaquetado y las plataformas ya salen en
`release.yml` (v0.1.0, sin firmar)—, pulido visual (2), y los puntos **3–9** (rmcp,
`.d.ts` generado, i18n, semántica de merge/`--range`, esquema de `lodestar.toml`, benches/threat model),
que solo necesitan tu criterio o pueden esperar sin deuda. El punto **10** (ghosts como primitiva
de planificación + templates) es la **siguiente feature acordada**, pendiente de diseño.
