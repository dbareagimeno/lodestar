# E27 — Producto, distribución y apertura OSS

> **Fase**: posterior a v0.5.0. No es una fase de `§20.14` ni de `§19.8`: es la primera épica de
> **superficie externa** — el repo está técnicamente por delante de su adopción, y esta épica cubre
> lo que un adoptante ve antes de decidir: release con guardarraíles, README en inglés, demo
> ejecutable, docs de usuario, docs internas ordenadas y embudo de contribución.
> **Objetivo de la épica**: que instalar, probar y evaluar lodestar cueste minutos y no exija leer
> specs internas; que el pipeline de release no pueda repetir el fallo del tag `0.5.0`; y que el
> repo público tenga el embudo mínimo de contribución — sin cambiar ni una línea de comportamiento
> del motor.
> Referencias maestras: `ARCHITECTURE.md §21` (superficie externa y distribución, ratificada
> 2026-08-01) · `decisiones §17` (cierre de la puerta 1: crates.io diferido, issues-first,
> REFACTOR_PHASE_2 se queda, Covenant 2.1) · `decisiones §14` (la restricción transversal) ·
> `CLAUDE.md` (invariantes #1 y #6, que la superficie pública debe describir con honestidad).

**Origen**: review OSS externa (2026-08-01), evaluada y verificada punto a punto contra `main`
(v0.5.0). Confirmado: README sin binarios hasta los quick fixes, sin demo end-to-end, `docs/` sin un
solo documento de usuario, `.github/` con solo 2 workflows, y el tag `0.5.0` sin prefijo `v` que
nunca disparó `release.yml`. El retro-tag `v0.5.0` y los quick fixes de la rama
`chore/higiene-docs-release` (README enlaza Releases; RELEASING sin el aviso de repo privado y con la
advertencia del prefijo `v`; docstring de fixtures) **ya están hechos y no forman parte de esta
épica**.

**Principio rector**: *la superficie externa solo promete lo que el motor ejecuta hoy* (`§21.5`).
Corolario operativo, vigente mientras `decisiones §14` siga abierta: **ningún documento público
presenta `reindex`/la cache SQLite como camino de lectura del producto ni promete rendimiento a
escala** — la cache se describe como derivada y reconstruible, que es lo que es. Ante cualquier duda
de redacción, se ejecuta el comando y se pega la salida real.

**Regla de idioma** (`§21.1`, decisión D1): las historias H02, H03, H05, H08, H09 y H11 producen
superficie **pública en inglés**; H01, H06 y H07 tocan material **interno en español**. Esta épica,
como todo `requirements/`, se escribe en español.

**Fuera de alcance (explícito)**:

- **Conectar el store + watcher.** `decisiones §14` sigue abierta; irá a una épica posterior con
  puerta de diseño propia (cambia el camino de lectura de las 10 tools y toca el invariante #3).
  Esta épica solo la **respeta**: es la fuente de la regla transversal.
- **Firma/notarización de binarios.** Diferida en `RELEASING.md`; los checksums de H01 cubren la
  parte barata de la integridad.
- **Dogfooding y benchmarks.** Posteriores: el dogfooding necesita la superficie de esta épica; los
  benchmarks son el criterio de salida natural de la épica del store.
- **Cualquier cambio de comportamiento del motor o de la frontera MCP.** Ninguna historia toca
  `crates/*/src` ni `contracts/mcp.yml`: **cero deltas de contrato en toda la épica**.
- **La matriz de trazabilidad retroactiva de E15–E24** (`decisiones §16(k)`): es una historia
  propia con su propio criterio de verificación, no un apéndice de esta épica.

---

## Bloque A — El pipeline no puede volver a fallar así

### E27-H01 — Guardarraíles del pipeline de release

- **Objetivo**: que el fallo del tag `0.5.0` (release publicada con 0 assets) sea **imposible de
  repetir sin que el CI lo grite**, y que cada release publique checksums verificables.
- **Contexto del defecto**: el tag `0.5.0` sin prefijo `v` no matcheó `tags: ["v*"]` de
  `release.yml`, la release se publicó a mano con 0 assets y nadie lo detectó hasta la review
  externa. Además, nada impide hoy empujar `v0.6.0` sobre un árbol cuyo `workspace.package.version`
  sigue en `0.5.0`: los assets se nombrarían con una versión que el binario no declara.
- **Referencias**: `ARCHITECTURE.md §21.2` · `.github/workflows/release.yml` · `RELEASING.md`
  (pasos 4–6) · `decisiones §17` (la firma sigue diferida; esto no la sustituye).
- **Alcance**:
  - Un script `scripts/verifica-tag-release.sh` (interno, en español) que recibe el nombre del tag,
    extrae `version` de `[workspace.package]` en `Cargo.toml` y **sale con error** si el tag no es
    exactamente `v` + esa versión. Sin dependencias fuera de POSIX sh/grep/sed (corre en el runner
    de ubuntu sin instalar nada).
  - `release.yml` gana un step temprano en el job `create-release` que ejecuta ese script con
    `github.ref_name`. Si falla, **no se crea el release en borrador ni se compila nada**.
  - Cada job de `binaries` genera `SHA256SUMS-<target>.txt` con el checksum del tarball/zip que
    acaba de empaquetar y lo sube como asset del release (mismo `gh release upload --clobber`).
  - `RELEASING.md` documenta las dos guardas: el paso 4 avisa de que un tag desincronizado con
    `Cargo.toml` fallará en CI, y el paso 6 añade los ficheros de checksums a la lista de artefactos
    que hay que comprobar antes de publicar, con la línea de verificación para el usuario
    (`shasum -a 256 -c SHA256SUMS-<target>.txt`).
- **Fuera de alcance**: firmar los artefactos; agregar los checksums en un único fichero (un job de
  agregación añade complejidad sin valor: tres ficheros por-target bastan); tocar `ci.yml`.
- **Criterios de aceptación** (los tres primeros son las ejecuciones del script, pegadas en la PR;
  los demás, checklist):
  - **Dado** `Cargo.toml` con `version = "0.5.0"`, **Cuando** el script corre con el argumento
    `v0.5.0`, **Entonces** sale con código 0 → ejecución `tag_correcto`.
  - **Dado** ese mismo `Cargo.toml`, **Cuando** el script corre con `v0.6.0`, **Entonces** sale con
    código ≠0 y el mensaje nombra **las dos** versiones (la del tag y la del workspace) → ejecución
    `tag_desincronizado`.
  - **Dado** ese mismo `Cargo.toml`, **Cuando** el script corre con `0.5.0` (sin prefijo),
    **Entonces** sale con código ≠0 y el mensaje menciona el prefijo `v` obligatorio → ejecución
    `tag_sin_prefijo` (defensa en profundidad: cubre el disparo por `workflow_dispatch`, que no pasa
    por el filtro `tags: ["v*"]`).
  - `release.yml`: el step de verificación corre **antes** de `gh release create`, y los jobs
    `binaries` generan y suben `SHA256SUMS-<target>.txt` cuyo contenido verifica con
    `shasum -a 256 -c` contra el artefacto empaquetado (assert dentro del propio job, para que un
    checksum mal generado falle en CI y no en la máquina del usuario).
  - `RELEASING.md` documenta ambas guardas y la línea de verificación del usuario.
  - **Verificación diferida declarada**: el workflow completo solo se ejercita con un tag real; la
    PR lo deja escrito y la **próxima release** (`gh release view` mostrando 3 archivos + 3
    checksums) cierra la verificación. Es la misma limitación que tuvo siempre `release.yml` y no
    bloquea el Done de la historia si las ejecuciones locales del script están pegadas.
- **Dependencias**: ninguna.
- **Pruebas**: las tres ejecuciones locales del script (pegadas en la PR) · el assert de
  `shasum -c` dentro del job `binaries` · la próxima release real como verificación end-to-end.

---

## Bloque B — Demostrable en dos minutos

### E27-H03 — `examples/demo/`: un workspace que enseña el producto

- **Objetivo**: que exista un workspace real, pequeño y con defectos **deliberados**, contra el que
  se escriben (y se verifican) el quickstart del README y las docs de usuario.
- **Referencias**: `ARCHITECTURE.md §21.3` (demo como documentación ejecutable) · `§20`
  (comportamiento que la demo ejercita: frontmatter arbitrario, enlaces por path, grafo) ·
  `contracts/mcp.yml` (la semántica que el guion narra).
- **Alcance**:
  - `examples/demo/` con **8–12 documentos `.md` en inglés** (es superficie pública) organizados en
    2–3 subdirectorios, que ejerciten: frontmatter YAML variado con tipos reales (strings, números,
    listas, mapas anidados — p. ej. `service: {tier: …}`), documentos **sin** frontmatter, enlaces
    relativos entre profundidades distintas, **exactamente un enlace roto deliberado** y
    **exactamente un documento huérfano deliberado** (sin enlaces entrantes ni salientes), ambos
    señalados con un comentario en el propio documento para que nadie los «arregle» por error.
  - Un `examples/demo/.gitignore` que ignora `.lodestar/` **desde el origen**, para que ni la cache
    ni el runtime de transacciones ensucien el árbol al ejecutar el guion (y para que el motor no
    tenga que tocar un `.gitignore` versionado — `E25-H06`).
  - `examples/demo/README.md` (inglés): el **guion de 2 minutos** — `lodestar check` (mostrando el
    enlace roto y el huérfano), una consulta `where` sobre la metadata de la demo, un
    `impact_analyze` de mover un documento referenciado, y el ciclo `change_plan` → `change_apply` →
    `change_revert` por MCP dejando el workspace **exactamente** como estaba. Cada salida mostrada
    procede de una ejecución real.
  - El guion respeta el principio rector: no menciona `reindex` ni la cache como parte del recorrido.
- **Fuera de alcance**: que la demo cubra las 10 tools (cubre el recorrido narrativo, no el
  inventario); tests de cargo (la protección en CI es E27-H04); traducirla al español.
- **Criterios de aceptación**:
  - **Dado** `examples/demo/`, **Cuando** se ejecuta `lodestar check` desde su raíz, **Entonces** la
    salida reporta el enlace roto y el huérfano deliberados, y **solo** esos defectos (la demo no
    tiene defectos accidentales) → ejecución pegada en la PR; el assert permanente es de E27-H04.
  - **Dado** el guion completo del README de la demo, **Cuando** se ejecuta paso a paso, **Entonces**
    cada salida mostrada coincide con la real y, tras `change_revert`, `git status --porcelain` sobre
    `examples/demo/` queda vacío → ejecución pegada en la PR.
  - Checklist: 8–12 documentos; los dos defectos deliberados comentados en el fuente; `.gitignore`
    con `.lodestar/`; todo en inglés; ni `reindex` ni la cache aparecen en el guion.
- **Dependencias**: ninguna.
- **Pruebas**: las ejecuciones del guion pegadas en la PR; E27-H04 las convierte en asserts de CI.

### E27-H02 — README en inglés

- **Objetivo**: que la primera pantalla del repo —lo único que la mayoría de adoptantes leerá— esté
  en inglés, instale por binarios, conecte un cliente MCP y demuestre el producto contra la demo.
- **Referencias**: `ARCHITECTURE.md §21.1` (idioma) · `§21.5` (regla transversal) · `README.md`
  actual (la estructura y las afirmaciones ya calibradas por E22/E23 y los quick fixes: la
  traducción **no relaja** ninguna de sus precisiones) · `examples/demo/` (E27-H03).
- **Alcance**:
  - Reescritura completa de `README.md` en inglés, conservando la estructura actual (qué aporta,
    cómo trabaja un agente, inicio rápido, Markdown tal cual, grafo e impacto, cambios seguros, CLI,
    migración OKF, arquitectura, desarrollo, documentación, licencia).
  - **Inicio rápido** en tres pasos verificados: (1) instalar binarios desde Releases (con la
    verificación de checksums de E27-H01 mencionada en una línea) o `cargo install --git`; (2)
    `lodestar check` — con la demo como terreno de juego: clonar el repo y ejecutarlo contra
    `examples/demo/`, pegando la salida real; (3) conectar un cliente MCP, con el snippet genérico
    JSON **y** el ejemplo de una línea para Claude Code (`claude mcp add …`).
  - Sección de documentación actualizada: enlaza `docs/user/` (cuando exista, E27-H05/H11 la
    completan) y los documentos internos con la nota de que están en español por diseño (`§21.1`).
  - La sección de arquitectura mantiene la cache SQLite etiquetada como **derivada y reconstruible**
    y no añade ninguna afirmación de rendimiento; `reindex` se lista como comando de mantenimiento
    de esa cache, sin presentarla como camino de lectura (regla `§21.5`).
- **Fuera de alcance**: traducir CHANGELOG/RELEASING (internos, siguen en español); añadir capturas
  o GIFs; reordenar la arquitectura del documento.
- **Criterios de aceptación** (checklist binario):
  - El README está íntegramente en inglés (sin restos de secciones en español).
  - Todos los comandos del quickstart se ejecutaron tal cual y su salida pegada coincide con la real
    (comprobable de nuevo por E27-H04 para los que tocan la demo).
  - El snippet de Claude Code y el JSON genérico configuran un servidor funcional (probado con un
    cliente real; vale la transcripción de la sesión en la PR).
  - `grep -in "fts5\|fast\|performance\|scale" README.md` no devuelve ninguna promesa de
    rendimiento apoyada en la cache (la palabra puede aparecer; la **promesa**, no).
  - Ningún enlace interno del README apunta a un fichero inexistente.
- **Dependencias**: E27-H03 (el quickstart se escribe contra la demo).
- **Pruebas**: ejecución real de cada comando (pegada en la PR) · E27-H04 protege los pasos que
  tocan la demo.

### E27-H04 — Smoke de la demo en CI

- **Objetivo**: que README y demo **no puedan pudrirse en silencio**: un job de CI ejecuta el guion
  y aserta las salidas clave.
- **Referencias**: `ARCHITECTURE.md §21.3` · `.github/workflows/ci.yml` · `examples/demo/README.md`
  (el guion que este job mecaniza) · precedente: la lección de E23 («cuando dudes de si algo
  funciona, ejecútalo») convertida en infraestructura.
- **Alcance**:
  - Job nuevo `demo-smoke` en `ci.yml` (solo `ubuntu-latest`: el guion es agnóstico de plataforma y
    el job `rust` ya cubre las tres), que compila `lodestar-cli` y `lodestar-mcp` en debug y ejecuta
    contra `examples/demo/`:
    1. `lodestar check --json` y aserta (con `jq`) que aparecen el diagnóstico del enlace roto y el
       del huérfano deliberados, y que el exit code es el que el guion documenta;
    2. una sesión MCP por stdio (initialize → `knowledge_search` con la consulta del guion →
       `change_plan` → `change_apply` → `change_revert`) asertando que cada respuesta contiene el
       resultado clave que el guion muestra;
    3. tras el revert, `git status --porcelain -- examples/demo` **vacío**: el ciclo completo deja
       el árbol byte a byte como estaba (la garantía que el guion promete).
  - El job vive en un script (`scripts/demo-smoke.sh`) que también corre en local, para que el guion
    se pueda re-verificar sin empujar.
- **Fuera de alcance**: correr el smoke en macOS/Windows; cubrir las 10 tools; medir tiempos.
- **Criterios de aceptación** (mapeados a asserts del job):
  - **Dado** la demo intacta, **Cuando** corre el paso 1, **Entonces** el JSON de `check` contiene
    exactamente los dos defectos deliberados → assert `check_reporta_los_defectos_deliberados`.
  - **Dado** la sesión MCP del paso 2, **Cuando** se aplica y revierte el cambio del guion,
    **Entonces** cada respuesta intermedia contiene su resultado clave (el plan válido, el apply con
    recibo, el revert aplicado) → assert `el_ciclo_del_guion_responde_lo_documentado`.
  - **Dado** el final de la sesión, **Cuando** corre el paso 3, **Entonces**
    `git status --porcelain -- examples/demo` no imprime nada → assert
    `el_revert_deja_el_arbol_intacto`.
  - **Control anti-vacuo**: romper adrede un enlace más en la demo (en local, sin commitear) hace
    fallar el paso 1 — la ejecución se pega en la PR como prueba de que el job muerde.
  - El job aparece en `ci.yml` con un comentario de cabecera que explica qué protege (estilo de los
    jobs existentes) y `scripts/demo-smoke.sh` corre en local con el mismo resultado.
- **Dependencias**: E27-H02, E27-H03.
- **Pruebas**: el propio job (es el test) · la ejecución local del script pegada en la PR, incluido
  el control anti-vacuo.

---

## Bloque C — Documentación por audiencias

### E27-H06 — Reorganizar `docs/`: vigente vs. superseded

- **Objetivo**: que `docs/` distinga lo que gobierna de lo que es historia, sin romper ni una cita.
- **Referencias**: `ARCHITECTURE.md §21.3` · `decisiones §17` (decisión DC: el criterio es
  *vigente/superseded*, y `REFACTOR_PHASE_2.md` —citado por ~51 ficheros— **no se mueve**).
- **Alcance**:
  - Crear `docs/history/` y mover ahí los 4 documentos superseded: `REFACTOR.md`,
    `REFACTOR_DISENO_PROPUESTA.md`, `PROPUESTA_CLI.md`, `PROPUESTA_FIXES.md` (con `git mv`).
  - `docs/history/README.md` breve (español): qué es este directorio y por qué estos documentos ya
    no gobiernan (cada uno con una línea: qué lo supersedió).
  - `docs/README.md` nuevo: índice por audiencias — usuarios → `docs/user/` (inglés; H05/H11 lo
    pueblan), desarrollo del repo → `REFACTOR_PHASE_2.md` (spec vigente) y `WORKFLOWS.md`,
    arqueología → `docs/history/` y `prototype/`.
  - Actualizar **todas** las referencias a los 4 ficheros movidos en el resto del repo.
- **Fuera de alcance**: mover `REFACTOR_PHASE_2.md` o `WORKFLOWS.md` (decisión DC); editar el
  contenido de los documentos movidos (se mueven, no se reescriben); crear los docs de usuario.
- **Criterios de aceptación** (checklist binario):
  - `git log --follow` conserva la historia de los 4 ficheros movidos.
  - `grep -rn "docs/REFACTOR.md\|docs/REFACTOR_DISENO_PROPUESTA.md\|docs/PROPUESTA_CLI.md\|docs/PROPUESTA_FIXES.md" --include="*.md" --include="*.rs" --include="*.yml" .`
    no devuelve nada fuera de `docs/history/` (cero referencias a las rutas viejas).
  - `docs/README.md` existe, lista las tres audiencias y no enlaza ningún fichero inexistente.
  - `docs/REFACTOR_PHASE_2.md` sigue en su ruta (las ~51 citas siguen válidas sin tocarse).
- **Dependencias**: ninguna.
- **Pruebas**: los greps de arriba, pegados en la PR.

### E27-H05 — `docs/user/` operativos (inglés): quickstart, mcp-clients, ci

- **Objetivo**: que el camino operativo completo —instalar, conectar un cliente, poner `check` de
  puerta en CI— esté documentado para usuarios, en inglés, sin remitir a specs internas.
- **Referencias**: `ARCHITECTURE.md §21.1`/`§21.5` · `README.md` (E27-H02: el README resume, estos
  documentos desarrollan) · `examples/demo/` (E27-H03) · `RELEASING.md` (qué artefactos existen).
- **Alcance**:
  - `docs/user/quickstart.md`: instalación (binarios + checksums, `cargo install --git`), primer
    `check` contra la demo, lectura de la salida (severidades, exit codes), y el mapa de «qué leer
    después».
  - `docs/user/mcp-clients.md`: configuración por cliente — Claude Code (`claude mcp add`), el JSON
    genérico de `mcpServers`, `--root` y cuándo omitirlo, perfiles `readonly`/`standard` y qué
    oculta/rechaza cada uno, y un recorrido comentado de las 10 tools (qué pregunta responde cada
    una, sin duplicar la semántica del contrato: para el detalle remite a `contracts/mcp.yml`).
  - `docs/user/ci.md`: `lodestar check` como puerta de CI — exit codes congelados (0/1/2/3),
    `--json` y `--sarif`, ejemplo completo de workflow de GitHub Actions (verificado ejecutándolo), y
    subida del SARIF a code scanning.
  - Los tres documentos respetan la regla `§21.5` y cada comando/salida mostrado procede de una
    ejecución real contra la demo (o contra un workspace mínimo descrito en el propio documento).
- **Fuera de alcance**: query-language y safe-changes (E27-H11, la mitad de referencia); traducir
  nada al español; documentar tools o flags que no existen.
- **Criterios de aceptación** (checklist binario):
  - Los tres ficheros existen, en inglés, enlazados desde `docs/README.md` y desde el README raíz.
  - Cada bloque de comando+salida se ejecutó y coincide con la real (ejecuciones pegadas en la PR).
  - El ejemplo de GitHub Actions de `ci.md` se ejecutó tal cual (vale un workflow temporal en una
    rama, con el run enlazado en la PR).
  - Ningún documento presenta `reindex`/la cache como camino de lectura ni contiene promesas de
    rendimiento (mismo grep-criterio que E27-H02).
  - Ningún enlace roto (grep de rutas relativas).
- **Dependencias**: E27-H03, E27-H06.
- **Pruebas**: ejecuciones reales pegadas en la PR · el run del workflow de ejemplo.

### E27-H11 — `docs/user/` de referencia (inglés): query-language y safe-changes

- **Objetivo**: que el lenguaje de consulta y el modelo transaccional —las dos capacidades
  diferenciales del producto— tengan referencia de usuario, honesta con sus límites declarados.
- **Referencias**: `ARCHITECTURE.md §20.8` (lenguaje) · `§19.5` (transacciones) ·
  `contracts/mcp.yml` (la semántica que estas páginas narran y **no pueden contradecir**) ·
  `decisiones §12` (fechas como strings), `§16(a)` (límites de quoting declarados) ·
  `README.md §Cambios seguros` (el resumen que aquí se desarrolla).
- **Alcance**:
  - `docs/user/query-language.md`: sintaxis de `where` y su equivalente `filter` JSON, operadores y
    tipos, dot-paths y el anclaje `frontmatter.`, namespaces reservados (`graph.*`, `document.*`),
    `has`/`missing`, ejemplos ejecutados contra la demo — y una sección de **límites declarados**:
    fechas comparadas como strings (`§12`) y los tres casos de quoting que el dialecto no expresa
    (`§16.a`), contados como límites, no escondidos.
  - `docs/user/safe-changes.md`: el ciclo `change_plan` → `change_apply` → `change_revert`, qué
    valida el plan, el control optimista de concurrencia (revisiones, `REVISION_CONFLICT`,
    `PLAN_STALE`), los recibos y su retención, qué garantiza la recuperación ante crash (y qué no),
    y las operaciones disponibles — remitiendo a `contracts/mcp.yml` para el detalle de parámetros
    en vez de duplicarlo.
  - Ambos con ejemplos por el wire (peticiones y respuestas MCP reales, saneadas de rutas locales).
- **Fuera de alcance**: cambiar o «mejorar» semántica alguna al documentarla (si documentar destapa
  un defecto, se registra en `decisiones/`, no se arregla aquí); documentar el envelope de
  `lodestar-app` (sin llamantes, `§16.b`).
- **Criterios de aceptación** (checklist binario):
  - Los dos ficheros existen, en inglés, enlazados desde `docs/README.md` y el README raíz.
  - Cada ejemplo se ejecutó y su salida coincide (ejecuciones pegadas en la PR).
  - `query-language.md` documenta los límites de `§12` y `§16.a`; `safe-changes.md` no promete
    ninguna garantía que `§19.5`/el contrato no den.
  - Ninguna afirmación contradice `contracts/mcp.yml` (revisión cruzada declarada en la PR).
  - Ningún enlace roto.
- **Dependencias**: E27-H05 (comparten índice y convenciones; H05 fija el tono).
- **Pruebas**: ejecuciones reales pegadas en la PR · revisión cruzada contra el contrato.

### E27-H07 — `requirements/` dice la verdad sobre su propia historia

- **Objetivo**: que las épicas pre-giro queden marcadas como históricas **en los propios ficheros**,
  que la lista de invariantes de `requirements/README.md` deje de afirmar dos invariantes retirados,
  y que la regla de idioma ampliada quede registrada donde los agentes la leen.
- **Contexto del defecto** (review externa + verificación propia): `epica-00`…`epica-08` describen
  Tauri/VCS/OKF sin ninguna marca de histórico en los ficheros; y los «Invariantes que toda historia
  debe preservar» de `requirements/README.md` listan como vigentes el **#4** («el `.d.ts` se genera
  desde Rust» — el espejo TS desapareció al retirar la UI) y el **#7** («git con vocabulario
  directo; transporte híbrido» — retirado en `E15-H01`, `§20.13`).
