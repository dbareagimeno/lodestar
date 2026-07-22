# E10 — Esquemas y lectura headless

> **Fase**: `§19.8` fase 1 (`REFACTOR §16`). **Objetivo de la épica**: dar a un agente la capacidad de
> **comprender y auditar** la base sin tocar el filesystem: el subsistema de **esquemas** (`core::schema`,
> PURO), las nuevas primitivas de **identidad de contenido** (`ConceptRevision`/`WorkspaceRevision`), el
> crate **`lodestar-app`** con el **envelope** y los **códigos de error**, y las **5 tools de lectura/
> verificación** (`workspace_status`/`knowledge_search`/`knowledge_get`/`schema_inspect`/`knowledge_check`).
> Criterio de salida (`REFACTOR §16`): *un agente puede comprender y auditar la base sin usar
> herramientas de filesystem*.
> Referencias maestras: `ARCHITECTURE.md §19.2`, `§19.3`, `§19.6`, `§19.7` · `REFACTOR §6, §8–§10, §13`.

**Principio rector de la épica**: *el dominio en el core, el framing en `lodestar-app`, cero lógica en la
fachada*. Toda validación de schema y todo cálculo de revisión es **puro** (`core`); el envelope y el mapa
de códigos de error son **`lodestar-app`**; la tool MCP es un shell que llama un servicio y serializa.
Invariante #4: cada tipo nuevo se define **una vez** en `core::types` (salvo el envelope).

---

### E10-H01 — Scaffold del crate `lodestar-app` (servicios de caso de uso + envelope)
- **Objetivo**: crear el crate fino que ambas fachadas consumen, con el envelope y el punto de entrada de servicios.
- **Referencias**: `ARCHITECTURE.md §19.2` · decisión **D1 (C)**, **D3** · `REFACTOR §3, §13`.
- **Alcance**:
  - Nuevo crate `crates/lodestar-app` que depende de `lodestar-core` + `lodestar-workspace` (no de
    `rusqlite`/`git2`/`tokio`).
  - Struct `Envelope<T> { ok, workspace_revision: WorkspaceRevision, summary: String, data: T,
    diagnostics: Vec<Check>, warnings: Vec<String>, resource_links: Vec<ResourceLink> }` (wire camelCase).
  - Fachada de servicios `App` que abre un `Workspace` y expone métodos por caso de uso (se irán poblando).
- **Fuera de alcance**: los códigos de error (E10-H02); las tools concretas (E10-H08+).
- **Criterios de aceptación**:
  - Estructural (checklist): `cargo tree -p lodestar-app` **no** contiene `rusqlite`/`git2`/`tokio`.
  - **Dado** un `Envelope<Value>` serializado, **Cuando** se inspecciona, **Entonces** lleva las 7 claves
    `ok`/`workspaceRevision`/`summary`/`data`/`diagnostics`/`warnings`/`resourceLinks` → `envelope_shape`.
- **Dependencias**: E10-H03 (necesita `WorkspaceRevision`).
- **Pruebas**: `crates/lodestar-app/tests/`: `envelope_shape`.
- **Frontera (mcp.yml)**: no (infra).

### E10-H02 — Códigos de error estables en `core::types`
- **Objetivo**: el enum de 16 códigos de error del contrato (`REFACTOR §13`), con wire estable y mapeo desde errores.
- **Referencias**: `ARCHITECTURE.md §19.3` · `REFACTOR §13` · patrón `CheckCode` (`core/types.rs:102`).
- **Alcance**:
  - `pub enum ErrorCode` con `#[serde(rename)]` para `WORKSPACE_NOT_FOUND`, `WORKSPACE_RECOVERY_REQUIRED`,
    `CONCEPT_NOT_FOUND`, `AMBIGUOUS_REFERENCE`, `REVISION_CONFLICT`, `PLAN_STALE`, `PLAN_EXPIRED`,
    `PERMISSION_DENIED`, `INVALID_SCHEMA`, `NONCONFORMANT_RESULT`, `INBOUND_LINKS_EXIST`,
    `RELATION_CONSTRAINT_VIOLATION`, `WRITE_CONFLICT`, `RESULT_TOO_LARGE`, `RECOVERY_FAILED`, `INTERNAL_IO_ERROR`.
  - En `lodestar-app`: mapeo `CoreError`/`WorkspaceError` → `ErrorCode` + estructura de error con
    `code`/`message`/campos de recuperación (`expectedRevision`/`actualRevision`/`recovery`).
