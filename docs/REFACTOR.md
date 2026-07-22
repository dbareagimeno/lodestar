# Plan integral redefinido del MCP de Lodestar

## 1. Decisión de producto

Lodestar debe convertirse en un **motor headless de integridad semántica para bases de conocimiento Markdown gestionadas por humanos y agentes**.

No debe competir con Obsidian, implementar un editor generalista ni gestionar Git. Su función es ofrecer una capa fiable para:

- buscar y consultar conocimiento;
- entender esquemas, tipos y relaciones;
- detectar inconsistencias;
- analizar el impacto de cambios;
- planificar modificaciones semánticas;
- validar el resultado antes de escribir;
- publicar cambios recuperables;
- proteger el workspace frente a sobrescrituras y estados incoherentes.

El flujo principal del producto será:

```text
descubrir
→ buscar
→ leer
→ analizar
→ planificar
→ validar
→ aplicar
→ verificar
```

Lodestar podrá utilizarse desde Claude Code, Codex, otros clientes MCP y la CLI, sin depender de un editor, de una interfaz gráfica ni de Git.

---

## 2. Límites del producto

### Lodestar sí debe poseer

```text
esquemas
grafo de conocimiento
validación
diagnósticos
análisis de impacto
diferencias semánticas
planificación de cambios
control de concurrencia
publicación recuperable
```

### Lodestar no debe poseer

```text
edición general de archivos
gestión de código fuente
Git
commits
ramas
merges
remotos
terminal
ejecución arbitraria de comandos
sincronización de repositorios
interfaz gráfica generalista
```

Claude Code y Codex ya disponen de herramientas para editar código, ejecutar comandos y utilizar Git. Lodestar debe aportar únicamente las capacidades que esos entornos no pueden proporcionar por sí solos de forma fiable.

---

## 3. Principio fundamental

El MCP no debe exponer otra versión de:

```text
read_file
write_file
replace_text
list_directory
```

Debe exponer operaciones sobre el **modelo de conocimiento**:

```text
Crea, modifica, mueve o elimina conocimiento
solo cuando la operación respeta el esquema,
las relaciones y las invariantes del workspace.
```

Arquitectura conceptual:

```text
Claude Code / Codex
        │
        ├── herramientas propias
        │      ├── filesystem
        │      ├── código
        │      ├── terminal
        │      └── Git
        │
        └── Lodestar MCP
               ├── búsqueda estructurada
               ├── esquemas
               ├── grafo
               ├── validación
               ├── impacto
               └── cambios semánticos
```

Internamente:

```text
lodestar-core
├── parser
├── schema
├── graph
├── validation
├── semantic-diff
└── change-planning

lodestar-workspace
├── filesystem
├── indexing
├── revisions
├── locks
├── staging
├── write journal
├── publication
└── recovery

lodestar-cli
└── adaptador sobre los casos de uso

lodestar-mcp
└── adaptador sobre los mismos casos de uso
```

El MCP y la CLI no deben contener lógica de dominio. Ambos deben invocar los mismos servicios de aplicación.

---

# 4. Modelo de workspace

## 4.1. Estructura recomendada

Para un proyecto de software:

```text
project/
├── src/
├── tests/
├── infrastructure/
├── knowledge/
│   ├── architecture/
│   ├── decisions/
│   ├── domains/
│   ├── requirements/
│   ├── interfaces/
│   ├── operations/
│   └── glossary/
│
└── .lodestar/
    ├── config.yaml
    ├── schema.yaml
    ├── templates/
    └── runtime/
```

La base de conocimiento puede vivir dentro del mismo repositorio que el código, pero Lodestar solo debe controlar las raíces configuradas como escribibles.

## 4.2. Configuración

```yaml
workspace:
  writableRoots:
    - knowledge

  referenceRoots:
    - src
    - tests
    - infrastructure

  ignored:
    - node_modules
    - target
    - dist
    - .git
    - .lodestar/runtime
```

### `writableRoots`

Contienen los documentos que Lodestar puede modificar mediante transacciones.

### `referenceRoots`

Son visibles para validaciones y referencias, pero el MCP no puede modificarlos.

Ejemplo:

```yaml
type: component
title: Token Service

implemented_by:
  - src/auth/token_service.rs

verified_by:
  - tests/auth/token_service_test.rs
```

Lodestar puede comprobar que esos paths existen, pero no editar el código.

### `ignored`

