# Propuesta: la CLI como gestor de bases de conocimiento

> **Estado**: PROPUESTA DE DISEÑO, no ratificada. **Nada de este documento está implementado.**
> Se escribe en la PR #17 (`E23-H15`) para que `/planificar` la consuma en una PR posterior.
> Autoridad de diseño: `ARCHITECTURE.md §19.2` (capa de servicios) y `§20` (modelo documental).
> Origen: revisión de la PR #17 (2026-07-25) y decisión del usuario de separar el arreglo de la
> migración (E23) de la construcción de la CLI.

---

## 1. Diagnóstico

Hoy la CLI (`crates/lodestar-cli`) tiene **tres subcomandos**:

| Subcomando | Qué hace |
|---|---|
| `check [--json\|--sarif]` | Puerta de CI: ¿el workspace es interpretable y consistente? Exit `0`/`1`/`2`/`3`. |
| `reindex` | Reconstruye la cache `.lodestar/index.db`. |
| `migrate-from-okf --dry-run` | Diagnostica convenciones OKF legadas sin tocar nada. |

Y **cero** capacidad de gestionar conocimiento: no puede buscar, leer un documento, consultar el
grafo, ver backlinks, inspeccionar metadata, crear, mover ni borrar. La única lectura estructurada
que ofrece es `check --json`, que es el `Analysis` completo por un canal pensado para CI.

Consecuencia: **todo el valor del producto vive detrás del MCP**. Quien quiera usar su propia base
de notas desde una terminal —o desde un script, un Makefile, un hook— tiene que levantar un servidor
JSON-RPC y hablar el protocolo a mano. El README describe Lodestar como consumible «desde Claude
Code, Codex u otros clientes MCP y desde la **CLI**»; la segunda mitad de esa frase no se sostiene.

Asimetrías concretas frente al MCP:

- **Lectura**: el MCP tiene 7 tools de lectura/consulta; la CLI, ninguna.
- **Escritura**: el MCP tiene el ciclo `change_plan`/`change_apply`/`change_revert`; la CLI no puede
  escribir conocimiento en absoluto.
- **Perfiles**: `--profile readonly|standard` solo existe en el MCP.
- **Nombres**: la CLI usa `--path` y el MCP `--root` para el mismo concepto.
- **Formatos**: solo `check` tiene `--json`/`--sarif`; `reindex` y `migrate-from-okf` son texto
  humano y por tanto no automatizables.

---

## 2. Principio rector

> **Paridad de capacidades, no paridad de forma.**

`lodestar-app` ya existe **exactamente** para esto: es la capa de casos de uso compartida por las dos
fachadas (`ARCHITECTURE.md §19.2`), con el envelope y los códigos de error, y **cero lógica de
dominio**. Los métodos públicos que la CLI necesitaría ya están escritos y probados:
`workspace_status`, `knowledge_search`, `knowledge_get`, `metadata_inspect`, `knowledge_check`,
`graph_query`, `impact_analyze`, `change_plan`, `change_apply`, `change_revert`.

Es decir: **la lectura es casi gratis**. Son shells finos —del orden de 15-30 líneas de clap más una
llamada y una serialización— sobre código ya cubierto por tests. Eso mantiene el invariante de que
las fachadas no tienen lógica propia.

La escritura es lo contrario: copiar la forma del MCP significaría escribir JSON de operaciones en la
línea de comandos, que es mala ergonomía para el caso humano. Lo que un humano quiere es
`lodestar mv notas/a.md archivo/a.md` y que los backlinks se reescriban solos.

---

## 3. Lectura propuesta

Todos con `--json` (salida estable para scripts) y una salida humana legible por defecto.

| Subcomando | Sobre | Notas |
|---|---|---|
| `lodestar search [texto] [--where <expr>] [--filter <json>] [--limit N]` | `App::knowledge_search` | El lenguaje tipado de `§20.8` tal cual; es la pieza más valiosa del motor y hoy es inalcanzable desde una terminal. |
| `lodestar get <path> [--include frontmatter,body,backlinks,…] [--section <heading>]` | `App::knowledge_get` | Mismo `include` selectivo que la tool. |
| `lodestar graph <backlinks\|outgoing\|neighborhood\|isolated\|dangling\|path-between\|cycles\|components> [<path>]` | `App::graph_query` | |
| `lodestar status` | `App::workspace_status` | Config efectiva, recuentos, capacidades. |
| `lodestar metadata [<campo>]` | `App::metadata_inspect` | Sin campo, el catálogo; con campo, sus valores. |
| `lodestar impact <path> --kind <move\|delete>` | `App::impact_analyze` | |

