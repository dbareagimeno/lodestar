# E33 — Épica de evidencia: banco de pruebas permanente y gate de rendimiento

> **Origen**: `decisiones §9` punto 1 (gate de rendimiento, prio 4, **condición de entrada de
> `decisiones §14`**) + orden de trabajo acordado de `decisiones/README.md`, punto 3 («épica de
> evidencia — banco de pruebas + dogfooding; §23 aporta la primera corrida completa y el arnés
> re-ejecutable; cierra §14 **con datos**»). Diseño **ratificado el 2026-08-10** como
> [`ARCHITECTURE.md §22`](../ARCHITECTURE.md) (D1–D9 de la puerta de diseño, todas con la
> recomendación aceptada sin correcciones).
>
> **Objetivo de la épica**: convertir el testbench de `decisiones §23` (`docs/qa/testbench/`, 189
> casos contra el homelab) en un **banco permanente por release** con veredicto mecánico, medir
> cold-open y coste por llamada MCP a ~10k documentos en tres variantes de camino de lectura, y
> producir —junto al dogfooding acotado— el **paquete de evidencia** que deja `decisiones §14`
> lista para decidir. La épica produce datos y análisis; **no cierra `§14`, ni `§22`, ni `§24`**:
> esas decisiones son del usuario.
>
> Referencias maestras: `ARCHITECTURE.md §22` (el diseño ratificado, autoridad de esta épica) ·
> `§21.5` (mientras `§14` siga abierta, nada de lo medido se promete fuera) · `decisiones §9`
> punto 1 · `decisiones §14` (incluidas sus absorciones `§16(c)` watcher y `§16(l)` `field_path`) ·
> `decisiones §22`/`§24` (centinelas, sin cerrarlas) · `decisiones §23` + informe
> [`docs/qa/informe-homelab-2026-08-06.md`](../docs/qa/informe-homelab-2026-08-06.md) (el activo
> heredado) · `crates/lodestar-app/tests/escala.rs` (E14-H05, el arnés de escala que se reutiliza) ·
> `RELEASING.md` (el runbook que H07 amplía) · `CLAUDE.md` invariantes #1–#3 (el bench mide, no
> conecta: SQLite sigue siendo cache derivada y el core sigue siendo la verdad).

**Principio rector**: *medir antes que opinar, registrar antes que decidir*. El banco produce
hechos re-ejecutables; toda decisión —los umbrales (puerta interna de H05), el destino del store
(`§14`), las fichas abiertas (`§22`/`§24`)— es del usuario y ocurre **fuera** del banco, con los
números delante. Ante la duda de si un caso del banco debe «arreglar» algo: no — fija el statu quo
y cita la ficha.

**Frontera MCP**: **ninguna historia toca `contracts/mcp.yml`** ni el comportamiento de ninguna
tool. La épica añade herramientas de medición y verificación *alrededor* del motor (scripts del
banco, un crate interno `publish = false`, docs); el único código Rust nuevo consume APIs públicas
existentes de `lodestar-app`/`lodestar-store`/`lodestar-fixtures`. Cero delta de contrato,
explícitamente.

## Historias

| ID | Título | Frontera | Puerta |
|---|---|---|---|
| E33-H01 | Corpus canónico determinista y generador de escala compartido | no | — |
| E33-H02 | Runner asertable y portable del banco de conformidad | no | — |
| E33-H03 | Centinelas de las decisiones abiertas `§22` y `§24` | no | — |
| E33-H04 | Banco de rendimiento: primera corrida completa (3 variantes × 3 escalas) | no | — |
| E33-H05 | Umbrales ratificados, gate codificado y smoke en CI | no | **PUERTA INTERNA: ratificación de umbrales (D4)** |
| E33-H06 | Dogfooding acotado con registro | no | — |
| E33-H07 | Enganche a release: runbook, workflow y corrida datada | no | — |
| E33-H08 | Paquete de evidencia para decidir `decisiones §14` | no | — |

---

## E33-H01 — Corpus canónico determinista y generador de escala compartido

- **Objetivo**: los dos corpus del banco existen y son reproducibles — el corpus canónico de
  conformidad (generado con semilla fija, con la fauna completa de diagnósticos) y el generador de
  escala con perfiles plano/realista a tres tamaños —, y el homelab deja de ser un requisito.
