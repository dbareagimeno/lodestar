# Goal: migrar Lodestar de OKF a workspaces Markdown universales

## Objetivo

Refactorizar Lodestar para abandonar OKF como formato obligatorio y convertirlo en un motor local para analizar, consultar y modificar de forma transaccional cualquier base de conocimiento compuesta por archivos Markdown dentro del directorio de trabajo.

Lodestar debe poder iniciarse desde el `cwd` de un proyecto, descubrir recursivamente todos los archivos Markdown incluidos, interpretar su frontmatter YAML cuando exista, resolver enlaces Markdown estándar entre documentos situados a cualquier profundidad y construir un grafo global del workspace.

El usuario no debe tener que:

* Adoptar OKF.
* Ejecutar una migración previa.
* Crear un `index.md`.
* Añadir frontmatter.
* Declarar un campo `type`.
* Ejecutar `lodestar init`.
* Organizar la documentación en una estructura concreta.
* Utilizar Obsidian ni ninguna sintaxis específica de Obsidian.

La funcionalidad de consultas sobre frontmatter debe conservarse y generalizarse para permitir consultar cualquier propiedad YAML, no solo un conjunto de campos definido por Lodestar.

---

# Resultado esperado

Desde cualquier proyecto:

```bash
cd my-project
lodestar-mcp
```

Lodestar debe usar el directorio actual como raíz del workspace y descubrir estructuras arbitrarias como:

```text
my-project/
├── README.md
├── AGENTS.md
├── architecture.md
├── docs/
│   ├── product.md
│   └── decisions/
│       ├── authentication.md
│       └── storage.md
├── packages/
│   ├── api/
│   │   ├── README.md
│   │   └── docs/
│   │       └── endpoints.md
│   └── web/
│       └── docs/
│           └── frontend.md
└── knowledge/
    └── roadmap/
        └── 2027.md
```

Todos los `.md` incluidos deben formar parte de una misma base de conocimiento, independientemente de la carpeta en la que se encuentren.

Un archivo situado en la raíz debe poder enlazar a otro situado varios niveles por debajo:

```markdown
Consulta la [arquitectura de autenticación](packages/api/docs/authentication.md).
```

Y un archivo profundo debe poder enlazar de vuelta a la raíz:

```markdown
Volver a la [visión general](../../../README.md).
```

Ambos enlaces deben resolverse dentro del mismo grafo.

---

# Principios de diseño

## 1. Markdown es el formato

Lodestar trabaja sobre Markdown estándar.

El frontmatter YAML es opcional y se considera metadata arbitraria del usuario.

Lodestar no define un formato documental propio.

## 2. El `cwd` es el workspace

La raíz del workspace será:

1. El valor de `--root`, cuando se proporcione.
2. En caso contrario, `std::env::current_dir()`.

La raíz debe permanecer fija durante toda la sesión MCP.

Todas las rutas expuestas por CLI, MCP, almacenamiento, receipts y diagnósticos deben ser relativas a esa raíz.

## 3. La estructura de carpetas no tiene semántica propia

No existen carpetas reservadas para conceptos, índices, logs, decisiones ni conocimiento.

No se debe asumir que:

* `docs/` contiene toda la documentación.
* `knowledge/` es la única raíz válida.
* `index.md` representa una colección.
* `README.md` representa el documento principal.
* Una carpeta contiene necesariamente un `index.md`.

## 4. Todos los documentos Markdown son iguales

`README.md`, `index.md`, `AGENTS.md` y cualquier otro `.md` deben tratarse como documentos normales.

Ningún nombre de archivo debe activar reglas especiales.

## 5. Compatible por defecto

Lodestar debe poder analizar cualquier workspace sin configuración.

La configuración debe ser opcional y servir para limitar descubrimiento, escrituras o diagnósticos, no para convertir el workspace en válido.

## 6. Sin soporte específico de Obsidian

En esta migración no implementar:

* Wikilinks `[[document]]`.
* Embeds `![[document]]`.
* Block references.
* Resolución por alias.
* Resolución por basename.
* Semántica de `.obsidian`.
* Enlaces inferidos mediante títulos.

Solo se deben procesar enlaces Markdown estándar.

## 7. Determinismo

La misma colección de archivos debe producir siempre:

* El mismo inventario.
* El mismo grafo.
* Los mismos backlinks.
* Los mismos diagnósticos.
* El mismo resultado de consulta.

No debe existir resolución heurística o ambigua de enlaces.

## 8. Seguridad de escritura

Ninguna operación debe:

* Escapar de la raíz del workspace.
* Escribir sobre archivos excluidos.
* Seguir symlinks no autorizados.
* Modificar archivos no incluidos en el plan.
* Aplicarse si la revisión del workspace ha cambiado de forma incompatible.

## 9. Transacciones independientes de OKF

Deben conservarse:

* `change_plan`.
* `change_apply`.
* `change_revert`.
* Revisiones de workspace y documento.
* Staging.
* Optimistic concurrency.
* Escrituras atómicas.
* Write-ahead journal.
* Recovery.
* Receipts.
* Auditoría.
* Rollback.

La seguridad transaccional debe aplicarse a Markdown genérico, no a documentos conformes con OKF.

---

# No objetivos

Esta migración no debe incluir:

* Compatibilidad específica con Obsidian.
* Un nuevo sistema formal de schemas.
* Validación de dominios documentales.
* Estados obligatorios.
* Tipos documentales obligatorios.
* Relaciones tipadas.
* Búsqueda vectorial.
* Embeddings.
* Una interfaz gráfica.
* Integración Git de primer nivel.
* Soporte para formatos distintos de Markdown.
* Resolución de enlaces por similitud.
* Interpretación semántica de carpetas.
* Generación obligatoria de índices.
* Migración automática destructiva de documentos existentes.

---

# Modelo conceptual nuevo

Lodestar debe dejar de modelar un bundle OKF y pasar a modelar un workspace Markdown.