No forman parte del índice, de la revisión del workspace ni de las validaciones normales.

---

# 5. Modelo transaccional

## 5.1. Qué significa “transacción”

No se trata de una transacción de base de datos. Los archivos Markdown siguen siendo la fuente de verdad.

Una transacción de Lodestar es:

```text
leer estado actual
→ recibir una propuesta de cambios
→ materializar el resultado en staging
→ validar el workspace resultante
→ calcular impacto y diff semántico
→ publicar el cambio
→ verificar el resultado
```

El objetivo es evitar estados parciales silenciosos cuando una modificación afecta a varios documentos.

## 5.2. Garantía realista

Un filesystem no proporciona una única operación atómica para sustituir múltiples archivos. Lodestar no debe prometer atomicidad absoluta del conjunto.

Debe ofrecer **semántica transaccional recuperable** mediante:

- staging completo;
- validación previa;
- bloqueo de escritura;
- control optimista de concurrencia;
- write-ahead journal;
- reemplazo atómico por archivo;
- copias de recuperación;
- recuperación después de cierre o fallo;
- validación posterior.

Proceso:

```text
1. Crear un journal de transacción.
2. Materializar el resultado completo en staging.
3. Validar staging.
4. Adquirir el lock del workspace.
5. Verificar que la revisión base no cambió.
6. Guardar copias de recuperación.
7. Sustituir archivos mediante renames atómicos.
8. Registrar cada sustitución en el journal.
9. Validar el workspace publicado.
10. Marcar la transacción como completada.
11. Limpiar staging y datos temporales.
```

Si Lodestar se cierra durante la publicación, al arrancar debe detectar el journal incompleto y ejecutar una estrategia determinista:

```text
completar publicación
o
restaurar estado anterior
```

## 5.3. Escrituras externas

Lodestar no puede impedir que Claude Code, Codex, un editor o el usuario modifiquen directamente los archivos.

El principio correcto es:

> Lodestar ofrece el camino seguro para modificar conocimiento y detecta los cambios realizados fuera de ese camino.

El sistema debe:

- recalcular o invalidar revisiones cuando cambien archivos;
- volver a indexar contenido externo;
- detectar conflictos antes de aplicar un plan;
- validar cambios directos mediante la CLI o CI;
- no asumir acceso exclusivo al workspace.

---

# 6. Tipos compartidos

## 6.1. `ConceptRef`

Primera versión:

```rust
pub struct ConceptRef {
    pub path: RelPath,
}
```

Los títulos no deben identificar una mutación porque pueden repetirse o cambiar.

Los IDs estables pueden ser opcionales:

```rust
pub struct ConceptRef {
    pub path: Option<RelPath>,
    pub id: Option<ConceptId>,
}
```

No deberían ser obligatorios en el primer rediseño. Podrán elevarse a capacidad principal cuando existan casos de uso claros de federación, referencias externas o identidad persistente después de movimientos.

## 6.2. `ConceptRevision`

Cada documento leído devuelve un hash de su contenido canónico:

```json
{
  "revision": "blake3:842fe..."
}
```

Las operaciones de escritura sobre un documento existente deben especificar la revisión leída:

```json
{
  "expectedRevision": "blake3:842fe..."
}
```

## 6.3. `WorkspaceRevision`

Representa el estado canónico completo de las raíces escribibles.

Cálculo conceptual:

```text
ordenar paths normalizados
→ calcular hash de cada contenido
→ combinar path + hash
→ calcular hash raíz
```

No debe depender de fechas de modificación, orden del filesystem, cachés, índices regenerables, `.lodestar/runtime`, `referenceRoots` ni contenido ignorado.

## 6.4. `ChangeSet`

Describe una propuesta normalizada y validada:

```rust
pub struct ChangeSet {
    pub id: ChangeSetId,
    pub base_revision: WorkspaceRevision,
    pub operations: Vec<NormalizedOperation>,
    pub plan_hash: PlanHash,
    pub risk: RiskAssessment,
    pub semantic_diff: SemanticDiff,
    pub validation: ValidationReport,
    pub expires_at: Timestamp,
}
```

## 6.5. `ChangeReceipt`

Registra una aplicación completada:

```rust
pub struct ChangeReceipt {
    pub id: ReceiptId,
    pub change_set_id: ChangeSetId,
    pub previous_revision: WorkspaceRevision,
    pub result_revision: WorkspaceRevision,
    pub changed_paths: Vec<RelPath>,
    pub semantic_diff: SemanticDiff,
}
```

