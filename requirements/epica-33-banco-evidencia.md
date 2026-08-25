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

### Adenda ratificada 2026-08-23 — retención de evidencia fuera de Git

La evidencia permanente de E33 se compone de **resumen y manifiesto versionados**. Los resultados
brutos pueden almacenarse como artefactos duraderos fuera de Git, identificados en el manifiesto
por URL estable, SHA-256, tamaño y versión de esquema. Las pruebas usan fixtures pequeñas y validan
el manifiesto; no dependen de volcados completos de 10k/100k. Esta adenda sustituye únicamente las
exigencias posteriores de commitear o versionar el JSON bruto: no modifica el benchmark, los
umbrales ni las métricas ratificadas.

Los JSON que son entradas, fixtures, matrices o calibraciones pequeñas siguen siendo fuente
versionada. Los resultados generados de una corrida se conservan fuera de Git; sus resúmenes y
manifiestos permiten localizar el bruto, comprobar sus bytes y reproducir la medición.

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
| E33-H09 | Sonda extrema parametrizable: Realista/100k, tamaños y footprint de memoria | no | **RATIFICADA 2026-08-22** |

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
    → resumen y manifiesto de la corrida completa publicados como resultado datado; el bruto se
    conserva como artefacto externo de la release (primera corrida del banco nuevo).
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

### Adenda ratificada 2026-08-22 — seam interno de lectura independiente

Esta adenda precisa únicamente la mecánica interna necesaria para medir H04. La nueva pieza debe
ser independiente y conservar intacta la implementación observable actual: no cambia el producto,
la frontera MCP ni las decisiones abiertas. Sustituye cualquier freeze byte a byte de la
implementación interna de `lodestar-app`.

- Se extrae una única pieza interna de servicios de lectura que recibe un `DocumentSet` ya
  adquirido y concentra la lógica vigente de `workspace_status`, `knowledge_search`,
  `knowledge_get`, `metadata_inspect`, `graph_query`, `impact_analyze` y `knowledge_check`.
  `App` y el banco consumen esa misma pieza sin cambiar firmas, resultados ni errores públicos.
- Las siete lecturas públicas de `App` conservan su adquisición actual desde disco y delegan
  únicamente la lógica posterior. El producto sigue entrando por `App::open`; no se conecta
  ninguna cache.
- El banco ejecuta las siete lecturas mediante la misma pieza interna. Disco-reparseo conserva la
  adquisición vigente en cada muestra; SQLite-raw incluye una nueva llamada a
  `Store::document_set()` en cada muestra y registra `Store::rebuild()` por separado; RAM-memoizado
  reutiliza un único `DocumentSet` construido antes de medir.
- La equivalencia funcional se comprueba para cada una de las siete tools. Normalizar significa
  solo canonicalizar la representación JSON de los objetos: no permite eliminar campos,
  reordenar arrays, sustituir valores, ignorar códigos o mensajes de error ni recortar resultados.
- `change_plan`→`change_apply` permanece fuera de la pieza de lectura y se mide exclusivamente por
  `App`, sobre el camino vigente de disco y bajo el escritor único. La matriz de tres variantes
  aplica a las siete lecturas, no a la escritura.
- La mecánica de visibilidad o enlace interno entre crates queda a elección de implementación,
  pero la superficie pública por defecto de `lodestar-app` no gana símbolos ni firmas,
  `lodestar-app` no añade una dependencia **directa** de `lodestar-store` y el crate interno del
  banco puede depender directamente de ambos. La dependencia transitiva ya vigente
  `lodestar-app`→`lodestar-workspace`→`lodestar-store` permanece intacta.

**Fuera de alcance de la adenda**: conectar `lodestar-store` a `App` o `lodestar-workspace`;
introducir una API pública soportada para el banco; cambiar firmas públicas de `App`, schemas, wire
MCP, códigos, mensajes u orden observable; duplicar en el banco la lógica de las siete lecturas;
usar la pieza para planificar o aplicar; optimizar; cerrar `decisiones §14`; o prometer rendimiento
externamente.

**Criterios adicionales de aceptación**:

- **[BDD-A1] Dado** un corpus de control no vacío con metadata heterogénea, enlaces internos, un
  dangling, backlinks, impacto transitivo y diagnósticos, **Cuando** cada lectura se ejecuta por el
  `App` vigente y por la pieza interna sobre el mismo `DocumentSet`, **Entonces** cada par devuelve
  exactamente el mismo `Result` normalizado, incluidos campos, orden de listas y, donde aplique,
  código y mensaje de error → `las_siete_lecturas_conservan_resultado_y_error_de_app`.
