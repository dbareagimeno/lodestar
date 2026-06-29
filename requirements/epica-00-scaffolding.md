# E0 — Scaffolding del workspace

> **Fase**: previa a `§14.1`. **Objetivo de la épica**: levantar el Cargo workspace, el proyecto
> frontend, la generación de tipos y la infraestructura de CI/fixtures **sin lógica OKF todavía**.
> Al cerrar E0 existen `Cargo.toml`, `package.json`, `crates/`, `src-tauri/` y los comandos
> canónicos de `CLAUDE.md` arrancan (aunque hagan poco). Referencia: `ARCHITECTURE.md §3`.

**Estado de partida**: greenfield. Hoy solo existen `ARCHITECTURE.md`, `CLAUDE.md` y
`prototype/index.html`. No hay nada que compile.

---

### E0-H01 — Cargo workspace con los 7 crates y direcciones de dependencia
- **Objetivo**: crear el `Cargo.toml` raíz y los miembros vacíos con el grafo de dependencias del `§3`.
- **Referencias**: `ARCHITECTURE.md §3` (mapa de crates) · `CLAUDE.md` (grafo de crates).
- **Alcance**:
  - `Cargo.toml` raíz con `[workspace]`, `resolver = "2"`, `members = [crates/lodestar-core,
    crates/lodestar-store, crates/lodestar-vcs, crates/lodestar-workspace, crates/lodestar-cli,
    crates/lodestar-mcp, src-tauri]`.
  - `[workspace.package]` compartido (version, edition `2021`/`2024`, license, repository).
  - `[workspace.dependencies]` para versionar una sola vez `serde`, `serde_yaml`, `blake3`,
    `thiserror`, etc. (los crates referencian `workspace = true`).
  - Cada crate con su `Cargo.toml` y un `lib.rs`/`main.rs` mínimo (`pub fn placeholder()` o `fn main(){}`).
  - **Direcciones de dependencia exactas del `§3`**: `store`/`vcs` dependen de `core`; `workspace`
    depende de `core`+`store`+`vcs`; las 3 fachadas dependen de `workspace` (NO de `store`).
- **Fuera de alcance**: cualquier lógica OKF, tipos del contrato, DDL, comandos reales.
- **Criterios de aceptación**:
  - `cargo build` compila el workspace entero.
  - `cargo tree` muestra que ninguna fachada depende directamente de `lodestar-store`/`lodestar-vcs`.
  - `lodestar-core` NO declara `tauri`/`rusqlite`/`notify`/`tokio`/`git2` en su `Cargo.toml`.
- **Dependencias**: —
- **Pruebas**: `cargo build` + un `cargo tree -i lodestar-store` que liste solo `workspace` como dependiente.

### E0-H02 — Lints, toolchain y `#![forbid(unsafe_code)]` en el core
- **Objetivo**: congelar la política de lints y la pureza del core a nivel de compilación.
- **Referencias**: `ARCHITECTURE.md §3` (core lleva `#![forbid(unsafe_code)]`) · `§2.2`.
- **Alcance**:
  - `rust-toolchain.toml` fijando canal estable y componentes (`rustfmt`, `clippy`).
  - `[workspace.lints]` con `clippy::all`, `rust.unsafe_code = "forbid"` heredado donde aplique;
    `lodestar-core/src/lib.rs` con `#![forbid(unsafe_code)]` explícito.
  - `rustfmt.toml` con config compartida.
  - Features del core declaradas pero vacías: `schemars` (gated) y `render` (pulldown-cmark), apagadas por defecto.
- **Criterios de aceptación**:
  - `cargo clippy --workspace -- -D warnings` pasa.
  - Introducir un `unsafe {}` en el core **rompe** la compilación.
  - `cargo build -p lodestar-core --features schemars,render` compila (aunque las features no hagan nada aún).
- **Dependencias**: E0-H01.
- **Pruebas**: CI job que ejecuta clippy con `-D warnings`.

### E0-H03 — Crate de fixtures compartida
- **Objetivo**: una crate `lodestar-fixtures` (dev-dependency) con bundles de ejemplo reusables por todos los tests.
- **Referencias**: `ARCHITECTURE.md §12` (Testing/paridad: "Crate de fixtures") · `§5`, `§13.4`.
- **Alcance**:
  - Crate `crates/lodestar-fixtures` (o `tests/fixtures` con loader) que exponga `FileMap`s y
    directorios de `.md` para: bundle conforme mínimo, bundle con cada `CheckCode` disparado,
    bundle con huérfanos/dangling/index/log, bundle con marcadores de conflicto, y una fixture
    **sintética de 10k concepts** generable (para los benches del `§11`).
  - Loader que lee un directorio fixture a `FileMap` (reusa `RelPath` cuando exista; antes, `String`).
- **Fuera de alcance**: los tests que las consumen (viven en cada crate).
- **Criterios de aceptación**:
  - Existe al menos una fixture por cada uno de los 15 `CheckCode`.
  - Un generador parametrizable produce N concepts deterministas (semilla fija → bytes idénticos).
- **Dependencias**: E0-H01.
- **Pruebas**: test que carga cada fixture y asserta su forma básica (nº de ficheros).