El receipt permite una reversión inmediata y limitada, pero no sustituye a un sistema de versionado.

---

# 7. Flujo recomendado para el agente

Las instrucciones del servidor MCP deben orientar al agente:

```text
1. Llama a workspace_status al comenzar.
2. Usa knowledge_search para localizar conceptos.
3. Usa knowledge_get antes de modificar un concepto.
4. Consulta schema_inspect cuando no conozcas el tipo o sus reglas.
5. Usa graph_query o impact_analyze antes de cambios estructurales.
6. Construye una propuesta con change_plan.
7. Revisa operaciones normalizadas, riesgo, diff y diagnósticos.
8. Ejecuta change_apply con el changeSetId.
9. Ejecuta knowledge_check cuando necesites una auditoría explícita.
10. Usa change_revert solo para revertir una transacción reciente y no alterada.
```

---

# 8. Superficie principal del MCP

La propuesta final consta de diez tools:

```text
READ
workspace_status
knowledge_search
knowledge_get
schema_inspect
graph_query
impact_analyze

VERIFY
knowledge_check

CHANGE
change_plan
change_apply
change_revert
```

---

# 9. Tools de lectura

## 9.1. `workspace_status`

Primera tool recomendada de cada sesión.

### Entrada

```json
{}
```

### Salida

```json
{
  "workspaceRevision": "blake3:...",
  "root": "/project",
  "knowledgeRoots": ["knowledge"],
  "referenceRoots": ["src", "tests", "infrastructure"],
  "formatVersion": "0.2",
  "schemaVersion": "1",
  "conformant": true,
  "counts": {
    "concepts": 183,
    "links": 521,
    "orphans": 3,
    "dangling": 1,
    "errors": 0,
    "warnings": 7
  },
  "capabilities": {
    "writes": true,
    "transactions": true,
    "revert": true,
    "schemas": true,
    "externalReferences": true
  },
  "recovery": {
    "pendingTransaction": false
  }
}
```

Responsabilidades:

- informar de la configuración activa;
- exponer las capacidades habilitadas;
- detectar transacciones incompletas;
- indicar el estado general de conformidad;
- dar al agente una visión compacta del workspace.

## 9.2. `knowledge_search`

Localiza conceptos sin devolver cuerpos completos.

### Entrada

```json
{
  "text": "autenticación de usuarios",
  "filters": {
    "types": ["decision", "architecture"],
    "statuses": ["accepted"],
    "tags": ["security"],
    "pathPrefix": "architecture/",
    "linkedTo": {
      "path": "systems/identity-provider.md"
    }
  },
  "sort": "relevance",
  "limit": 20,
  "cursor": null
}
```

### Salida

```json
{
  "results": [
    {
      "path": "architecture/authentication.md",
      "id": "authentication",
      "type": "architecture",
      "title": "Authentication architecture",
      "status": "accepted",
      "description": "Modelo de autenticación de la plataforma.",
      "tags": ["security", "identity"],
      "snippet": "...tokens de acceso y renovación...",
      "score": 0.92,
      "revision": "blake3:..."
    }
  ],
  "nextCursor": null,
  "totalApproximate": 1
}
```

Filtros previstos:

```text
type
status
tag
pathPrefix
references
referencedBy
linkedTo
is:orphan
is:dangling
has:diagnostics
has:backlinks
```

Reglas:

- 20 resultados por defecto;
- máximo 100;
- snippets compactos;
- paginación mediante cursor;
- nunca devolver todos los cuerpos.

## 9.3. `knowledge_get`

Obtiene un concepto concreto.

### Entrada

```json
{
  "ref": {
    "path": "architecture/authentication.md"
  },
  "include": [
    "frontmatter",
    "body",
    "outgoingLinks",
    "backlinks",
    "diagnostics",
    "externalReferences"
  ]
}
```

### Salida

```json
{
  "concept": {
    "path": "architecture/authentication.md",
    "revision": "blake3:...",
    "frontmatter": {
      "type": "architecture",
      "title": "Authentication architecture",
      "status": "accepted",
      "tags": ["security"]
    },
    "body": "# Authentication architecture\n\n...",
    "outgoingLinks": [],
    "backlinks": [],
    "externalReferences": [
      {
        "path": "src/auth/token_service.rs",
        "exists": true
      }
    ],
    "diagnostics": []
  }
}
```

