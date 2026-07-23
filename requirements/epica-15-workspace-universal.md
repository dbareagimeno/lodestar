# E15 — Workspace universal (migración a Markdown genérico)

> **Fase**: `§20.14` PRs 0 (retirada) + 1 (`REFACTOR_PHASE_2 §Orden de implementación`).
> **Objetivo de la épica**: dejar el repo **sin las capacidades OKF que no tienen sustituto** y hacer
> que Lodestar arranque desde **cualquier directorio** descubriendo recursivamente sus `.md`. Al
> cerrar E15, `cd my-project && lodestar-mcp` funciona sobre un proyecto que nunca ha visto Lodestar:
> sin `init`, sin `.lodestar/`, sin `index.md`, sin frontmatter.
> Referencias maestras: `ARCHITECTURE.md §20` (entero), `§20.1`, `§20.5`, `§20.13`; `CLAUDE.md`
> invariantes #1/#2/#5/#6.

**Principio rector de la épica** — *y su diferencia con E9*: aquí sí **se borra**. E9 retiró
exposición conservando mecánica; E15 elimina capacidad que el producto nuevo no tiene (git,
generadores de índices, formato zip, el prototipo como oráculo). Ante la duda "¿lo dejo dormido?": no.
El documento es explícito — *"no mantener un modo OKF permanente en runtime"*.

**Estado de la puerta de diseño**: `ARCHITECTURE.md §20` **ya está escrita** (ratificada 2026-07-23);
esta épica **no** la redacta, la **implementa**.

**Nota de compilabilidad**: E15 no toca el modelo documental (eso es E16). `Frontmatter` con sus 7
campos tipados, `FileKind` y los códigos `OKF-*` **siguen vivos** al cerrar E15 — lo que se retira aquí
son subsistemas enteros con sus consumidores, no el modelo. Cada historia deja el workspace compilando
y la suite en verde.

---

### E15-H01 — Borrar el crate `lodestar-vcs` y su cableado

