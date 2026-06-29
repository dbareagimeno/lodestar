# Requisitos de implementación de **lodestar**

> Este directorio descompone el contrato ratificado de [`ARCHITECTURE.md`](../ARCHITECTURE.md)
> en **épicas** e **historias** lo bastante granulares como para que un agente las implemente
> de una en una. **No reabre decisiones de diseño**: las traduce a unidades de trabajo
> verificables. Si una historia parece contradecir `ARCHITECTURE.md`, gana `ARCHITECTURE.md`
> (y se corrige la historia, no el diseño).

## Cómo leer estos documentos

- **`ARCHITECTURE.md` es la autoridad.** Cada historia cita la sección (`§N`) y las funciones del
  prototipo (`prototype/index.html`) que porta. Lee siempre la sección citada antes de implementar.
- **El prototipo es la spec de comportamiento.** Portar = encontrar la función original y mantener
  su semántica *incluidos sus quirks*. El arnés diferencial JS-vs-Rust (E1) es la red de seguridad.
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

**Orden de construcción**: estrictamente E0 → E1 → E2 → E3 → E4 → E5 → E6 → E7, con E8 entrelazada
(sus historias declaran de qué fase dependen). Cada fase se valida con el arnés de paridad antes de la
siguiente (`§14`). Una historia **no se puede empezar** hasta que sus dependencias (`Dependencias:`)
estén `Done`.

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