Puede permitir selección de secciones para controlar el contexto:

```json
{
  "ref": {
    "path": "architecture/authentication.md"
  },
  "sections": [
    ["Security", "Token rotation"]
  ]
}
```

## 9.4. `schema_inspect`

Permite que el agente descubra contratos antes de crear o modificar documentos.

### Entrada

```json
{
  "type": "decision"
}
```

### Salida

```json
{
  "schemaVersion": "1",
  "type": {
    "name": "decision",
    "description": "Decisión técnica o de producto.",
    "requiredFields": ["title", "status", "rationale"],
    "allowedStatuses": [
      "proposed",
      "accepted",
      "rejected",
      "deprecated"
    ],
    "fields": {},
    "relations": {},
    "rules": [],
    "bodyTemplate": "# {{title}}\n\n## Context\n\n## Decision\n"
  }
}
```

Modos previstos:

```text
catalog
type
field
relation
diagnosticCode
lifecycle
template
```

## 9.5. `graph_query`

Consulta el estado actual del grafo.

Operaciones:

```text
backlinks
outgoing
neighborhood
path_between
orphans
dangling
cycles
components
```

### Ejemplo

```json
{
  "operation": "neighborhood",
  "ref": {
    "path": "architecture/authentication.md"
  },
  "depth": 2,
  "direction": "both",
  "limit": 100
}
```

### Salida

```json
{
  "nodes": [],
  "edges": [],
  "summary": {
    "nodeCount": 17,
    "edgeCount": 32,
    "truncated": false
  },
  "nextCursor": null
}
```

## 9.6. `impact_analyze`

Analiza un cambio hipotético sin crear un change set completo.

### Entrada

```json
{
  "ref": {
    "path": "architecture/authentication.md"
  },
  "proposedOperation": {
    "kind": "deprecate"
  },
  "depth": 3
}
```

### Salida

```json
{
  "summary": {
    "directlyAffected": 7,
    "transitivelyAffected": 21,
    "blockingReferences": 3,
    "risk": "high"
  },
  "affectedConcepts": [],
  "blockingReferences": [
    {
      "path": "decisions/mobile-login.md",
      "reason": "Depende de un concepto que pasaría a deprecated."
    }
  ],
  "recommendations": [
    "Actualizar o redirigir las tres relaciones obligatorias antes de aplicar el cambio."
  ]
}
```

Casos previstos:

```text
move
delete
deprecate
transition_status
change_relation
replace_concept
```

---

# 10. Tool de validación

## 10.1. `knowledge_check`

Audita conocimiento existente.

### Entrada

```json
{
  "scope": {
    "kind": "workspace"
  },
  "minimumSeverity": "warning",
  "includeSuggestedFixes": true,
  "limit": 100,
  "cursor": null
}
```

Scopes:

```json
{ "kind": "workspace" }
```

```json
{
  "kind": "concept",
  "ref": {
    "path": "architecture/authentication.md"
  }
}
```

```json
{
  "kind": "paths",
  "paths": [
    "architecture/authentication.md",
    "decisions/token-rotation.md"
  ]
}
```

```json
{
  "kind": "affected",
  "refs": [
    {
      "path": "architecture/authentication.md"
    }
  ],
  "depth": 2
}
```

### Salida

```json
{
  "conformant": false,
  "summary": {
    "errors": 2,
    "warnings": 5,
    "info": 1
  },
  "diagnostics": [
    {
      "id": "diag:blake3:...",
      "code": "REL-TARGET-MISSING",
      "severity": "error",
      "path": "characters/alice.md",
      "range": {
        "startLine": 18,
        "endLine": 18
      },
      "message": "La relación «appears_in» apunta a un capítulo inexistente.",
      "related": [],
      "fixes": [
        {
          "fixId": "fix:...",
          "title": "Eliminar la relación rota",
          "safe": true
        }
      ]
    }
  ],
  "workspaceRevision": "blake3:...",
  "nextCursor": null
}
```

Los IDs de diagnóstico deben ser estables dentro de una revisión concreta.

---

# 11. Tools de escritura

## 11.1. `change_plan`

Es la tool central. No modifica el workspace: normaliza, simula y valida una propuesta.

### Entrada