- **Referencias**: `ARCHITECTURE.md §22.2` · `crates/lodestar-app/tests/escala.rs` (E14-H05:
  `genera_workspace_grande`/`cuerpo_grande`, el generador que se extrae) ·
  `docs/qa/testbench/make_fixtures.py` (los 15 sets patológicos que se integran) ·
  `crates/lodestar-fixtures` (el crate compartido de fixtures, `publish = false`) · regla del repo:
  fixtures grandes generadas en runtime, nunca commiteadas.
- **Alcance**:
  - `docs/qa/testbench/make_corpus.py`: genera el **corpus canónico** (~50–100 `.md`) de forma
    **determinista** (semilla fija, sin timestamps ni orden de iteración no determinista): grafo de
    enlaces con huérfanos, dangling y case-mismatch; frontmatter heterogéneo y consultable (tipos
    mezclados, fechas, listas, campos de referencia estilo `relacionadas:`); los sets patológicos
    de `make_fixtures.py` integrados; y las semillas de los centinelas de H03 (una referencia de
    frontmatter rota; un par de paths que difieren solo en caja y un par NFC/NFD).
  - Generador Rust de escala **extraído a `lodestar-fixtures`** desde `escala.rs`, parametrizado
    por tamaño (~100 / ~1k / ~10k) y semilla, con dos perfiles: **plano** (el actual de E14-H05,
    homogéneo, para comparabilidad con sus cifras) y **realista** (enlaces entre documentos,
    frontmatter consultable, tamaños heterogéneos).
  - `crates/lodestar-app/tests/escala.rs` pasa a consumir el generador compartido **sin cambiar
    sus aserciones** (mismas propiedades, mismo corpus plano de 10k).
  - `docs/qa/testbench/README.md` (esqueleto): qué genera cada script y cómo (H02 lo completa).
- **Fuera de alcance**: el runner y el formato `expect` (H02); medir nada (H04); commitear corpus
  como ficheros al árbol; tocar `lodestar-core` (invariante #2: el generador vive en
  `lodestar-fixtures`, no en el core).
- **Criterios de aceptación**:
  - **[BDD-1] Dado** la misma semilla, **Cuando** el generador Rust genera dos veces el mismo
    perfil y tamaño, **Entonces** los dos árboles son byte-idénticos
    → test: `generador_de_escala_es_determinista_con_la_misma_semilla`
    (`crates/lodestar-fixtures`).
  - **[BDD-2] Dado** el perfil realista a escala mínima, **Cuando** se abre con `App::open` y se
    corre `knowledge_check`, **Entonces** el corpus contiene documentos enlazados, huérfanos y al
    menos un diagnóstico de enlace — no es un corpus trivial
    → test: `perfil_realista_produce_grafo_y_diagnosticos` (`crates/lodestar-fixtures` o
    `lodestar-app`, donde ya hay dev-deps de apertura).
  - **[BDD-3] Dado** el arnés de escala de E14-H05 migrado al generador compartido, **Cuando**
    corre `cargo test -p lodestar-app`, **Entonces** `bench_search_payload_acotado` y
    `bench_concurrencia_segura` siguen en verde con sus aserciones intactas
    → tests existentes de `escala.rs` (sin renombrar).
  - **[Estructural]** `make_corpus.py` con la misma semilla produce dos árboles con el mismo hash
    recursivo (comando de verificación documentado en el README del banco); ningún corpus generado
    entra al git del repo; `cargo test --workspace --locked`, fmt, clippy `-D warnings` en verde;
    cero cambios en `crates/lodestar-core` y cero delta en `contracts/mcp.yml`.
- **Dependencias**: ninguna.
- **Pruebas**: los dos tests nuevos de arriba + `escala.rs` migrado como no-regresión; para python,
  la verificación de hash del README (estructural, sin framework de test).

---

## E33-H02 — Runner asertable y portable del banco de conformidad

- **Objetivo**: el testbench deja de necesitar veredicto manual y rutas de una máquina concreta:
  casos con `expect` evaluable, veredicto mecánico PASS/FAIL con exit code, y el corpus canónico de
  H01 como campo de pruebas por defecto.
- **Referencias**: `ARCHITECTURE.md §22.1`/`§22.3` · `docs/qa/testbench/lodestar_harness.py` (el
  arnés que se amplía: sesiones, worktrees, placeholders `@stepN`, regla dura de readonly) ·
  informe 2026-08-06 §5 (los invariantes transversales verificados conformes que se portan) ·
  `docs/qa/testbench/batches/verify_*.json` (las 18 repros que se portan) · `decisiones §23`
  («re-ejecutable contra cada release»).