- **Fuera de alcance**: producir cada código (se emiten en E12/E13).
- **Criterios de aceptación**:
  - **Dado** `ErrorCode::RevisionConflict`, **Cuando** se serializa, **Entonces** `"REVISION_CONFLICT"`
    → `error_code_wire`.
  - **Dado** un `CoreError::InvalidRelPath`, **Cuando** se mapea, **Entonces** `PERMISSION_DENIED` o el
    código acordado (documentado) → `mapeo_core_error`.
  - Estructural: no existe otra definición de estos códigos fuera de `core::types` (grep en CI).
- **Dependencias**: —.
- **Pruebas**: `crates/lodestar-core/tests/`: `error_code_wire`; `lodestar-app`: `mapeo_core_error`.
- **Frontera (mcp.yml)**: no.

### E10-H03 — `ConceptRevision` + `WorkspaceRevision` (identidad de contenido determinista)
- **Objetivo**: elevar blake3 a identidad expuesta y calcular la revisión determinista del workspace.
- **Referencias**: `ARCHITECTURE.md §19.3` · `REFACTOR §6.2, §6.3` · `bundle.rs:361`, `store/index.rs:89`.
- **Alcance**:
  - `ConceptRevision(String)` = `blake3:<hex>` del contenido en disco de un `.md`; helper desde `[u8;32]`
    (reusa el `WriteOutcome.hash`).
  - `WorkspaceRevision(String)` con función pura `workspace_revision(files: &FileMap, writable: &[RelPath])`:
    filtra a `writableRoots`, ordena por `RelPath`, hashea cada contenido, combina `path+hash`, hash raíz.
  - **Excluye**: mtime, orden de fs, cachés/índices, **todo `.lodestar/`**, `referenceRoots`, ignorados.
- **Fuera de alcance**: exponerla en tools (E10-H08); optimistic concurrency (E12).
- **Criterios de aceptación**:
  - **Dado** el mismo `FileMap` en dos órdenes de inserción distintos, **Cuando** se calcula
    `workspace_revision`, **Entonces** el hash es idéntico → `revision_independiente_del_orden`.
  - **Dado** un `FileMap` y una copia con un fichero bajo `.lodestar/` añadido, **Cuando** se calcula,
    **Entonces** la revisión no cambia → `revision_excluye_lodestar`.
  - **Dado** un `FileMap` y una copia con `referenceRoots` distinto, **Cuando** se calcula, **Entonces**
    la revisión no cambia → `revision_excluye_reference_roots`.
  - **Dado** un cambio de un byte en un `.md` escribible, **Cuando** se calcula, **Entonces** la revisión
    cambia → `revision_sensible_al_contenido`.
- **Dependencias**: E9-H05 (necesita `writableRoots`).
- **Pruebas**: `crates/lodestar-core/tests/`: `revision_independiente_del_orden`, `revision_excluye_lodestar`,
  `revision_excluye_reference_roots`, `revision_sensible_al_contenido`.
- **Frontera (mcp.yml)**: no.

### E10-H04 — `ConceptRef` (identidad por path, id opcional/diferido)
- **Objetivo**: el tipo de referencia a un concepto usado por todas las tools.
- **Referencias**: `ARCHITECTURE.md §19.3` · `REFACTOR §6.1` · `RelPath` (`core/types.rs:27`).
- **Alcance**:
  - `ConceptRef { path: RelPath, id: Option<ConceptId> }` con deserialización que acepta `{ "path": … }`.
  - Resolución `App::resolve_ref(&ConceptRef) -> Result<RelPath, ErrorCode>`: `CONCEPT_NOT_FOUND` si no
    existe; `AMBIGUOUS_REFERENCE` reservado para el futuro id.
