# E16 — Modelo documental genérico

> **Fase**: `§20.14` PR 2 (`REFACTOR_PHASE_2 §Fase 4` y `§Fase 1` en terminología).
> **Objetivo de la épica**: que un `.md` **cualquiera** sea un documento de primera clase. Sin
> frontmatter obligatorio, sin campos conocidos, sin `type`, sin ficheros con nombre mágico. Al
> cerrar E16, `README.md`, `index.md`, `AGENTS.md` y `docs/decisions/auth.md` se parsean, analizan y
> consultan exactamente igual.
> Referencias maestras: `ARCHITECTURE.md §20.4`, `§20.3` (terminología), `§20.9` (diagnósticos);
> `CLAUDE.md` invariantes #1/#2/#3/#4.

**Principio rector**: *el frontmatter es metadata arbitraria del usuario, no un formato de Lodestar*.
Cualquier clave YAML es válida; no se eliminan claves desconocidas; no se convierten valores entre
tipos; `title` es una heurística de presentación, nunca una propiedad reservada.

**Nota de secuencia** (`§20.14`, ajuste 2): al retirar los campos tipados y las clases de fichero,
los checks `OKF-TYPE`/`OKF-IDX`/`OKF-LOG`/`ORPHAN`/`REC-*`/`FMT-*`/`BODY-STRUCT` se quedan sin nada
sobre lo que operar. Por eso la reducción de `conform` al catálogo mínimo de `§20.9` ocurre **aquí**
(E16-H05) y no en el PR 8 del documento; E20 aporta solo la política y la semántica de
`knowledge_check`.

**Lo que E16 NO toca**: la resolución de enlaces y el grafo (E17 — `resolve_link` sigue convirtiendo
`foo/` en `foo/index.md` hasta entonces), el DDL del store (E18), el lenguaje de consulta (E19).

---

### E16-H01 — `ParsedFrontmatter`: YAML arbitrario en vez de 7 campos tipados

- **Objetivo**: el frontmatter deja de tener forma conocida. Se conserva íntegro, con su tipo YAML
  real y su texto original.
- **Referencias**: `ARCHITECTURE.md §20.4` · `REFACTOR_PHASE_2 §Fase 4` ·
  `crates/lodestar-core/src/types.rs:356-390` (`Frontmatter`), `crates/lodestar-core/src/model.rs:53-117`
  (`parse_yaml`, `frontmatter_from_mapping`, `js_string`).