- **[BDD-A2] Dado** ese corpus y los mismos argumentos, **Cuando** se ejecutan disco-reparseo,
  SQLite-raw y RAM-memoizado, **Entonces** sus respuestas son exactamente iguales tool por tool y
  el fallo identifica la tool divergente → `equivalencia_exacta_por_tool_en_tres_variantes`.
- **[BDD-A3] Dado** un marcador funcional distinto para cada lectura, **Cuando** se valida el
  corpus antes de comparar, **Entonces** las siete lecturas demuestran respectivamente recuentos,
  búsqueda, obtención, metadata, grafo, impacto y diagnóstico no vacíos →
  `corpus_control_ejercita_las_siete_lecturas`.
- **[BDD-A4] Dadas** dos muestras con una mutación controlada de la fuente entre ambas, **Cuando**
  se ejecuta cada proveedor, **Entonces** disco y SQLite reflejan su segunda adquisición, RAM
  conserva el `DocumentSet` inicial y `rebuild` figura separado de los percentiles por tool →
  `cada_muestra_respeta_la_adquisicion_de_su_variante`.
- **[BDD-A5] Dado** un cambio aplicable, **Cuando** el banco mide `change_plan`→`change_apply`,
  **Entonces** ambas operaciones pasan por las APIs públicas de `App`, el Markdown cambia una sola
  vez bajo el escritor único y el informe solo contiene la fuente producto/disco para el ciclo →
  `ciclo_de_cambio_permanece_en_app_y_escritor_unico`.
- **[Estructural-A6]** La API pública por defecto de `lodestar-app` permanece igual;
  `contracts/mcp.yml` no cambia; el manifiesto y `cargo metadata` de `lodestar-app` no contienen
  una dependencia **directa** de `lodestar-store`; el banco sí puede depender directamente de
  ambos crates; y los tests MCP/CLI vigentes siguen verdes. La ruta transitiva ya existente a
  través de `lodestar-workspace` no se elimina ni cuenta como delta de H04. Estos guards reemplazan
  cualquier aserción sobre bytes, texto o disposición interna de `lodestar-app`.

**Delta de contrato de la adenda**: ninguno en `contracts/mcp.yml`, API pública Rust, CLI o wire
MCP. La documentación interna de H04 sí explicará qué incluye cada muestra, separará `rebuild`,
limitará la matriz de tres variantes a las siete lecturas y recordará que SQLite-raw ahorra
walk+I/O, no parseo.

- **Alcance**:
  - Crate interno **`crates/lodestar-bench`** (`publish = false`, como `lodestar-fixtures`), con un
    binario que: genera el corpus (generador de H01, perfiles plano y realista), y mide —
    **cold-open** (`App::open` + `workspace_status`) y **p50/p95 de N iteraciones** por tool de
    lectura (`workspace_status`, `knowledge_search` con `where`, `knowledge_get`,
    `metadata_inspect`, `graph_query`, `impact_analyze`, `knowledge_check`), con **payload en
    bytes** por respuesta serializada, en las **tres variantes**: (1) el producto actual;
    (2) `Store::rebuild()` + una adquisición `Store::document_set()` dentro de cada muestra,
    **registrando el coste del `rebuild` por separado**; (3) un `DocumentSet` construido una vez y
    reutilizado entre llamadas. El ciclo `change_plan`→`change_apply` se mide aparte y solo por el
    producto actual mediante `App`.
  - **Salida en formato estable**: JSON por corrida + resumen Markdown datado (máquina, binario,
    commit, semilla), pensado para comparar releases. De la **primera corrida completa a 10k** se
    versionan el resumen y el manifiesto en `docs/qa/`; el JSON bruto se conserva como artefacto
    externo duradero conforme a la adenda de retención.
  - **Calibración de wire**: una muestra (al menos `workspace_status` y `knowledge_search` a 10k)
    medida también por el arnés python contra el binario `lodestar-mcp` real, para acotar el
    overhead del framing JSON-RPC/stdio frente a la medición sobre `App`.
  - El binario acepta `--smoke` (escala mínima, pocas iteraciones) — H05 lo usa en CI.
- **Fuera de alcance**: umbrales y gate (H05 — esta historia **mide y registra, no juzga**);
  conectar el store al camino de lectura del producto o alinear su walker (eso es la épica
  posterior a `§14`); optimizar nada; prometer nada en la superficie externa (`§21.5`).
