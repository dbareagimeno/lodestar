# Lodestar

**Haz que cualquier repositorio Markdown sea navegable, consultable y seguro para agentes de IA.**

[![CI](https://github.com/dbareagimeno/lodestar/actions/workflows/ci.yml/badge.svg)](https://github.com/dbareagimeno/lodestar/actions/workflows/ci.yml)
[![Rust 1.80+](https://img.shields.io/badge/Rust-1.80%2B-dea584?logo=rust)](rust-toolchain.toml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#licencia)

Lodestar es un motor local y transaccional que permite a los agentes descubrir, consultar,
comprender y modificar una red de documentos Markdown sin convertirla a un formato propietario.
Lee la estructura que ya existe, interpreta el frontmatter que ya utilizas, resuelve sus enlaces y
protege los cambios con validación, control de concurrencia y rollback.

```bash
cd mi-proyecto
lodestar-mcp
```

Eso es todo. No necesitas inicializar el repositorio, crear un índice especial ni adaptar tus
documentos. Lodestar descubre recursivamente los `.md`, respeta `.gitignore` y
`.lodestarignore`, y usa el directorio actual como workspace.

> Tus Markdown siguen siendo la única fuente de verdad: legibles por personas, versionables con las
> herramientas que prefieras y utilizables sin Lodestar.

## Qué aporta

- **Contexto preciso para agentes.** Busca primero y recupera después solo el documento, la sección
  o los campos necesarios; no vuelca todo el repositorio en el contexto.
- **Consultas sobre tu propia metadata.** Filtra cualquier propiedad YAML con operadores tipados,
  dot notation y condiciones sobre el grafo, sin imponer un schema.
- **Una visión real de las relaciones.** Resuelve enlaces Markdown, backlinks, documentos aislados,
  enlaces rotos, caminos, ciclos y componentes a cualquier profundidad.
- **Análisis antes de cambiar.** Calcula el radio de impacto y las referencias afectadas antes de
  mover o eliminar un documento.
- **Cambios recuperables.** Planifica y valida en memoria, aplica mediante una transacción atómica y
  conserva un recibo con el que poder revertir.
- **Una puerta de calidad para CI.** Audita el working tree y produce salida humana, JSON o SARIF
  con códigos de salida estables.
- **Local-first y file-first.** El servidor se comunica por `stdio`; la caché SQLite/FTS5 es
  derivada y puede reconstruirse desde los Markdown.

## Cómo trabaja un agente con Lodestar

```text
orientarse → buscar → leer → inspeccionar metadata y relaciones
    → analizar impacto → planificar → aplicar → validar → revertir si hace falta
```

Lodestar expone ese recorrido mediante MCP:

| Necesidad | Tools |
|---|---|
| Entender el workspace | `workspace_status`, `knowledge_search`, `knowledge_get`, `metadata_inspect` |
| Analizar el conocimiento | `graph_query`, `impact_analyze`, `knowledge_check` |
| Cambiar con seguridad | `change_plan`, `change_apply`, `change_revert` |

El perfil `readonly` ofrece las siete tools de lectura y verificación. El perfil `standard`, usado
por defecto, añade planificación, aplicación y reversión de cambios.

## Inicio rápido

### 1. Instala los binarios

Lodestar requiere Rust 1.80 o posterior. Desde una copia del repositorio:

```bash
cargo install --path crates/lodestar-cli
cargo install --path crates/lodestar-mcp
```

Esto instala:

- `lodestar`, la CLI para validación y mantenimiento;
- `lodestar-mcp`, el servidor local que conecta el workspace con un agente.

No hacen falta Node.js, git ni librerías de interfaz gráfica.

### 2. Comprueba un proyecto

```bash
cd /ruta/a/mi-proyecto
lodestar check
```

Lodestar recorrerá todos los Markdown visibles del proyecto sin modificar ningún fichero.

```bash
lodestar check --json
lodestar check --sarif > lodestar.sarif
```

### 3. Conecta tu cliente MCP

Configura el cliente para lanzar `lodestar-mcp` por `stdio`. Estos son los valores esenciales:

```text
command: lodestar-mcp
args:    --root /ruta/absoluta/mi-proyecto --profile readonly
```

En clientes con configuración MCP en JSON, la definición equivalente es:

```json
{
  "mcpServers": {
    "lodestar": {
      "command": "lodestar-mcp",
      "args": [
        "--root",
        "/ruta/absoluta/mi-proyecto",
        "--profile",
        "readonly"
      ]
    }
  }
}
```

Puedes omitir `--root` si el cliente arranca el proceso dentro del proyecto. Usa `readonly` para
explorar o revisar; cambia a `standard` cuando quieras permitir cambios transaccionales.

## Funciona con el Markdown que ya tienes

No hay campos obligatorios ni nombres de fichero reservados. Un documento puede ser Markdown plano:

```markdown
# Rotación de credenciales

Consulta también el [runbook de despliegue](../runbooks/deploy.md).
```

O puede utilizar cualquier frontmatter YAML que tenga sentido para tu equipo:

```markdown
---
status: accepted
priority: 2
owners:
  - platform
service:
  tier: critical
---

# Rotación de credenciales

Consulta también el [runbook de despliegue](../runbooks/deploy.md).
```

Lodestar conserva los tipos YAML reales y permite consultas como:

```text
status = "accepted" and priority >= 2
owners contains "platform"
service.tier = "critical"
graph.backlinks = 0
```

`metadata_inspect` permite que un agente descubra primero qué campos existen, sus tipos, su
cobertura y sus valores frecuentes. Así puede entender las convenciones de un proyecto desconocido
sin que tengas que mantener un schema paralelo.

## Grafo e impacto

Cada enlace Markdown interno forma una arista del grafo. Lodestar reconoce enlaces inline, enlaces
de referencia, anchors, destinos externos y rutas relativas entre documentos situados a cualquier
profundidad.

`graph_query` permite consultar:

- backlinks y enlaces salientes;
- vecindarios entrantes, salientes o bidireccionales;
- documentos aislados y enlaces sin destino;
- el camino entre dos documentos;
- ciclos y componentes del grafo.

Antes de un `move` o `delete`, `impact_analyze` identifica afectados directos y transitivos y
calcula el nivel de riesgo sin tocar disco.

## Cambios seguros y recuperables

El perfil `standard` separa deliberadamente pensar de escribir:

1. `change_plan` normaliza las operaciones, simula el resultado en memoria, calcula el diff
   semántico, evalúa el impacto y valida si el cambio se puede aplicar.
2. `change_apply` comprueba que el workspace no haya cambiado desde el plan y publica mediante
   staging, lock, copias de recuperación, write-ahead journal y renames atómicos.
3. `knowledge_check` confirma el estado resultante.
4. `change_revert` restaura una transacción reciente desde su recibo si necesitas deshacerla.

Las operaciones disponibles cubren creación, modificación quirúrgica del frontmatter, sustitución
de cuerpo o texto, edición de secciones, movimientos y borrados. También pueden aplicarse
operaciones compatibles sobre selecciones obtenidas mediante consulta.

Las revisiones deterministas de documento y workspace proporcionan control optimista de
concurrencia: si una persona u otra herramienta cambia un fichero entre el plan y la aplicación,
Lodestar rechaza la escritura obsoleta.

## CLI

La CLI es una fachada pequeña para personas, scripts y CI:

| Comando | Uso |
|---|---|
| `lodestar check` | Audita el working tree |
| `lodestar reindex` | Reconstruye `.lodestar/index.db` desde los Markdown |
| `lodestar migrate-from-okf --dry-run` | Diagnostica convenciones OKF heredadas sin modificar ficheros |

Para operar sobre otro directorio sin cambiar el `cwd`:

```bash
lodestar --path /ruta/al/proyecto check
```

Los códigos de salida de `check` son estables: `0` sin errores, `1` validación bloqueada, `2` uso
inválido y `3` error de runtime o I/O.

## Migración desde OKF

Los repositorios creados con el antiguo formato OKF siguen siendo Markdown válido y pueden abrirse
directamente. El comando de migración es únicamente diagnóstico:

```bash
lodestar --path /ruta/al/proyecto migrate-from-okf --dry-run
```

Informa sobre índices, `okf_version` e índices de tags heredados, pero nunca modifica el proyecto.
Consulta el [changelog](CHANGELOG.md) para conocer la evolución del formato y las incompatibilidades
entre releases.

## Arquitectura

```text
                         ┌──────────────────────────┐
Repositorio Markdown ───► descubrimiento + parser  │
 fuente de verdad        │ metadata · links · query├──► MCP / agentes
                         │ grafo · impacto · diff   ├──► CLI / CI
                         │ transacciones · recovery │
                         └────────────┬─────────────┘
                                      ▼
                              SQLite / FTS5
                             caché reconstruible
```

La lógica de dominio es compartida por las dos fachadas:

```text
crates/
  lodestar-core/        modelo documental, metadata, enlaces, query, grafo y diff
  lodestar-store/       caché SQLite/FTS5 y watcher
  lodestar-workspace/   descubrimiento, I/O y publicación recuperable
  lodestar-app/         casos de uso compartidos por CLI y MCP
  lodestar-cli/         fachada para personas y CI
  lodestar-mcp/         fachada MCP por stdio para agentes
  lodestar-fixtures/    workspaces compartidos de test
```

El core no realiza I/O y las fachadas no reimplementan la semántica. Una consulta o validación
produce el mismo resultado independientemente del consumidor.

## Desarrollo

```bash
cargo test --workspace --locked
cargo test -p lodestar-workspace --features test-failpoints --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

La CI ejecuta formato, lint estricto, build, documentación y tests —incluidos los escenarios de
crash-recovery— en Linux, macOS y Windows.

## Documentación

| Documento | Contenido |
|---|---|
| [Arquitectura](ARCHITECTURE.md) | Diseño vigente e invariantes del motor |
| [Contrato MCP](contracts/mcp.yml) | Superficie y semántica de las tools |
| [Estado de implementación](IMPLEMENTATION_STATUS.md) | Capacidades verificadas y trazabilidad |
| [Decisiones](DECISIONES.md) | Decisiones de producto abiertas o ratificadas |
| [Changelog](CHANGELOG.md) | Historial de cambios por release |
| [Releasing](RELEASING.md) | Proceso de publicación |

## Licencia

Lodestar se distribuye bajo **MIT OR Apache-2.0**, a tu elección. Consulta
[LICENSE-MIT](LICENSE-MIT) y [LICENSE-APACHE](LICENSE-APACHE).