```text
Workspace
├── root
├── discovery policy
├── write policy
├── document inventory
├── metadata index
├── link graph
├── diagnostics
├── search index
└── transaction state
```

Cada documento debe representarse aproximadamente como:

```rust
pub struct Document {
    pub path: RelativePath,
    pub raw: String,
    pub frontmatter: Option<ParsedFrontmatter>,
    pub body: String,
    pub content_hash: ContentHash,
}
```

El agregado analizable debe ser independiente del sistema de archivos:

```rust
pub struct DocumentSet {
    pub documents: FileMap,
}
```

El workspace con I/O debe encargarse de:

* Descubrimiento.
* Lectura.
* Watcher.
* Store.
* Planificación.
* Aplicación.
* Recovery.
* Persistencia derivada.

---

# Fase 1: congelar y retirar OKF

## Objetivo

Separar claramente la versión anterior del nuevo modelo y evitar compatibilidad histórica indefinida.

## Tareas

1. Etiquetar el comportamiento actual como última versión basada en OKF.
2. Conservar fixtures OKF solo para pruebas de compatibilidad y migración.
3. Declarar la siguiente versión como incompatible.
4. Eliminar OKF del posicionamiento principal del proyecto.
5. No mantener un modo OKF permanente en runtime.
6. Documentar que los antiguos workspaces seguirán siendo Markdown válido, pero perderán la semántica especial de OKF.

## Terminología que debe desaparecer de la API pública

* `OKF`.
* `bundle`.
* `concept`.
* `conformance`.
* `okf_version`.
* `OKF-IDX`.
* `OKF-LOG`.
* `in_index`.
* `concept type`.
* `concept status`.

## Sustituciones recomendadas

| Terminología anterior | Terminología nueva |
| --------------------- | ------------------ |
| Bundle                | Workspace          |
| Concept               | Document           |
| ConceptRef            | DocumentRef        |
| ConceptSummary        | DocumentSummary    |
| OKF diff              | Semantic diff      |
| Conformance           | Validation         |
| Conformant            | Valid              |
| Orphan                | Isolated document  |
| Bundle revision       | Workspace revision |

---

# Fase 2: hacer que el `cwd` sea la raíz

## Comportamiento

```bash
lodestar-mcp
```

Debe equivaler a:

```bash
lodestar-mcp --root .
```

## Requisitos

* Resolver y canonicalizar la raíz al iniciar.
* No permitir cambiar la raíz durante la sesión.
* Representar públicamente todas las rutas como relativas.
* Rechazar rutas absolutas en operaciones MCP.
* Rechazar componentes que escapen con `..`.
* Verificar nuevamente los límites antes de aplicar cada escritura.
* No seguir symlinks por defecto.

## Configuración opcional

```yaml
workspace:
  root: "."
```

Esta configuración no debe ser necesaria para el caso normal.

### Adenda E35-H01 — presupuesto de memoria (issue #53)

La issue #53 está titulada originalmente **E34-H01**; la trazabilidad local normativa es
**E34-H01 → E35-H01**. La configuración pública puede incluir un presupuesto de memoria retenida:

```yaml
performance:
  maxMemory: 256MiB
```