```json
{
  "expectedWorkspaceRevision": "blake3:...",
  "operations": [
    {
      "op": "patch_frontmatter",
      "ref": {
        "path": "architecture/authentication.md"
      },
      "expectedRevision": "blake3:...",
      "patch": {
        "status": "deprecated"
      }
    },
    {
      "op": "create",
      "path": "architecture/authentication-v2.md",
      "concept": {
        "type": "architecture",
        "title": "Authentication architecture v2",
        "status": "proposed",
        "body": "# Authentication architecture v2\n\n..."
      }
    },
    {
      "op": "add_relation",
      "source": {
        "path": "architecture/authentication-v2.md"
      },
      "relation": "supersedes",
      "target": {
        "path": "architecture/authentication.md"
      }
    }
  ],
  "policy": {
    "requireConformantResult": true,
    "allowWarnings": true
  }
}
```

### Salida

```json
{
  "changeSetId": "cs_01J...",
  "baseWorkspaceRevision": "blake3:...",
  "planHash": "blake3:...",
  "canApply": true,
  "expiresAt": "2026-07-21T12:30:00Z",
  "normalizedOperations": [],
  "risk": {
    "level": "medium",
    "reasons": [
      "Depreca un concepto con siete backlinks."
    ]
  },
  "semanticDiff": {
    "created": ["architecture/authentication-v2.md"],
    "modified": ["architecture/authentication.md"],
    "deleted": [],
    "moved": [],
    "frontmatterChanges": [],
    "bodyChanges": [],
    "relationChanges": [],
    "diagnosticsIntroduced": [],
    "diagnosticsResolved": []
  },
  "impact": {
    "affectedConcepts": 8,
    "generatedFiles": 0
  },
  "diagnosticsBefore": {
    "errors": 0,
    "warnings": 1
  },
  "diagnosticsAfter": {
    "errors": 0,
    "warnings": 1
  }
}
```

Operaciones admitidas:

### Contenido

```text
create
patch_frontmatter
replace_body
edit_section
replace_text
```

### Estructura

```text
move
delete
```

### Semántica

```text
add_relation
remove_relation
transition_status
apply_fix
```

#### `edit_section`

```json
{
  "op": "edit_section",
  "ref": {
    "path": "architecture/authentication.md"
  },
  "expectedRevision": "blake3:...",
  "headingPath": ["Security", "Token rotation"],
  "mode": "replace",
  "content": "Los refresh tokens se rotan..."
}
```

#### `replace_text`

```json
{
  "op": "replace_text",
  "ref": {
    "path": "architecture/authentication.md"
  },
  "expectedRevision": "blake3:...",
  "oldText": "Access tokens expire after 60 minutes.",
  "newText": "Access tokens expire after 15 minutes.",
  "expectedOccurrences": 1
}
```

#### `move`

```json
{
  "op": "move",
  "ref": {
    "path": "architecture/authentication.md"
  },
  "destination": "security/authentication.md",
  "rewriteInboundLinks": true
}
```

#### `delete`

```json
{
  "op": "delete",
  "ref": {
    "path": "architecture/legacy-auth.md"
  },
  "inboundLinksPolicy": "reject"
}
```

Políticas:

```text
reject
retarget
remove_links
create_stub
```

El valor predeterminado debe ser `reject`.

#### Persistencia del plan

```text
.lodestar/runtime/plans/cs_01J.json
```

Cada plan conserva operaciones normalizadas, revisión base, hash, caducidad, diff, impacto y validación.

## 11.2. `change_apply`

Solo aplica un plan vigente y previamente calculado.

### Entrada

```json
{
  "changeSetId": "cs_01J...",
  "expectedWorkspaceRevision": "blake3:..."
}
```

### Salida

```json
{
  "receiptId": "receipt_01J...",
  "applied": true,
  "previousWorkspaceRevision": "blake3:...",
  "workspaceRevision": "blake3:...",
  "changedPaths": [],
  "semanticDiff": {},
  "conformance": {
    "conformant": true,
    "errors": 0,
    "warnings": 1
  }
}
```

Proceso interno:

```text
1. Cargar el plan.
2. Comprobar su caducidad.
3. Comprobar la revisión esperada.
4. Volver a normalizar y validar.
5. Verificar el planHash.
6. Materializar el resultado en staging.
7. Validar staging.
8. Adquirir el lock.
9. Verificar de nuevo WorkspaceRevision.
10. Crear journal y copias de recuperación.
11. Publicar reemplazos.
12. Reindexar.
13. Validar el resultado.
14. Crear receipt.
15. Completar y limpiar la transacción.
```