- **Objetivo**: git desaparece del repo, no solo de la superficie. `cargo tree` no muestra `git2`.
- **Referencias**: `ARCHITECTURE.md §20.13`, `§10` (nota §20: filas #15–#21 retiradas), `§13`
  (histórico) · `crates/lodestar-vcs/` · `crates/lodestar-workspace/src/lib.rs:23,72,218,582-620`.
- **Alcance**:
  - Borrar `crates/lodestar-vcs/` completo y su entrada en `Cargo.toml` (`members` +
    `workspace.dependencies.lodestar-vcs` + `git2`).
  - Quitar de `Workspace`: el campo `vcs`, `Vcs::discover` en `open`, `Vcs::init`, `set_identity`, el
    campo `identity` y los métodos `vcs_log`/`branches`/`conformance`/`conformance_of`/`merge`/
    `push`/`pull`/`checkpoint` y cuantos deleguen en el crate; `From<VcsError>` en `error.rs:84`.
  - Retirar de `core::types` los tipos git sin más consumidores: `Sha`, `Author`, `CommitRow`,
    `CommitConformance`, `RepoState`, `Branch`, `SyncKind`, `SyncOutcome`; y `CoreError::InvalidSha`.
  - Retirar del store la tabla `commit_conformance` (DDL, probes, `drop_schema`) y sus accesores
    (`crates/lodestar-store/src/lib.rs:318-350`); **bump de `USER_VERSION`** (`schema.rs:9`).
  - Retirar `identity` de `Config`/`WorkspaceConfig` (`crates/lodestar-workspace/src/config.rs`).
  - **Conservar** `crates/lodestar-workspace/src/gitignore.rs`: gestiona el `.gitignore` del proyecto
    como texto plano, sin `git2`, y sigue siendo necesario para que la cache no se versione.
- **Fuera de alcance**: `core::diff` (`OkfDiff`/`diff_snap`) — no es git, alimenta el `SemanticDiff`
  del motor transaccional; se renombra en E21, no se borra.
- **Criterios de aceptación**:
  - **Dado** el workspace completo, **Cuando** se corre `cargo tree --workspace`, **Entonces** no
    aparece `git2` ni `lodestar-vcs` → checklist de CI.
  - **Dado** un directorio que **no** es un repo git, **Cuando** se abre con `Workspace::open`,
    **Entonces** abre sin error y sin rama de descubrimiento de repo → `abre_sin_repo_git`.
  - **Dado** una cache `.lodestar/index.db` escrita por v0.2 (con `commit_conformance`), **Cuando** se
    abre con el código nuevo, **Entonces** se detecta `user_version` distinta y se reconstruye limpia
    → `cache_v2_se_reconstruye`.
- **Dependencias**: —.
- **Pruebas**: `crates/lodestar-workspace/tests/workspace.rs`: `abre_sin_repo_git`; borrar los ~10
  tests que llaman `vcs_log`/`set_identity` y `crates/lodestar-vcs/tests/`.
  `crates/lodestar-store/tests/store.rs`: `cache_v2_se_reconstruye`.
- **Frontera (mcp.yml)**: no (git ya no estaba en la superficie desde E9).

### E15-H02 — Borrar los generadores de índices (`core::generate`)

- **Objetivo**: Lodestar deja de generar y de exigir índices. Ningún fichero tiene semántica de catálogo.
- **Referencias**: `ARCHITECTURE.md §20.4`, `§20.13` · `REFACTOR_PHASE_2 §Fase 8` ("eliminar:
  `FileKind::Index`, `in_index`, semántica especial de ficheros generados") ·
  `crates/lodestar-core/src/generate.rs` · `crates/lodestar-core/src/bundle.rs:418-428`.
- **Alcance**:
  - Borrar `crates/lodestar-core/src/generate.rs`, su `pub mod` en `lib.rs`, `Bundle::gen_index` y
    `Bundle::gen_tag_indexes`.
  - Borrar los subcomandos `index` y `tags` de la CLI (`main.rs`, `commands.rs`) y el **exit code 4**
    (drift de generadores) de la tabla de códigos congelados: sin generadores no hay drift.
  - Retirar la **auto-regeneración de `index`/`tags` dentro de `change_apply`** (E13-H11,
    `crates/lodestar-app/src/lib.rs`) y sus tests (`crates/lodestar-app/tests/regen.rs`).
- **Fuera de alcance**: los checks `OKF-IDX`/`OKF-LOG` y `FileKind` (mueren en E16, con el modelo);
  el tipo `Mutation` (lo usa el motor transaccional).
- **Criterios de aceptación**:
  - **Dado** `lodestar --help`, **Cuando** se imprime, **Entonces** no aparecen `index` ni `tags`
    → `help_sin_generadores`.
  - **Dado** `lodestar index`, **Cuando** se ejecuta, **Entonces** exit code `2` (uso: subcomando
    retirado) → `index_es_uso`.
  - **Dado** un workspace con `index.md` desactualizado, **Cuando** se aplica un `change_plan` que
    crea un documento, **Entonces** el receipt lista **solo** el documento creado — ningún índice
    regenerado → `apply_no_regenera_indices`.
- **Dependencias**: —.
- **Pruebas**: `crates/lodestar-cli/tests/cli.rs`: `help_sin_generadores`, `index_es_uso`.
  `crates/lodestar-app/tests/`: `apply_no_regenera_indices` (sustituye a `regen.rs`).
- **Frontera (mcp.yml)**: no (`generate_*` salió de la superficie en E14-H06).

### E15-H03 — Borrar `init`, `export` e `import` de la CLI

- **Objetivo**: la CLI queda en `check` + `reindex`. No hay ceremonia de creación ni formato propio
  de intercambio.
- **Referencias**: `ARCHITECTURE.md §20.1` ("no es obligatorio `lodestar init`"), `§20.13` ·
  `crates/lodestar-cli/src/main.rs:29-64`, `commands.rs`, `bundle_io.rs`.
- **Alcance**:
  - Quitar los subcomandos `init`, `export`, `import` del enum de clap y del dispatch.
  - Borrar `Bundle::export_zip` (`crates/lodestar-core/src/bundle.rs:430-444`), `CoreError::Export`,
    la dependencia `zip` de `Cargo.toml` y `crates/lodestar-cli/src/bundle_io.rs` en lo que sea de
    import/export (conservando lo que use `check`/`reindex`).
- **Fuera de alcance**: `reindex` (sigue: la cache es reconstruible por diseño) y `check` (se
  generaliza en E20).
- **Criterios de aceptación**:
  - **Dado** `lodestar --help`, **Cuando** se imprime, **Entonces** los únicos subcomandos son `check`
    y `reindex` → `help_solo_check_y_reindex`.
  - **Dado** `lodestar init`, **Cuando** se ejecuta, **Entonces** exit code `2` → `init_es_uso`.
  - **Dado** el workspace, **Cuando** se corre `cargo tree -p lodestar-core`, **Entonces** no aparece
    `zip` → checklist de pureza del core.
- **Dependencias**: —.
- **Pruebas**: `crates/lodestar-cli/tests/cli.rs`: `help_solo_check_y_reindex`, `init_es_uso`.
- **Frontera (mcp.yml)**: no.

### E15-H04 — Retirar el prototipo JS como spec de comportamiento

- **Objetivo**: la spec pasa a ser `docs/REFACTOR_PHASE_2.md`. El CI deja de necesitar node.
- **Referencias**: decisión del usuario 2026-07-23 (`ARCHITECTURE.md §20.13`) ·
  `crates/lodestar-core/tests/differential.rs` · `prototype/harness/` · `.github/workflows/ci.yml`.
- **Alcance**:
  - Borrar `crates/lodestar-core/tests/differential.rs` (6 tests) y el paso `npm ci` del CI.
  - Reescribir las referencias que declaran el prototipo **spec de comportamiento** en `CLAUDE.md`
    ("Cómo trabajar aquí"), `requirements/README.md` ("Cómo leer estos documentos") y
    `docs/WORKFLOWS.md`: pasa a ser **referencia histórica de v0.2.x**.
  - **Conservar** `prototype/` en el árbol (no estorba y documenta el origen), sin `node_modules`.
- **Fuera de alcance**: borrar el directorio `prototype/`.
- **Criterios de aceptación**:
  - **Dado** un clon limpio **sin node instalado**, **Cuando** se corre `cargo test --workspace`,
    **Entonces** pasa entero → checklist de CI.
  - Estructural: `grep -r "spec de comportamiento" CLAUDE.md requirements/README.md` no atribuye ese
    papel al prototipo.
- **Dependencias**: —.
- **Pruebas**: negativas (ausencia); se verifica en el job de CI.
- **Frontera (mcp.yml)**: no.

### E15-H05 — Fixtures de estructuras arbitrarias

- **Objetivo**: dar a las historias siguientes los workspaces de prueba que exige
  `REFACTOR_PHASE_2 §Tests imprescindibles § Descubrimiento`.
- **Referencias**: `ARCHITECTURE.md §20.5` · `crates/lodestar-fixtures/src/lib.rs`.
- **Alcance** (**aditivo**: los fixtures OKF actuales siguen ahí hasta que E16/E17 retiren a sus
  consumidores):
  - `arbitrary()` — `README.md` en la raíz + `one/first.md` + `two/levels/second.md` +
    `three/levels/deep/third.md`, con enlaces cruzados raíz↔profundo (los del §Resultado esperado).
  - `with_edge_cases()` — paths con espacios, `%20` en un href, directorio oculto `.oculto/`,
    dos ficheros con el mismo basename en árboles distintos, un enlace con capitalización errónea.
  - Helper de materialización en disco (`materialize(&FileMap, &Path)`) para los tests de
    descubrimiento, que necesitan ficheros reales y no un `FileMap`.
  - Fixtures de disco no representables en `FileMap`: fichero no UTF-8, fichero sobre el límite de
    tamaño, symlink, `.gitignore` y `.lodestarignore`.
- **Fuera de alcance**: borrar `conformant()`/`with_issues()`/`synthetic()` (los consumen tests que
  siguen vivos hasta E16/E17).
- **Criterios de aceptación**:
  - **Dado** `arbitrary()`, **Cuando** se materializa y se vuelve a leer, **Entonces** el `FileMap`
    resultante es idéntico → `fixture_arbitrary_roundtrip`.
  - **Dado** `with_edge_cases()`, **Cuando** se materializa, **Entonces** existen en disco los paths
    con espacios y el directorio oculto → `fixture_edge_cases_materializa`.
- **Dependencias**: —.
- **Pruebas**: `crates/lodestar-fixtures/` (tests del propio crate).
- **Frontera (mcp.yml)**: no.

### E15-H06 — La raíz del workspace es el `cwd`

- **Objetivo**: `lodestar-mcp` arranca **sin argumentos** desde cualquier directorio; `--root` la
  fija explícitamente; la raíz es inmutable durante la sesión.
- **Referencias**: `ARCHITECTURE.md §20.5`, `§20.1` · `REFACTOR_PHASE_2 §Fase 2` ·
  `crates/lodestar-mcp/src/main.rs:44-90` · `crates/lodestar-cli/src/main.rs:67-80`.
- **Alcance**:
  - MCP: **eliminar el gate** `main.rs:80-88` que aborta con exit 3 si no hay `index.md`/`.lodestar/`.
    Sustituir el argumento posicional por `--root <dir>` (aceptando el posicional como alias
    deprecado no es necesario: v0.3 es incompatible). Sin `--root` → `std::env::current_dir()`.
  - **Canonicalizar la raíz una sola vez al arrancar** y guardarla; toda ruta pública se emite
    relativa a ella (ya lo garantiza `RelPath`).
  - CLI: `resolve_root` (`main.rs:67`) deja de **subir por los ancestros** buscando
    `index.md`/`.lodestar`; usa `--path` o el cwd, sin ascender.
  - Rechazo explícito de rutas absolutas y de `..` en las operaciones MCP: ya lo hace
    `RelPath::new` (`crates/lodestar-core/src/types.rs:33`) — esta historia añade el **test** que lo
    fija como contrato de la frontera, no código nuevo.
- **Fuera de alcance**: `workspace.root` en la config (E15-H08); qué ficheros se descubren (E15-H07).
- **Criterios de aceptación**:
  - **Dado** un directorio con `notas.md` y **sin** `index.md` ni `.lodestar/`, **Cuando** se lanza
    `lodestar-mcp` con el cwd ahí, **Entonces** arranca y responde `tools/list`
    → `arranca_en_directorio_arbitrario`.
  - **Dado** `lodestar-mcp --root /otro/dir`, **Cuando** arranca, **Entonces** opera sobre ese
    directorio aunque el cwd sea otro → `root_explicito_gana`.
  - **Dado** un servidor arrancado, **Cuando** una tool recibe `path: "/etc/passwd"` o
    `path: "../fuera.md"`, **Entonces** responde error de path inválido sin tocar disco
    → `rechaza_absoluta`, `rechaza_escape`.
  - **Dado** un cwd que es subdirectorio de un proyecto con `index.md` en un ancestro, **Cuando** se
    corre `lodestar check`, **Entonces** juzga el cwd, no el ancestro → `cli_no_asciende`.
- **Dependencias**: E15-H01 (`Workspace::open` cambia de firma al perder git).
- **Pruebas**: `crates/lodestar-mcp/tests/mcp.rs`: `arranca_en_directorio_arbitrario`,
  `root_explicito_gana`, `rechaza_absoluta`, `rechaza_escape`.
  `crates/lodestar-cli/tests/cli.rs`: `cli_no_asciende`.
- **Frontera (mcp.yml)**: **sí** (arranque y contrato de paths).

### E15-H07 — Descubrimiento recursivo universal

- **Objetivo**: todos los `.md` del proyecto, a cualquier profundidad, forman una sola base de
  conocimiento — con exclusiones seguras y diagnósticos en vez de silencio.
- **Referencias**: `ARCHITECTURE.md §20.5`, `§20.9` (códigos) · `REFACTOR_PHASE_2 §Fase 3` ·
  `crates/lodestar-workspace/src/io.rs:10-55` (el `load_bundle` actual).
- **Alcance**:
  - Módulo nuevo `crates/lodestar-workspace/src/discovery.rs` que sustituye a `io::load_bundle`,
    sobre el `ignore::WalkBuilder` que ese fichero ya usa. Aporta:
    - globs `include` (por defecto `**/*.md`) y `exclude` (por defecto `.git/**`,
      `.lodestar/runtime/**`);
    - `.gitignore` respetado por defecto (ya activo) **y** `.lodestarignore` vía
      `add_custom_ignore_filename`. **Ojo**: `WalkBuilder` solo aplica `.gitignore` dentro de un repo
      git salvo que se le pase `require_git(false)` — sin eso, un proyecto sin `.git/` ignoraría su
      propio `.gitignore`, que es justo el caso "directorio arbitrario" que persigue la épica;
    - `follow_links(false)` + diagnóstico `SYMLINK-UNSUPPORTED` cuando se encuentra uno;
    - límite de tamaño por documento, configurable → `DOC-TOO-LARGE`. **No** usar
      `WalkBuilder::max_filesize`: descarta el fichero en silencio, y aquí hace falta el diagnóstico;
      comprobar el tamaño al leer;
    - no-UTF-8 y rutas no representables como **diagnósticos** (`DOC-NOT-UTF8`, `PATH-NOT-UTF8`) en
      vez del `eprintln!` silencioso de hoy (`io.rs:27,46`);
    - detección de **colisiones de capitalización** entre paths descubiertos → `LINK-CASE-MISMATCH`
      a nivel de inventario;
    - sin profundidad máxima.
  - Añadir a `CheckCode` los códigos nuevos de descubrimiento. Los `OKF-*` conviven hasta E16.
  - El inventario es completo y **determinista**: mismo árbol ⇒ mismo orden (lo garantiza `FileMap`
    = `BTreeMap<RelPath, _>`).
- **Fuera de alcance**: reaccionar a cambios en vivo (el watcher de `lodestar-store` ya existe;
  su reconfiguración a la política nueva es parte de E18).
- **Criterios de aceptación**:
  - **Dado** el fixture `arbitrary()` materializado, **Cuando** se descubre, **Entonces** el
    inventario tiene los 4 documentos, incluido `three/levels/deep/third.md`
    → `descubre_a_cualquier_profundidad`.
  - **Dado** un `.gitignore` con `vendor/`, **Cuando** se descubre, **Entonces** `vendor/x.md` no
    está en el inventario → `respeta_gitignore`.
  - **Dado** un `.lodestarignore` con `borradores/`, **Cuando** se descubre, **Entonces**
    `borradores/x.md` no está → `respeta_lodestarignore`.
  - **Dado** un `.md` que es symlink, **Cuando** se descubre, **Entonces** no entra en el inventario
    y se emite `SYMLINK-UNSUPPORTED` → `symlink_rechazado_con_diagnostico`.
  - **Dado** un `.md` no UTF-8 y otro sobre el límite, **Cuando** se descubre, **Entonces** se emiten
    `DOC-NOT-UTF8` y `DOC-TOO-LARGE` y **el resto del inventario se carga**
    → `no_utf8_y_grande_no_abortan`.
  - **Dado** `docs/auth.md` y un directorio `Docs/`, **Cuando** se descubre en un sistema de ficheros
    case-insensitive, **Entonces** se emite un diagnóstico de portabilidad → `colision_capitalizacion`.
  - **Dado** un `.md` con espacios en el path, **Cuando** se descubre, **Entonces** entra en el
    inventario con su ruta exacta → `paths_con_espacios`.
- **Dependencias**: E15-H05 (fixtures), E15-H06 (raíz).
- **Pruebas**: `crates/lodestar-workspace/tests/discovery.rs` (fichero nuevo), con los 7 nombres.
- **Frontera (mcp.yml)**: no (cambia `CheckCode`, que sí está en el contrato → `/contrato --check`).

### E15-H08 — Configuración opcional del workspace

- **Objetivo**: la config **limita**, nunca habilita. Su ausencia no impide usar Lodestar
  (invariante #18 del documento).
- **Referencias**: `ARCHITECTURE.md §20.5`, `§20.9` (política de validación) ·
  `crates/lodestar-workspace/src/config.rs:80-185` (`WorkspaceConfig`, que ya tiene el patrón
  correcto: ausencia de fichero ⇒ defaults, YAML inválido ⇒ error explícito).
- **Alcance**:
  - Extender `WorkspaceConfig` con `workspace.root`, `discovery` (`include`/`exclude`/
    `respectGitignore`/`respectLodestarIgnore`/`followSymlinks`/tamaño máximo) y `validation`
    (severidad por familia de diagnóstico) + `transactions.rejectNewErrors`/`allowExistingErrors`.
  - **Borrar** `Config`/`lodestar.toml` (legado, `config.rs:14-63`): dos ficheros de configuración
    para lo mismo es deuda, y `identity` ya murió en E15-H01. Cierra `DECISIONES.md §8`.
  - Renombrar `writableRoots` conservando semántica (es la *write policy* de `§20.1`);
    `referenceRoots` se retira en E20 con las refs externas por frontmatter.
- **Fuera de alcance**: aplicar la política de validación (E20); `metadata_inspect` (E20).
- **Criterios de aceptación**:
  - **Dado** un directorio **sin** `.lodestar/`, **Cuando** se abre, **Entonces** se usa la política
    por defecto de `§20.5` y no se crea ningún fichero de config → `sin_config_funciona`.
  - **Dado** un `.lodestar/config.yaml` con `discovery.exclude: ["notas/**"]`, **Cuando** se
    descubre, **Entonces** `notas/x.md` queda fuera del inventario → `exclude_configurado`.
  - **Dado** un `.lodestar/config.yaml` con YAML malformado, **Cuando** se abre, **Entonces** error
    explícito — nunca caída silenciosa a defaults → `config_malformada_es_error`.
  - **Dado** un `lodestar.toml` en la raíz, **Cuando** se abre, **Entonces** se ignora por completo
    (es un fichero más del proyecto) → `lodestar_toml_ignorado`.
- **Dependencias**: E15-H01 (`identity` fuera), E15-H07 (hay una política que configurar).
- **Pruebas**: `crates/lodestar-workspace/tests/config.rs`: los 4 nombres.
- **Frontera (mcp.yml)**: **sí** (`workspace_status` expone `discovery` en su salida, `§20.10`).

---

## Orden de construcción

```
H01 (borra vcs) ─┐
H02 (generadores)├─→ H06 (raíz cwd) ─→ H07 (descubrimiento) ─→ H08 (config)
H03 (init/zip)   │        ▲
H04 (prototipo)  ┘        │
H05 (fixtures) ───────────┘
```

H01–H05 son independientes entre sí y pueden ir en cualquier orden; H06 necesita H01 (la firma de
`Workspace::open` cambia) y H05 (fixtures); H07 necesita H06; H08 necesita H07.

## Criterio de salida de la épica

`cd` a un proyecto arbitrario con `.md` anidados y **sin nada de Lodestar** → `lodestar-mcp` arranca,
`workspace_status` reporta el inventario completo a cualquier profundidad, y `cargo tree` no muestra
`git2` ni `zip`. Los `.md` siguen interpretándose con el modelo OKF (eso lo cambia E16), pero ya se
**descubren** universalmente.