- **Alcance**:
  - **Portabilidad**: `BINARY` y `HOMELAB` dejan de estar hardcodeados — el binario se toma de
    `--binary`/`LODESTAR_MCP_BIN` (con fallback a `target/release/lodestar-mcp` relativo al repo) y
    el root real es un argumento. La **regla dura se generaliza**: contra cualquier root declarado
    real (no desechable), solo `readonly`.
  - **Formato `expect`** por caso/paso: código de error esperado (`error_code`), aserciones sobre
    subcampos de `structured` (igualdad, presencia/ausencia), `is_error`, y los invariantes
    transversales (p. ej. «`workspaceRevision` idéntica entre el paso i y el j»). El runner emite
    veredicto por caso, resumen agregado, y **exit ≠ 0 si hay algún FAIL**. Los casos sin `expect`
    quedan soportados como «exploratorios» (se ejecutan, no computan al veredicto).
  - **Porte de la selección asertable**, reescrita contra el corpus canónico de H01: las 18
    `verify_*` (regresiones de los hallazgos de `§23`), los invariantes transversales del informe
    §5 (hash determinista de planes, revert byte a byte, familia de conflictos, exit codes de la
    CLI, cursores firmados, equivalencia `where ≡ filter` en errores…), y **al menos un caso por
    lote temático** de la matriz original (L1–L12, G, H).
  - Los lotes y matrices históricos del homelab **se conservan tal cual** como campaña exploratoria
    repetible (modo `--root`), fuera del gate.
  - `docs/qa/testbench/README.md` completo: cómo se corre (banco canónico, campaña homelab,
    caso suelto), cómo se añade un caso con `expect`, qué cubre y qué no.
- **Fuera de alcance**: los centinelas de `§22`/`§24` (H03); rendimiento (H04); el enganche a
  CI/release (H05/H07); reescribir el arnés en Rust; tocar el motor o el contrato.
- **Criterios de aceptación**:
  - **[BDD-1] Dado** un caso cuyo `expect` casa con la respuesta real, **Cuando** corre el lote,
    **Entonces** el caso es PASS y el runner sale con exit 0
    → caso de autotest del banco: `META-01` (lote `docs/qa/testbench/batches/meta_runner.json`,
    corre contra el corpus canónico).
  - **[BDD-2] Dado** un caso cuyo `expect` **no** casa (esperado deliberadamente falso), **Cuando**
    corre el lote, **Entonces** el caso es FAIL, el resumen lo lista con el subcampo que discrepó,
    y el runner sale con exit ≠ 0 → caso de autotest `META-02` (mismo lote, invocado por el README
    como demostración; no forma parte del gate).
  - **[BDD-3] Dado** el banco completo sobre el corpus canónico recién generado, **Cuando** corre
    `run_all` (el modo que ejecuta todos los lotes del gate), **Entonces** termina con 0 FAIL
    → la corrida completa commiteada como resultado datado (primera corrida del banco nuevo).
  - **[Estructural]** Ni una ruta absoluta de máquina en `docs/qa/testbench/` (grep sin
    `/Users/`); la regla dura generalizada está implementada y documentada; los 18 `verify_*`
    portados conservan la referencia a su hallazgo de origen (`§23` fila / caso del informe);
    README completo.
- **Dependencias**: E33-H01.
- **Pruebas**: los casos `META-*` (el banco se prueba a sí mismo) + la corrida completa en verde
  sobre el corpus canónico.

---

## E33-H03 — Centinelas de las decisiones abiertas `§22` y `§24`

- **Objetivo**: el banco fija el **statu quo** de las dos fichas abiertas nacidas del dogfooding —
  silencio ante referencias de frontmatter rotas (`§22`) y colisión byte a byte por caja/Unicode
  (`§24`) — de modo que cualquier cambio de comportamiento, deliberado o accidental, haga fallar
  un caso que cita la ficha. **Sin cerrar ninguna de las dos.**
- **Referencias**: `ARCHITECTURE.md §22.3` (centinelas) ·
  `decisiones/22-integridad-referencial-frontmatter.md` ·
  `decisiones/24-equivalencia-caja-unicode.md` · `E28-H02`/`E28-H04` (el guard de colisión byte a
  byte cuyo comportamiento vigente se fija) · semillas del corpus creadas en H01.
