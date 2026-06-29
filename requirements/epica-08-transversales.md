# E8 — Transversales de producto

> **Fase**: `§14.8` (entrelazada con las demás). **Objetivo de la épica**: los concerns transversales con
> dueño asignado del `§12`, más migración, packaging, i18n, seguridad, config por-bundle, first-run, errores,
> identidad y el gate de rendimiento. Cada historia declara de qué fase depende.
> Referencias: `ARCHITECTURE.md §12` (tabla de concerns con dueño), `§11`, `§13.8`.

**Regla de la épica**: estas historias **no** introducen lógica OKF nueva; cablean políticas, packaging y
garantías sobre lo ya construido. Varias se pueden empezar en cuanto su fase base esté `Done`.

---

### E8-H01 — `lodestar.toml` (config por-bundle) + config app-global
- **Objetivo**: los dos niveles de config para que GUI/CLI/MCP coincidan.
- **Referencias**: `ARCHITECTURE.md §12` (config), `§13.4` (strictness al leer), `§12` (identidad: `[identity]`).
- **Alcance**:
  - **Por-bundle** (`lodestar.toml`, **commiteado**): `strictness` (¿warns bloquean?), `write_policy`
    (allow_nonconformant), `locale` de artefactos generados, **`[identity]`** (autor/committer override).
  - **App-global**: tema/layout/recents (no commiteado).
  - La **strictness se deriva al leer** (nunca se hornea en la cache de conformidad, `§13.4`/`§4.4`).
  - Schema de `lodestar.toml` documentado y validado; `[identity]` añadido al schema (`§12`).
- **Criterios de aceptación**:
  - `lodestar check` respeta `strictness` de `lodestar.toml`; cambiarlo cambia el veredicto sin recomputar la cache.
  - Las 3 fachadas leen la misma config por-bundle.
  - `[identity]` override se aplica al commit (precede a git config).
- **Dependencias**: E1-H03 (tipos), E2-H02; (consumido por E4-H05, E4-H09, E5).
- **Pruebas**: strictness cambia veredicto; identity override; precedencia config.

### E8-H02 — Migración del prototipo: `lodestar import` (localStorage → `.md` + replay de historial)
- **Objetivo**: materializar el `STORE_KEY` del proto a `.md` + cache, `git init`, y **replicar `versions[]` como commits retro-fechados**.
- **Referencias**: `ARCHITECTURE.md §12` (migración) · prototipo `STORE_KEY`, `versions[]`, `seedVersions`, `mkVer`, `verId`.
- **Alcance**:
  - `lodestar import <export.json>`: materializa `STORE_KEY` (files) a `.md`, construye la cache, `git init`.
  - **Replay del historial**: cada snapshot de `versions[]` → un commit **retro-fechado** (autor/fecha/mensaje
    vía `git2::Signature`) → reproduce el historial del prototipo en vez de tirarlo.
  - Completa el stub de E2-H06.
- **Criterios de aceptación**:
  - Tras `import`, el bundle tiene los `.md` del export y un `git log` con un commit por snapshot, con fechas/autores originales.
  - Sin pérdida de datos ni de historial respecto al export del proto.
- **Dependencias**: E4-H05 (commit con autor/fecha), E2-H06.
- **Pruebas**: import de un export real del proto; verificación de historial replicado.

### E8-H03 — i18n: catálogo de strings + mensajes de conformidad keyed por código
- **Objetivo**: externalizar todo texto a catálogo; conformidad **keyed por código** (la UI localiza).
- **Referencias**: `ARCHITECTURE.md §12` (i18n), `§4.1`/`§4.4` (MessageHint i18n en la fachada).
- **Alcance**:
  - Mensajes de conformidad **keyed por `CheckCode`** (el core produce code+targets+params, no prosa).
  - Catálogo español canónico que reproduce los textos del proto; `MessageHint` localizado en la fachada.
  - **Cabeceras de artefactos generados** (`index.md`/tags) **fijas canónicas** como consts (NO localizadas:
    los bytes generados son ficheros commiteados; cambiar locale los churnea, `§12`).
  - UI en español con strings externalizadas.
- **Criterios de aceptación**:
  - Ningún mensaje de conformidad lleva prosa hardcodeada en el core (solo code+params).
  - Cambiar el locale de UI NO cambia los bytes de los artefactos generados.
- **Dependencias**: E1-H06, E1-H14, E1-H17.
- **Pruebas**: render de cada código desde catálogo; estabilidad de bytes generados ante cambio de locale.