- **Criterios de aceptación**:
  - **[BDD-1] Dado** el corpus plano a escala mínima, **Cuando** corre el banco en modo smoke,
    **Entonces** el informe JSON contiene las 3 variantes × las siete tools de lectura con p50/p95
    y payload, más el ciclo de cambio rotulado solo como producto/disco,
    y las tres variantes reportan el **mismo** número de documentos y los mismos resultados
    funcionales de una consulta de control (las variantes miden lo mismo, no cosas distintas)
    → test: `informe_completo_y_variantes_equivalentes_en_smoke` (`crates/lodestar-bench`).
  - **[BDD-2] Dado** dos corridas smoke con la misma semilla, **Cuando** se comparan sus informes,
    **Entonces** la estructura (claves, escalas, tools, variantes) es idéntica — solo cambian los
    tiempos → test: `formato_de_informe_estable` (`crates/lodestar-bench`).
  - **[Estructural]** La corrida completa real (3 escalas, 3 variantes, ambos perfiles) tiene
    resumen datado y manifiesto commiteados en `docs/qa/`; el manifiesto enlaza el JSON bruto con
    URL estable, SHA-256, tamaño y versión de esquema. La calibración de wire está en el mismo
    paquete; el resumen MD rotula explícitamente la advertencia de `from_store` (SQLite-raw ahorra
    walk+IO, no parse); `cargo tree -p lodestar-core` sigue puro (el bench no añade nada al core) y
    el workspace compila con clippy `-D warnings`; cero delta en `contracts/mcp.yml`.
- **Dependencias**: E33-H01.
- **Pruebas**: los dos tests del crate `lodestar-bench` en modo smoke (rápidos, entran a la suite
  normal) + fixtures pequeñas para el formato + validación del manifiesto de la corrida real.

---

## E33-H05 — Umbrales ratificados, gate codificado y smoke en CI

> **PUERTA INTERNA (D4, ratificada)**: esta historia tiene dos mitades separadas por una
> ratificación del usuario. La primera mitad **presenta** los números de H04 con una propuesta de
> umbrales (anclas acordadas: **p95 ≤ 1 s por tool de lectura a 10k** y **cold-open ≤ 5 s**, sobre
> la variante disco-reparseo, que es el producto). **Nada de la segunda mitad se implementa hasta
> que el usuario ratifique los umbrales con la primera corrida delante.** No es una
> `[BLOQUEADA por decisiones §N]` — la decisión no existe aún porque su input es H04 — pero el
> efecto es el mismo: sin ratificación no hay gate.
>
> **Ratificación de la puerta (2026-08-22):** con la corrida H04 delante, el usuario ratifica
> **p95 ≤ 1 s por tool de lectura a 10k** y **cold-open ≤ 5 s** como cifras **máximas** del gate de
> regresión del producto disco-reparseo. Son techos para detectar una degradación, no objetivos de
> optimización ni una razón automática para descartar SQLite. Queda autorizada la mitad 2.

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
    **Entonces** el informe incluye las tres variantes sobre ese corpus → resumen y manifiesto
    anexados al diario; el bruto externo fue generado por el binario ya testeado en H04.
- **Dependencias**: E33-H04 (el número frío; el uso y el diario pueden empezar antes y en
  paralelo).
- **Pruebas**: el diario, el resumen y el manifiesto de su corrida anexa son el entregable
  verificable; no hay tests nuevos de código (H04 ya prueba el binario).

---

## E33-H07 — Enganche a release: runbook, workflow y corrida datada

- **Objetivo**: el banco pasa de re-ejecutable a **permanente por release**: paso del runbook en
  `RELEASING.md`, workflow manual opcional, y el patrón de resumen y manifiesto datados
  commiteados con resultados brutos externos — la definición literal de lo que
  `decisiones/README.md` pedía para `§9`.
- **Referencias**: `ARCHITECTURE.md §22.6` · `RELEASING.md` (pasos 1–8 actuales; el nuevo entra
  tras el paso 2) · decisión de la puerta de diseño: el paso del runbook se escribe **aquí**, no en
  la adenda, para que el runbook nunca instruya correr una herramienta inexistente ·
  `.github/workflows/` (el `workflow_dispatch`).
