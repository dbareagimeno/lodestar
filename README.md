# Lodestar

**Conocimiento en Markdown que los agentes pueden entender, validar y cambiar con seguridad.**

[![CI](https://github.com/dbareagimeno/lodestar/actions/workflows/ci.yml/badge.svg)](https://github.com/dbareagimeno/lodestar/actions/workflows/ci.yml)
[![Rust 1.80+](https://img.shields.io/badge/Rust-1.80%2B-dea584?logo=rust)](rust-toolchain.toml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#licencia)

Lodestar convierte un directorio de documentos Markdown en una base de conocimiento que agentes,
automatizaciones y equipos pueden consultar con semántica, no como una colección de texto suelto.
Entiende conceptos, tipos, relaciones y estados; detecta incoherencias; calcula el impacto de un
cambio y permite aplicarlo mediante transacciones recuperables.

Los ficheros `.md` siguen siendo la única fuente de verdad. No hay un formato binario propietario,
un servidor que mantener ni una base de datos de la que dependa tu conocimiento. Todo permanece
legible, portable y versionable.

> Lodestar es un motor headless: se integra con agentes mediante MCP y con personas o CI mediante
> su CLI.

## Por qué Lodestar

- **Contexto preciso para agentes.** Busca conceptos, recupera solo las secciones necesarias e
  inspecciona el esquema sin cargar todo el repositorio en el contexto.
- **Conocimiento con estructura.** Markdown y frontmatter YAML se convierten en tipos, estados,
  relaciones y reglas comprobables.
- **Grafo e impacto.** Encuentra backlinks, huérfanos, enlaces rotos, ciclos, caminos y el radio de
  impacto de un cambio antes de realizarlo.
- **Cambios seguros.** Separa planificación y escritura: simula, valida y evalúa el riesgo antes de
  publicar; después genera un recibo y permite revertir.
- **Una puerta de calidad para CI.** `lodestar check` produce salida humana, JSON o SARIF y devuelve
  códigos de salida estables.
- **Local-first y file-first.** La caché SQLite/FTS5 es derivada y desechable; se puede reconstruir
  siempre desde los Markdown.

## Cómo funciona

Un agente puede seguir un flujo completo sin escribir a ciegas:

```text
orientarse → buscar → leer → inspeccionar relaciones → analizar impacto
    → planificar en memoria → aplicar → validar → revertir si es necesario
```

Lodestar expone ese recorrido mediante diez tools MCP:

| Necesidad | Tools |
|---|---|
| Entender el workspace | `workspace_status`, `knowledge_search`, `knowledge_get`, `schema_inspect` |
| Analizar la base | `graph_query`, `impact_analyze`, `knowledge_check` |
| Cambiar con seguridad | `change_plan`, `change_apply`, `change_revert` |

El perfil `readonly` ofrece únicamente lectura y análisis. El perfil `standard` añade el flujo
transaccional de cambios.

## Inicio rápido

### 1. Instala Lodestar

Descarga los binarios de `lodestar` y `lodestar-mcp` desde
[GitHub Releases](https://github.com/dbareagimeno/lodestar/releases), o compílalos desde este
repositorio:

```bash
cargo install --path crates/lodestar-cli
cargo install --path crates/lodestar-mcp
```

Requiere Rust 1.80 o posterior.

### 2. Crea una base de conocimiento

```bash
lodestar init mi-conocimiento
cd mi-conocimiento
```

Añade conceptos como Markdown con frontmatter YAML:

```markdown
---
type: Decision
title: Usar PostgreSQL
description: Decisión sobre la base de datos principal
status: accepted
tags: [architecture, data]
---

# Usar PostgreSQL

Elegimos PostgreSQL por su soporte transaccional y su ecosistema.

Esta decisión queda recogida en [el índice del bundle](/index.md).
```

### 3. Valídala

```bash
lodestar check
```

Una base conforme devuelve `0`; una violación que bloquea devuelve `1`.

```bash
lodestar check --json                 # integraciones y automatización
lodestar check --sarif > results.sarif # plataformas de análisis de código
```

### 4. Conecta tu agente

Configura tu cliente MCP para ejecutar el servidor por `stdio`. La forma exacta del fichero cambia
según el cliente, pero la definición equivalente es:

```json
{
  "mcpServers": {
    "lodestar": {
      "command": "lodestar-mcp",
      "args": ["/ruta/absoluta/mi-conocimiento", "--profile", "readonly"]
    }
  }
}
```

Usa `readonly` para exploración y revisión. Cambia el perfil a `standard` cuando quieras habilitar
`change_plan`, `change_apply` y `change_revert`.

## Qué puede hacer

### Descubrir y recuperar conocimiento

`knowledge_search` combina texto y filtros por tipo, estado, tags o prefijo de ruta. Devuelve
snippets y revisiones, no documentos completos. Después, `knowledge_get` permite solicitar
frontmatter, cuerpo, enlaces, diagnósticos o secciones concretas de un concepto.

### Validar reglas propias

Además de las comprobaciones de conformidad OKF, un bundle puede declarar sus tipos y reglas en
`.lodestar/schema.yaml`: campos obligatorios, estados permitidos y relaciones tipadas con
cardinalidad. `schema_inspect` permite que el agente descubra esas reglas antes de proponer cambios.

### Razonar sobre relaciones e impacto

`graph_query` consulta backlinks, enlaces salientes, vecindarios, huérfanos, enlaces rotos, caminos,
ciclos y componentes. `impact_analyze` estima conceptos afectados, referencias bloqueantes y nivel
de riesgo para operaciones como mover, eliminar, deprecar o sustituir un concepto.

### Publicar sin dejar estados parciales silenciosos

El flujo de escritura tiene tres pasos:

1. `change_plan` normaliza las operaciones, simula el resultado en memoria, calcula el diff
   semántico y valida la conformidad sin tocar disco.
2. `change_apply` comprueba que el workspace no haya cambiado y publica mediante staging, lock,
   copias de recuperación, journal previo a escritura y renames atómicos.
3. `change_revert` restaura una transacción reciente desde su recibo si el resultado necesita
   deshacerse.

Las revisiones deterministas de conceptos y workspace proporcionan control optimista de
concurrencia cuando un agente, una persona y otra herramienta editan los mismos ficheros.

## CLI

La CLI cubre el ciclo de mantenimiento y automatización:

| Comando | Uso |
|---|---|
| `lodestar init [dir]` | Inicializa un bundle |
| `lodestar check` | Valida el working tree |
| `lodestar index [dir]` | Genera índices de navegación |
| `lodestar tags` | Genera o purga índices de tags |
| `lodestar reindex` | Reconstruye la caché local |
| `lodestar export` | Exporta el bundle a ZIP |
| `lodestar import <source>` | Importa un ZIP o directorio |

`index` y `tags` aceptan `--check` para detectar artefactos desactualizados sin modificarlos.

Los códigos de salida son estables: `0` correcto, `1` no conforme, `2` uso inválido, `3` error de
runtime o I/O y `4` drift de generadores.

## El formato OKF

Un bundle **OKF (Open Knowledge Format)** es un directorio de `.md` con un `index.md` raíz. Cada
concepto combina:

- frontmatter YAML para los datos que deben poder consultarse y validarse;
- Markdown para el conocimiento que debe seguir siendo cómodo de leer y editar;
- enlaces Markdown para formar el grafo entre conceptos.

El esquema es opcional: sin `.lodestar/schema.yaml`, Lodestar conserva un modelo permisivo y aplica
las reglas base de OKF. La configuración del workspace puede limitar las raíces escribibles,
declarar raíces externas de referencia y endurecer la puerta de calidad.

## Arquitectura

```text
                 ┌──────────────────────────┐
Markdown + YAML ─►  motor semántico en Rust ├──► CLI / CI
  fuente real    │  conformidad · grafo     ├──► MCP / agentes
                 │  impacto · transacciones │
                 └────────────┬─────────────┘
                              ▼
                     SQLite / FTS5
                    caché reconstruible
```

El workspace está dividido por responsabilidades:

```text
crates/
  lodestar-core/        modelo, conformidad, query, grafo, generación y diff
  lodestar-store/       índice SQLite/FTS5 y watcher
  lodestar-workspace/   I/O, configuración y publicación recuperable
  lodestar-app/         casos de uso compartidos por CLI y MCP
  lodestar-cli/         fachada para personas y CI
  lodestar-mcp/         fachada MCP por stdio para agentes
  lodestar-vcs/         capacidad git conservada, fuera de la superficie actual
  lodestar-fixtures/    fixtures compartidos de test
```

La misma lógica de dominio sirve a la CLI y al servidor MCP. Las fachadas no reimplementan la
semántica, por lo que una validación produce el mismo veredicto independientemente del consumidor.

## Estado del proyecto

La versión actual es **0.2.0** y el foco del proyecto es el motor headless (CLI + MCP). La antigua
aplicación Tauri/Svelte no forma parte de `main`; su última versión se conserva en la rama
`experimental/ui-desktop`.

El servidor implementa MCP sobre JSON-RPC por `stdio`, con schemas de entrada y salida para todas
sus tools y perfiles de acceso `readonly` y `standard`. La adopción del transporte oficial `rmcp`
y sus capacidades adicionales permanece en evaluación.

Consulta el [changelog](CHANGELOG.md) para los cambios por versión y el
[estado de implementación](IMPLEMENTATION_STATUS.md) para el detalle de capacidades verificadas.

## Desarrollo

```bash
# Dependencias del arnés diferencial JS ↔ Rust
npm ci --prefix prototype/harness

# Suite completa
cargo test --workspace

# Gates usados en CI
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

El arnés diferencial ejecuta el prototipo JavaScript original como oráculo y comprueba la paridad
del motor Rust. La CI compila y prueba el workspace en Linux, macOS y Windows.

## Documentación

| Documento | Contenido |
|---|---|
| [Arquitectura](ARCHITECTURE.md) | Diseño ratificado e invariantes |
| [Contrato MCP](contracts/mcp.yml) | Superficie y semántica de las diez tools |
| [Workflows](docs/WORKFLOWS.md) | Flujo de desarrollo del proyecto |
| [Estado de implementación](IMPLEMENTATION_STATUS.md) | Trazabilidad de épicas y verificación |
| [Changelog](CHANGELOG.md) | Historial de versiones |
| [Releasing](RELEASING.md) | Proceso de publicación |

## Licencia

Lodestar se distribuye bajo **MIT OR Apache-2.0**, a tu elección. Consulta
[LICENSE-MIT](LICENSE-MIT) y [LICENSE-APACHE](LICENSE-APACHE).