- **Referencias**: `ARCHITECTURE.md §21.1` (regla de idioma) · `CLAUDE.md` (los 7 invariantes
  vigentes, con el #7 ya marcado RETIRADO — el patrón a replicar) · `§20.13`.
- **Alcance**:
  - Banner al inicio de los 9 ficheros `epica-00-scaffolding.md` … `epica-08-transversales.md`:
    histórica de v0.2.x, qué la supersedió (`§19`/`§20`), y que se conserva como registro — sin
    tocar el resto del contenido.
  - En `requirements/README.md`: el invariante #4 pasa a la formulación vigente (un solo contrato en
    `core::types`, sin capa DTO; lo derivado es el JSON Schema de `outputSchema` vía `schemars`; el
    espejo TS ya no existe) y el #7 se marca **RETIRADO** citando `E15-H01`/`§20.13`, replicando el
    tratamiento de `CLAUDE.md`. La «Definición de Done» deja de exigir gates del frontend retirado
    (`svelte-check`/`tsc`) y el «vocabulario git directo» de su punto 3.
  - En `requirements/README.md`, la viñeta **Idioma** registra la ampliación de `§21.1`: público en
    inglés / interno en español, con la cita.
- **Fuera de alcance**: reescribir el contenido de las épicas históricas; marcar E9–E14 (describen
  el giro vigente); la matriz de trazabilidad retroactiva (`§16.k`).