- **Alcance**:
  - **Paso nuevo en `RELEASING.md`** (entre el changelog y el PR a `develop`): correr el banco
    completo —conformidad (`run_all`) + rendimiento con `--gate`— contra el **binario release**
    en la máquina de la baseline, publicar los JSON brutos como artefactos duraderos y commitear
    en `docs/qa/corridas/vX.Y.Z/` el resumen MD de ambos bancos más su manifiesto. Un FAIL del
    banco es un stop-the-line del release (mismo rango que un CI rojo).
  - **`workflow_dispatch`** opcional (`.github/workflows/testbench.yml`): corre conformidad sobre
    el corpus canónico + smoke de rendimiento en el runner, **sin juzgar umbrales absolutos**
    (documentado en el propio workflow) — para disparar el banco a demanda sin la máquina de
    baseline.
  - Plantilla/convención de la corrida datada documentada en el README del banco.
  - **Primera corrida oficial completa** con el patrón nuevo, con resúmenes y manifiesto
    commiteados (puede ser la de H04/H02 re-etiquetada si no hay release en curso; si la épica
    coincide con un release, la de ese release).
- **Fuera de alcance**: automatizar el gate absoluto en runners compartidos; firma de binarios
  (`decisiones §1`, congelada por `§20`); cualquier cambio al pipeline `release.yml` de binarios.
- **Criterios de aceptación**:
  - **[BDD-1] Dado** el `workflow_dispatch`, **Cuando** se dispara manualmente, **Entonces** corre
    conformidad + smoke y publica el resumen como artefacto del run, sin juzgar umbrales absolutos
    → verificación: un run real del workflow en verde enlazado en el PR.
  - **[Estructural]** `RELEASING.md` contiene el paso con el comando exacto y el destino de la
    corrida; `docs/qa/corridas/` existe con la primera corrida datada conforme a la plantilla y
    con manifiesto verificable; el README del banco documenta la convención; el paso deja claro
    que el gate absoluto solo vale en la máquina de la baseline (coherente con H05).
- **Dependencias**: E33-H02, E33-H05.
- **Pruebas**: el run real del workflow + resúmenes y manifiesto commiteados; no hay lógica nueva
  que testear (el runbook y el workflow orquestan lo ya probado en H02/H04/H05).

---

## E33-H08 — Paquete de evidencia para decidir `decisiones §14`

- **Objetivo**: `decisiones §14` pasa de «no se decide sin medir» a **«lista para decidir»**: un
  documento único con todos los datos, el inventario del coste de conexión y el análisis de las
  tres salidas — **sin tomar la decisión**, que es del usuario y ocurre fuera de esta épica.
- **Referencias**: `ARCHITECTURE.md §22.8` · `decisiones §14` (las tres salidas a/b/c, su
  recomendación escrita, y su absorción `§16(c)` watcher; `§16(l)` quedó resuelta por E35-H02) ·
  `ARCHITECTURE.md §21.5` (que esta historia NO desactiva) · H04 (mediciones) · H06 (dogfooding).
- **Alcance**:
  - `docs/qa/evidencia-14-store-2026-08.md` con: (1) la **tabla de mediciones** (3 variantes × 3
    escalas × tools, p50/p95 + payload + coste del `rebuild`, con la calibración de wire); (2) el
    **dato de dogfooding** (veredicto de fricción a escala repo + número frío); (3) el **inventario
    del coste de conexión**, verificado contra el árbol actual, no citado de memoria: walker del
    store sin `DiscoveryPolicy` (`§20.5`) y el destino del watcher (`§16(c)` — qué papel tendría en
    cada salida); la divergencia de nombres `metadata.field_path` core↔store (`§16(l)`) quedó
    resuelta por E35-H02; (4) el
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
    alcance; cada cifra citada existe en un resumen commiteado y su manifiesto trazable enlaza el
    resultado bruto; el inventario del coste de conexión cita fichero y línea del árbol actual; el
    análisis cubre las tres salidas sin declarar ninguna elegida; el documento termina en «lista
    para decidir»; las anotaciones de `§14`, `§9` y `decisiones/README.md` están hechas con los
    estados **intactos**; `IMPLEMENTATION_STATUS.md` refleja la épica.
- **Dependencias**: E33-H04, E33-H06 (y cita los umbrales de E33-H05 si ya están ratificados).
- **Pruebas**: no aplica código; la verificación es documental (juez ciego con encargo de comprobar
  que ninguna afirmación numérica carece de corrida trazable y que ninguna decisión abierta se da
  por tomada).

---

## E33-H09 — Sonda extrema parametrizable: Realista/100k, tamaños y footprint de memoria