Además: `reindex` y `migrate-from-okf` ganan `--json`.

**Decisión pendiente**: `check` ya existe y su exit code está **congelado** (`0/1/2/3`). Los
subcomandos nuevos no deben heredar esa semántica —una búsqueda sin resultados no es un fallo—, así
que hay que fijar explícitamente qué exit code usan (propuesta: `0` siempre que la operación se
complete; `2` uso; `3` runtime/IO; nunca `1`).

---

## 4. Escritura propuesta

**Verbos, no operaciones.** Cada verbo construye internamente un plan y lo aplica por el único
escritor, con todas las salvaguardas transaccionales (staging → lock → copias de recuperación →
journal → renames atómicos → receipt):

| Verbo | Equivale a | Notas |
|---|---|---|
| `lodestar new <path> [--set k=v]… [--body -]` | `create_document` | Tras `E23-H02`, sin frontmatter inyectado. |
| `lodestar mv <origen> <destino>` | `move_document` con `rewriteInboundLinks` | Reescribir backlinks debe ser el **default** del verbo humano; `--no-rewrite` para lo contrario. |
| `lodestar rm <path> --links <reject\|remove>` | `delete_document` | La política es obligatoria, como en el MCP (`§20.11`: no elegir en silencio). |
| `lodestar set <path> k=v [k=v]…` | `patch_frontmatter` | Merge-patch: `k=` borra la clave. |

Más una **escotilla para scripting**, que es donde sí tiene sentido el JSON crudo:

```bash
lodestar plan --file cambios.json     # imprime el changeSetId y el diff, sin escribir
lodestar apply <changeSetId>
lodestar revert <receiptId>
```

Con `--dry-run` en todos los verbos (que es, literalmente, quedarse en `change_plan`).

---

## 5. Riesgo que hay que resolver ANTES de la escritura

Una CLI que escribe convierte en escenario cotidiano el caso que **hoy no tiene ni un test**: dos
procesos escribiendo el mismo workspace. El lock es `O_CREAT|O_EXCL` sobre un fichero —una primitiva
**inter-proceso**— pero la única prueba de concurrencia del repo (`bench_concurrencia_segura`) usa
dos hilos dentro del mismo proceso. Y no hay ninguna prueba de lock huérfano (un `.lodestar/lock`
dejado por un proceso muerto).

El despliegue real es exactamente ese: un `lodestar-mcp` sirviendo a un agente mientras el usuario
teclea `lodestar mv` en otra terminal, o un `lodestar check` corriendo en CI sobre el mismo checkout.

> **Condición de entrada**: la escritura por CLI no se implementa hasta que estén los tests de
> concurrencia entre procesos y de lock huérfano (`E23-H09`). La lectura no tiene esa dependencia y
> puede ir antes.

---

## 6. Preguntas abiertas para la puerta de diseño

1. **Perfiles**: ¿la CLI hereda `--profile readonly|standard`? Un `readonly` que impida escribir
   sería coherente con el MCP, pero la CLI ya se distingue por subcomandos.
2. **`--path` vs `--root`**: ¿se renombra por simetría con el MCP? Rompe la CLI existente, pero v0.3
   ya es incompatible y es el momento barato.
3. **Salida por defecto**: ¿humana con `--json` opcional (como `check` hoy), o JSON por defecto por
   ser una herramienta de agentes? Propuesta: humana, porque el consumidor agéntico ya tiene el MCP.
4. **`writableRoots`**: la escritura por CLI ¿respeta la write policy de `.lodestar/config.yaml`?
   Debería, por el mismo `assert_writable` — conviene dejarlo escrito para que no se re-decida.
5. **Alcance de v1**: ¿entra la escritura en la primera entrega, o se publica primero la lectura
   (barata, sin riesgo nuevo) y la escritura va en una segunda? Propuesta: dos entregas.
6. **¿Y `resources` MCP?**: fuera de alcance de este documento, pero la misma discusión de
   descubribilidad aparece ahí (hoy el servidor solo implementa `tools/*`).

---

## 7. Lo que este documento NO propone

- No propone que la CLI reimplemente nada: si un caso de uso no está en `lodestar-app`, la historia
  correspondiente lo añade **ahí**, no en la fachada (invariante: fachadas sin lógica de dominio).
- No propone una TUI, un editor ni un modo interactivo. El motor es headless (`§19.1`).
- No propone reintroducir git en la superficie (`§20.13`).
