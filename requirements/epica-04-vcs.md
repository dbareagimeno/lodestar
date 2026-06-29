# E4 — `lodestar-vcs` (libgit2 local + binario `git` para red)

> **Fase**: `§14.4`. **Objetivo de la épica**: la crate dueña **única** de git, hermana de `store`.
> **Transporte híbrido**: libgit2 para TODO lo local (status/log/diff/commit/branch/merge/restore/init,
> ref-watch) — abrir/indexar un bundle ajeno **nunca** ejecuta sus hooks/aliases/`include.path` (garantía
> RCE-safe); el binario `git` confinado a la **red** (push/pull/fetch). `git2` vive **SOLO aquí**; `git2::Oid`
> **nunca** cruza la frontera (se expone `Sha`/`Branch`). **vcs NO escribe el working tree** en operaciones
> locales: devuelve file-maps que la workspace aplica por el único escritor (E5).
> Referencias: `ARCHITECTURE.md §13` entero, `§4.4`, `§10` filas 15/16/17/18/19/20/21.

**Cuatro correcciones de seguridad (ship-blockers, `§13.6`)** que esta épica DEBE entregar:
no perder trabajo sin commitear (checkpoint), `OKF-CONFLICT`, `RepoState`, pill nunca obsoleto.

---

### E4-H01 — `Vcs::discover` con techo en el root (degradación sin `.git`)
- **Objetivo**: descubrir el repo SIN enganchar un repo ancestro, distinguiendo los tres estados.
- **Referencias**: `ARCHITECTURE.md §13.2` · `git2::Repository`.
- **Alcance**:
  - `discover(root: &Path) -> Result<Option<Vcs>>` con **techo en el root del bundle** (no engancha `~/.git`).
  - Tres estados: **sin-repo** (`None` → "activar git"), **repo-vacío** (sin commits), **con-historial**.
  - `git2::Repository` es `!Sync` → se diseña para vivir tras `Mutex<Vcs>` (el único escritor lo posee en E5).
  - **libgit2 nunca ejecuta hooks/config al abrir**: documentar y testear que `discover` no corre `include.path`/aliases.
- **Criterios de aceptación**:
  - Un `.git` en un ancestro (`~/.git`) NO se engancha; solo el del root del bundle.
  - Los tres estados se distinguen correctamente.
- **Dependencias**: E1-H19, E0-H01.
- **Pruebas**: fixtures con/sin `.git`, repo vacío, repo con historial; test del techo de descubrimiento.

### E4-H02 — `Vcs::init` (first-run: `git init` + `.gitignore` + commit inicial)
- **Objetivo**: inicializar un repo para un bundle nuevo.
- **Referencias**: `ARCHITECTURE.md §13.2`, `§12` (first-run).
- **Alcance**:
  - `init(root) -> Result<Vcs>`: `git init`, escribe/asegura `.gitignore` (incluye `.lodestar/`), commit inicial.
  - Verifica idempotentemente que `.lodestar/` está ignorado; si estaba trackeada, ofrece "dejar de trackear" (lo aplica la fachada).
  - Completa la historia E2-H05 (`lodestar init`).
- **Criterios de aceptación**: tras `init`, hay un repo con 1 commit y `.lodestar/` ignorado.
- **Dependencias**: E4-H01.
- **Pruebas**: init en dir limpio → repo con commit inicial; `.lodestar/` en `.gitignore`.

### E4-H03 — `status` + `RepoState` (detección de merge/rebase en curso)
- **Objetivo**: el dirty-set vs HEAD y el estado del repo (ship-blocker `§13.6.3`).
- **Referencias**: `ARCHITECTURE.md §13.2`, `§13.6.3`, `§10` fila 18, `§4.4` (`RepoState`).
- **Alcance**:
  - `status(&self) -> RepoStatus` con el dirty-set (cambios vs HEAD, **menos generados**) y `RepoState`
    desde `repository.state()` (`Clean`/`Merging`/`Rebasing`/`CherryPicking`/`Reverting`).
  - El conteo "sin commitear" es comparación de **hash por path** contra el HEAD-map en RAM (O(cambiados)),
    **nunca** `diffSnap` (`§10` fila 21 / `§13.3`).