> **RATIFICADA el 2026-08-22.** `--scale N` admite cualquier entero positivo; la entrega ejecuta
> y registra Realista/100k con una iteración. Full, smoke y el gate H05/10k no cambian. 1M queda
> admitido con preflight y confirmación explícita, pero no es obligatorio. `§14` permanece abierta
> y pendiente de esta evidencia; cualquier optimización de memoria se decide después de medir.
>
> **ADENDA ratificada el 2026-08-23.** El presupuesto actual de disco
> (`32 KiB × scale + 256 MiB`) es una heurística conservadora y modificable, no parte del contrato
> de H09. Las pruebas exigen una estimación positiva, trazable y coherente con el espacio disponible,
> pero no fijan esa fórmula para futuras mejoras.

- **Objetivo**: extender el banco interno con una sonda extrema opt-in que parametriza perfil,
  escala e iteraciones y produce una medición Realista/100k trazable de las tres variantes,
  incluyendo tiempos, tamaños y memoria. La historia mide; no optimiza ni conecta el store.
- **Referencias**: `ARCHITECTURE.md §21.5`, `§22.2`, `§22.4` y `§22.5` · E33-H01/H04/H05/H08 ·
  `decisiones §14` · `docs/qa/evidencia-14-store-2026-08.md`.
- **Alcance**:
  - modo extremo separado y explícito, con una escala por ejecución y parámetros obligatorios
    equivalentes a `--extreme --profile realista --scale N --iterations M`;
  - `N` acepta cualquier entero positivo representable, sin whitelist; `M` es positivo y explícito;
  - las siete lecturas de H04 en `disk-reparseo`, `sqlite-raw` y `ram-memoizado`, con los mismos
    argumentos y equivalencia exacta de resultados;
  - cold-open y muestras por lectura; `Store::rebuild()` separado; conteo y bytes reales del corpus;
    SQLite `main_bytes`, `wal_bytes`, `shm_bytes`, `auxiliary_bytes` y `total_bytes` medidos y
    coherentes;
  - pico RSS absoluto por variante y, si el método lo permite, delta respecto a su proceso base;
    cada valor declara método, unidades, plataforma y ámbito. Una plataforma sin medición fiable
    declara `unavailable` con motivo, nunca cero ni una estimación disfrazada;
  - preflight antes de materializar el corpus: comprueba espacio de disco y declara que no verifica
    memoria. Para 1M o más, `--confirm-extreme` es siempre obligatorio después del preflight; la
    insuficiencia comprobada falla incluso con confirmación y sin dejar corpus parcial;
  - un `--root` explícito debe ser inicialmente inexistente, queda bajo guard RAII y se elimina al
    terminar; las salidas persistidas deben estar fuera del root autolimpiable;
  - corrida real Realista/100k, una iteración, resumen Markdown y manifiesto datados en
    `docs/qa/`; el JSON bruto se conserva fuera de Git y el corpus temporal no se versiona.
- **Fuera de alcance**: cambiar el full oficial, smoke, H05/10k o CI; ejecutar obligatoriamente 1M;
  wire y `change_plan`→`change_apply` en la sonda extrema; conectar SQLite a producto; optimizar
  RAM, parsing, store, walker o invalidación; prometer rendimiento; cambiar `contracts/mcp.yml`,
  API pública, CLI de producto o MCP; escoger o cerrar `§14`.