El campo es opcional (default **256 MiB**) y admite como mínimo **64 MiB**. Su scalar YAML
semántico, una vez deserializado, debe satisfacer exactamente `[1-9][0-9]*(MiB|GiB)`: no admite
espacios en su contenido, fracciones, ceros iniciales ni cambios de mayúsculas. El whitespace
sintáctico separado antes de newline, comentario, coma o `}` no integra el valor; por ello
`performance: {maxMemory: 256MiB }` equivale a `256MiB`. Sí son inválidos `"256MiB "`,
`" 256MiB"` y `256 MiB`, además de `0MiB`, `064MiB`, `1.5GiB`, `256mib` y `256MB`. Se convierte
a bytes `u64` con aritmética *checked*; valores inválidos, fuera de rango o por debajo del mínimo
producen un error accionable por el camino existente, sin añadir un código MCP. No se introduce un
scanner/lexer source-aware ni un parser YAML paralelo. `N` es el total de `performance.maxMemory`;
SQLite tiene una cuota interna blanda de
`floor(30 * N / 100)` y W-TinyLFU otra de `floor(20 * N / 100)`. `Work = N - SQLite - W-TinyLFU`
es la reserva protegida dentro de `N`, recibe todo residuo y las caches nunca la invaden. La suma
es exactamente `N`: no existe una cuarta reserva, un pool sin tope ni un tamaño abierto. El único
owner de `MemoryBudget` es `lodestar-workspace::Workspace::open`; core, store y fachadas no lo
crean ni lo poseen. La política completa está ratificada en
[`ARCHITECTURE.md §23`](../ARCHITECTURE.md#23-presupuesto-de-memoria-retenida-e35-h01).

La adenda es compatible por adición: una config antigua que omite el campo usa el default, mientras
que un binario antiguo puede rechazar el campo nuevo. No se consulta cgroup/RSS al abrir el
workspace y no se definen presets ni knobs adicionales, públicos o internos. El alcance de E35-H01
incluye config, `MemoryBudget`/subpresupuestos, tests, mensajes y documentación; no conecta SQLite,
no implementa la cache W-TinyLFU ni cambia el contrato MCP.

---

# Fase 3: descubrimiento recursivo universal

## Comportamiento por defecto

Descubrir todos los archivos:

```text
**/*.md
```

dentro del workspace.

## Política recomendada

```yaml
discovery:
  include:
    - "**/*.md"

  exclude:
    - ".git/**"
    - ".lodestar/runtime/**"

  respectGitignore: true
  respectLodestarIgnore: true
  followSymlinks: false
```

## Requisitos

* Recorrer cualquier profundidad.
* No imponer una profundidad máxima artificial.
* Respetar `.gitignore` por defecto.
* Soportar `.lodestarignore`.
* Permitir configurar `include` y `exclude`.
* Evitar indexar dependencias, builds y runtime derivado.
* Registrar rutas normalizadas relativas al root.
* Detectar colisiones de rutas según capitalización.
* Mantener un inventario completo de documentos.

## Restricciones iniciales

* Documentos UTF-8.
* Paths representables de manera segura.
* Tamaño máximo por documento configurable.
* Symlinks desactivados.

`discovery.maxDocumentBytes` es un límite de admisión, no una obligación de retener cada documento
en cache. La semántica ratificada permite procesar un documento admitido fuera de cache cuando sea
seguro; si no lo es, debe fallar explícitamente por el camino de error existente, sin reintentos que
provoquen *thrashing*. La memoria normativa es la retenida y controlable, no el RSS del proceso. La
implementación efectiva de esas rutas corresponde a las issues posteriores **#55, #57, #59 y #62**,
según la historia concreta; E35-H01 no promete ejecutarlas.

---

# Fase 4: sustituir el frontmatter OKF por YAML genérico

## Objetivo

Conservar la inteligencia sobre frontmatter sin imponer propiedades concretas.

## Requisitos

* El frontmatter es opcional.
* Un documento sin frontmatter es válido.
* Un documento con frontmatter vacío es válido.
* Cualquier clave YAML es válida.
* Se deben soportar:

  * Strings.
  * Números.
  * Booleanos.
  * Fechas interpretadas como valores YAML.
  * `null`.
  * Listas.
  * Objetos anidados.
* No debe ser obligatorio `type`.
* No debe ser obligatorio `status`.
* No debe existir una lista cerrada de campos.
* No se deben eliminar claves desconocidas.
* No se deben convertir valores automáticamente entre tipos.

## Modelo recomendado

```rust
pub struct ParsedFrontmatter {
    pub value: serde_yaml::Value,
    pub raw: String,
    pub span: Range<usize>,
}
```

## Título derivado

Para resultados y UI textual, derivar un título mediante:

```text
frontmatter.title
→ primer heading H1
→ nombre del archivo
```

Esta es únicamente una heurística de presentación.

`title` no debe convertirse en una propiedad reservada.

## Modificación de propiedades

La operación genérica debe ser:

```text
patch_frontmatter
```

Ejemplo:

```json
{
  "op": "patch_frontmatter",
  "path": "docs/authentication.md",
  "patch": {
    "status": "accepted",
    "owners": ["platform", "security"],
    "reviewed": true,
    "deprecated_field": null
  }
}
```

## Requisitos de edición

* Aplicar el merge-patch RFC 7386 de forma recursiva: los objetos fusionan sus claves a cualquier
  profundidad y un target que no sea objeto empieza como objeto vacío.
* Preservar semánticamente las claves y descendientes que el patch no menciona; un `null` en un
  objeto del patch elimina únicamente la clave nombrada.
* Sustituir arrays y escalares de forma atómica, sin mergear arrays por índice.
* La función pura del core devuelve `PatchedDocument.reserialized` para indicar si tuvo que
  reserializar todo el bloque; esa señal es interna y no forma parte del wire de `change_plan`.
* Evitar reordenamientos innecesarios y mantener el cuerpo intacto.
* No existe, por el wire RFC 7386, una forma distinta de asignar literalmente `null` y eliminar una
  clave: `null` es el sentinel de borrado. Un `null` dentro de una lista es un elemento de datos
  porque la lista se sustituye completa.

---

# Fase 5: conservar y generalizar el lenguaje de consultas

## Objetivo

Mantener el lenguaje de consulta por propiedades de frontmatter como capacidad central de Lodestar.

El lenguaje debe dejar de depender de campos OKF y permitir consultar cualquier propiedad YAML.

## Consultas básicas

Dado:

```yaml
---
type: decision
status: accepted
priority: 2
reviewed: true
owners:
  - platform
  - security
service:
  name: authentication
  tier: critical
---
```

Deben funcionar:

```text
type = "decision"
status = "accepted"
priority >= 2
reviewed = true
owners contains "security"
service.name = "authentication"
service.tier = "critical"
```

## Expresiones booleanas

```text
type = "decision" and status = "accepted"
```

```text
status = "draft" or status = "review"
```

```text
not tags contains "deprecated"
```

```text
type = "decision"
and (status = "draft" or status = "review")
and not tags contains "archived"
```

## Existencia

```text
has(status)
missing(status)
has(frontmatter)
missing(frontmatter)
has(service.tier)
```

## Operadores mínimos

### Comparación

```text
=
!=
>
>=
<
<=
```

### Texto

```text
contains
starts_with
ends_with
```

### Listas

```text
contains
contains_any
contains_all
```

### Lógica

```text
and
or
not
(...)
```

## Namespaces

Separar metadata de propiedades calculadas:

```text
frontmatter.status
document.path
document.title
document.has_frontmatter
graph.backlinks
graph.outgoing_links
graph.dangling_links
graph.isolated
```

Permitir que:

```text
status = "accepted"
```

sea una abreviatura de:

```text
frontmatter.status = "accepted"
```

Las propiedades internas deben requerir namespace explícito.

## Ejemplos

```text
document.path starts_with "docs/"
```

```text
document.has_frontmatter = false
```

```text
graph.backlinks = 0
```

```text
graph.dangling_links > 0
```

```text
status = "draft" and missing(reviewed_at)
```

```text
criticality = "high" and missing(owner)
```

## Semántica de tipos

Los operadores deben respetar los tipos YAML reales.

Esto debe funcionar:

```text
priority >= 2
```

Esto debe producir un error de tipo:

```text
priority >= "high"
```

No realizar coerciones implícitas entre:

* String y número.
* String y booleano.
* Valor escalar y lista.
* Lista y objeto.

Cuando una propiedad tenga distintos tipos en distintos documentos, Lodestar debe poder inspeccionarlo y comunicarlo.

## AST unificado

La consulta textual y el filtro JSON deben traducirse al mismo AST.

```rust
pub enum Expression {
    Comparison {
        field: FieldPath,
        operator: ComparisonOperator,
        value: QueryValue,
    },
    Function {
        name: FunctionName,
        arguments: Vec<QueryValue>,
    },
    And(Vec<Expression>),
    Or(Vec<Expression>),
    Not(Box<Expression>),
}
```

## Superficie MCP

`knowledge_search` debe aceptar:

```json
{
  "query": "authentication",
  "where": "status = \"accepted\" and owners contains \"platform\""
}
```

También debe aceptar una forma estructurada:

```json
{
  "query": "authentication",
  "filter": {
    "and": [
      {
        "field": "frontmatter.status",
        "operator": "equals",
        "value": "accepted"
      },
      {
        "field": "frontmatter.owners",
        "operator": "contains",
        "value": "platform"
      }
    ]
  }
}
```

Ambas formas deben producir exactamente el mismo resultado.

---

# Fase 6: añadir inspección genérica de metadata

## Objetivo

Permitir que un agente comprenda las convenciones de una base desconocida sin necesitar un schema.

Sustituir `schema_inspect` por:

```text
metadata_inspect
```

## Catálogo de propiedades

Entrada:

```json
{
  "mode": "catalog"
}
```

Salida conceptual:

```json
{
  "fields": [
    {
      "name": "status",
      "presentIn": 84,
      "inferredTypes": {
        "string": 84
      }
    },
    {
      "name": "priority",
      "presentIn": 48,
      "inferredTypes": {
        "number": 42,
        "string": 6
      }
    }
  ]
}
```

## Inspección de una propiedad

Entrada:

```json
{
  "field": "status"
}
```

Salida conceptual:

```json
{
  "field": "status",
  "presentIn": 84,
  "missingIn": 26,
  "inferredTypes": {
    "string": 84
  },
  "values": [
    {
      "value": "draft",
      "count": 21
    },
    {
      "value": "accepted",
      "count": 57
    },
    {
      "value": "deprecated",
      "count": 6
    }
  ]
}
```

## Propiedades anidadas

Debe poder inspeccionar:

```text
service.name
service.tier
release.target.date
```

---

# Fase 7: resolución de enlaces Markdown estándar

## Tipos admitidos

Procesar:

```markdown
[Texto](relative/path.md)
[Texto](../document.md)
[Texto](document.md#section)
[Texto][reference-id]

[reference-id]: ../../reference.md
```

Procesar enlaces al documento actual:

```markdown
[Instalación](#installation)
```

Clasificar URLs externas:

```markdown
[Website](https://example.com)
```

## Tipos no admitidos

No implementar:

```markdown
[[document]]
![[document]]
[[document#heading]]
[[document|alias]]
```

## Algoritmo

Para cada enlace:

1. Parsear el destino mediante el parser Markdown.
2. Separar path, query y fragment.
3. Detectar si es una URI externa.
4. Detectar si es un anchor del documento actual.
5. Resolver paths relativos contra el directorio del documento origen.
6. Normalizar `.` y `..`.
7. Verificar que el destino permanece dentro del workspace.
8. Resolver el path contra el inventario.
9. Clasificar el resultado.
10. Registrar el enlace original y el destino normalizado.

## Modelo

```rust
pub enum LinkTarget {
    Document(RelativePath),
    WorkspaceFile(RelativePath),
    ExternalUri(String),
    SelfAnchor(String),
    Missing(RelativePath),
    EscapesWorkspace,
}
```

## Enlaces a otros archivos del proyecto

Un Markdown puede enlazar a código u otros recursos:

```markdown
Consulta [token_service.rs](../../src/auth/token_service.rs).
```

Lodestar debe indicar que el archivo existe, pero no incorporarlo como nodo Markdown del grafo.

## Prohibiciones

No:

* Buscar por basename.
* Buscar por título.
* Añadir `.md` automáticamente.
* Resolver un directorio como `index.md`.
* Resolver ambigüedades mediante heurísticas.
* Tratar `README.md` como fallback.
* Interpretar aliases.

## Capitalización

Detectar diferencias de capitalización incluso en sistemas de archivos case-insensitive.

Ejemplo:

```text
Enlace: Docs/Auth.md
Real:   docs/auth.md
```

Debe generar un diagnóstico de portabilidad.

---

# Fase 8: reconstruir el grafo

## Nodos

Todos los documentos Markdown descubiertos.

## Aristas

Enlaces Markdown resueltos entre documentos Markdown.

## Información calculada

```rust
pub struct Analysis {
    pub documents: Vec<RelativePath>,
    pub outgoing: BTreeMap<RelativePath, Vec<ResolvedLink>>,
    pub incoming: BTreeMap<RelativePath, Vec<LinkReference>>,
    pub isolated: Vec<RelativePath>,
    pub dangling: Vec<DanglingLink>,
    pub diagnostics: BTreeMap<RelativePath, Vec<Diagnostic>>,
}
```

## Documento aislado

Definir como:

> Documento Markdown que no tiene enlaces internos entrantes ni salientes.

Un documento aislado no es inválido.

Debe ser una propiedad consultable:

```text
graph.isolated = true
```

No debe generar warning por defecto.

## Eliminar

* `FileKind::Index`.
* `FileKind::Log`.
* `in_index`.
* `okf_version`.
* `index_refs`.
* `src_is_index`.
* Pertenencia determinada mediante índices.
* Semántica especial de archivos generados.

---

# Fase 9: migrar el store

## Estrategia

El índice SQLite es derivado y desechable.

Incrementar la versión interna del store y ejecutar una reconstrucción completa.

No realizar una migración compleja de datos OKF.

## Modelo conceptual

```sql
documents (
    path TEXT PRIMARY KEY,
    title TEXT,
    body TEXT,
    raw TEXT,
    frontmatter_json TEXT,
    content_hash BLOB
);

metadata (
    document_path TEXT NOT NULL,
    field_path TEXT NOT NULL,
    value_json TEXT NOT NULL,
    value_type TEXT NOT NULL
);

links (
    source_path TEXT NOT NULL,
    raw_href TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    target_path TEXT,
    fragment TEXT,
    resolved INTEGER NOT NULL
);

diagnostics (
    document_path TEXT,
    code TEXT NOT NULL,
    severity TEXT NOT NULL,
    message TEXT NOT NULL,
    range_json TEXT
);
```

## Indexación de metadata

Indexar recursivamente propiedades anidadas:

```yaml
service:
  name: authentication
  tier: critical
```

como:

```text
service.name
service.tier
```

Conservar el valor JSON original y su tipo.

## FTS

Indexar:

* Path.
* Título derivado.
* Body.
* Valores textuales de frontmatter.

No depender de campos concretos como `type`, `status` o `tags`.

## Consistencia

El resultado calculado por el core puro debe coincidir con el recuperado desde SQLite.

### Adenda ratificada E35-H02 — esquema vNext (issue #54; E34-H02 → E35-H02)

La issue #54 materializa el DDL descrito arriba como un esquema relacional por IDs. Esta adenda
supersede el modelo conceptual anterior en lo relativo a las columnas persistidas: `USER_VERSION = 6`
y cualquier versión o DDL incompatible provoca un rebuild completo de la cache, nunca una migración
in-place. El rebuild no modifica los ficheros Markdown.

El modelo efectivo es:

```sql
documents(doc_id INTEGER PRIMARY KEY, path TEXT UNIQUE NOT NULL, title TEXT NOT NULL,
          body TEXT NOT NULL, frontmatter_json TEXT NOT NULL, frontmatter_text TEXT NOT NULL,
          content_hash BLOB NOT NULL, mtime INTEGER NOT NULL, size INTEGER NOT NULL);
fields(field_id INTEGER PRIMARY KEY, field_path TEXT UNIQUE NOT NULL);
metadata(doc_id INTEGER NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
         field_id INTEGER NOT NULL REFERENCES fields(field_id),
         value_json TEXT NOT NULL, value_type TEXT NOT NULL,
         PRIMARY KEY (doc_id, field_id));
links(link_id INTEGER PRIMARY KEY,
      source_doc_id INTEGER NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
      target_doc_id INTEGER REFERENCES documents(doc_id) ON DELETE SET NULL,
      raw_href TEXT NOT NULL, target_kind TEXT NOT NULL, target_path TEXT,
      fragment TEXT, resolved INTEGER NOT NULL, is_edge INTEGER NOT NULL);
diagnostics(diagnostic_id INTEGER PRIMARY KEY,
            doc_id INTEGER NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
            code TEXT NOT NULL, severity TEXT NOT NULL, message TEXT NOT NULL, range_json TEXT);
```

`other_files(path PRIMARY KEY)` conserva el inventario de ficheros no Markdown para que el core
clasifique los enlaces. Metadata, enlaces y diagnósticos referencian `doc_id`; los paths repetidos de
metadata se internan una sola vez en `fields` y los namespaces reservados se guardan en forma
anclada. Un destino-documento conocido usa `target_doc_id` (con `target_path` nulo); un destino no
materializable conserva `target_path` con ID nulo para la invalidación dirigida futura.

FTS5 se implementa como contentless (`content=''`, `columnsize=0`) sobre `path`, título derivado,
`body` y `frontmatter_text`. Al no tener tabla de contenido, cada inserción escribe manualmente
`rowid = doc_id`; los candidatos se recuperan con un `JOIN` a `documents` y después se confirman con
el core. `raw` desaparece del DDL vNext: `body` conserva el snapshot Markdown completo y exacto
(frontmatter y cuerpo incluidos), es la única copia completa del contenido en SQLite y es la fuente
de los valores enviados al índice. `DocumentStore`/core leen ese snapshot desde SQLite en el camino
cacheado; el Markdown en disco sigue siendo la fuente canónica y la cache continúa siendo derivada,
reconstruible y no predeterminada.

Contentless exige mantenimiento explícito: antes de actualizar o borrar, el único escritor lee desde
`documents` los valores antiguos exactos (`path`, `title`, `body`, `frontmatter_text`), ejecuta el
comando FTS5 `delete` con `rowid = doc_id` y esos valores, y solo después escribe la nueva versión (o
elimina el documento). El spike medido, la elección de variante y la métrica `dbstat` están en
[`docs/qa/e35-h02-fts-spike-2026-08-25.md`](qa/e35-h02-fts-spike-2026-08-25.md).

Esta adenda no conecta SQLite a App/MCP, no modifica el contrato MCP/CLI y no cierra `decisiones
§14`. El objetivo de footprint Realista/100k `≤ 2,5×` se informa como objetivo de ingeniería, no
como gate ni como permiso para hacer SQLite la lectura por defecto.

### Adenda ratificada E35-H03 — rebuild streaming, insert-only y atómico

El rebuild efectivo construye `index.db.next` en dos pasadas con la política canónica: inventario
compacto de candidatos —sin abrir cuerpos e incluidos los directorios recorridos— primero y una
única lectura después. Esa lectura clasifica UTF-8; los válidos se parsean una vez, se proyectan y
liberan, mientras los inválidos pasan a `other_files` con `DOC-NOT-UTF8` sin parseo ni proyección.
Los enlaces adelantados se reatan al promocionar el destino válido, con semántica derivada de
`LinkTarget` y actualización `O(log N)` del inventario. Fingerprints de raíz —y de su destino real si es symlink—, entradas y directorios se
capturan y estabilizan en discovery; Store los compara antes de leer cuerpos, tras el streaming y
justo antes del swap, detectando modificaciones y cambios de pertenencia sin repetir el walker. La
carga no usa deletes lógicos por documento y reutiliza
statements; un authorizer SQLite deniega cualquier bypass y reconcilia los diez prepares, permitiendo
solo mantenimiento shadow FTS5 dentro del INSERT auditado/commit. Solo tras comprobar integridad,
cerrar y sincronizar la generación se hace rename y
fsync del directorio. El índice anterior permanece consultable hasta la publicación; los escritores
se serializan también entre procesos mediante lock nativo RAII y no pueden perder cambios detrás
del swap.

La telemetría separa `max_live_body_bytes` del RSS específico del rebuild y expone las cuatro fases;
el pico de cada una procede de muestras de RSS residente actual contenidas en su ventana monotónica.
Los objetivos 60 s/512 MiB siguen siendo evidencia no bloqueante. No cambia Markdown, configuración,
MCP/CLI ni la decisión abierta §14.

---

# Fase 10: validación genérica

## Eliminar diagnósticos

No diagnosticar como error:

* Falta de frontmatter.
* Falta de `type`.
* Falta de `status`.
* Formato particular de `tags`.
* Ausencia en un índice.
* Falta de `okf_version`.
* Documento aislado.
* Estructura de headings no conforme con OKF.
* Transiciones de estado no definidas.
* Relaciones no tipadas.

## Diagnósticos mínimos

| Código                   | Significado                |
| ------------------------ | -------------------------- |
| `FM-UNCLOSED`            | Frontmatter sin cierre     |
| `FM-YAML-INVALID`        | YAML inválido              |
| `DOC-CONFLICT-MARKER`    | Marcadores de merge        |
| `DOC-NOT-UTF8`           | Documento no UTF-8         |
| `DOC-TOO-LARGE`          | Documento sobre el límite  |
| `PATH-NOT-UTF8`          | Ruta no representable      |
| `LINK-TARGET-MISSING`    | Destino local inexistente  |
| `LINK-ESCAPES-WORKSPACE` | Destino fuera del root     |
| `LINK-CASE-MISMATCH`     | Capitalización no portable |
| `SYMLINK-UNSUPPORTED`    | Symlink no admitido        |

## Semántica de `knowledge_check`

Debe responder:

> ¿Puede Lodestar interpretar y modificar este workspace de manera consistente y segura?

No:

> ¿Cumple el workspace una especificación documental?

## Política de cambios

Configuración recomendada:

```yaml
validation:
  malformedFrontmatter: error
  danglingDocumentLinks: error
  missingWorkspaceFiles: warning
  isolatedDocuments: ignore
  caseMismatch: warning

transactions:
  rejectNewErrors: true
  allowExistingErrors: true
```

`allowExistingErrors: true` significa:

* Lodestar puede trabajar en un repositorio que ya tenga problemas.
* Un cambio no debe introducir errores nuevos.
* Un cambio no debe empeorar errores existentes.
* Una reparación parcial debe poder aplicarse.

---

# Fase 11: rediseñar la superficie MCP

## Tools de lectura

```text
workspace_status
knowledge_search
knowledge_get
metadata_inspect
graph_query
impact_analyze
```

## Tool de verificación

```text
knowledge_check
```

## Tools de cambio

```text
change_plan
change_apply
change_revert
```

## `workspace_status`

Salida conceptual:

```json
{
  "root": "/project",
  "workspaceRevision": "blake3:...",
  "discovery": {
    "include": ["**/*.md"],
    "respectGitignore": true
  },
  "counts": {
    "documents": 183,
    "documentsWithFrontmatter": 121,
    "internalLinks": 521,
    "workspaceFileLinks": 74,
    "externalLinks": 91,
    "isolatedDocuments": 12,
    "danglingLinks": 3,
    "errors": 0,
    "warnings": 4
  }
}
```

## `knowledge_get`

Debe devolver:

* Path.
* Título derivado.
* Frontmatter completo.
* Body.
* Enlaces salientes.
* Backlinks.
* Diagnósticos.
* Revisión del documento.

## `knowledge_search`

Debe combinar:

* Full-text search.
* Restricción por paths.
* Filtros por metadata.
* Propiedades calculadas del documento.
* Propiedades calculadas del grafo.

## `graph_query`

Operaciones mínimas:

* Backlinks.
* Enlaces salientes.
* Vecinos.
* Dependientes transitivos.
* Dependencias transitivas.
* Documentos aislados.
* Enlaces rotos.
* Caminos entre documentos.

## `impact_analyze`

Debe calcular el impacto basándose en:

* Backlinks.
* Enlaces salientes.
* Movimiento de paths.
* Eliminación de documentos.
* Reescritura de referencias.
* Documentos afectados por una selección de metadata.

No debe depender de tipos OKF.

---

# Fase 12: simplificar las operaciones transaccionales

## Operaciones universales

Mantener o implementar:

```text
create_document
patch_frontmatter
replace_body
replace_text
edit_section
move_document
delete_document
apply_fix
```

## Eliminar operaciones semánticas específicas

Eliminar:

```text
add_relation
remove_relation
transition_status
deprecate
replace_concept
```

Una relación es un enlace Markdown.

Un estado es una propiedad arbitraria del frontmatter.

## Operaciones masivas basadas en consulta

Debe ser posible seleccionar documentos mediante el lenguaje de consulta y generar un plan.

Ejemplo:

```json
{
  "selection": {
    "where": "type = \"decision\" and status = \"draft\""
  },
  "operation": {
    "patch_frontmatter": {
      "status": "review"
    }
  }
}
```

Flujo:

```text
query
→ documentos seleccionados
→ snapshot de revisiones
→ semantic diff
→ impact analysis
→ validation
→ change plan
→ change apply
→ receipt
```

## Movimiento de documentos

Ejemplo:

```json
{
  "op": "move_document",
  "path": "docs/auth.md",
  "destination": "docs/security/auth.md",
  "rewriteInboundLinks": true
}
```

El plan debe:

1. Encontrar todos los backlinks.
2. Calcular el nuevo enlace relativo desde cada origen.
3. Reescribir únicamente el destino.
4. Mantener label y fragment.
5. Mostrar todos los documentos modificados.
6. Verificar que no aparecen enlaces rotos.
7. Aplicar todas las escrituras como una única transacción lógica.

## Eliminación

`delete_document` debe:

* Mostrar backlinks.
* Marcar los enlaces que quedarían rotos.
* Requerir una política explícita:

  * Rechazar si hay backlinks.
  * Permitir enlaces rotos.
  * Eliminar referencias.
  * Sustituir referencias.
* No elegir una política automáticamente.

---

# Fase 13: conservar el motor transaccional

## No modificar conceptualmente

* `WorkspaceRevision`.
* `DocumentRevision`.
* Hashes de contenido.
* Plan inmutable.
* Snapshot de precondiciones.
* Staging.
* Journal.
* Escritura atómica.
* Recovery.
* Receipt.
* Revert.

## Nueva validación previa

Antes:

```text
¿El resultado es conforme con OKF?
```

Después:

```text
¿El resultado es parseable?
¿Permanece dentro del workspace?
¿Respeta la política de escritura?
¿Introduce diagnósticos nuevos?
¿Coincide con las revisiones del plan?
¿Mantiene consistencia entre inventario, store y grafo?
```

## Revisión del workspace

Debe depender como mínimo de:

* Rutas Markdown incluidas.
* Hash de cada documento.
* Configuración de descubrimiento.
* Configuración de escritura.
* Versión del parser.
* Versión del esquema del store.

---

# Fase 14: migración de repositorios OKF existentes

## Principio

No modificar destructivamente documentos anteriores.

## Frontmatter anterior

Esto:

```yaml
type: decision
status: accepted
```

se conserva exactamente.

`type` y `status` pasan a ser metadata normal y siguen siendo consultables.

## `index.md`

Se conserva como documento Markdown normal.

Ya no:

* Determina pertenencia.
* Declara versión.
* Evita aislamiento.
* Tiene enlaces especiales.
* Actúa como catálogo obligatorio.

## Índices de tags

Se conservan como documentos normales.

No deben eliminarse automáticamente.

## `okf_version`

Puede conservarse como metadata desconocida.

Debe ofrecerse como recomendación de limpieza, no como error.

## Índice SQLite

Eliminar y reconstruir.

## Comando de diagnóstico opcional

```bash
lodestar migrate-from-okf --dry-run
```

Salida conceptual:

```text
Legacy OKF conventions detected.

Documents will remain valid Markdown.

Detected:
- root index.md
- 4 nested index files
- okf_version metadata
- 8 generated tag indexes

No files were modified.

Recommended cleanup:
- Treat index files as optional navigation documents.
- Remove okf_version when convenient.
- Review generated tag indexes before deleting them.
```

---

# Orden de implementación

## PR 1: workspace universal

* `cwd` como root.
* `--root`.
* Configuración opcional.
* Seguridad de paths.
* Descubrimiento recursivo.
* Fixtures de estructuras arbitrarias.

## PR 2: modelo documental genérico

* `Concept` → `Document`.
* `Bundle` → `Workspace` o `DocumentSet`.
* Frontmatter opcional.
* YAML arbitrario.
* Eliminar obligatoriedad de `type`.

## PR 3: resolución de enlaces

* Enlaces inline.
* Enlaces de referencia.
* Anchors.
* URLs externas.
* Enlaces a otros archivos del workspace.
* Detección de escapes.
* Detección de case mismatch.

## PR 4: grafo universal

* Todos los `.md` como nodos.
* Backlinks globales.
* Enlaces salientes.
* Dangling links.
* Isolated documents.
* Eliminar semántica de índices.

## PR 5: store v2

* Nuevo DDL.
* Metadata genérica.
* Paths anidados.
* Links genéricos.
* Cold rebuild.
* Consistencia core/store.

## PR 6: lenguaje de consulta genérico

* Parser.
* AST.
* Type checking.
* Dot notation.
* Operadores de listas.
* `has` y `missing`.
* Namespaces.
* Filtro JSON equivalente.

## PR 7: inspección de metadata

* `metadata_inspect`.
* Catálogo de campos.
* Tipos inferidos.
* Valores frecuentes.
* Paths anidados.
* Heterogeneidad de tipos.

## PR 8: validación genérica

* Nuevos diagnósticos.
* Eliminar códigos OKF.
* `knowledge_check` genérico.
* Política `rejectNewErrors`.
* `allowExistingErrors`.

## PR 9: contrato MCP nuevo

* Actualizar nombres.
* Eliminar DTOs OKF.
* Actualizar `contracts/mcp.yml`.
* Actualizar ejemplos.
* Garantizar equivalencia entre consulta textual y estructurada.

## PR 10: operaciones transaccionales genéricas

* `create_document`.
* `patch_frontmatter`.
* `replace_body`.
* `replace_text`.
* `edit_section`.
* `move_document`.
* `delete_document`.
* Selecciones masivas por consulta.

## PR 11: migración y limpieza pública

* Eliminar generadores obligatorios de índices.
* Eliminar terminología OKF.
* Añadir guía de migración.
* Actualizar README.
* Actualizar arquitectura.
* Publicar la versión incompatible.

---

# Tests imprescindibles

## Descubrimiento

Fixture:

```text
fixture/
├── README.md
├── one/
│   └── first.md
├── two/
│   └── levels/
│       └── second.md
└── three/
    └── levels/
        └── deep/
            └── third.md
```

Probar:

1. Descubrimiento en raíz.
2. Descubrimiento a varios niveles.
3. Exclusión por `.gitignore`.
4. Exclusión por `.lodestarignore`.
5. Paths con espacios.
6. Directorios ocultos.
7. Symlinks rechazados.
8. Archivo demasiado grande.
9. Documento no UTF-8.
10. Cambios detectados por watcher.

## Enlaces

Probar:

1. Raíz hacia tres niveles.
2. Tres niveles hacia raíz.
3. Hermanos en árboles diferentes.
4. `./document.md`.
5. `../document.md`.
6. Múltiples `..`.
7. Paths con espacios.
8. Paths con `%20`.
9. Fragmentos.
10. Reference links.
11. Enlaces externos.
12. Enlaces a código.
13. Destino inexistente.
14. Escape del workspace.
15. Dos archivos con el mismo basename.
16. Capitalización incorrecta.
17. Anchor al documento actual.
18. Movimiento con reescritura de backlinks.

## Frontmatter

Probar:

1. Sin frontmatter.
2. Frontmatter vacío.
3. String.
4. Número.
5. Booleano.
6. `null`.
7. Lista.
8. Objeto anidado.
9. Lista de objetos.
10. YAML inválido.
11. Frontmatter sin cierre.
12. Patch que preserva claves desconocidas.
13. Eliminación explícita de claves.
14. Ausencia de reordenamientos innecesarios.

## Consultas

Probar:

1. Igualdad de string.
2. Comparación numérica.
3. Booleanos.
4. `contains` sobre strings.
5. `contains` sobre listas.
6. `contains_any`.
7. `contains_all`.
8. Dot notation.
9. `has`.
10. `missing`.
11. `and`.
12. `or`.
13. `not`.
14. Paréntesis.
15. Precedencia.
16. Campo inexistente.
17. Tipos heterogéneos.
18. Error de comparación incompatible.
19. Namespace `document`.
20. Namespace `graph`.
21. Equivalencia query textual/filtro JSON.
22. Resultado equivalente core/SQLite.

## Transacciones

Probar:

1. Plan sin cambios.
2. Cambio de un documento.
3. Cambio masivo por consulta.
4. Cambio con workspace revision obsoleta.
5. Cambio externo después del plan.
6. Escritura atómica.
7. Fallo a mitad del apply.
8. Recovery.
9. Receipt completo.
10. Revert.
11. Movimiento con múltiples backlinks.
12. Eliminación con backlinks.
13. Rechazo de path fuera del root.
14. Rechazo de escritura sobre archivo excluido.
15. Preservación de archivos no incluidos.

---

# Invariantes

Las siguientes condiciones deben mantenerse en todo momento:

1. Ningún path público es absoluto.
2. Ninguna operación puede escapar del workspace.
3. Todo documento descubierto tiene una ruta canónica única.
4. Todo enlace resuelto utiliza paths, no títulos.
5. Los documentos con el mismo basename permanecen inequívocos.
6. El frontmatter nunca es obligatorio.
7. Las claves del frontmatter no tienen semántica impuesta.
8. El lenguaje de consulta funciona sobre cualquier propiedad.
9. Los tipos YAML se respetan sin coerción implícita.
10. Los documentos aislados no son errores.
11. `index.md` no tiene tratamiento especial.
12. El store puede reconstruirse completamente desde los archivos.
13. El análisis puro y el store producen resultados equivalentes.
14. Un plan no puede aplicar cambios sobre una revisión incompatible.
15. Un apply no puede introducir errores nuevos cuando la política lo prohíbe.
16. Un receipt contiene información suficiente para auditar y revertir.
17. La ausencia de `.lodestar/` no impide utilizar Lodestar.
18. La ausencia de configuración no impide utilizar Lodestar.
19. La estructura de carpetas no altera el significado de los documentos.
20. El proyecto no depende de sintaxis específica de Obsidian.

---

# Criterios de aceptación

La migración se considera completada cuando:

* `lodestar-mcp` arranca sin argumentos desde cualquier `cwd`.
* Se descubren recursivamente todos los `.md` elegibles.
* No es obligatorio ejecutar `lodestar init`.
* No es obligatorio tener `.lodestar/config.yaml`.
* No es obligatorio tener frontmatter.
* No es obligatorio tener `type`.
* No es obligatorio tener `status`.
* No es obligatorio tener `index.md`.
* `README.md` e `index.md` son documentos normales.
* Los enlaces Markdown relativos funcionan entre cualquier profundidad.
* Los enlaces se resuelven únicamente por paths.
* Los escapes fuera del workspace se rechazan.
* Los enlaces a archivos no Markdown se clasifican correctamente.
* El grafo contiene todos los documentos Markdown.
* Los backlinks funcionan globalmente.
* Los documentos aislados son consultables, no inválidos.
* Cualquier propiedad YAML puede consultarse.
* Se soportan propiedades anidadas mediante dot notation.
* El lenguaje admite comparación, listas, existencia y operadores booleanos.
* La consulta textual y la consulta JSON generan el mismo AST y resultado.
* `metadata_inspect` permite descubrir las convenciones existentes.
* Los cambios masivos pueden seleccionarse mediante consultas de frontmatter.
* `change_plan`, `change_apply` y `change_revert` siguen funcionando.
* Un movimiento puede reescribir backlinks relativos correctamente.
* El store se reconstruye sin datos OKF.
* No existe terminología OKF en la API pública.
* No existe comportamiento específico de Obsidian.
* Los tests de equivalencia entre core y store pasan.
* Los tests de recovery y rollback pasan.

---

# Definición final del producto

Lodestar debe quedar definido como:

> Un motor local y transaccional para que agentes de IA puedan descubrir, consultar, comprender y modificar de forma segura una red arbitraria de documentos Markdown contenida dentro de un proyecto.

Su unidad fundamental es:

```text
Workspace
└── documentos Markdown descubiertos recursivamente
    ├── contenido
    ├── frontmatter YAML opcional
    ├── enlaces Markdown estándar
    ├── metadata consultable
    └── rutas relativas al cwd
```

El valor diferencial de Lodestar no debe depender de un formato propio.

Debe residir en:

* Descubrimiento global.
* Consultas estructuradas sobre frontmatter.
* Grafo de documentos.
* Backlinks.
* Análisis de impacto.
* Planificación de cambios.
* Validación previa.
* Escrituras atómicas.
* Auditoría.
* Recovery.
* Rollback.
