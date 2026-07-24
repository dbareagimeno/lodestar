# E20 — Inspección de metadata y validación genérica

> **Fase**: `§20.14` PRs 7 + 8 (`REFACTOR_PHASE_2 §Fase 6` y `§Fase 10`).
> **Objetivo de la épica**: dar a un agente lo que necesita para **comprender una base desconocida sin
> un schema** (`metadata_inspect`), y hacer que la validación conteste *"¿puedo interpretar y modificar
> este workspace con seguridad?"* en vez de *"¿cumple una especificación documental?"*. Al cerrar E20,
> `schema_inspect` y `core::schema` (con `DocType`, relaciones tipadas y `.lodestar/schema.yaml`)
> desaparecen, y los diagnósticos de descubrimiento por fin llegan a las fachadas.
> Referencias maestras: `ARCHITECTURE.md §20.9`, `§20.10`; `docs/REFACTOR_PHASE_2 §Fase 6`, `§Fase 10`;
> `CLAUDE.md` invariantes #1/#3.

**Principio rector**: `§Fase 6` — *"permitir que un agente comprenda las convenciones de una base
desconocida **sin necesitar un schema**"*. Y `§Fase 10` — deja de ser error la falta de frontmatter,
de `type`, de `status`, el formato de `tags`, la ausencia en un índice, `okf_version`, el documento
aislado, la estructura de headings, las transiciones de estado y las relaciones no tipadas.

**Herencia**: `ParsedFrontmatter::walk` (E18) es el iterador `(FieldPath, &Value)` sobre el que se
construye el catálogo; `ValueType::of` (E19) clasifica cada valor. E20 los **consume**.

**Deuda que E20 salda** (con dueño explícito desde E17): los diagnósticos de descubrimiento
(`DOC-NOT-UTF8`, `DOC-TOO-LARGE`, `SYMLINK-UNSUPPORTED`, `PATH-NOT-UTF8`, `LINK-CASE-MISMATCH`) se
computan en `discovery` y **su único llamador los descarta**. Media tabla de `§20.9` es hoy invisible
para `knowledge_check` y `lodestar check`. E20 los cablea, porque lo que faltaba no era el cable sino
el **criterio de política de severidad** — que es lo que esta épica introduce.

---

### E20-H01 — `metadata_inspect`: catálogo de propiedades

- **Objetivo**: el modo `catalog` de `§Fase 6` — la lista de campos con en cuántos documentos aparece
  cada uno y qué tipos toma.
- **Referencias**: `ARCHITECTURE.md §20.10` · `REFACTOR_PHASE_2 §Fase 6 (Catálogo de propiedades)` ·
  `crates/lodestar-core/src/types.rs` (`walk`, `ValueType`), `crates/lodestar-app/src/lib.rs`.
- **Alcance**:
  - Función pura en el core que, dado el `DocumentSet`, produce el catálogo: por cada `field_path`
    que aparece en algún documento, `presentIn` (nº de documentos) e `inferredTypes` (`{tipo: conteo}`).
  - Recorre cada documento con `walk` (una fila por par); los `field_path` son los mismos que indexa
    el store (E18) — **una sola verdad de qué es un campo**.
  - Paths anidados: `service.name`, `service.tier`, `release.target.date` aparecen como campos
    propios (`§Fase 6`, «Propiedades anidadas»).
- **Fuera de alcance**: la inspección de un campo concreto (E20-H02); la tool MCP (E20-H03).
- **Criterios de aceptación**:
  - **Dado** 3 documentos con `status` string y 1 con `status` número, **Cuando** se pide el catálogo,
    **Entonces** `status` tiene `presentIn: 4` e `inferredTypes: {string: 3, number: 1}` →
    `catalogo_presencia_y_tipos`.
  - **Dado** documentos con `service: {name, tier}`, **Cuando** se pide, **Entonces** `service.name` y
    `service.tier` son campos del catálogo → `catalogo_paths_anidados`.
  - **Dado** un workspace sin frontmatter en ningún documento, **Cuando** se pide, **Entonces** el
    catálogo es vacío, sin error → `catalogo_vacio`.
- **Dependencias**: E19 completa.
- **Pruebas**: `crates/lodestar-core/tests/`.
- **Frontera (mcp.yml)**: no (aún; la tool es H03).