### E8-H04 — Seguridad: threat model + escapado FTS5 + confinamiento del subproceso git + zip-slip
- **Objetivo**: cerrar las superficies de ataque con un threat model de una página y sus mitigaciones.
- **Referencias**: `ARCHITECTURE.md §12` (seguridad), `§5` (escapar FTS5), `§13.2` (subproceso git confinado), `§4.1` (RelPath).
- **Alcance**:
  - **Threat model de una página**: webview, MCP confianza-local, zip-slip, path-traversal, subproceso git.
  - Verificar/documentar: DOMPurify (E6-H07), escapado FTS5 (E3-H02), subproceso `git` con args fijos validados
    que **nunca** interpola input no confiable y **nunca** corre en open/index (E4-H07), zip-slip imposible por
    `RelPath` (E1-H16), path-traversal cerrado por `RelPath` (E1-H01).
- **Criterios de aceptación**:
  - El threat model existe y cada vector tiene su mitigación enlazada a la historia que la implementa.
  - Auditoría: el subproceso git jamás recibe input de usuario interpolado.
- **Dependencias**: E1-H01, E1-H16, E3-H02, E4-H07, E6-H07.
- **Pruebas**: tests de cada vector (XSS, zip-slip, traversal, inyección de args); revisión del threat model.

### E8-H05 — Versionado OKF (`okf_version`): política warn-and-degrade + aditivo-solo
- **Objetivo**: la política para versión OKF desconocida/futura y la evolución de `CheckCode`.
- **Referencias**: `ARCHITECTURE.md §12` (versionado OKF), `§4.1` (`okf_version` en Analysis).
- **Alcance**:
  - Versión desconocida/futura → **warn-and-degrade** (no crashea); `okf_version` expuesto en la conformidad.
  - `CheckCode` **aditivo-solo** con deprecación explícita (nunca renombrar/reordenar wire values).
  - Distinguir de `user_version` de la cache (E3-H01).
- **Criterios de aceptación**:
  - Un `okf_version` futuro produce un warn y degrada (no error fatal).
  - Un test congela los wire values de los 15 `CheckCode` (cualquier cambio rompe el test → deprecación explícita).
- **Dependencias**: E1-H03, E1-H07.
- **Pruebas**: okf_version futuro → warn; golden de wire values de CheckCode.

### E8-H06 — Packaging: Tauri updater + firma/notarización + release CI
- **Objetivo**: empaquetar los 3 binarios desde un release etiquetado, firmados/notarizados.
- **Referencias**: `ARCHITECTURE.md §12` (packaging).
- **Alcance**:
  - Tauri updater + firma/notarización (macOS/Windows); los 3 binarios (app/CLI/MCP) desde un release etiquetado.
  - Política de compat app/CLI/MCP/schema; CI de release.
- **Criterios de aceptación**:
  - Un tag de release produce los 3 artefactos firmados; el updater verifica la firma.
  - La política de compat está documentada y testeada (versión de schema).
- **Dependencias**: E0-H07, E2, E6, E7.
- **Pruebas**: dry-run de release en CI; verificación de firma.

### E8-H07 — Identidad / atribución (autor+committer, override, agente distinguible)
- **Objetivo**: la atribución correcta de commits (humano vs agente) con override.
- **Referencias**: `ARCHITECTURE.md §12` (identidad), `§13.7` (MCP commit con trailer).
- **Alcance**:
  - Autor+committer **separados**; override `lodestar.toml [identity]` → git config → fallback **marcado**.
  - Commits del **agente** (MCP) con trailer `Co-Authored-By` distinguible.
- **Criterios de aceptación**:
  - Un commit humano y uno del agente se distinguen en `git log`/blame.
  - El override de `[identity]` precede a la git config.
- **Dependencias**: E4-H05, E7-H05, E8-H01.
- **Pruebas**: atribución humano vs agente; precedencia de identidad.

### E8-H08 — Taxonomía de errores end-to-end (CoreError → AppError/exit-code) + recuperación
- **Objetivo**: el mapa estable de errores cruzando capas, con afford de recuperación.
- **Referencias**: `ARCHITECTURE.md §12` (errores), `§6` (WorkspaceError), `§7.1` (code estable).
- **Alcance**:
  - Taxonomía **fatal/recuperable/transitorio** + afford de recuperación.
  - Código **estable** cruzando `CoreError` → `WorkspaceError` → `AppError`/exit-code/`{code,message}`.
  - Supervisar el watcher (panic → restart + banner, nunca UI obsoleta en silencio) — verifica E5-H06.