### E0-H04 — Pipeline de generación de tipos Rust → TypeScript
- **Objetivo**: cablear ts-rs/specta para que el `.d.ts` se genere desde `lodestar-core::types`.
- **Referencias**: `ARCHITECTURE.md §2.4`, `§4.1`, `§8` (contrato IPC), `§10` fila 6/7.
- **Alcance**:
  - Elegir **ts-rs** o **specta** (decisión: ts-rs por madurez; documentarla) y añadir el derive
    detrás de una feature `ts` en el core.
  - Un test/comando `cargo test --features ts export_bindings` (o `xtask gen-ts`) que emite
    `frontend/src/lib/ipc/types.gen.ts`.
  - El fichero generado se marca como **artefacto generado** (header "NO EDITAR") y se versiona.
  - Un check de CI que regenera y falla si hay drift (mismo patrón que el exit code 4 de generadores OKF).
- **Fuera de alcance**: el `ipc.ts` que envuelve los comandos (E6).
- **Criterios de aceptación**:
  - Cambiar un campo de un tipo del contrato y regenerar produce un `.d.ts` distinto.
  - El job de CI "ts drift" falla si el `.gen.ts` versionado no coincide con el recién generado.
- **Dependencias**: E0-H01. (Los tipos reales llegan en E1; aquí solo el pipeline con un tipo de prueba.)
- **Pruebas**: CI "ts drift" + snapshot del `.gen.ts` de un tipo de ejemplo.

### E0-H05 — Frontend Svelte 5 + Vite scaffolding
- **Objetivo**: proyecto frontend que arranca en dev y produce un build estático para Tauri.
- **Referencias**: `ARCHITECTURE.md §1`, `§8`.
- **Alcance**:
  - `frontend/` con `package.json`, Svelte 5 + Vite + TypeScript, `svelte-check`, `vitest`.
  - Estructura `src/lib/` (stores, ipc, componentes) y `src/routes`/`App.svelte` placeholder.
  - Copiar el `<style>` y las variables CSS del prototipo a `src/app.css` (base para el port verbatim de E6).
  - Scripts: `npm run dev`, `npm run build`, `npm run check`, `npm run test`.
- **Fuera de alcance**: portar la UI (E6).
- **Criterios de aceptación**:
  - `npm run build` produce `dist/` consumible por Tauri.
  - `npm run check` (svelte-check + tsc) pasa.
  - Las variables CSS del prototipo (`--*`, `data-theme`) están disponibles globalmente.
- **Dependencias**: —
- **Pruebas**: `npm run build` + `npm run check` en CI.

### E0-H06 — Tauri v2 skeleton (`src-tauri`)
- **Objetivo**: app Tauri v2 que abre una ventana con el frontend y sin permisos peligrosos.
- **Referencias**: `ARCHITECTURE.md §1`, `§7.1`.
- **Alcance**:
  - `src-tauri/` con `tauri.conf.json`, `Cargo.toml`, `main.rs` que levanta la webview sobre `frontend/dist`.
  - **Allowlist mínima**: la webview NO recibe permisos `fs`/`shell`/`dialog` (el `§7.1` lo exige).
  - Un comando Tauri `ping` de humo que devuelve un string (placeholder para validar el IPC).
- **Fuera de alcance**: la tabla de comandos real (E6).
- **Criterios de aceptación**:
  - `cargo tauri dev` (o `npm run tauri dev`) abre la ventana con el frontend.
  - El `tauri.conf.json` no concede capacidades `fs`/`shell`/`dialog` a la webview.
- **Dependencias**: E0-H01, E0-H05.
- **Pruebas**: build de `cargo tauri build` en CI (al menos en una plataforma).

### E0-H07 — CI base (fmt, clippy, test, frontend)
- **Objetivo**: un pipeline de CI reproducible que corra los gates de cada lenguaje.
- **Referencias**: `ARCHITECTURE.md §12` (packaging/testing) · `CLAUDE.md` (comandos canónicos).
- **Alcance**:
  - Workflow de CI (GitHub Actions) con jobs: `rustfmt --check`, `clippy -D warnings`,
    `cargo test --workspace`, `frontend check+test`, `ts drift` (E0-H04).
  - Cache de cargo/npm; matriz mínima (linux; macos/windows para el job de Tauri build).
  - Un job `bench` placeholder (se llena en E8 con la fixture de 10k).
- **Fuera de alcance**: release/packaging/firma (E8).
- **Criterios de aceptación**:
  - El CI pasa en verde sobre el workspace scaffoldeado.
  - Un `cargo fmt` mal aplicado o un warning de clippy rompen el CI.
- **Dependencias**: E0-H01..E0-H06.
- **Pruebas**: el propio run de CI.

### E0-H08 — `xtask` / automatización de comandos repetibles
- **Objetivo**: un crate/bin `xtask` (o `justfile`) que centralice generar TS, correr el arnés diferencial y los benches.
- **Referencias**: `ARCHITECTURE.md §12` · `CLAUDE.md` (comandos planificados).
- **Alcance**:
  - `xtask gen-ts`, `xtask diff-harness` (placeholder hasta E1), `xtask bench` (placeholder hasta E8).
  - Documentar en `requirements/README.md`/`CONTRIBUTING` los comandos canónicos reales una vez existen.
- **Criterios de aceptación**: `cargo xtask --help` lista los subcomandos; `gen-ts` funciona end-to-end.
- **Dependencias**: E0-H01, E0-H04.
- **Pruebas**: invocar cada subcomando en CI (los placeholder devuelven 0 con aviso "pendiente fase N").