- **Alcance**:
  - **Centinela `§22`** (lote `sentinela_s22.json`): sobre el corpus canónico, un documento con
    `relacionadas: [99]` (referencia a ficha inexistente) y un `affects: [typo-inexistente]`;
    esperado = comportamiento **vigente**: `knowledge_check` no emite ningún diagnóstico por ello
    (`valid: true`, silencio), y `metadata_inspect {mode: field}` sobre el campo muestra el valor
    huérfano con recuento 1 — el control manual que la propia ficha documenta como «el 80 % que ya
    se puede hacer hoy». El caso cita `decisiones §22` en su descripción.
  - **Centinela `§24`** (lote `sentinela_s24.json`, siempre sobre fixture desechable): un
    `change_plan` de `create` hacia un path que difiere **solo en caja** de uno existente, y otro
    hacia la forma NFD de un nombre existente en NFC; esperado = comportamiento **vigente**: el
    guard compara byte a byte, así que `canApply: true` sin `DOCUMENT_ALREADY_EXISTS`. El caso se
    queda en el **plan** (no aplica el cambio: el banco no reproduce la pérdida de datos, la
    documenta) y cita `decisiones §24` y la dependencia de plataforma (en APFS/NTFS el apply
    fusionaría; en ext4 coexistirían).
  - Anotación **no decisoria** en ambas fichas: una línea «centinela en el banco: caso X de
    `docs/qa/testbench/`» + `revisada_en` actualizado. **El `estado` de ambas no cambia.**
- **Fuera de alcance**: decidir o cerrar `§22`/`§24`; cualquier cambio de motor o de contrato;
  ejecutar el `change_apply` destructivo del centinela `§24`.
- **Criterios de aceptación**:
  - **[BDD-1] Dado** el corpus canónico con la referencia de frontmatter rota, **Cuando** corre el
    centinela, **Entonces** `knowledge_check` da `valid: true` sin diagnóstico por la referencia y
    `metadata_inspect` muestra el valor huérfano → caso `S22-01` (+ `S22-02` para
    `metadata_inspect`).
  - **[BDD-2] Dado** un `create` hacia el gemelo por caja y otro hacia el gemelo NFD, **Cuando**
    se planifica, **Entonces** ambos planes devuelven `canApply: true` sin
    `DOCUMENT_ALREADY_EXISTS` → casos `S24-01` (caja) y `S24-02` (NFD).
  - **[Estructural]** Los cuatro casos citan su ficha; las dos fichas llevan la anotación del
    centinela con `revisada_en: 2026-08-XX` y **`estado` intacto** (`abierta`); los centinelas
    forman parte del `run_all` del gate.
- **Dependencias**: E33-H01, E33-H02.
- **Pruebas**: los casos `S22-*`/`S24-*` dentro del banco; su naturaleza es ser tests.

---

## E33-H04 — Banco de rendimiento: primera corrida completa (3 variantes × 3 escalas)

- **Objetivo**: los números que `decisiones §9` punto 1 pide y `§14` exige como condición de
  entrada: cold-open y coste por llamada (p50/p95) más payload, a ~100/~1k/~10k documentos, en las
  tres variantes de camino de lectura — disco-reparseo (producto actual), SQLite-raw
  (`Store::document_set()`, la cache tal como existe) y RAM-memoizado (cota superior).
- **Referencias**: `ARCHITECTURE.md §22.4`/`§22.5` · `decisiones §14` («las mediciones de E14-H05
  son el rendimiento real del producto, no el de la cache») · `crates/lodestar-app/tests/escala.rs`
  (patrón: medir servicios de `App`, registrar) · `Store::rebuild`/`Store::document_set`
  (`crates/lodestar-store/src/lib.rs`) y `DocumentSet::from_store`
  (`crates/lodestar-core/src/store_trait.rs` — **re-parsea los `raw`**: la advertencia que el
  informe debe rotular) · generador compartido de H01 · invariantes #1–#3 (medir, no conectar).
- **Alcance**:
  - Crate interno **`crates/lodestar-bench`** (`publish = false`, como `lodestar-fixtures`), con un
    binario que: genera el corpus (generador de H01, perfiles plano y realista), y mide —
    **cold-open** (`App::open` + `workspace_status`) y **p50/p95 de N iteraciones** por tool de
    lectura (`workspace_status`, `knowledge_search` con `where`, `knowledge_get`,
    `metadata_inspect`, `graph_query`, `impact_analyze`, `knowledge_check`) más el ciclo
    `change_plan`→`change_apply`, con **payload en bytes** por respuesta serializada — en las
    **tres variantes**: (1) el producto actual; (2) `Store::rebuild()` + `Store::document_set()`
    por la API pública, **registrando también el coste del `rebuild`**; (3) un `DocumentSet`
    construido una vez y reutilizado entre llamadas.
  - **Salida en formato estable**: JSON por corrida + resumen Markdown datado (máquina, binario,
    commit, semilla), pensado para comparar releases. La **primera corrida completa a 10k** se
    commitea datada en `docs/qa/`.
  - **Calibración de wire**: una muestra (al menos `workspace_status` y `knowledge_search` a 10k)
    medida también por el arnés python contra el binario `lodestar-mcp` real, para acotar el
    overhead del framing JSON-RPC/stdio frente a la medición sobre `App`.
  - El binario acepta `--smoke` (escala mínima, pocas iteraciones) — H05 lo usa en CI.