- **Criterios de aceptación**:
  - **[BDD-1 — opt-in y validación] Dado** full/smoke sin el modo extremo, **Cuando** corren,
    **Entonces** mantienen escalas, iteraciones, formato y semántica. **Dado** el modo extremo sin
    perfil/escala/iteraciones, con cero o perfil desconocido, **Entonces** falla antes de crear el
    corpus y nombra el parámetro → `modo_extremo_exige_parametros_y_no_altera_full_smoke`.
  - **[BDD-2 — escala abierta] Dada** cualquier escala positiva representable, incluida 1M,
    **Cuando** se valida la invocación, **Entonces** no se rechaza por no pertenecer a una lista.
    Una escala cero, negativa o desbordada se rechaza → `scale_acepta_entero_positivo_sin_whitelist`.
  - **[BDD-3 — equivalencia] Dado** un corpus extremo no vacío, **Cuando** se ejecutan las siete
    lecturas en las tres variantes, **Entonces** los resultados normalizados son exactamente
    iguales; el fallo identifica variante, tool y camino divergente →
    `variantes_extremas_conservan_equivalencia_funcional`.
  - **[BDD-4 — métricas] Dadas** `M` iteraciones, **Cuando** termina la sonda, **Entonces** cada
    lectura conserva exactamente `M` muestras y sus estadísticas; rebuild queda separado de los
    percentiles de SQLite → `extremo_registra_muestras_y_rebuild_separado`.
  - **[BDD-5 — footprint] Dada** una corrida, **Cuando** se inspecciona el informe, **Entonces**
    contiene conteos y tamaños medidos del corpus y SQLite, y RSS por variante con método/unidades/
    ámbito o un estado no disponible honesto → `extremo_registra_tamanos_y_rss_honesto`.
  - **[BDD-6 — preflight] Dada** una escala cuyo espacio requerido supera al disponible, **Cuando**
    corre el preflight, **Entonces** falla antes de escribir y comunica disponible/requerido.
    **Dada** 1M con recursos no verificables, **Entonces** exige confirmación explícita →
    `preflight_extremo_falla_sin_parciales_y_1m_exige_confirmacion_si_es_incierto`.
  - **[Estructural-7 — evidencia 100k]** existe una corrida datada Realista/100k, una iteración,
    tres variantes × siete lecturas, resultados no vacíos, conteo independiente, tamaños y RSS;
    el resumen conserva esas métricas y el manifiesto identifica el JSON bruto sin rutas privadas
    mediante URL estable, SHA-256, tamaño y versión de esquema.
  - **[Estructural-8 — no regresión]** H05 sigue seleccionando solo disco/10k; el informe extremo
    no se usa como umbral; CI no ejecuta la sonda; `contracts/mcp.yml` y la API pública no cambian;
    `§14` continúa abierta y la documentación interna no convierte 100k en promesa.
- **Pruebas**: integración en `crates/lodestar-bench/tests/` con fixtures pequeñas para negativos,
  formato y anti-vacuidad, locks verificables y validación del manifiesto; ninguna prueba depende
  del volcado 100k ni lo ejecuta rutinariamente en CI.
- **Dependencias**: E33-H01, E33-H04, E33-H05 y E33-H08. No depende del BDD remoto de H07.
- **Delta de contrato**: ninguno. Se actualizan README del banco, `ARCHITECTURE.md §22`, estado,
  trazabilidad, changelog, `decisiones §14` y el paquete H08 para reflejar que la decisión espera
  la extensión 100k, sin alterar prioridad ni escoger salida.

---

## Orden de construcción

```
H01 ──► H02 ──► H03 ─────────────┐
  │                              ├──► H07
  └──► H04 ──► (PUERTA) H05 ─────┘
         │
         └──► H06 ──► H08 ──► H09
```

**Secuencia ejecutada/ampliada**: `H01 → H02 → H04 → H03 → H05 → H06 → H07 → H08 → H09`.

- **H01 es prerrequisito de todo** (sin corpus no hay banco ni bench).
- Tras H01, las dos ramas son **paralelizables**: conformidad (H02 → H03) y rendimiento (H04 →
  H05); no comparten ficheros salvo el README del banco.
- **H05 lleva la puerta interna** (ratificación de umbrales con la corrida de H04 delante): su
  mitad 2 no se implementa sin esa ratificación.
- **H06** puede empezar su diario en cualquier momento, pero cierra tras H04 (necesita el número
  frío).
- **H07** exige banco corrible (H02) y gate ratificado (H05).
- **H08** cerró el paquete 10k; **H09** entregó la extensión ratificada con escala abierta,
  footprint y la evidencia 100k. H07 puede completar su BDD remoto en paralelo.

Ninguna historia está `[BLOQUEADA por decisiones §N]`: las decisiones abiertas que la épica roza
(`§14`, `§22`, `§24`, `§20`) están **explícitamente fuera de alcance** en las historias que las
tocan, y la única puerta es la interna de H05, que se resuelve con datos producidos por la propia
épica.

## Cierre de la épica

- `decisiones §9`: punto 1 anotado como ejecutado (el banco corre por release); la ficha **sigue
  abierta** por firma/notarización (congelada por `§20`) y threat model.
- `decisiones §14`: evidencia 10k y extensión H09/100k disponibles; **estado intacto** — la
  decisión sigue siendo del usuario y no se escoge ninguna salida.
- `decisiones §22`/`§24`: centinelas anotados; **estados intactos**.
- `IMPLEMENTATION_STATUS.md`, fila de E33 en `requirements/README.md` y sección E33 de
  [`trazabilidad.md`](trazabilidad.md).
- `CHANGELOG.md`: entrada interna (el banco no es superficie de usuario; nada de lo medido se
  promete fuera mientras `§14` siga abierta — `§21.5`).