- **Criterios de aceptación**:
  - Un merge a medias del `git` CLI externo se reporta `RepoState::Merging`.
  - El conteo "sin commitear" excluye index/tags/log generados y no usa `OkfDiff`.
- **Dependencias**: E4-H01, E1-H17 (tipos), E1-H13 (hash blake3).
- **Pruebas**: estados de repo; conteo por hash vs `OkfDiff` (debe diferir en coste, no en resultado para el conteo).

### E4-H04 — `log` · `log_for_path` · `tree_files` (lecturas baratas, sin tocar el working tree)
- **Objetivo**: metadatos de historial (revwalk) y materializar el árbol de un commit a `FileMap`.
- **Referencias**: `ARCHITECTURE.md §13.2`, `§13.4`, `§4.4` (`CommitRow`).
- **Alcance**:
  - `log(limit) -> Vec<CommitRow>`: metadatos baratos por revwalk, **sin** leer árboles (conformance = `None` aquí).
  - `log_for_path(p, limit)`: con **techo de commits escaneados** (no recorre todo el DAG).
  - `tree_files(sha) -> Result<FileMap>`: árbol de un commit → file-map **SIN tocar el working tree**.
    **Blobs binarios/no-UTF8 se saltan y diagnostican** (no abortan el árbol, `§13.2`).
- **Criterios de aceptación**:
  - `log` no lee blobs (rápido sobre historiales grandes).
  - `tree_files` reconstruye el `FileMap` de un commit; un blob binario se omite con diagnóstico, sin panic.
  - `git2::Oid` no aparece en ninguna firma pública (solo `Sha`).
- **Dependencias**: E4-H01, E1-H19.
- **Pruebas**: log sobre repo con N commits; tree_files vs `git show`; blob binario saltado.

### E4-H05 — `commit` (libgit2: stage + commit del working tree, sin hooks/firma)
- **Objetivo**: crear commits por libgit2, con la salvedad documentada de que NO disparan hooks ni firman.
- **Referencias**: `ARCHITECTURE.md §13.2`, `§13.5`, `§13.8` (firma/LFS), `§12` (paridad git CLI, identidad).
- **Alcance**:
  - `commit(msg, author: &Author) -> Result<Sha>`: stage del working tree + commit.
  - **No corre hooks** (libgit2), **no firma** (`commit.gpgsign` ignorado), **no aplica filtros LFS/`.gitattributes`**.
  - Si el repo **exige firma** o detecta un **blob LFS crudo**: la fachada **avisa** y ofrece commitear vía CLI
    (la detección vive aquí; el aviso lo hace la fachada).
  - **Identidad** (`§12`): autor+committer separados; override `[identity]` de `lodestar.toml` → git config → fallback marcado.
- **Criterios de aceptación**:
  - Un commit aparece en `log` con el autor correcto.
  - `commit.gpgsign=true` en config NO produce firma (y se puede detectar para avisar).
  - Un blob bajo regla LFS se detecta (no se commitea crudo silenciosamente).
- **Dependencias**: E4-H01, E4-H04.
- **Pruebas**: commit + verificación en log; detección de firma exigida y de LFS.