- **Fuera de alcance**: umbrales y gate (H05 — esta historia **mide y registra, no juzga**);
  conectar el store al camino de lectura del producto o alinear su walker (eso es la épica
  posterior a `§14`); optimizar nada; prometer nada en la superficie externa (`§21.5`).
- **Criterios de aceptación**:
  - **[BDD-1] Dado** el corpus plano a escala mínima, **Cuando** corre el banco en modo smoke,
    **Entonces** el informe JSON contiene las 3 variantes × todas las tools con p50/p95 y payload,
    y las tres variantes reportan el **mismo** número de documentos y los mismos resultados
    funcionales de una consulta de control (las variantes miden lo mismo, no cosas distintas)
    → test: `informe_completo_y_variantes_equivalentes_en_smoke` (`crates/lodestar-bench`).
  - **[BDD-2] Dado** dos corridas smoke con la misma semilla, **Cuando** se comparan sus informes,
    **Entonces** la estructura (claves, escalas, tools, variantes) es idéntica — solo cambian los
    tiempos → test: `formato_de_informe_estable` (`crates/lodestar-bench`).
  - **[Estructural]** La corrida completa real (3 escalas, 3 variantes, ambos perfiles) está
    commiteada datada en `docs/qa/`; la calibración de wire está en el mismo informe; el resumen
    MD rotula explícitamente la advertencia de `from_store` (SQLite-raw ahorra walk+IO, no parse);
    `cargo tree -p lodestar-core` sigue puro (el bench no añade nada al core) y el workspace
    compila con clippy `-D warnings`; cero delta en `contracts/mcp.yml`.
- **Dependencias**: E33-H01.
- **Pruebas**: los dos tests del crate `lodestar-bench` en modo smoke (rápidos, entran a la suite
  normal) + la corrida real como artefacto commiteado.

---

## E33-H05 — Umbrales ratificados, gate codificado y smoke en CI

> **PUERTA INTERNA (D4, ratificada)**: esta historia tiene dos mitades separadas por una
> ratificación del usuario. La primera mitad **presenta** los números de H04 con una propuesta de
> umbrales (anclas acordadas: **p95 ≤ 1 s por tool de lectura a 10k** y **cold-open ≤ 5 s**, sobre
> la variante disco-reparseo, que es el producto). **Nada de la segunda mitad se implementa hasta
> que el usuario ratifique los umbrales con la primera corrida delante.** No es una
> `[BLOQUEADA por decisiones §N]` — la decisión no existe aún porque su input es H04 — pero el
> efecto es el mismo: sin ratificación no hay gate.

- **Objetivo**: convertir la medición en **gate**: umbrales explícitos ratificados con datos, un
  modo `--gate` que falla si se violan, baseline por máquina para tendencia, y un smoke en CI que
  impide que el banco se pudra.
- **Referencias**: `ARCHITECTURE.md §22.4` · `decisiones §9` punto 1 («con umbrales») · H04 (los
  números) · `.github/workflows/ci.yml` (donde entra el smoke).
- **Alcance**:
  - **Mitad 1 (pre-puerta)**: documento breve de propuesta de umbrales derivado de la corrida de
    H04 (por tool y escala, con las anclas como punto de partida y los p95 medidos al lado), para
    la conversación de ratificación.
  - **Mitad 2 (post-puerta)**: modo `--gate` en `lodestar-bench`: compara la corrida contra los
    umbrales ratificados (codificados en un fichero versionado, p. ej.
    `docs/qa/testbench/umbrales.json`, con fecha de ratificación) **y** contra la baseline de la
    máquina (fichero de baseline identificado por host, commiteado en `docs/qa/`); **exit ≠ 0** con
    el detalle de qué umbral se violó. Los absolutos **solo se juzgan en la máquina de la
    baseline**; en cualquier otra, `--gate` degrada a comparación de tendencia y lo dice.
  - **Smoke en CI**: step en `ci.yml` que ejecuta `cargo run -p lodestar-bench -- --smoke` (escala
    mínima, **sin** juzgar umbrales absolutos): garantiza que el banco compila, corre y produce un
    informe bien formado en cada PR.
