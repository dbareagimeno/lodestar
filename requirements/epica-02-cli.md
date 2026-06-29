# E2 — `lodestar-cli` mínima

> **Fase**: `§14.2`. **Objetivo de la épica**: una CLI `clap` sobre el **core efímero** (sin store,
> sin watcher) que ya sirve como **puerta de CI** (`lodestar check`). Cada subcomando es un shell de
> 5–15 líneas: resuelve root → llama un método → serializa. **Cero lógica OKF en la fachada.**
> Referencias: `ARCHITECTURE.md §7.3`, `§13.5`, `§13.7`. Los exit codes están **congelados**.

**Nota de fases**: en E2 la CLI corre sobre `core` directamente (lee ficheros a un `FileMap` y construye
`Bundle`), porque `store`/`vcs`/`workspace` aún no existen. Los subcomandos git (`log`/`diff`/`branch`/
`merge`/`pull`/`push`/`hooks install`) y `check --staged/--rev/--range` se **stubbean** aquí y se
completan en E4 (vcs). El `--check` de drift de generadores se completa cuando exista la workspace (E5),
pero su exit code (4) se reserva ya.

---

### E2-H01 — Esqueleto `clap` + exit codes congelados + lector de bundle efímero
- **Objetivo**: el binario `lodestar` con el árbol de subcomandos y los exit codes del `§7.3`.
- **Referencias**: `ARCHITECTURE.md §7.3` (exit codes), `§6` (`open_ephemeral`) · `CLAUDE.md` (exit codes congelados).
- **Alcance**:
  - `clap` (derive) con subcomandos: `init`, `check`, `index`, `tags`, `export`, `reindex`, `import`
    (+ placeholders de los git de §13.7).
  - **Exit codes congelados**: `0` conforme · `1` hard-fail · `2` uso · `3` runtime/IO · `4` drift de generadores.
    Un tipo `ExitCode` central que mapea `CoreError`/resultados a estos códigos (`§12` errores).
  - Lector de bundle efímero: `ignore::WalkBuilder` → `core::parse_file` → `FileMap` → `Bundle::from_files`
    (equivalente a `Workspace::open_ephemeral` hasta que exista la workspace).
  - Resolución de root: arg `--path` o cwd; descubrir el root del bundle (marca `index.md` raíz / `.lodestar`).
- **Criterios de aceptación**:
  - `lodestar --help` lista todos los subcomandos.
  - Un error de uso devuelve **exactamente** exit code 2; un error de IO, 3.
  - El walker respeta `.gitignore` y excluye `.lodestar/` y `.git/`.
- **Dependencias**: E1-H08 (Bundle), E1-H02 (CoreError), E0-H01.
- **Pruebas**: tests de exit code por rama; smoke sobre fixture.

### E2-H02 — `lodestar check` (la puerta de CI) con salida humana / `--json` / SARIF
- **Objetivo**: el gate que decide conformidad y devuelve 0/1, con tres formatos de salida.
- **Referencias**: `ARCHITECTURE.md §7.3`, `§13.5`, `§12` (config: strictness de `lodestar.toml`).
- **Alcance**:
  - `lodestar check` corre `analyze` y devuelve `0` si conforme, `1` si hard-fail.
  - La **strictness** (¿warns bloquean?) se lee de `lodestar.toml` (`§12`); por defecto solo Err bloquea.
  - Formatos: humano (por defecto), `--json` (el `Analysis`/lista de `Check` serializado camelCase),
    `--sarif` (para integraciones de CI).
  - `check` **reconcilia o corre efímero antes de leer** (aquí: siempre efímero hasta E5) para que una
    cache obsoleta nunca deje pasar el gate.
- **Fuera de alcance**: `--staged/--rev/--range` (E4); el `--json` debe ser idéntico al `structuredContent` del MCP (golden, E7).
- **Criterios de aceptación**:
  - Bundle conforme → exit 0; bundle con un `OKF-TYPE` Err → exit 1.
  - `--json` produce el `Analysis` con nombres camelCase del contrato.
  - `--sarif` valida contra el schema SARIF 2.1.0.
  - Con `strictness=warn` en `lodestar.toml`, un solo `Warn` hace exit 1.
- **Dependencias**: E2-H01, E1-H07, E8-H10 (lectura de `lodestar.toml`; si no está, usar default y dejar TODO).
- **Pruebas**: matriz conforme/hard-fail/strict; validación SARIF; golden `--json`.

