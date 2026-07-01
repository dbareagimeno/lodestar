# Decisiones pendientes (requieren tu criterio)

> Este documento recoge las decisiones que **no se pueden tomar por inercia** desde el código o
> `ARCHITECTURE.md` y que dependen de tu criterio de producto/entorno. Cada una lleva el estado
> actual, el porqué de que quede abierta y una **recomendación**. Nada aquí bloquea lo ya
> implementado (backend completo y testeado); son decisiones para cerrar el último tramo (sobre todo
> E6 desktop) y para afinar comportamiento.

---

## 1. Build y empaquetado de la fachada de escritorio Tauri (E6)

- **Estado**: `src-tauri` es un binario placeholder que compone `core` + `workspace`. **No** declara
  las dependencias de `tauri`/`tauri-build`.
- **Por qué está abierta**: añadir `tauri` obliga a librerías de sistema (en Linux:
  `webkit2gtk-4.1`, `libsoup-3`, `libjavascriptcoregtk`…) que **no están** en este entorno de CI/
  ejecución. Meterlas ahora **rompería `cargo build`** para todo el mundo que no las tenga.
- **Qué decidir**:
  1. ¿Aislamos la fachada Tauri en su propio flujo de build (feature/CI dedicada de escritorio) para
     que el workspace siga compilando en CI sin libs de sistema? **(Recomendado.)**
  2. Plataformas objetivo (macOS/Windows/Linux) y estrategia de **updater** + **firma/notarización**
     (§12 packaging, E8-H06).
- **Recomendación**: sí, aislar Tauri detrás de una CI de escritorio separada. El resto del workspace
  (core/store/vcs/workspace/cli/mcp) se mantiene como la puerta de CI portable.

## 2. Alcance del port verbatim de la UI del prototipo (E6)

- **Estado**: el frontend Svelte ya consume el `BundleSnapshot` empujado: árbol filtrable, selección,
  píldora de conformidad y **panel de diagnósticos localizado** (i18n keyed por código). CSS/variables
  portadas del prototipo. Compila (`npm run build`) y pasa `svelte-check` (0 errores).
- **Por qué está abierta**: el prototipo son ~2900 líneas (rails redimensionables, tabs, editor
  multi-escritor, **isla imperativa del grafo** `createStarMap` con loop rAF, overlay/modo «Cambios»).
  Portarlo verbatim es un esfuerzo grande y **no verificable de extremo a extremo sin la fachada Tauri**
  (punto 1).
- **Qué decidir**: ¿port incremental (vista a vista, empezando por editor y grafo) o un port completo
  de una vez antes de cablear Tauri?
- **Recomendación**: incremental, priorizando editor + isla del grafo, una vez decidido el punto 1.

## 3. Transporte MCP: stdio propio vs `rmcp` oficial (E7)

- **Estado**: el MCP funciona como servidor **JSON-RPC por stdio** (stdout puro), con 13 tools y
  test golden cross-fachada (salida de cada tool == `Workspace` directo). Falta el transporte oficial
  `rmcp` + `resources` + `outputSchema` (feature `schemars` ya preparada en el core).
- **Qué decidir**: ¿adoptamos `rmcp` ahora (transporte oficial, resources, negociación de capacidades)
  o mantenemos el stdio propio hasta tener un consumidor que exija `rmcp`?
- **Recomendación**: mantener stdio hasta tener un cliente MCP real que lo requiera; el contrato de
  tools ya está congelado, migrar el transporte después es mecánico.

## 4. Generación del `.d.ts` desde Rust (ts-rs/specta) — E0-H04/E6-H03

- **Estado**: `frontend/src/lib/ipc/types.ts` es un **espejo a mano** del contrato de `core::types`,
  marcado como «a generar». Los nombres/orden coinciden con Rust.
- **Por qué está abierta**: cablear `ts-rs`/`specta` añade dependencias y un paso de build; es la forma
  de **hacer cumplir** el invariante «un solo contrato de tipos» (principio #4) y matar la deriva.
- **Qué decidir**: ¿enganchamos el generador ahora (recomendado antes de crecer la UI) o seguimos con
  el espejo manual mientras la superficie es pequeña?
- **Recomendación**: engancharlo antes de portar más UI (punto 2), para que el `.d.ts` sea derivado.

## 5. i18n multi-idioma

- **Estado**: la app es **español-only** en v1 (decisión ya tomada en `CLAUDE.md`). El catálogo de
  conformidad está **keyed por `CheckCode`** (`frontend/src/lib/i18n.ts`) y el core emite `code`+
  `targets`, así que añadir un locale = añadir un objeto con las mismas claves.
- **Qué decidir**: ¿hay que soportar inglés u otro idioma en v1? Si no, esto queda cerrado.
- **Recomendación**: mantener español-only en v1; la arquitectura ya no lo impide en el futuro.

## 6. Semántica de `merge` local

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
- **Packaging/release CI + updater + firma** (ligado al punto 1).
- **Threat model** documentado (§12 seguridad); las piezas ya están (RelPath anti path/zip-slip,
  FTS5 escapado, git de red confinado al binario, libgit2 local sin hooks).
- **Arnés diferencial JS-vs-Rust (E1-H18)**: hoy los tests fijan la semántica del prototipo
  directamente en Rust; ejecutar el JS del prototipo en Node como oráculo es un extra de confianza.

---

### Resumen de la recomendación

Cerrar el punto **1** (aislar el build de Tauri) desbloquea el punto **2** (crecer la UI) y el **4**
(generar el `.d.ts`). Los puntos **6**, **7** y **8** solo necesitan tu «sí» para darlos por cerrados
con el comportamiento actual. El resto (3, 5, 9) puede esperar sin deuda arquitectónica.