- **Alcance**:
  - Sustituir `Frontmatter` por:
    ```rust
    pub struct ParsedFrontmatter { pub value: serde_yaml::Value, pub raw: String, pub span: Range<usize> }
    ```
    `span` es el rango de bytes del bloque de frontmatter dentro del `raw` del documento (lo necesita
    E16-H04 para el patch quirúrgico y `§20.9` para los rangos de diagnóstico).
  - **Accesor por `FieldPath` con dot-notation** — `get(&self, path: &FieldPath) -> Option<&Value>` —
    que resuelve `service.tier` y `release.target.date` descendiendo por mapas. Es **la única verdad
    de acceso a metadata** del repo: la reutilizan E18 (indexado), E19 (query) y E20
    (`metadata_inspect`). Ningún consumidor vuelve a navegar el `Value` a mano (invariante #3).
  - Un documento **sin** frontmatter (`None`) y uno con frontmatter **vacío** son ambos válidos y
    distinguibles.
  - Se conservan: strings, números, booleanos, fechas como valores YAML, `null`, listas, objetos
    anidados y listas de objetos. **No** se eliminan claves desconocidas ni se convierten tipos.
  - Retirar `KNOWN_FM`, `Frontmatter::known_null`, `Frontmatter::as_pairs`, `js_string` y la coerción
    a string de `frontmatter_from_mapping` (era la paridad con `String(v)` de JS, sin sentido ya).
  - Migrar mecánicamente los ~44 puntos que leen `fm.r#type`/`fm.status`/`fm.title` a través del
    accesor: `conform.rs`, `bundle.rs`, `query.rs`, `schema.rs`, `graph.rs`,
    `crates/lodestar-store/src/index.rs`, `crates/lodestar-app/src/lib.rs`.
- **Fuera de alcance**: el título derivado (E16-H03), el patch (E16-H04), el DDL del store (E18) —
  las columnas `type`/`title`/`status` de `files` siguen existiendo, alimentadas por el accesor.
- **Criterios de aceptación**:
  - **Dado** un `.md` sin frontmatter, **Cuando** se parsea, **Entonces** `frontmatter` es `None`, el
    body es el fichero entero y no se emite ningún diagnóstico → `sin_frontmatter_es_valido`.
  - **Dado** un `.md` con `---\n---\n`, **Cuando** se parsea, **Entonces** el frontmatter está
    presente y vacío, distinguible del caso anterior → `frontmatter_vacio_es_valido`.
  - **Dado** un frontmatter con string, número, booleano, `null`, lista, objeto anidado y lista de
    objetos, **Cuando** se parsea, **Entonces** cada valor conserva su **tipo YAML real** — un `2`
    sigue siendo número y un `true` booleano, no strings → `preserva_tipos_yaml`.
  - **Dado** `service: {name: auth, tier: critical}`, **Cuando** se consulta `service.tier`,
    **Entonces** devuelve `critical`; **y** `service.ausente` devuelve `None` → `dot_notation`.
  - **Dado** un frontmatter con claves que Lodestar nunca ha visto, **Cuando** se parsea y se vuelve
    a serializar sin patch, **Entonces** están todas, con su valor intacto → `no_borra_desconocidas`.
- **Dependencias**: E15 completa.
- **Pruebas**: `crates/lodestar-core/tests/`: los 5 nombres.
- **Frontera (mcp.yml)**: **sí** (`Frontmatter` viaja en `knowledge_get`).

### E16-H02 — Ningún nombre de fichero activa reglas especiales

- **Objetivo**: `index.md`, `log.md` y `README.md` son documentos normales. La pertenencia deja de
  determinarse por índices.
- **Referencias**: `ARCHITECTURE.md §20.4`, `§20.7` · `REFACTOR_PHASE_2 §Principios 3 y 4`,
  `§Fase 8 (Eliminar)` · `crates/lodestar-core/src/model.rs:138-168,462-473` (`is_reserved`,
  `file_kind`, la rama de `parse_file`) · `crates/lodestar-core/src/bundle.rs:50-137`.
- **Alcance**:
  - Borrar `FileKind`, `model::file_kind`, `model::is_reserved`, `RelPath::is_reserved` y
    `RelPath::concept_id`.
  - `parse_file` deja de ramificar por basename: **todo** `.md` se parsea igual (hoy un `index.md`
    devuelve `fm: None` y el raw entero como body — `model.rs:465-473`).
  - `compute_analysis` (`bundle.rs:57-78`) deja de saltarse `index.md`/`log.md` y de alimentar
    `in_index`: todos los `.md` son nodos del análisis.
  - Retirar de `Analysis` los campos `in_index` y `okf_version`, `Bundle::root_okf_version`
    (`bundle.rs:139-145`) y el `index_refs` de `Backlinks` (`types.rs:531`, `bundle.rs:202-216`).
  - Retirar el gating de fichero reservado de `query.rs:104-123` (el quirk "reservado antes de negar"
    del prototipo) y de `is_predicate` (`query.rs:217-234`).
  - `orphans` pasa a llamarse `isolated` con la definición de `§20.7`: **sin enlaces internos
    entrantes ni salientes** (hoy `orphans` es "sin entrantes y no listado en un índice",
    `bundle.rs:117-121`). Deja de ser un diagnóstico.
- **Fuera de alcance**: la forma final de `Analysis` con `ResolvedLink`/`DanglingLink` (E17); el
  cálculo de aristas sigue con el `resolve_link` actual hasta E17.
- **Criterios de aceptación**:
  - **Dado** un workspace con `index.md` que tiene frontmatter, **Cuando** se parsea, **Entonces**
    su frontmatter se lee como el de cualquier otro documento → `index_md_es_documento_normal`.
  - **Dado** un `index.md` que enlaza a `alfa.md`, **Cuando** se analiza, **Entonces** hay una
    **arista** de `index.md` a `alfa.md` como con cualquier origen — no una relación de pertenencia
    → `enlace_desde_indice_es_arista`.
  - **Dado** un documento sin enlaces entrantes pero **con** salientes, **Cuando** se analiza,
    **Entonces** NO es aislado → `con_salientes_no_es_aislado`.
  - **Dado** un documento sin enlaces de ningún tipo, **Cuando** se analiza, **Entonces** es aislado
    y **no** genera diagnóstico → `aislado_no_es_error`.
  - **Dado** un workspace cuyo `index.md` declara `okf_version`, **Cuando** se analiza, **Entonces**
    esa clave es metadata consultable normal y no aparece en `Analysis`
    → `okf_version_es_metadata_normal`.
- **Dependencias**: E16-H01.
- **Pruebas**: `crates/lodestar-core/tests/`: los 5 nombres.
- **Frontera (mcp.yml)**: **sí** (`Analysis` y `Backlinks` cambian de forma).

### E16-H03 — Título derivado

- **Objetivo**: dar a cada documento un título presentable sin convertir `title` en propiedad
  reservada.
- **Referencias**: `ARCHITECTURE.md §20.4` · `REFACTOR_PHASE_2 §Fase 4 (Título derivado)` ·
  `crates/lodestar-core/src/model.rs:145-159` (`title_from_path`, con el quirk de `\b` de JS),
  `model.rs:536` (`parse_headings`, que ya reconoce fences de código).
- **Alcance**:
  - Función pura `derived_title(fm: Option<&ParsedFrontmatter>, body: &str, path: &RelPath) -> String`
    con la cadena `frontmatter.title` → **primer heading H1 del cuerpo** → nombre del fichero.
  - Reutilizar `model::parse_headings` para el H1 (no reimplementar detección de headings: ya maneja
    fences ` ``` `, y un `# comentario` dentro de un bloque de código no es un título).
  - Sustituir `title_from_path` (que hace Title Case con el quirk de word-boundary de JS) por el
    **nombre del fichero tal cual**, sin `.md`: la spec dice "nombre del archivo", y el Title Case
    era paridad con el prototipo, ya retirado.
  - Sustituir sus usos en `Bundle::list_concepts` (`bundle.rs:157-161`) y en el `create_concept` que
    componía `# {type} - {título}` (`bundle.rs:339-348`).
- **Fuera de alcance**: exponerlo en el store/FTS (E18) ni en `document.title` de la query (E19).
- **Criterios de aceptación**:
  - **Dado** un documento con `title: Autenticación` en el frontmatter y un H1 distinto, **Cuando**
    se deriva el título, **Entonces** gana el del frontmatter → `titulo_frontmatter_gana`.
  - **Dado** un documento sin `title` pero con `# Rotación de tokens`, **Cuando** se deriva,
    **Entonces** es `Rotación de tokens` → `titulo_del_h1`.
  - **Dado** un documento sin `title` cuyo primer `#` está dentro de un bloque de código, **Cuando**
    se deriva, **Entonces** ese `#` se ignora → `h1_en_fence_no_cuenta`.
  - **Dado** `docs/decisions/auth-tokens.md` sin `title` ni H1, **Cuando** se deriva, **Entonces** es
    `auth-tokens` → `titulo_del_nombre_de_fichero`.
  - **Dado** un documento con `title: 42`, **Cuando** se deriva, **Entonces** no revienta y `title`
    **no** queda marcado como reservado: sigue siendo metadata consultable con su tipo numérico
    → `title_no_es_reservada`.
- **Dependencias**: E16-H01.
- **Pruebas**: `crates/lodestar-core/tests/`: los 5 nombres.
- **Frontera (mcp.yml)**: **sí** (`DocumentSummary.title`).

### E16-H04 — `patch_frontmatter` quirúrgico

- **Objetivo**: modificar solo las claves pedidas, sin reordenar ni perder nada del resto.
- **Referencias**: `ARCHITECTURE.md §20.4` · `REFACTOR_PHASE_2 §Fase 4 (Requisitos de edición)` ·
  `crates/lodestar-core/src/model.rs:391-451` (`dump_frontmatter`/`build_raw`, que **canonicalizan**
  el orden: known fields primero) · `crates/lodestar-core/src/bundle.rs:453-499` (`apply_patch`).
- **Alcance**:
  - `build_raw` deja de imponer orden canónico. La reconstrucción **preserva el orden de aparición**
    de las claves existentes y añade las nuevas al final.
  - **Edición quirúrgica del bloque YAML crudo** cuando el patch lo permite (usando el `raw`/`span`
    de `ParsedFrontmatter`, E16-H01): tocar solo las líneas de las claves afectadas. Cuando no sea
    posible (p. ej. la clave está dentro de una estructura anidada compleja), se reserializa el
    bloque entero y **el plan debe declararlo** — campo booleano en el resultado de la operación,
    consumido por `change_plan` en E21.
  - Distinguir explícitamente **asignar `null`** (la clave queda con valor `null`) de **eliminar la
    clave** (desaparece). `FrontmatterPatch` (`types.rs:551`) ya modela los 3 estados con
    `BTreeMap<String, Option<Value>>`; lo que falta es que `apply_patch` deje de tratar los 5 known
    strings como casos especiales (`bundle.rs:465-487`).
  - El cuerpo del documento queda **intacto** byte a byte.
- **Fuera de alcance**: la operación MCP `patch_frontmatter` (E21); las selecciones masivas (E21).
- **Criterios de aceptación**:
  - **Dado** un frontmatter con 6 claves en orden no alfabético, **Cuando** se parchea una del medio,
    **Entonces** las otras 5 conservan su orden y su formato original
    → `patch_preserva_orden_y_claves`.
  - **Dado** un patch `{status: null}`, **Cuando** se aplica, **Entonces** el frontmatter tiene
    `status:` con valor nulo; **Dado** un patch que elimina `status`, **Entonces** la clave no está
    → `null_no_es_borrado`.
  - **Dado** un documento con cuerpo que contiene `---` en una línea, **Cuando** se parchea el
    frontmatter, **Entonces** el cuerpo queda idéntico byte a byte → `cuerpo_intacto`.
  - **Dado** un patch que obliga a reserializar el bloque entero, **Cuando** se calcula, **Entonces**
    el resultado lo señala explícitamente → `declara_reserializacion`.
  - **Dado** un documento **sin** frontmatter, **Cuando** se le aplica un patch, **Entonces** se crea
    el bloque al principio y el cuerpo queda intacto → `patch_crea_bloque`.
- **Dependencias**: E16-H01.
- **Pruebas**: `crates/lodestar-core/tests/`: los 5 nombres.
- **Frontera (mcp.yml)**: no (la operación se expone en E21).

### E16-H05 — Diagnósticos mínimos: retirar el catálogo OKF

- **Objetivo**: Lodestar deja de juzgar si un documento cumple una especificación documental. Solo
  informa de lo que le impide interpretarlo o modificarlo con seguridad.
- **Referencias**: `ARCHITECTURE.md §20.9` · `REFACTOR_PHASE_2 §Fase 10` ·
  `crates/lodestar-core/src/conform.rs` (entero), `crates/lodestar-core/src/types.rs:156-243`
  (`CheckCode`).
- **Alcance**:
  - `CheckCode` pasa al catálogo de `§20.9`: `FM-UNCLOSED`, `FM-YAML-INVALID`, `DOC-CONFLICT-MARKER`,
    `DOC-NOT-UTF8`, `DOC-TOO-LARGE`, `PATH-NOT-UTF8`, `SYMLINK-UNSUPPORTED` (estos 4 últimos ya los
    introdujo E15-H07) y, en E17, `LINK-TARGET-MISSING`/`LINK-ESCAPES-WORKSPACE`/`LINK-CASE-MISMATCH`.
  - Se **borran**: `OKF-FM01` (falta frontmatter: ya no es error), `OKF-TYPE`, `REC-TITLE`,
    `REC-DESC`, `FMT-TAGS`, `FMT-TS`, `ORPHAN`, `BODY-STRUCT`, `OKF-IDX`, `OKF-LOG`, y las familias
    `SCHEMA-*`/`REL-*`/`EXTREF-MISSING` (estas últimas mueren del todo en E20, con `core::schema`;
    aquí basta con que `conform` deje de producirlas). `OKF-FM02`→`FM-UNCLOSED`,
    `OKF-FM03`→`FM-YAML-INVALID`, `OKF-CONFLICT`→`DOC-CONFLICT-MARKER`.
  - Borrar `validate_index`, `validate_log` y `model::is_iso` (existía solo para `FMT-TS`).
  - `Check` conserva su forma (`level`/`code`/`msg`/`targets` + los aditivos `id`/`range`/`related`/
    `fixes`): cambia el catálogo de códigos, no la estructura (`§10` fila #3 sigue vigente).
  - Los diagnósticos llevan `range` cuando se conoce (el `span` de E16-H01 lo hace posible para los
    de frontmatter).
- **Fuera de alcance**: la política `rejectNewErrors`/`allowExistingErrors` y la semántica nueva de
  `knowledge_check` (E20); el gate de `change_apply` pasa **temporalmente** a "el resultado parsea y
  no introduce diagnósticos nuevos".
- **Criterios de aceptación**:
  - **Dado** un documento sin frontmatter, sin `type` y sin `status`, **Cuando** se valida,
    **Entonces** no se emite ningún diagnóstico → `sin_frontmatter_no_diagnostica`.
  - **Dado** un documento con `tags: "no-es-lista"` y `timestamp: "ayer"`, **Cuando** se valida,
    **Entonces** no se emite ningún diagnóstico: son metadata arbitraria
    → `formato_de_tags_no_diagnostica`.
  - **Dado** un documento cuyo frontmatter abre `---` y no cierra, **Cuando** se valida, **Entonces**
    `FM-UNCLOSED` con severidad error → `frontmatter_sin_cierre`.
  - **Dado** un frontmatter con YAML inválido, **Cuando** se valida, **Entonces** `FM-YAML-INVALID`
    con el rango de líneas del bloque → `yaml_invalido_con_rango`.
  - **Dado** un documento con marcadores `<<<<<<<`, **Cuando** se valida, **Entonces**
    `DOC-CONFLICT-MARKER` con severidad error → `marcadores_de_merge`.
  - **Dado** un documento aislado y un documento con estructura de headings arbitraria, **Cuando** se
    valida, **Entonces** ninguno produce diagnóstico → `aislado_y_headings_no_diagnostican`.
- **Dependencias**: E16-H01, E16-H02.
- **Pruebas**: `crates/lodestar-core/tests/`: los 6 nombres.
- **Frontera (mcp.yml)**: **sí** (`CheckCode` es contrato de wire).

### E16-H06 — Terminología: `Concept` → `Document`

- **Objetivo**: la API pública deja de hablar de OKF. Es el cierre de `§20.3`.
- **Referencias**: `ARCHITECTURE.md §20.3` · `REFACTOR_PHASE_2 §Fase 1 (Terminología)`.
- **Alcance** (renombres, sin cambio de comportamiento):
  - `Bundle` → `DocumentSet` · `ConceptRef` → `DocumentRef` · `ConceptId` → `DocumentId` ·
    `ConceptSummary` → `DocumentSummary` · `ConceptRevision` → `DocumentRevision` ·
    `ConceptStore` → `DocumentStore` · `Analysis.concepts` → `.documents`.
  - `ErrorCode::ConceptNotFound` → `DocumentNotFound` (y su valor de wire
    `CONCEPT_NOT_FOUND` → `DOCUMENT_NOT_FOUND`).
  - `App::knowledge_*` conservan su nombre (son la superficie MCP congelada de `§19.6`/`§20.10`);
    lo que cambia son los **tipos** que devuelven.
  - Nombres de fichero: `crates/lodestar-app/tests/concept_ref.rs` → `document_ref.rs`.
  - Comentarios y docstrings: sustituir "bundle"/"concepto"/"conformidad" por
    "workspace"/"documento"/"validación" allí donde describan la API, no la historia del proyecto.
- **Fuera de alcance**: `core::diff::OkfDiff` → `SemanticDiff` (E21, cuando se toque el motor
  transaccional); `okf_version` como dato del usuario (se conserva: `§20.13`).
- **Criterios de aceptación**:
  - Estructural: `grep -rn "Concept\|Bundle\|conformance" crates/*/src` no encuentra identificadores
    públicos con esa terminología (sí puede aparecer en comentarios históricos y en `okf_version`
    como clave de usuario) → checklist de CI.
  - **Dado** una tool que recibe un documento inexistente, **Cuando** falla, **Entonces** el código de
    error de wire es `DOCUMENT_NOT_FOUND` → `error_code_documento`.
  - **Dado** la suite completa, **Cuando** se ejecuta, **Entonces** pasa sin cambios de comportamiento
    respecto al commit anterior → el renombre es puramente léxico.
- **Dependencias**: E16-H01…H05.
- **Pruebas**: `crates/lodestar-app/tests/document_ref.rs`: `error_code_documento`.
- **Frontera (mcp.yml)**: **sí** (nombres de tipo y `ErrorCode`).

---

## Orden de construcción

```
H01 (ParsedFrontmatter) ─→ H02 (sin ficheros reservados) ─→ H05 (diagnósticos mínimos) ─→ H06 (renombres)
        ├─→ H03 (título derivado)                                                              ▲
        └─→ H04 (patch quirúrgico) ────────────────────────────────────────────────────────────┘
```

H01 es la base de todo. H03 y H04 solo dependen de H01 y pueden ir en paralelo con H02. H06 va al
final para no renombrar dos veces lo que las historias anteriores todavía están moviendo.

## Criterio de salida de la épica

Un workspace de `.md` sin una sola línea de frontmatter se parsea, analiza y valida sin emitir
diagnósticos; `index.md` y `README.md` no reciben trato distinto de cualquier otro documento; y
`grep -rn "OKF" crates/*/src` solo devuelve referencias históricas en comentarios.