### E2-H03 — `lodestar index` y `lodestar tags` (generadores, con `--check` de drift)
- **Objetivo**: aplicar los generadores puros del core y soportar el modo `--check` (exit 4).
- **Referencias**: `ARCHITECTURE.md §4.2`, `§7.3`, `§10` fila 12 · prototipo `genIndex`/`generateTagIndex`.
- **Alcance**:
  - `lodestar index [dir]` aplica `gen_index`; `lodestar tags` aplica `gen_tag_indexes` (escribe los `.md`).
  - Modo `--check`: NO escribe; diffea la `Mutation` contra disco y devuelve **exit 4** si hay drift
    (artefacto generado desactualizado en CI), 0 si está al día.
  - En E2 la escritura es directa (no hay workspace/único-escritor todavía); documentar que en E5 pasará por la workspace.
- **Criterios de aceptación**:
  - `index --check` sobre un bundle con `index.md` desactualizado → exit 4.
  - Tras `lodestar index`, `index --check` → exit 0.
  - Los bytes generados son deterministas (cabeceras canónicas, `§12`).
- **Dependencias**: E2-H01, E1-H14.
- **Pruebas**: drift→4, al-día→0; idempotencia.

### E2-H04 — `lodestar export`
- **Objetivo**: empaquetar el bundle a un `.zip` vía `core::export_zip`.
- **Referencias**: `ARCHITECTURE.md §4.2`, `§7.3`.
- **Alcance**: `lodestar export [--out file.zip]` que escribe el zip; sin zip-slip (heredado de E1-H16).
- **Criterios de aceptación**: el zip descomprime al árbol del bundle; exit 3 si el destino no es escribible.
- **Dependencias**: E2-H01, E1-H16.
- **Pruebas**: round-trip; error de IO → exit 3.

### E2-H05 — `lodestar init` (first-run desde CLI)
- **Objetivo**: scaffold de un bundle nuevo (index raíz + `.gitignore` + `git init` + commit inicial).
- **Referencias**: `ARCHITECTURE.md §12` (first-run), `§7.3`.
- **Alcance**:
  - Crea `index.md` raíz con `okf_version`, `.gitignore` (incluye `.lodestar/`).
  - `git init` + commit inicial. **Hasta E4** el `git init`/commit se puede stubbear o delegar a un helper
    mínimo; la historia se **completa** cuando exista `vcs::init` (E4-H02). Dejar TODO explícito y test marcado `#[ignore]` hasta entonces.
- **Criterios de aceptación**:
  - `lodestar init dir/` crea `dir/index.md` conforme y `.gitignore` con `.lodestar/`.
  - Tras el cierre de E4, `init` deja un repo git con un commit inicial.
- **Dependencias**: E2-H01, E1-H13; (completar con E4-H02).
- **Pruebas**: estructura creada; (post-E4) repo con 1 commit.

### E2-H06 — `lodestar reindex` y `lodestar import` (stubs con contrato)
- **Objetivo**: reservar los subcomandos `reindex` (reconstruir cache) e `import` (migración) con su CLI.
- **Referencias**: `ARCHITECTURE.md §7.3`, `§12` (migración).
- **Alcance**:
  - `reindex`: en E2 es no-op informativo (no hay cache aún); se implementa de verdad en E5 (workspace) sobre `store`.
  - `import`: define los args (origen del `localStorage` exportado del prototipo); la lógica real (materializar
    `STORE_KEY` a `.md` + replay de `versions[]` a commits) es **E8-H02**. Aquí solo el parsing de args + stub.
- **Criterios de aceptación**: ambos subcomandos existen, validan args y devuelven un mensaje "pendiente fase N" con exit 0; tests marcados para completar.
- **Dependencias**: E2-H01.
- **Pruebas**: `--help` de cada uno; stub devuelve mensaje claro.

### E2-H07 — Placeholders de subcomandos git de la CLI
- **Objetivo**: registrar `log`/`diff`/`last-conforming`/`branch`/`merge`/`pull`/`push`/`hooks install` en el árbol clap.
- **Referencias**: `ARCHITECTURE.md §7.3`, `§13.7`.
- **Alcance**: declarar los subcomandos y sus args **sin** implementarlos (devuelven "pendiente E4"); reservar
  `check --staged/--rev/--range`. Se completan en E4-H10.
- **Criterios de aceptación**: el árbol de ayuda muestra todos los subcomandos git con sus flags; ninguno crashea.
- **Dependencias**: E2-H01.
- **Pruebas**: `--help` recursivo no falla.