- **Criterios de aceptación**:
  - Cada error tiene una clasificación y un código estable; el mismo error da el mismo exit code en CLI y el mismo `code` en Tauri.
  - Un panic del watcher se recupera con banner visible.
- **Dependencias**: E1-H02, E5-H06, E6-H01.
- **Pruebas**: tabla error→clasificación→código; recuperación del watcher.

### E8-H09 — Gate de rendimiento (bench con fixture sintética de 10k)
- **Objetivo**: el gate de bench que congela el presupuesto del `§11`.
- **Referencias**: `ARCHITECTURE.md §11`, `§5`/`§8`/`§13` (Barnes-Hut, proyecciones SQL, hash por path).
- **Alcance**:
  - Bench con la fixture de 10k (E0-H03): **cold open < ~2s**, **edit→UI < 150ms**, **grafo 60 fps** hasta N visibles.
  - Gate en CI que falla si se exceden los presupuestos.
  - Verifica los caminos de escala: proyecciones SQL (E3-H08), eventos delta (E5-H03), Barnes-Hut (E6-H08),
    hash por path (E4-H03), virtualización del árbol (E6-H05).
- **Criterios de aceptación**:
  - Los 3 presupuestos se cumplen con la fixture 10k; un regression los rompe en CI.
- **Dependencias**: E0-H03, E3-H03, E5-H03, E6-H05, E6-H08.
- **Pruebas**: el propio gate de bench en CI.

### E8-H10 — First-run end-to-end + verificación de `.lodestar/` ignorada
- **Objetivo**: cerrar el flujo de primer uso en CLI y GUI con la verificación idempotente de gitignore.
- **Referencias**: `ARCHITECTURE.md §12` (first-run), `§13.2`.
- **Alcance**:
  - `lodestar init` / "crear bundle" en GUI: scaffold de `index.md` raíz con `okf_version`, `.gitignore`
    (incluye `.lodestar/`), `git init` + commit inicial.
  - En cada `open` de repo existente: verificar `.lodestar/` ignorada (idempotente; oferta "dejar de trackear" si estaba trackeada).
  - Integra E2-H05, E4-H02, E6-H13.
- **Criterios de aceptación**:
  - First-run deja un bundle conforme con git inicializado y `.lodestar/` ignorada.
  - Abrir un repo con `.lodestar/` trackeada ofrece dejar de trackearla (sin romper nada si se declina).
- **Dependencias**: E2-H05, E4-H02, E6-H13.
- **Pruebas**: first-run CLI+GUI; verificación idempotente de gitignore.

### E8-H11 — Política CRDT-futuro y core sin I/O (nota de diseño verificada)
- **Objetivo**: documentar y **proteger por test** que el core sigue sin I/O para un futuro server `axum`/CRDT.
- **Referencias**: `ARCHITECTURE.md §12` (CRDT futuro), `§2.2`.
- **Alcance**:
  - Documentar que `build_raw` (canonicalización) + LWW por fichero sesga contra un CRDT por-bloque (decisión consciente).
  - Test/guard de CI que falla si `lodestar-core` adquiere una dependencia de I/O/DB/git/runtime
    (protege la reusabilidad por un futuro server `axum`).
- **Criterios de aceptación**:
  - La nota de diseño existe; el guard de CI rompe si el core importa `tokio`/`rusqlite`/`git2`/`notify`/`tauri`.
- **Dependencias**: E1 (todo el core).
- **Pruebas**: guard de dependencias del core en CI (p. ej. `cargo-deny`/script).

### E8-H12 — LFS / `.gitattributes` / firma: detección y avisos (degradar, no crashear)
- **Objetivo**: cerrar los no-goals de git con detección y avisos accionables.
- **Referencias**: `ARCHITECTURE.md §13.8`, `§12` (paridad git CLI).
- **Alcance**:
  - **commit** (libgit2) detecta blobs LFS / firma exigida y **avisa** (ofrece commit vía CLI), no commitea crudo.
  - tags/submódulos/worktrees/repos bare: **degradan, no crashean** (mensaje claro de "no soportado v1").
  - **push/pull** (binario `git`) sí respetan LFS/hooks (es acción explícita del usuario).
- **Criterios de aceptación**:
  - Un repo con regla LFS no se corrompe al commitear desde la app (se avisa).
  - Un submódulo/worktree/bare no crashea la app (degrada con aviso).
- **Dependencias**: E4-H05, E4-H07.
- **Pruebas**: repo con LFS → aviso; repo bare/submódulo → degradación sin crash.