- **Fuera de alcance**: IDs estables/federación (no-goal, `REFACTOR §16`).
- **Criterios de aceptación**:
  - **Dado** `{ "path": "a/b.md" }`, **Cuando** se deserializa, **Entonces** `path == RelPath("a/b.md")`
    → `ref_por_path`.
  - **Dado** un `ConceptRef` a un path inexistente, **Cuando** se resuelve, **Entonces** `CONCEPT_NOT_FOUND`
    → `ref_inexistente`.
  - **Dado** `{ "path": "../x" }`, **Cuando** se deserializa, **Entonces** error (`RelPath` rechaza) →
    `ref_rechaza_traversal`.
- **Dependencias**: E10-H02.
- **Pruebas**: `crates/lodestar-core/tests/`: `ref_por_path`, `ref_rechaza_traversal`; `app`: `ref_inexistente`.
- **Frontera (mcp.yml)**: no.

### E10-H05 — `core::schema`: tipo `Schema` + parser de `.lodestar/schema.yaml`
- **Objetivo**: el modelo puro de esquemas (DocType, campos, relaciones, lifecycle, plantillas) y su carga.
- **Referencias**: `ARCHITECTURE.md §19.2` · `REFACTOR §4, §9.4` · patrón `Config::load` (`config.rs:41`).
- **Alcance**:
  - Módulo `core::schema` con `Schema { version, types: BTreeMap<String, DocType> }`,
    `DocType { name, description, required_fields: Vec<String>, allowed_statuses: Vec<String>,
    fields, relations: BTreeMap<String, RelationDef>, rules, body_template }`,
    `RelationDef { target_types: Vec<String>, cardinality }`.
  - Loader en `workspace`: leer `.lodestar/schema.yaml` y `.lodestar/templates/` a `Schema` (I/O en
    workspace; el core solo recibe el `Schema` ya deserializado). **El core nunca abre ficheros.**
  - Un bundle **sin** `schema.yaml` → `Schema` vacío/permisivo (compat con bundles OKF actuales).
- **Fuera de alcance**: validar contra el schema (E10-H07, E11-H03); aplicar plantillas (E12-H05).
- **Criterios de aceptación**:
  - **Dado** un `schema.yaml` con un `DocType` `decision` (requiredFields/allowedStatuses), **Cuando** se
    carga, **Entonces** `schema.types["decision"].required_fields == ["title","status","rationale"]`
    → `carga_doctype`.
  - **Dado** un bundle sin `schema.yaml`, **Cuando** se carga, **Entonces** `Schema` vacío permisivo (no
    error) → `sin_schema_permisivo`.
  - Estructural (pureza): `core::schema` no importa `std::fs`/`rusqlite`/`git2` (grep + `core-purity` de CI).
- **Dependencias**: E9-H05.
- **Pruebas**: `crates/lodestar-core/tests/`: `carga_doctype` (con `Schema` en memoria), `sin_schema_permisivo`;
  loader en `crates/lodestar-workspace/tests/`.
- **Frontera (mcp.yml)**: no.

### E10-H06 — Extensión de `Check` + nuevas familias de `CheckCode` (SCHEMA-*, REL-*)
- **Objetivo**: enriquecer el diagnóstico (id/range/related/fixes) y añadir los códigos schema-driven, aditivamente.
- **Referencias**: `ARCHITECTURE.md §19.3` · decisión **D-CheckCode** · `REFACTOR §10` · `core/types.rs:102,117`.
- **Alcance**:
  - Extender `Check` con campos **opcionales**: `id: Option<String>` (estable dentro de una revisión),
    `range: Option<Range { start_line, end_line }>`, `related: Vec<RelPath>`,
    `fixes: Vec<Fix { fix_id, title, safe }>`. Los 15 checks OKF los dejan `None`/vacíos.
  - Añadir a `CheckCode` las variantes **estáticas** `SCHEMA-REQFIELD`, `SCHEMA-STATUS`, `REL-TARGET`,
    `REL-CARD`, `REL-TYPE` (wire con guion) + sus claves i18n.
- **Fuera de alcance**: producir estos checks (E10-H07, E11-H03); espacio de códigos dinámico (descartado).
- **Criterios de aceptación**:
  - **Dado** un `Check` de un código OKF clásico, **Cuando** se serializa, **Entonces** `fixes` es `[]` y
    `range` ausente/`null` (retro-compat) → `check_extension_retrocompat`.
  - **Dado** `CheckCode::SchemaReqfield`, **Cuando** se serializa, **Entonces** `"SCHEMA-REQFIELD"` →
    `schema_code_wire`.
  - Estructural: existe clave i18n para cada código nuevo (checklist del catálogo).
