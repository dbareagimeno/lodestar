# E23 — Dónde se quedó y cómo seguir

> Documento de **traspaso de sesión**, no de producto. La spec vive en
> [`requirements/epica-23-cierre-migracion.md`](../requirements/epica-23-cierre-migracion.md);
> esto es el estado real, lo que falta y las trampas que ya se pisaron.
> Rama `refactor/markdown-universal` (PR #17). Última sesión: 2026-07-25/26.

## Contexto en una frase

La PR #17 (migración de OKF a Markdown universal, E15–E22) estaba abierta y verde, pero una revisión
destapó defectos reales en el camino de escritura. **E23 es la épica de cierre**: se decidió
arreglarlo todo dentro de la PR #17, y construir la CLI como gestor de KB queda **fuera**, propuesto
en [`docs/PROPUESTA_CLI.md`](PROPUESTA_CLI.md) para planificar en una PR posterior.

## Estado: 14 historias cerradas, 3 pendientes

Suite: **419 tests** + **4 de crash-recovery** (`--features test-failpoints`) = los 423 `#[test]` del
árbol. `fmt`, `clippy -D warnings` y `doc -D warnings` limpios. Árbol de trabajo limpio.

### Cerradas (12 commits, desde `71f72be` hasta `4d6b6a7`)

| Historia | Qué arregló |
|---|---|
| **H06** | El CI corre los 4 tests de crash-recovery. Estaban tras `#[cfg(feature = "test-failpoints")]` y `cargo test --workspace` **no activa features opcionales**: no se ejecutaban desde E13. |
| **H01** | `lodestar check` y `knowledge_check` daban **veredictos contradictorios** sobre el mismo workspace. `full_analysis` ignoraba la sección `validation` de la config y los diagnósticos de descubrimiento. |
| **H02** | `create` escribía `type: ''` (residuo OKF) y un `title` que nadie pidió en cada documento nuevo. |
| **H03** | **No se podía mover una nota que enlazara a sus vecinas**: los salientes del documento movido no se recalculaban, el gate lo veía como errores nuevos y el move era imposible. |
| **H04** | `recovery.pendingTransaction` era un `false` literal: tras un crash, la primera tool que llama un agente le mentía. |
| **H05** | `delete` aceptaba `retarget`/`create_stub` **sin ejecutarlas**. Retiradas. |
| **H07** | e2e de ciclo de vida en **una sola sesión** MCP (antes cada paso levantaba un proceso, lo que enmascaraba los bugs de invalidación). |
| **H08** | `reindex` no tenía **ni un test**; `.gitignore`/`.lodestarignore` no se probaban por ninguna fachada. |
| **H09** | Bordes: concurrencia entre **procesos**, lock huérfano, unicode en rutas, `patch_frontmatter` sobre YAML ilegible, `--root` inexistente. |
| **H10** | El `inputSchema` de `change_plan` declaraba 4 campos; ahora declara los **18** que el código lee. |
| **H11** (core) | `metadata_inspect` no daba el **vocabulario de tags**; `[volver](../)` tumbaba la puerta de CI. |
| **H13** (1/2) | El ledger decía «E15–E22 EN CURSO» y describía la UI Tauri. |
| **H14** | `DECISIONES §12` (fechas lexicográficas) y **§13** (`Conformant → Valid`, abriendo el catálogo de errores la única vez). |
| **H15/H16** | `docs/PROPUESTA_CLI.md` y `DECISIONES §14` (el store sin consumidor). |
| **H23/H24** | Lock reclamable por TTL+PID; NFC/NFD deja de tumbar el CI; 4 códigos sin emisor registrados. |

### Pendientes

**1. E23-H12 — efectos secundarios de abrir el workspace.** El único de riesgo.
`Workspace::open` llama a `gitignore::ensure_gitignore(root)` y `runtime::ensure_runtime_scaffold(root)`
(`crates/lodestar-workspace/src/lib.rs:83-84`), o sea que **tanto `lodestar check` como arrancar el
MCP reescriben el `.gitignore` del proyecto** y crean `.lodestar/runtime/` antes de leer nada. Para
el pitch «`cd my-project && lodestar-mcp` sobre cualquier proyecto» es una escritura no solicitada;
en CI deja el working tree sucio.

- Criterios en la épica: `check_no_ensucia_el_working_tree` y `readonly_no_escribe_nada`.
- Idea: hacer los dos efectos **perezosos** — que ocurran cuando se va a escribir de verdad
  (`enable_cache` crea el `index.db`; `change_plan` persiste planes; la transacción escribe), no al
  abrir. `Workspace::open_ephemeral` ya existe y salta ambos: lo usa `migrate-from-okf`.
- ⚠️ **Cuidado**: `crates/lodestar-mcp/tests/concurrencia.rs` y varios tests de
  `lodestar-workspace` dan por hecho que `.lodestar/runtime/` existe tras `open`. Hay que mirarlos
  antes de mover nada.
- También en el alcance: retirar `implemented_by`/`verified_by` como claves de frontmatter
  privilegiadas y no configurables (`crates/lodestar-workspace/src/external_refs.rs:25`), último
  residuo OKF con semántica impuesta.
- Nota: `snapshot_canonico` en `crates/lodestar-cli/tests/cli.rs` **excluye el `.gitignore` a
  propósito** por este defecto; cuando se arregle, ese comentario sobra.

**2. E23-H11 (resto) — descubribilidad, la parte de `App`/MCP.** La parte del core está hecha.
Queda:
- `knowledge_search` **no devuelve ningún campo de frontmatter** (solo path/title/snippet/score/
  revision), así que ver el `status` de 30 resultados cuesta 30 `knowledge_get`. E19-H05 retiró
  `type`/`status`/`tags` sin poner nada genérico. Añadir proyección pedida por el llamador.
- `sort` se acepta y **se ignora en silencio** (`_sort` en `lodestar-app/src/lib.rs`): implementarlo
  o retirarlo del schema.
- `apply_fix` sigue anunciada como una de las 8 ops pero **siempre falla** (`FixNotFound`): no hay
  productor de `fixes` desde E20-H03. Sacarla del enum hasta que lo haya.
- No hay forma de **listar receipts**: si el agente pierde el `receiptId`, el undo es inalcanzable
  pese a estar persistido. Decidir si entra aquí o es historia propia.

**3. E23-H13 (2/2)** — añadir la sección de E23 a `IMPLEMENTATION_STATUS.md` con el detalle por
historia, y marcar la épica completa cuando lo esté.

## Cómo se ha estado trabajando

Proceso del repo (`CLAUDE.md`): `/historia` → `/tdd` → `/juzgar`. En la práctica:

1. Lanzar `autor-tests` (**siempre Opus, effort `xhigh`** — política del usuario) con la historia y
   el **síntoma reproducible**, no solo la spec.
2. Verificar el rojo **uno mismo** antes de implementar.
3. Implementar, verificar verde, y **reproducir el síntoma original con los binarios reales**.
4. Gates: `cargo test --workspace` + `cargo test -p lodestar-workspace --features test-failpoints` +
   `fmt` + `clippy -D warnings` + `doc -D warnings`.

Se pueden correr **dos autores de tests en paralelo** si trabajan sobre ficheros disjuntos; hay que
decírselo explícitamente en el brief («no toques X, otro agente está en ello»).

## Trampas ya pisadas (no repetirlas)

- **`pkill -f "cargo test"` se mata a sí mismo**: la propia línea de comando contiene «cargo test».
- **zsh no hace word-splitting** de `$VAR` sin comillas: un `for f in $FILES` pasa la lista entera
  como un solo argumento. Usar `while IFS= read -r`.
- **Un `perl -pi` sobre todo el árbol destroza las tablas de terminología**: al renombrar
  `Conformant → Valid` dejó `ARCHITECTURE §20.3`, `REFACTOR_PHASE_2` y `DECISIONES §13` diciendo
  «Valid → Valid», porque ahí `Conformant` es el término **de partida**. Los documentos históricos
  (`docs/REFACTOR.md`, `docs/REFACTOR_DISENO_PROPUESTA.md`) tampoco deben renombrarse.
- **Nunca `git checkout <fichero_de_tests>` ni `cargo fmt -p <crate> -- <ruta>`**: destruyen los
  tests de la fase roja sin commitear. `cargo fmt --all` sí es seguro.
- **La suite tarda** (el arnés de escala genera 10.000 documentos). Correrla en background y no
  solapar dos ejecuciones: el recuento sale truncado y engaña.
- **Contar tests**: `[N] passed` sumado debe dar 419; con los 423 `#[test]` del árbol cuadra porque
  4 están tras la feature de failpoints.

## Los 4 defectos que NO estaban en la revisión inicial

Aparecieron **implementando**, todos en código ya publicado. Es el argumento de por qué el bloque B
(los bordes que nadie probaba) valía la pena:

1. **Corrupción real**: reescribir el cuerpo de un documento **sin frontmatter** le inyectaba
   `---\n{}\n---`. Como `move` con `rewriteInboundLinks` reescribe el cuerpo de *cada* emisor, mover
   un documento corrompía de una tacada todos sus enlazantes sin frontmatter.
2. **Definiciones de referencia sin usar** (`[b]: beta.md` sin `[…][b]`) no se recalculaban al mover:
   el parser solo emite eventos por **uso**.
3. **Lock huérfano irrecuperable**: un proceso muerto por SIGKILL dejaba la base cerrada a la
   escritura para siempre.
4. **NFC/NFD**: un enlace correcto tumbaba el CI en macOS, con el mismo veredicto que en Linux pero
   acertado solo en una de las dos plataformas.

## Decisiones que tomó el usuario en esta sesión

- Arreglarlo **todo dentro de la PR #17**; la CLI queda como propuesta escrita.
- **Abrir el catálogo de 16 códigos de error** para completar `Conformant → Valid` (§13),
  aprovechando que v0.3 ya es incompatible con v0.2.
- Lock huérfano: **reclamar por TTL + PID**.
- NFC/NFD: **resolución tolerante + aviso**, sin normalizar la ruta canónica.

## Decisiones abiertas que siguen esperando criterio

- **`DECISIONES §14`** — el store (épica E18 entera) **no tiene ningún consumidor**: ninguna tool lee
  de SQLite y `document_set()` reparsea la base completa en cada llamada. Conectarlo, acotarlo o
  retirarlo. Es la mayor cantidad de capacidad construida sin consumidor del repo.
- **`docs/PROPUESTA_CLI.md`** — pendiente de `/planificar` en una PR posterior. Lleva una condición
  de entrada dura: la escritura por CLI no se implementa hasta que existan los tests de concurrencia
  entre procesos (que **ya existen**, en `crates/lodestar-mcp/tests/concurrencia.rs`, desde E23-H09).

## Deuda conocida que sigue viva

- **El camino transaccional construye el `DocumentSet` sin `other_files`**, así que al planificar,
  un enlace a código existente parece roto (`diagnosticsAfter` con un warning de más). Hoy no
  bloquea porque un fichero no-Markdown ausente es `Warn`, pero con un `.md` excluido del inventario
  sería `Err` y tumbaría un `move` legítimo. Registrada al cerrar E17.
- **El walker del store no aplica la `DiscoveryPolicy`** (ni `.lodestarignore`, ni
  `include`/`exclude`, ni el límite de tamaño): la paridad core↔store solo se sostiene bajo política
  por defecto. Inocua mientras nadie lea el store; bug real en cuanto se conecte. Va con §14.