### E4-H06 — Ramas: `branches` · `current_branch` · `create_branch` · `switch_branch_target` · `merge_target`
- **Objetivo**: topología de ramas local; switch/merge devuelven file-maps (no escriben el working tree).
- **Referencias**: `ARCHITECTURE.md §13.2`, `§13.8` (scope ramas), `§4.4` (`Branch`), `§10` fila 16/17.
- **Alcance**:
  - `branches() -> Vec<Branch>` (locales + **ahead/behind vs upstream**), `current_branch() -> Option<String>`
    (HEAD desacoplado = `None`), `create_branch(name, from)` (no toca el working tree).
  - `switch_branch_target(name) -> Result<FileMap>`: devuelve el **árbol de la rama destino** (la workspace
    computa el `core::Mutation` y aplica por el único escritor, E5).
  - `merge_target(name) -> Result<FileMap>`: fija `MERGE_HEAD` en `.git` (commit de 2 padres) + file-map merged;
    **conflicto → marcadores inline** (los cazará `OKF-CONFLICT`) + `RepoState=Merging`.
- **Fuera de alcance**: aplicar los file-maps al working tree (E5); rebase (diferido, `§13.8`).
- **Criterios de aceptación**:
  - `switch_branch_target`/`merge_target` **no** modifican el working tree (solo devuelven file-map; merge fija MERGE_HEAD).
  - Un merge conflictivo devuelve un file-map con marcadores `<<<<<<<` y deja `RepoState::Merging`.
  - `branches` reporta ahead/behind correctos vs upstream (0/0 sin upstream).
- **Dependencias**: E4-H01, E4-H04.
- **Pruebas**: crear/listar ramas; switch target vs `git checkout`; merge conflictivo → marcadores + Merging.

### E4-H07 — Red confinada al binario `git`: `pull` (--ff-only) · `push`
- **Objetivo**: operaciones de red por subproceso `git` con args fijos validados, heredando el auth del usuario.
- **Referencias**: `ARCHITECTURE.md §13.2`, `§13.8`, `§12` (seguridad/sync/remoto), `§4.4` (`SyncOutcome`).
- **Alcance**:
  - `pull() -> Result<SyncOutcome>`: `git pull --ff-only`; si la rama divergió, **aborta limpio** → la UI sugiere
    merge (nunca conflicta in-app).
  - `push() -> Result<SyncOutcome>`: al upstream configurado; rechazo (non-ff) → `ok:false` + summary.
  - **Subproceso confinado**: argumentos **fijos validados**, **jamás** interpola input no confiable, **nunca**
    corre en `open`/`index` (solo acción explícita). Hereda el entorno de auth (SSH-agent/credential-helpers/tokens).
  - **Sin upstream** → push/pull deshabilitados (la fachada remite al `git` CLI; clone/añadir remoto = no-goal).
  - **Sin binario `git` en PATH (o versión incompatible)** → push/pull deshabilitados con aviso accionable;
    lo **local** (libgit2) sigue funcionando.
  - libgit2 **nunca** habla con la red.
- **Criterios de aceptación**:
  - `pull` sobre rama divergida aborta sin dejar merge a medias.
  - `push` non-ff → `ok:false` con summary explicativo.
  - El subproceso nunca recibe input de usuario interpolado (auditoría de la construcción de args).
  - Sin `git` en PATH: push/pull devuelven aviso; commit/log/diff locales siguen OK.
- **Dependencias**: E4-H01.
- **Pruebas**: pull ff-only/divergida; push ff/non-ff (con remoto local de test); ausencia de `git` en PATH.

### E4-H08 — Ref-watch del gitdir (pill nunca obsoleto, ship-blocker `§13.6.4`)
- **Objetivo**: vigilar el subconjunto de `.git` que cambia con commits/refs para no dejar el pill obsoleto.
- **Referencias**: `ARCHITECTURE.md §9` (Git), `§13.6.4`, `§10` fila 19.
- **Alcance**:
  - Vigila `HEAD`, `refs/heads/`, `packed-refs`, **`logs/HEAD`**; maneja **`.git`-como-fichero** (worktrees/gitdir real).
  - Emite `vcs:changed` (pista, no garantía). El update real del pill es **optimista** con el `Sha` que devuelve
    el commit + **reconcile al enfocar** la ventana (la fachada). Aquí: el productor del evento ref-watch.
- **Criterios de aceptación**:
  - Un commit (que no cambia bytes del working tree) dispara `vcs:changed`.
  - Funciona con `.git` como fichero (gitdir redirigido).