- **Dependencias**: —.
- **Pruebas**: `crates/lodestar-core/tests/`: `check_extension_retrocompat`, `schema_code_wire`; round-trip serde.
- **Frontera (mcp.yml)**: no.

### E10-H07 — Validación schema-driven en `core::schema` (requiredFields, allowedStatuses)
- **Objetivo**: producir `SCHEMA-REQFIELD`/`SCHEMA-STATUS` validando conceptos contra su `DocType`.
- **Referencias**: `ARCHITECTURE.md §19.2, §19.3` · `REFACTOR §9.4, §17` · `core::conform`.
- **Alcance**:
  - Función pura `validate_schema(bundle: &Bundle, schema: &Schema) -> Vec<Check>`: por cada concepto con
    `type` conocido, comprobar `required_fields` (falta → `SCHEMA-REQFIELD`, severidad `Err`) y que
    `status` ∈ `allowed_statuses` (fuera → `SCHEMA-STATUS`, `Err`).
  - Integrar en `analyze`/`conform` de forma **aditiva** (un bundle sin schema no cambia su veredicto actual).
- **Fuera de alcance**: relaciones tipadas (E11-H03); paths externos (E11-H04).
- **Criterios de aceptación**:
  - **Dado** un `DocType decision` con `requiredFields:[rationale]` y un concepto `decision` sin
    `rationale`, **Cuando** se valida, **Entonces** un `Check{code:SCHEMA-REQFIELD, level:Err}` sobre ese
    path → `falta_campo_obligatorio` (benchmark §17: "Crear un concepto sin campo obligatorio → rechazado").
  - **Dado** un concepto con `status: invented` fuera de `allowedStatuses`, **Cuando** se valida,
    **Entonces** `SCHEMA-STATUS` → `status_no_permitido`.
  - **Dado** un bundle sin `schema.yaml`, **Cuando** se valida, **Entonces** cero checks schema (compat) →
    `sin_schema_sin_checks`.
- **Dependencias**: E10-H05, E10-H06.
- **Pruebas**: `crates/lodestar-core/tests/`: `falta_campo_obligatorio`, `status_no_permitido`,
  `sin_schema_sin_checks`; fixture con `schema.yaml`.
- **Frontera (mcp.yml)**: no.

### E10-H08 — Tool `workspace_status`
- **Objetivo**: la primera tool de cada sesión: config activa, capacidades, conformidad, recovery.
- **Referencias**: `ARCHITECTURE.md §19.6` · `REFACTOR §9.1, §7`.
- **Alcance**:
  - Servicio `App::workspace_status()` → `{ workspaceRevision, root, knowledgeRoots, referenceRoots,
    formatVersion, schemaVersion, conformant, counts{concepts,links,orphans,dangling,errors,warnings},
    capabilities{writes,transactions,revert,schemas,externalReferences}, recovery{pendingTransaction} }`.
  - Tool MCP `workspace_status` (shell sobre el servicio) con `inputSchema` `{}` + `outputSchema` (schemars).
- **Fuera de alcance**: `recovery.pendingTransaction` real (E13-H06 lo puebla; aquí siempre `false`).
- **Criterios de aceptación**:
  - **Dado** un workspace con 3 orphans, **Cuando** se llama `workspace_status`, **Entonces**
    `counts.orphans == 3` y `workspaceRevision` presente → `status_counts` (benchmark §17).
  - **Dado** el perfil `readonly`, **Cuando** se llama, **Entonces** `capabilities.writes == false` →
    `status_capabilities_readonly`.
  - Estructural: la tool declara `outputSchema` (checklist mcp.yml).
- **Dependencias**: E10-H01, E10-H03.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `status_counts`, `status_capabilities_readonly`.
- **Frontera (mcp.yml)**: **sí**.