- **Fuera de alcance**: correr el gate completo (10k) en CI por PR; umbrales sobre las variantes
  SQLite-raw/RAM-memo (miden alternativas, no el producto); cualquier promesa externa de
  rendimiento (`§21.5` sigue vigente).
- **Criterios de aceptación**:
  - **[PUERTA] Dado** la primera corrida de H04, **Cuando** se presenta la propuesta de umbrales,
    **Entonces** existe constancia escrita de la ratificación del usuario (documento/PR) **antes**
    de cualquier commit de la mitad 2. *(Criterio de proceso, binario.)*
  - **[BDD-1] Dado** umbrales ratificados y una corrida que los viola, **Cuando** corre `--gate` en
    la máquina de la baseline, **Entonces** exit ≠ 0 y el informe nombra el umbral violado, la tool
    y el valor medido → test: `gate_falla_con_umbral_violado` (`crates/lodestar-bench`, con
    umbrales sintéticos imposiblemente bajos sobre una corrida smoke).
  - **[BDD-2] Dado** una corrida que respeta los umbrales, **Cuando** corre `--gate`, **Entonces**
    exit 0 y el informe registra los márgenes → test: `gate_pasa_con_umbrales_holgados` (ídem, con
    umbrales sintéticos altísimos).
  - **[BDD-3] Dado** una máquina distinta a la de la baseline, **Cuando** corre `--gate`,
    **Entonces** no juzga absolutos, compara tendencia contra su propia baseline si existe y lo
    declara en la salida → test: `gate_degrada_a_tendencia_fuera_de_la_maquina_baseline`.
  - **[Estructural]** `umbrales.json` versionado con la fecha y referencia de la ratificación; el
    step de smoke está en `ci.yml` y el CI del PR lo ejecuta en verde; los umbrales ratificados
    quedan reflejados en `ARCHITECTURE.md §22.4` (adenda de una línea con los valores).
- **Dependencias**: E33-H04 (y su puerta interna de ratificación).
- **Pruebas**: los tres tests de `lodestar-bench` sobre corridas smoke con umbrales sintéticos; el
  step de CI como prueba estructural.

---

## E33-H06 — Dogfooding acotado con registro

- **Objetivo**: la otra mitad del dato de `decisiones §14`: si el reparseo por llamada **molesta en
  el uso real** (~100 documentos: `decisiones/` + `requirements/` de este repo) o solo se nota en
  el arnés sintético — percepción registrada + número frío, con ventana acotada.
- **Referencias**: `ARCHITECTURE.md §22.7` · `decisiones §14` («el dogfooding es la otra mitad del
  dato») · `decisiones/README.md` §«Cómo consultarlas» (las queries que se ejecutan de verdad) ·
  `decisiones §22` (el precedente: el dogfooding produce hallazgos con ficha, no arreglos por
  inercia).
- **Alcance**:
  - El repo se declara y usa como workspace lodestar en las sesiones de trabajo: las consultas
    operativas de `decisiones/README.md` (`prioridad >= 4 and estado = "abierta"`, `etiquetas
    contains …`, `revisada_en < …`) se ejecutan **vía el MCP real** (p. ej. arnés en modo
    `--call`, o el servidor configurado en el cliente de agentes), en `readonly` contra el árbol
    real.
  - **Diario de fricciones** `docs/qa/dogfooding-2026-08.md`: una entrada por sesión (fecha · tool
    · qué se preguntó · fricción observada · latencia percibida), incluyendo las sesiones sin
    fricción — la ausencia también es dato.
  - **Número frío**: corrida del banco de rendimiento (H04) a escala «repo real» sobre un snapshot
    del propio repo (~100 docs), tres variantes, anexada al diario.
  - **Cierre de ventana**: el diario termina con un veredicto explícito («a esta escala el reparseo
    molesta / no molesta», con los números al lado) y la declaración de que lo que llegue después
    alimenta fichas nuevas, no reabre este dato.
- **Fuera de alcance**: instrumentar el motor (telemetría); arreglar fricciones encontradas (se
  registran como candidatas a ficha, como hicieron `§22`/`§24`); prolongar la ventana más allá del
  cierre de H08.