### E20-H02 — `metadata_inspect`: inspección de un campo

- **Objetivo**: el modo `field` de `§Fase 6` — `presentIn`/`missingIn`, tipos inferidos y **valores
  frecuentes**.
- **Referencias**: `REFACTOR_PHASE_2 §Fase 6 (Inspección de una propiedad, Propiedades anidadas)`.
- **Alcance**:
  - Dado un `field_path`, `presentIn`/`missingIn` (nº de documentos), `inferredTypes`, y `values`:
    lista de `{value, count}` de los valores escalares más frecuentes (orden determinista: por conteo
    desc, luego por valor).
  - Funciona sobre paths anidados (`service.tier`, `release.target.date`).
  - Los valores de lista y objeto no se cuentan como «valores frecuentes» (no son escalares); su
    presencia sí cuenta en `presentIn` y su tipo en `inferredTypes`.
- **Criterios de aceptación**:
  - **Dado** `status` con 21 `draft`, 57 `accepted`, 6 `deprecated`, **Cuando** se inspecciona,
    **Entonces** `values` los lista con su conteo, ordenados → `inspecciona_valores_frecuentes`.
  - **Dado** `status` presente en 84 de 110 documentos, **Cuando** se inspecciona, **Entonces**
    `presentIn: 84`, `missingIn: 26` → `inspecciona_presencia`.
  - **Dado** `service.tier`, **Cuando** se inspecciona, **Entonces** funciona sobre el path anidado →
    `inspecciona_anidado`.
- **Dependencias**: E20-H01.
- **Pruebas**: `crates/lodestar-core/tests/`.
- **Frontera (mcp.yml)**: no.

### E20-H03 — Sustituir `schema_inspect` por `metadata_inspect` y retirar `core::schema`

- **Objetivo**: la tool MCP `schema_inspect` pasa a ser `metadata_inspect`; `core::schema` con
  `DocType`/relaciones tipadas/`.lodestar/schema.yaml` desaparece.
- **Referencias**: `ARCHITECTURE.md §20.10` (*"Sustituir `schema_inspect` por `metadata_inspect`"*) ·
  `crates/lodestar-app/src/lib.rs` (`schema_inspect`, `SchemaInspection`), `crates/lodestar-mcp/src/tools.rs`,
  `crates/lodestar-core/src/schema.rs`, `crates/lodestar-workspace/src/schema.rs`.
- **Alcance**:
  - Tool `metadata_inspect` con `mode: "catalog" | "field"` (+ `field` cuando `mode: field`),
    sirviendo los resultados de H01/H02.
  - **Retirar `schema_inspect`** de `tools::list()`/`call()` y `SchemaInspection`.
  - **Borrar `core::schema`** entero: `Schema`, `DocType`, `requiredFields`, `allowedStatuses`,
    `bodyTemplate`, `RelationDef`, y `validate_schema`/`validate_relations`. Con ellos mueren las
    variantes `CheckCode::SchemaReqfield`/`SchemaStatus`/`RelTarget`/`RelCard`/`RelType`/`ExtrefMissing`
    (que E16-H05 dejó sin productor) y `WorkspaceSchema`/`.lodestar/schema.yaml`
    (`crates/lodestar-workspace/src/schema.rs`).
  - Retirar las refs externas por frontmatter (`implemented_by`/`verified_by`, `external_refs.rs`) si
    su único propósito era el `EXTREF-MISSING` — o conservar `referenceRoots` si sostiene la write
    policy; **decidir y justificar** (interactúa con `assert_writable`).
  - La `guarda de test` `diagnosticos.rs` que nombra `LinkStub`/`LinkRel` (E17) puede resolverse aquí
    si toca ese fichero, retirando esas variantes ya sin productor.
- **Fuera de alcance**: la política de validación (E20-H04).
- **Criterios de aceptación**:
  - **Dado** el MCP, **Cuando** se pide `tools/list`, **Entonces** aparece `metadata_inspect` y **no**
    `schema_inspect` → `tool_es_metadata_inspect`.
  - **Dado** `metadata_inspect {mode: "catalog"}`, **Cuando** se llama, **Entonces** devuelve el
    catálogo de H01 → `metadata_inspect_catalog`.
  - **Dado** `metadata_inspect {mode: "field", field: "status"}`, **Entonces** devuelve la inspección
    de H02 → `metadata_inspect_field`.
  - Estructural: `grep -rn "DocType\|validate_schema\|schema.yaml" crates/*/src` no encuentra la
    maquinaria de schema → checklist.