No se expondrá `allow_nonconformant` en cada llamada. La política se configura al iniciar el servidor:

```bash
lodestar-mcp --policy strict
```

La política estricta debe ser la predeterminada.

## 11.3. `change_revert`

Revierte exclusivamente una transacción reciente y conocida.

### Entrada

```json
{
  "receiptId": "receipt_01J...",
  "expectedWorkspaceRevision": "blake3:..."
}
```

Condiciones:

- el receipt existe;
- no ha caducado;
- el workspace sigue en la revisión producida;
- los archivos afectados no han cambiado;
- las copias de recuperación siguen disponibles;
- el estado restaurado puede validarse.

Configuración:

```yaml
transactions:
  retainReceiptsFor: 24h
  maximumReceipts: 20
```

`change_revert` no es historial general y no reemplaza a Git.

---

# 12. Perfiles

## `readonly`

```text
workspace_status
knowledge_search
knowledge_get
schema_inspect
graph_query
impact_analyze
knowledge_check
```

## `standard`

Añade:

```text
change_plan
change_apply
change_revert
```

Importación, exportación y mantenimiento deben comenzar en la CLI:

```bash
lodestar import ./notes --dry-run
lodestar import ./notes --apply
lodestar export --format jsonl
lodestar doctor
lodestar rebuild-index
```

---

# 13. Contratos y errores

Los contratos deben definirse como tipos Rust y generar:

- `inputSchema`;
- `outputSchema`;
- documentación;
- fixtures;
- pruebas de compatibilidad.

Envelope común:

```json
{
  "ok": true,
  "workspaceRevision": "blake3:...",
  "summary": "Texto compacto para el modelo.",
  "data": {},
  "diagnostics": [],
  "warnings": [],
  "resourceLinks": []
}
```

Códigos de error:

```text
WORKSPACE_NOT_FOUND
WORKSPACE_RECOVERY_REQUIRED
CONCEPT_NOT_FOUND
AMBIGUOUS_REFERENCE
REVISION_CONFLICT
PLAN_STALE
PLAN_EXPIRED
PERMISSION_DENIED
INVALID_SCHEMA
NONCONFORMANT_RESULT
INBOUND_LINKS_EXIST
RELATION_CONSTRAINT_VIOLATION
WRITE_CONFLICT
RESULT_TOO_LARGE
RECOVERY_FAILED
INTERNAL_IO_ERROR
```

Ejemplo:

```json
{
  "code": "REVISION_CONFLICT",
  "message": "El concepto cambió desde que fue leído.",
  "expectedRevision": "blake3:842fe...",
  "actualRevision": "blake3:17cc9...",
  "recovery": "Ejecuta knowledge_get de nuevo y vuelve a crear el plan."
}
```

---

# 14. Seguridad

El servidor:

- se inicia con un único root;
- no permite cambiar de workspace mediante una tool;
- no acepta paths absolutos;
- rechaza `..`;
- impide escapes mediante symlinks;
- solo escribe dentro de `writableRoots`;
- no ejecuta comandos;
- no modifica código;
- no accede a red;
- no conoce Git;
- no aplica planes obsoletos;
- no borra conceptos referenciados sin una política explícita;
- no permite desactivar conformidad por llamada.

Auditoría:

```text
.lodestar/runtime/audit.jsonl
```

```json
{
  "timestamp": "2026-07-21T12:00:00Z",
  "client": "claude-code",
  "tool": "change_apply",
  "changeSetId": "cs_...",
  "baseRevision": "blake3:...",
  "resultRevision": "blake3:...",
  "paths": [],
  "result": "success"
}
```

La auditoría es local y no forma parte del conocimiento canónico.

---

# 15. Migración desde las tools actuales

| Tool actual | Decisión |
|---|---|
| `find_backlinks` | Integrar en `graph_query` |
| `find_orphans` | Integrar en `graph_query` y `knowledge_search` |
| `find_dangling` | Integrar en `graph_query` y `knowledge_check` |
| `neighborhood` | Integrar en `graph_query` |
| `conformance_check` | Sustituir por `knowledge_check` |
| `query` | Sustituir por `knowledge_search` |
| `create_concept` | Sustituir por `change_plan` + `change_apply` |
| `update_frontmatter` | Sustituir por `change_plan` + `change_apply` |
| `generate_index` | Automatizar o mover a CLI |
| `generate_tag_indexes` | Automatizar o mover a CLI |
| `history` | Eliminar |
| `last_conforming_commit` | Eliminar |
| `commit` | Eliminar |