### E10-H09 — Tool `knowledge_search` (sustituye `query`)
- **Objetivo**: localizar conceptos con filtros, snippets y paginación por cursor, sin devolver cuerpos.
- **Referencias**: `ARCHITECTURE.md §19.6` · `REFACTOR §9.2, §15` · `core::query` (`query.rs`), FTS5 del store.
- **Alcance**:
  - Servicio `App::knowledge_search(text, filters{types,statuses,tags,pathPrefix,references,referencedBy,
    linkedTo,is:orphan,is:dangling,has:diagnostics,has:backlinks}, sort, limit, cursor)` →
    `{ results:[{path,id,type,title,status,description,tags,snippet,score,revision}], nextCursor, totalApproximate }`.
  - Reusa el `tokenize_query`/`match_token` del core (subcadena) + FTS5 como acelerador superset (invariante #3/#11).
  - Límite por defecto 20, máximo 100; snippets compactos; **nunca** cuerpos completos.
- **Fuera de alcance**: enriquecer score con ranking avanzado (aditivo futuro).
- **Criterios de aceptación**:
  - **Dado** un corpus con un concepto que casa "autenticación", **Cuando** se busca ese texto, **Entonces**
    aparece con `snippet` y `revision`, sin `body` → `search_sin_cuerpos` (benchmark §17: "Encontrar una
    decisión por significado").
  - **Dado** `filters.types:[decision]`, **Cuando** se busca, **Entonces** solo conceptos `type:decision`
    → `search_filtra_tipo`.
  - **Dado** `limit:20` y 50 resultados, **Cuando** se pagina con `nextCursor`, **Entonces** la 2ª página
    no repite ni omite → `search_paginacion`.
- **Dependencias**: E10-H01, E10-H03.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `search_sin_cuerpos`, `search_filtra_tipo`, `search_paginacion`.
- **Frontera (mcp.yml)**: **sí**.

### E10-H10 — Tool `knowledge_get` (sustituye la lectura directa)
- **Objetivo**: obtener un concepto concreto con `include` selectivo y selección de secciones.
- **Referencias**: `ARCHITECTURE.md §19.6` · `REFACTOR §9.3` · `Bundle::backlinks` (`bundle.rs:198`).
- **Alcance**:
  - Servicio `App::knowledge_get(ref, include:[frontmatter,body,outgoingLinks,backlinks,diagnostics,
    externalReferences], sections?)` → `{ concept{path,revision,frontmatter,body,outgoingLinks,backlinks,
    externalReferences,diagnostics} }`.
  - Selección de secciones por `headingPath` para acotar el contexto (`REFACTOR §9.3`).
- **Fuera de alcance**: editar (E12).
- **Criterios de aceptación**:
  - **Dado** un concepto existente, **Cuando** se pide con `include:[frontmatter,revision]`, **Entonces**
    devuelve la `revision` (== `ConceptRevision`) → `get_incluye_revision`.
  - **Dado** `sections:[["Security","Token rotation"]]`, **Cuando** se pide, **Entonces** el body devuelto
    es solo esa subsección → `get_por_seccion`.
  - **Dado** un path inexistente, **Cuando** se pide, **Entonces** `CONCEPT_NOT_FOUND` → `get_inexistente`.
- **Dependencias**: E10-H01, E10-H03, E10-H04.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `get_incluye_revision`, `get_por_seccion`, `get_inexistente`.
- **Frontera (mcp.yml)**: **sí**.

### E10-H11 — Tool `schema_inspect`
- **Objetivo**: que el agente descubra contratos (tipos, campos, relaciones, lifecycle, plantillas) antes de escribir.
- **Referencias**: `ARCHITECTURE.md §19.2` · `REFACTOR §9.4`.
- **Alcance**:
  - Servicio `App::schema_inspect(mode)` con modos `catalog`/`type`/`field`/`relation`/`diagnosticCode`/
    `lifecycle`/`template` → proyección del `Schema` cargado.
  - Salida por tipo: `{ schemaVersion, type{name,description,requiredFields,allowedStatuses,fields,relations,
    rules,bodyTemplate} }`.
- **Fuera de alcance**: editar el schema (fuera de scope v2).
- **Criterios de aceptación**:
  - **Dado** un `DocType decision`, **Cuando** se llama `schema_inspect(type="decision")`, **Entonces**
    devuelve sus `requiredFields`/`allowedStatuses`/`bodyTemplate` → `inspect_type`.
  - **Dado** el modo `catalog`, **Cuando** se llama, **Entonces** lista todos los `DocType` disponibles →
    `inspect_catalog`.
  - **Dado** un bundle sin schema, **Cuando** se llama `catalog`, **Entonces** catálogo vacío (no error) →
    `inspect_sin_schema`.
- **Dependencias**: E10-H05.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `inspect_type`, `inspect_catalog`, `inspect_sin_schema`.
- **Frontera (mcp.yml)**: **sí**.

### E10-H12 — Tool `knowledge_check` (sustituye `conformance_check`)
- **Objetivo**: auditar conocimiento con scopes y severidad mínima; incluir fixes sugeridos.
- **Referencias**: `ARCHITECTURE.md §19.6` · `REFACTOR §10, §17` · `core::conform` + E10-H07.
- **Alcance**:
  - Servicio `App::knowledge_check(scope, minimumSeverity, includeSuggestedFixes, limit, cursor)` →
    `{ conformant, summary{errors,warnings,info}, diagnostics:[Check con id/range/fixes], workspaceRevision, nextCursor }`.
  - Scopes: `workspace` · `concept{ref}` · `paths{paths}` · `affected{refs,depth}` (usa vecindad/blast-radius).
  - IDs de diagnóstico **estables dentro de una revisión** (`diag:blake3:…`).
- **Fuera de alcance**: aplicar fixes (E12-H07 `apply_fix`).
- **Criterios de aceptación**:
  - **Dado** un `.md` editado a mano con frontmatter inválido, **Cuando** se hace `knowledge_check` de
    scope `workspace`, **Entonces** aparece el diagnóstico → `check_detecta_edicion_directa` (benchmark §17).
  - **Dado** `scope:affected` con un ref y `depth:2`, **Cuando** se llama, **Entonces** solo diagnósticos
    del vecindario → `check_scope_affected`.
  - **Dado** la misma revisión dos veces, **Cuando** se hace `knowledge_check`, **Entonces** los `id` de
    diagnóstico coinciden → `check_ids_estables`.
- **Dependencias**: E10-H01, E10-H03, E10-H06, E10-H07.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `check_detecta_edicion_directa`, `check_scope_affected`,
  `check_ids_estables`.
- **Frontera (mcp.yml)**: **sí**.

### E10-H13 — `outputSchema` (schemars) + reescritura de `contracts/mcp.yml` (lectura)
- **Objetivo**: activar `outputSchema` en las tools de lectura y reflejar la superficie nueva en el contrato.
- **Referencias**: `ARCHITECTURE.md §19.6` · decisión **D6b** · `REFACTOR §13` · `DECISIONES §3`.
- **Alcance**:
  - Derivar `outputSchema` con la feature `schemars` (ya preparada) en las respuestas de las 5 tools de esta épica.
  - Reescribir la parte de lectura de `contracts/mcp.yml` (10 tools objetivo; las de cambio llegan en E12/E13).
  - Golden cross-fachada: la salida de cada tool == el servicio de `lodestar-app` directo.
- **Fuera de alcance**: `rmcp` (diferido); tools de cambio (E12/E13).
- **Criterios de aceptación**:
  - **Dado** `tools/list`, **Cuando** se inspecciona `workspace_status`, **Entonces** incluye `outputSchema`
    → `tools_declaran_outputschema`.
  - Estructural (checklist): `/contrato --check` pasa contra el `mcp.yml` reescrito.
- **Dependencias**: E10-H08…E10-H12.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `tools_declaran_outputschema`; golden cross-fachada.
- **Frontera (mcp.yml)**: **sí**.

---

## Orden de construcción (E10)

Base de tipos primero: `E10-H02` (códigos), `E10-H06` (Check/CheckCode) y `E10-H03` (revisiones) son
independientes; `E10-H03` habilita `E10-H01` (crate app con envelope). Luego `E10-H04` (ConceptRef),
`E10-H05` (schema + loader) → `E10-H07` (validación schema). Con app+revisiones+schema listos van las
tools: `E10-H08`, `E10-H09`, `E10-H10`, `E10-H11`, `E10-H12` (paralelizables entre sí). Cierra
`E10-H13` (outputSchema + contrato). Ninguna historia **[BLOQUEADA]**.