- **Criterios de aceptación** (checklist binario):
  - `grep -L "HISTÓRICA" requirements/epica-0[0-8]-*.md` no devuelve ningún fichero.
  - `grep -n "\.d\.ts" requirements/README.md` no devuelve ninguna línea que lo afirme como
    invariante vigente; `grep -n "vocabulario directo" requirements/README.md` devuelve solo la fila
    marcada RETIRADO (o nada).
  - La viñeta de idioma cita `§21.1` y distingue público/interno.
  - La «Definición de Done» no exige herramientas de un frontend que no está en `main`.
- **Dependencias**: ninguna.
- **Pruebas**: los greps de arriba, pegados en la PR.

---

## Bloque D — El embudo de contribución

### E27-H08 — CONTRIBUTING, SECURITY y código de conducta

- **Objetivo**: que un externo sepa cómo contribuir, cómo reportar una vulnerabilidad y bajo qué
  normas de convivencia — sin que ninguno de los tres documentos contradiga el proceso real del repo.
- **Referencias**: `ARCHITECTURE.md §21.4` · `decisiones §17` (DB: issues-first + Discussions
  OFF; DD: Covenant 2.1 + Private Vulnerability Reporting + email) · `CLAUDE.md` y
  `docs/WORKFLOWS.md` (el proceso que CONTRIBUTING destila **sin duplicar**: enlaza la autoridad, no
  la copia).