- **Dependencias**: E4-H01.
- **Pruebas**: commit externo → evento; gitdir-como-fichero detectado.

### E4-H09 — Conformidad por commit cacheada por **tree-oid** (la pieza estrella)
- **Objetivo**: `confOf(snap)` real: conformidad de un commit, cacheada content-addressed e incremental.
- **Referencias**: `ARCHITECTURE.md §13.4`, `§4.4` (`CommitConformance`), `§10` fila 20.
- **Alcance**:
  - `Bundle::from_files(tree_files(sha)).analyze()` → `CommitConformance { hard_fail, warn_count, conform }`.
  - **Cache en `.lodestar/index.db` keyed por tree-oid** (dedup de reverts/cherry-picks), **gated por
    `ruleset_version`** (= **hash de las definiciones de reglas**: imposible cambiar un check sin invalidar la cache).
    El árbol es inmutable → la fila nunca se invalida por edición.
  - **Perezosa y acotada**: solo HEAD (gate/pill), los commits visibles del timeline (off-thread, persistidos),
    un commit abierto, o el barrido early-exit de `last_conforming()`. **Nunca** todo el DAG al abrir.
  - **Incremental**: reusa los checks locales por-fichero del commit **padre** para blobs con oid sin cambios;
    solo recomputa el pase global del grafo. O(M×cambiados + grafo).
  - Se cachea **crudo** (`hard_fail`/`warn_count`); el veredicto del gate (¿warns bloquean?) se deriva **al leer**
    de la strictness de `lodestar.toml` — la strictness **nunca** se hornea en la cache.
  - `last_conforming() -> Option<Sha>`: barrido early-exit hacia atrás.
- **Criterios de aceptación**:
  - Dos commits con el mismo árbol (revert) comparten fila de cache (1 cómputo).
  - Cambiar una definición de regla cambia `ruleset_version` e invalida la cache.
  - El cómputo incremental de un commit hijo reusa los checks locales del padre para blobs sin cambios.
  - La strictness no aparece en la fila cacheada.
- **Dependencias**: E4-H04, E3-H01 (cache DB), E1-H07.
- **Pruebas**: dedup por tree-oid; invalidación por `ruleset_version`; golden cross-fachada (con E7); incremental vs full.

### E4-H10 — `lodestar check --staged/--rev/--range` + `hooks install` (puerta OKF ↔ git)
- **Objetivo**: completar los subcomandos git de la CLI y cablear los hooks.
- **Referencias**: `ARCHITECTURE.md §13.5`, `§13.7`, `§7.3` · completa E2-H02/E2-H07.
- **Alcance**:
  - `check --staged` juzga el **índice staged** (no el working sucio); `--rev SHA` juzga ese árbol;
    `--range a..b` el rango. Todos vía `tree_files`/index → `Bundle::analyze`.
  - `hooks install`: cablea **pre-commit** → `check --staged`, **pre-push** → `check --rev HEAD`. CI corre el mismo binario.
  - Documentar que **los commits de la app (libgit2) NO disparan hooks** → la **workspace corre `check` ella misma
    antes de `commit`** (E5); los hooks instalados solo cubren commits del `git` CLI / CI.
  - Subcomandos CLI git completados: `log`/`diff`/`last-conforming`/`branch`(list/create/switch)/`merge`/`pull`/`push`.
- **Criterios de aceptación**:
  - `check --staged` falla (exit 1) si el índice staged tiene un hard-fail aunque el working esté limpio.
  - `hooks install` deja hooks ejecutables que invocan el binario; un commit por CLI con árbol no conforme se rechaza.
  - `last-conforming` imprime el `Sha` del último commit conforme.
- **Dependencias**: E4-H04, E4-H06, E4-H07, E4-H09, E2-H02.
- **Pruebas**: check por staged/rev/range; hook pre-commit rechaza; last-conforming.