- **Dependencias**: E20-H01, H02.
- **Pruebas**: `crates/lodestar-mcp/tests/`, `crates/lodestar-app/tests/`.
- **Frontera (mcp.yml)**: **sí** (retira `schema_inspect`, añade `metadata_inspect`).

### E20-H04 — Política de validación y diagnósticos de descubrimiento cableados

- **Objetivo**: `knowledge_check` responde la pregunta de `§20.9`; la política `validation`/
  `transactions` de la config se **aplica**; y los diagnósticos de descubrimiento llegan a las
  fachadas.
- **Referencias**: `ARCHITECTURE.md §20.9` · `REFACTOR_PHASE_2 §Fase 10 (Semántica de knowledge_check,
  Política de cambios)` · `crates/lodestar-workspace/src/config.rs` (secciones `validation`/
  `transactions` de E15-H08, cargadas pero sin aplicar), `crates/lodestar-workspace/src/discovery.rs`
  (los `Discovered::diagnostics` que hoy se descartan), `crates/lodestar-app/src/lib.rs`
  (`knowledge_check`).
- **Alcance**:
  - **Cablear los diagnósticos de descubrimiento**: `Workspace::document_set()`/`analyze()` incorporan
    los `Discovered::diagnostics` al `Analysis`, con la **severidad que fija la sección `validation`**
    de la config (default: `malformedFrontmatter: error`, `danglingDocumentLinks: error`,
    `missingWorkspaceFiles: warning`, `caseMismatch: warning`, `isolatedDocuments: ignore`).
  - La política `transactions.rejectNewErrors`/`allowExistingErrors` de `§Fase 10` se **aplica** en el
    gate de `change_apply`: un cambio no puede **introducir** errores nuevos, pero puede aplicarse
    sobre un workspace que ya los tiene (una reparación parcial es válida). Esto resucita
    `Severity::Warn`/`Err` como señal viva del pipeline (E16-H05 los dejó casi sin productor).
  - `knowledge_check` documenta y devuelve la semántica nueva: *"¿puede Lodestar interpretar y
    modificar este workspace con seguridad?"*.
- **Criterios de aceptación**:
  - **Dado** un workspace con un `.md` no-UTF8, **Cuando** se corre `knowledge_check`, **Entonces**
    el `DOC-NOT-UTF8` aparece en el reporte → `descubrimiento_llega_a_check`.
  - **Dado** `validation.caseMismatch: error` en la config, **Cuando** hay una colisión de
    capitalización, **Entonces** es error (no el warning por defecto) → `severidad_configurable`.
  - **Dado** un workspace que **ya** tiene un enlace roto, **Cuando** se aplica un cambio que **no**
    añade errores, **Entonces** el apply se permite (`allowExistingErrors`) → `apply_sobre_errores_previos`.
  - **Dado** un cambio que **introduciría** un enlace roto nuevo, **Cuando** se aplica con
    `rejectNewErrors`, **Entonces** se rechaza → `rechaza_errores_nuevos`.
- **Dependencias**: E20-H03.
- **Pruebas**: `crates/lodestar-app/tests/`, `crates/lodestar-workspace/tests/`.
- **Frontera (mcp.yml)**: **sí** (semántica de `knowledge_check`).

---

## Orden de construcción

```
H01 (catálogo) ─→ H02 (inspección de campo) ─→ H03 (tool + retirar core::schema) ─→ H04 (política)
```

## Criterio de salida

Un agente puede descubrir las convenciones de un workspace desconocido con `metadata_inspect` sin
que exista schema alguno; `grep -rn "DocType\|schema.yaml\|SCHEMA-REQFIELD" crates/*/src` no encuentra
la maquinaria OKF de schema; los diagnósticos de descubrimiento aparecen en `knowledge_check`; y un
cambio puede aplicarse sobre un repositorio que ya tiene problemas sin introducir otros nuevos.