- **Criterios de aceptación**:
  - **[Estructural]** El diario existe con ≥ 3 sesiones de uso real registradas, cada una con las
    queries ejecutadas de verdad (no transcritas de memoria); contiene la corrida a escala repo
    (tres variantes) y el veredicto de cierre explícito; toda fricción nueva con entidad de
    decisión quedó registrada como candidata (no arreglada por inercia).
  - **[BDD-1] Dado** el snapshot del repo real, **Cuando** corre el banco de H04 a esa escala,
    **Entonces** el informe incluye las tres variantes sobre ese corpus → la corrida anexada al
    diario (artefacto, generada por el binario ya testeado en H04).
- **Dependencias**: E33-H04 (el número frío; el uso y el diario pueden empezar antes y en
  paralelo).
- **Pruebas**: el diario y su corrida anexa son el entregable verificable; no hay tests nuevos de
  código (H04 ya prueba el binario).

---

## E33-H07 — Enganche a release: runbook, workflow y corrida datada

- **Objetivo**: el banco pasa de re-ejecutable a **permanente por release**: paso del runbook en
  `RELEASING.md`, workflow manual opcional, y el patrón de corrida datada commiteada — la
  definición literal de lo que `decisiones/README.md` pedía para `§9`.
- **Referencias**: `ARCHITECTURE.md §22.6` · `RELEASING.md` (pasos 1–8 actuales; el nuevo entra
  tras el paso 2) · decisión de la puerta de diseño: el paso del runbook se escribe **aquí**, no en
  la adenda, para que el runbook nunca instruya correr una herramienta inexistente ·
  `.github/workflows/` (el `workflow_dispatch`).
- **Alcance**:
  - **Paso nuevo en `RELEASING.md`** (entre el changelog y el PR a `develop`): correr el banco
    completo —conformidad (`run_all`) + rendimiento con `--gate`— contra el **binario release**
    en la máquina de la baseline, y commitear la corrida datada en `docs/qa/corridas/vX.Y.Z/`
    (JSON + resumen MD de ambos bancos) en el mismo PR de versión. Un FAIL del banco es un
    stop-the-line del release (mismo rango que un CI rojo).
  - **`workflow_dispatch`** opcional (`.github/workflows/testbench.yml`): corre conformidad sobre
    el corpus canónico + smoke de rendimiento en el runner, **sin juzgar umbrales absolutos**
    (documentado en el propio workflow) — para disparar el banco a demanda sin la máquina de
    baseline.
  - Plantilla/convención de la corrida datada documentada en el README del banco.
  - **Primera corrida oficial completa** con el patrón nuevo, commiteada (puede ser la de H04/H02
    re-etiquetada si no hay release en curso; si la épica coincide con un release, la de ese
    release).
- **Fuera de alcance**: automatizar el gate absoluto en runners compartidos; firma de binarios
  (`decisiones §1`, congelada por `§20`); cualquier cambio al pipeline `release.yml` de binarios.
- **Criterios de aceptación**:
  - **[BDD-1] Dado** el `workflow_dispatch`, **Cuando** se dispara manualmente, **Entonces** corre
    conformidad + smoke y publica el resumen como artefacto del run, sin juzgar umbrales absolutos
    → verificación: un run real del workflow en verde enlazado en el PR.
  - **[Estructural]** `RELEASING.md` contiene el paso con el comando exacto y el destino de la
    corrida; `docs/qa/corridas/` existe con la primera corrida datada conforme a la plantilla; el
    README del banco documenta la convención; el paso deja claro que el gate absoluto solo vale en
    la máquina de la baseline (coherente con H05).
- **Dependencias**: E33-H02, E33-H05.
- **Pruebas**: el run real del workflow + la corrida commiteada; no hay lógica nueva que testear
  (el runbook y el workflow orquestan lo ya probado en H02/H04/H05).

---

## E33-H08 — Paquete de evidencia para decidir `decisiones §14`

- **Objetivo**: `decisiones §14` pasa de «no se decide sin medir» a **«lista para decidir»**: un
  documento único con todos los datos, el inventario del coste de conexión y el análisis de las
  tres salidas — **sin tomar la decisión**, que es del usuario y ocurre fuera de esta épica.
- **Referencias**: `ARCHITECTURE.md §22.8` · `decisiones §14` (las tres salidas a/b/c, su
  recomendación escrita, y sus absorciones `§16(c)` watcher y `§16(l)` `field_path`) ·
  `ARCHITECTURE.md §21.5` (que esta historia NO desactiva) · H04 (mediciones) · H06 (dogfooding).