Aliases temporales:

```text
find_backlinks → graph_query(operation="backlinks")
query → knowledge_search
conformance_check → knowledge_check
```

---

# 16. Roadmap

## Fase 0 — Reducción de alcance

```text
Eliminar Git del core, MCP y roadmap.
Congelar la UI.
Definir writableRoots y referenceRoots.
Separar contenido canónico de runtime.
Documentar el nuevo posicionamiento.
```

## Fase 1 — Lectura headless

```text
workspace_status
knowledge_search
knowledge_get
schema_inspect
knowledge_check
```

Criterio de salida:

> Un agente puede comprender y auditar la base sin utilizar directamente herramientas de filesystem.

## Fase 2 — Grafo e impacto

```text
graph_query
impact_analyze
typed relations
cycle detection
external path validation
```

Criterio de salida:

> Lodestar responde preguntas estructurales y anticipa consecuencias de cambios.

## Fase 3 — Planificación

```text
change_plan
operaciones normalizadas
diff semántico
impacto
risk assessment
validación previa
optimistic concurrency
```

Criterio de salida:

> Un agente puede proponer refactors complejos sin modificar archivos.

## Fase 4 — Publicación recuperable

```text
change_apply
locks
staging
write-ahead journal
copias de recuperación
publicación por archivo
crash recovery
receipts
change_revert
```

Criterio de salida:

> Los cambios complejos se publican de forma recuperable y los fallos no dejan corrupción silenciosa.

## Fase 5 — Integración con proyectos de software

```text
referenceRoots
validación de paths de código
knowledge checks en CI
configuración por proyecto
instrucciones para agentes
```

Criterio de salida:

> La base de conocimiento puede convivir con el código sin que Lodestar gestione Git ni edite el proyecto.

## Fase 6 — Evaluación y optimización

```text
benchmarks con Claude Code
benchmarks con Codex
uso de tokens
selección correcta de tools
workspaces grandes
concurrencia
fallos durante publicación
recuperación
```

Fuera del roadmap inicial:

```text
Git
UI generalista
federación
IDs obligatorios
resources complejos
prompts MCP
sampling
importación masiva MCP
exportación MCP
ejecución remota
sincronización
```

---

# 17. Benchmark funcional

| Escenario | Resultado esperado |
|---|---|
| Encontrar una decisión por significado | `knowledge_search` + `knowledge_get` |
| Crear un concepto válido | Plan aceptado y aplicado |
| Crear un concepto sin campo obligatorio | Plan rechazado |
| Mover un concepto con 30 backlinks | Enlaces actualizados dentro del mismo plan |
| Borrar un concepto referenciado | Rechazo con blockers |
| Modificar un concepto cambiado externamente | `REVISION_CONFLICT` |
| Cambiar cinco conceptos relacionados | Un único change set |
| Introducir una relación inválida | Error antes de escribir |
| Corregir safe fixes | Operaciones `apply_fix` |
| Revisar un refactor | Diff semántico en `change_plan` |
| Recuperar un cambio reciente | `change_revert` |
| Cerrar Lodestar durante publicación | Recuperación determinista |
| Intentar escribir fuera de `writableRoots` | Rechazo |
| Referenciar un archivo de código inexistente | Diagnóstico |
| Editar directamente un Markdown inválido | Detectado por `knowledge_check` |

Métricas:

```text
task success rate
número medio de tool calls
tool selection errors
tokens consumidos
conflictos detectados
operaciones inseguras rechazadas
escrituras parciales recuperadas
diagnósticos introducidos
capacidad de recuperación
latencia en workspaces grandes
```

---

# 18. Posicionamiento final

> **Lodestar es un motor headless de integridad semántica para bases de conocimiento Markdown. Permite que agentes busquen, comprendan, validen y modifiquen conocimiento mediante cambios planificados y recuperables, sin poseer el editor, Git ni el entorno de desarrollo.**

Mensaje resumido:

```text
Store knowledge as Markdown.
Understand it as a graph.
Validate it with schemas.
Change it through safe plans.
Use it from any MCP agent.
```

La diferenciación no está en editar Markdown. Está en convertir una colección de archivos en un modelo de conocimiento:

```text
tipado
relacionado
validable
consultable
analizable
modificable de forma recuperable
```

Ese debe ser el núcleo de Lodestar.