- **Alcance**:
  - `CONTRIBUTING.md` (inglés): la política issues-first (bugs y docs → PR directo con checklist;
    features → issue previa donde el mantenedor decide); cómo correr los gates locales (los comandos
    de `CLAUDE.md §Comandos`, incluida la feature `test-failpoints`); qué esperar del proceso (el
    repo se desarrolla con specs ratificadas y revisión estricta — se explica en dos frases y se
    enlaza `docs/WORKFLOWS.md`, sin exigírselo al contribuidor); la regla de idioma de `§21.1`
    explicada (por qué encontrará docs internas en español, y que el código y los commits van en
    español).
  - `SECURITY.md` (inglés): GitHub Private Vulnerability Reporting como canal primario,
    `dbareagimeno@icloud.com` como fallback; declaración de alcance honesta — motor local por stdio,
    sin red; la superficie de ataque relevante es parsing de Markdown/YAML y path-traversal
    (chokepoint `RelPath`, invariante #6); versiones soportadas (la última release).
  - `CODE_OF_CONDUCT.md`: Contributor Covenant 2.1 verbatim (inglés), con el email de contacto.
  - **Acciones de settings del repo** (manuales, del mantenedor, verificadas en los criterios):
    habilitar Private Vulnerability Reporting; comprobar que Discussions sigue OFF.
- **Fuera de alcance**: los templates de issue/PR (E27-H09); gobernanza multi-mantenedor; SLAs de
  respuesta.
- **Criterios de aceptación** (checklist binario):
  - Los tres ficheros existen en la raíz, en inglés; el CoC es el Covenant 2.1 sin ediciones (más
    allá del contacto).
  - CONTRIBUTING enuncia issues-first exactamente como `§17`-DB y **no duplica** el contenido de
    `WORKFLOWS.md` (lo enlaza).
  - Los comandos de gates locales listados coinciden con los de `CLAUDE.md` (incluido
    `--features test-failpoints` en los dos crates).
  - Private Vulnerability Reporting habilitado (captura o enlace en la PR) y Discussions OFF.
  - El community profile de GitHub reconoce los tres documentos.
- **Dependencias**: ninguna (las decisiones que consume se cerraron en `§17`).
- **Pruebas**: revisión + el community profile · verificación de settings pegada en la PR.

### E27-H09 — Templates de GitHub y roadmap

- **Objetivo**: que abrir un buen issue o una buena PR sea el camino de menor esfuerzo, y que
  «¿qué viene después?» tenga respuesta pública sin inventar un documento nuevo que mantener.
- **Referencias**: `ARCHITECTURE.md §21.4` · `CONTRIBUTING.md` (E27-H08, cuya política los
  templates operacionalizan) · `decisiones/` (el roadmap real del proyecto: sus decisiones
  abiertas).
- **Alcance**:
  - `.github/ISSUE_TEMPLATE/bug_report.yml` (inglés): versión de lodestar, plataforma, forma del
    workspace (nº de documentos, con/sin frontmatter), comando o tool exacto, salida de
    `lodestar check --json` si aplica, comportamiento esperado vs. observado.
  - `.github/ISSUE_TEMPLATE/feature_request.yml` (inglés): problema antes que solución; recuerda la
    política issues-first (las features se discuten aquí antes de ninguna PR).
  - `.github/ISSUE_TEMPLATE/config.yml`: `blank_issues_enabled: false` + enlace al canal de
    seguridad (que un reporte de vulnerabilidad **no** se abra como issue público).
  - `.github/PULL_REQUEST_TEMPLATE.md` (inglés): checklist alineada con los gates del CI (fmt,
    clippy `-D warnings`, tests incluidos los de `test-failpoints`, docs actualizadas si cambia la
    superficie) y la casilla «para features: enlaza la issue previa».
  - Roadmap: sección breve en el README (E27-H02) que apunta a `decisiones/` como registro vivo de
    decisiones abiertas (con la nota de que está en español), en vez de un `ROADMAP.md` paralelo que
    envejecería — mismo criterio anti-duplicación del invariante #4, aplicado a documentos.
- **Fuera de alcance**: bots, labels automáticos, CODEOWNERS, GitHub Projects.
- **Criterios de aceptación** (checklist binario):
  - Los formularios YAML validan (GitHub los renderiza al abrir un issue nuevo; captura en la PR).
  - `config.yml` desactiva los issues en blanco y enlaza el reporte privado de vulnerabilidades.
  - La checklist de la PR template no exige nada que el CI no exija (ni menos: los cuatro gates
    están).
  - El README contiene la sección de roadmap apuntando a `decisiones/`.
- **Dependencias**: E27-H02, E27-H08.
- **Pruebas**: el render de los formularios en GitHub (capturas en la PR).

---

## Bloque E — Diferido

### E27-H10 — Publicación en crates.io — **[BLOQUEADA por decisiones §17]**

- **Objetivo**: que `cargo install lodestar-cli` funcione desde el registry público.
- **Estado**: **bloqueada.** `decisiones §17` (mitad DA, reabrible) difirió la publicación:
  permanente por naturaleza, sin demanda demostrada, con la disponibilidad del nombre sin verificar
  y una colisión de marca registrada (el Lodestar de ChainSafe). Esta historia **no se detalla ni se
  arranca** hasta que esa mitad se reabra y se cierre en publicar.
- **Alcance previsto (esbozo, no spec)**: verificación de nombres; metadatos de publicación en los
  6 crates (`description`/`repository`/`keywords` — `lodestar-fixtures` sigue `publish = false`);
  publicación en el orden topológico ya documentado en `RELEASING.md`; README e
  instrucciones de instalación actualizados; decidir si el pipeline de release automatiza el
  `cargo publish`.
- **Dependencias**: reapertura y cierre de `decisiones §17-DA` · E27-H01 (el pipeline guardado
  es prerrequisito de automatizar nada más).

---

## Orden de construcción

```
H01 ─────────────────────────────┐
H03 ─→ H02 ─→ H04                │   (la demo primero: el README se escribe contra ella,
        │                        │    y el smoke protege a ambos)
H06 ─→ H05 ─→ H11                │
H07  (independiente)             │
H08 ─→ H09  (H09 también ← H02)  │
                                 └─→ [H10] bloqueada
```

Secuencia recomendada (una sola persona/agente, sin paralelismo):
**H01 → H03 → H02 → H04 → H06 → H05 → H11 → H07 → H08 → H09 → [H10]**.

**H01 va primera** porque es la única con riesgo operativo (la próxima release ya debe salir
guardada). **H03 antes que H02**: el quickstart se escribe contra la demo, no al revés. **H04 en
cuanto existen ambos**, para que el resto de la épica trabaje con la red puesta. **H06 antes que
H05/H11** para escribir el índice de `docs/` una sola vez. **H07–H09** no dependen entre sí, pero el
orden dado evita conflictos en `requirements/README.md` y deja los templates (H09) para cuando
CONTRIBUTING (H08) y el README (H02) ya existen.

## Proceso por historia

| Proceso | Historias | Por qué |
|---|---|---|
| **Verificación ejecutable** (el test es el script/job; ciclo corto, sin fase roja de cargo) | H01 · H04 | Su comportamiento vive en CI: las ejecuciones del script (H01) y los asserts del job con su control anti-vacuo (H04) son la evidencia |
| **Documentación con criterios binarios** (sin TDD; ratificación + checklist + ejecuciones pegadas) | H02 · H03 · H05 · H06 · H07 · H08 · H09 · H11 | No hay código de producto que testear; la honestidad se verifica ejecutando cada comando documentado y con los greps declarados |
| — | H10 | Bloqueada; no se arranca |

**Ninguna historia toca la frontera MCP**: cero secciones «Delta de contrato» en esta épica.
`contracts/mcp.yml` solo aparece como **fuente** que H05/H11 no pueden contradecir.

## Criterio de salida

La próxima release sale con el tag verificado contra la versión del workspace y con checksums como
assets; el README en inglés instala por binarios, conecta un cliente MCP y demuestra el producto
contra `examples/demo/`; el guion de la demo corre en CI y muerde si algo deja de ser verdad;
`docs/user/` cubre quickstart, clientes MCP, CI, lenguaje de consulta y cambios seguros sin afirmar
nada que el motor no ejecute; `docs/` distingue vigente de superseded sin una cita rota;
`requirements/` marca su propia historia y no lista invariantes retirados; y el repo público tiene
CONTRIBUTING, SECURITY, código de conducta y templates coherentes con la política issues-first de
`decisiones §17`. crates.io queda diferido y registrado, no olvidado.