- **Alcance**:
  - `docs/qa/evidencia-14-store-2026-08.md` con: (1) la **tabla de mediciones** (3 variantes × 3
    escalas × tools, p50/p95 + payload + coste del `rebuild`, con la calibración de wire); (2) el
    **dato de dogfooding** (veredicto de fricción a escala repo + número frío); (3) el **inventario
    del coste de conexión**, verificado contra el árbol actual, no citado de memoria: walker del
    store sin `DiscoveryPolicy` (`§20.5`), divergencia de nombres `metadata.field_path` core↔store
    (`§16(l)`), y el destino del watcher (`§16(c)` — qué papel tendría en cada salida); (4) el
    **análisis de las tres salidas** — conectar / acotar / retirar — contra los datos, incluyendo
    qué dice la comparación SQLite-raw vs RAM-memoizado sobre qué significaría «conectar»
    (¿SQLite, memoización, ambas?), actualizando o refutando la recomendación (a) escrita en la
    ficha; (5) el cierre **«lista para decidir»**, con la lista exacta de lo que la decisión
    tendrá que pronunciar.
  - Anotación en `decisiones/14-store-sin-consumidor.md`: `revisada_en` actualizado,
    `bloqueada_por: "evidencia"` → constancia de que la evidencia **está disponible** (enlace al
    paquete), **sin cambiar `estado` ni `prioridad`**. Fila de `§14` en `decisiones/README.md`
    actualizada en el mismo sentido, y anotación de cierre del punto 1 en `decisiones §9` (el gate
    existe y corre por release; la ficha `§9` sigue abierta por firma y threat model).
- **Fuera de alcance**: tomar o insinuar como tomada la decisión `§14`; cambiar `§21.5`; conectar,
  acotar o retirar nada; cerrar `§22`/`§24`.
- **Criterios de aceptación**:
  - **[Estructural — el entregable es un documento]** El paquete contiene las cinco partes del
    alcance; cada cifra citada existe en una corrida commiteada (trazable por fecha/fichero); el
    inventario del coste de conexión cita fichero y línea del árbol actual; el análisis cubre las
    tres salidas sin declarar ninguna elegida; el documento termina en «lista para decidir»; las
    anotaciones de `§14`, `§9` y `decisiones/README.md` están hechas con los estados **intactos**;
    `IMPLEMENTATION_STATUS.md` refleja la épica.
- **Dependencias**: E33-H04, E33-H06 (y cita los umbrales de E33-H05 si ya están ratificados).
- **Pruebas**: no aplica código; la verificación es documental (juez ciego con encargo de comprobar
  que ninguna afirmación numérica carece de corrida trazable y que ninguna decisión abierta se da
  por tomada).

---

## Orden de construcción

```
H01 ──► H02 ──► H03 ─────────────┐
  │                              ├──► H07
  └──► H04 ──► (PUERTA) H05 ─────┘
         │
         └──► H06 ──► H08
```

**Secuencia propuesta**: `H01 → H02 → H04 → H03 → H05 → H06 → H07 → H08`.

- **H01 es prerrequisito de todo** (sin corpus no hay banco ni bench).
- Tras H01, las dos ramas son **paralelizables**: conformidad (H02 → H03) y rendimiento (H04 →
  H05); no comparten ficheros salvo el README del banco.
- **H05 lleva la puerta interna** (ratificación de umbrales con la corrida de H04 delante): su
  mitad 2 no se implementa sin esa ratificación.
- **H06** puede empezar su diario en cualquier momento, pero cierra tras H04 (necesita el número
  frío).
- **H07** exige banco corrible (H02) y gate ratificado (H05).
- **H08 cierra la épica**: necesita mediciones (H04) y dogfooding (H06); H07 puede ir en paralelo.

Ninguna historia está `[BLOQUEADA por decisiones §N]`: las decisiones abiertas que la épica roza
(`§14`, `§22`, `§24`, `§20`) están **explícitamente fuera de alcance** en las historias que las
tocan, y la única puerta es la interna de H05, que se resuelve con datos producidos por la propia
épica.

## Cierre de la épica

- `decisiones §9`: punto 1 anotado como ejecutado (el banco corre por release); la ficha **sigue
  abierta** por firma/notarización (congelada por `§20`) y threat model.
- `decisiones §14`: evidencia disponible enlazada; **estado intacto** — la decisión es el paso
  siguiente del usuario, fuera de la épica.
- `decisiones §22`/`§24`: centinelas anotados; **estados intactos**.
- `IMPLEMENTATION_STATUS.md`, fila de E33 en `requirements/README.md` y sección E33 de
  [`trazabilidad.md`](trazabilidad.md).
- `CHANGELOG.md`: entrada interna (el banco no es superficie de usuario; nada de lo medido se
  promete fuera mientras `§14` siga abierta — `§21.5`).
