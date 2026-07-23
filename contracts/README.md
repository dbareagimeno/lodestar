# Contratos de la frontera front↔back

> Estos YAML son la **spec de superficie y semántica** de la frontera: qué comandos, eventos y
> tools existen, con qué parámetros, qué devuelven, qué errores producen y qué invariantes debe
> respetar cada operación. Se verifican contra el código con `/contrato --check`
> (agente `guardian-contrato`).

## Qué es (y qué NO es) un contrato aquí

- **Los tipos NO se definen aquí.** El invariante #4 del repo manda: los tipos viven **una sola
  vez** en `crates/lodestar-core/src/types.rs` (ya no hay espejo TS: se fue con la UI a
  `experimental/ui-desktop`). En los YAML, los tipos se **referencian por nombre**
  (`Analysis`, `WorkspaceSnapshot`, `Check`, `RelPath`…). Si un YAML redefine la forma de un tipo,
  es un bug del contrato.
- **La superficie sí se define aquí**: nombres de comandos/eventos/tools (congelados por
  `ARCHITECTURE.md`), parámetros, retorno, errores y semántica (incluidos los invariantes que la
  operación debe respetar, p. ej. «escribe el `.md` atómico; nunca la cache»).

## Ficheros

| Fichero | Superficie | Lado de código que refleja |
|---|---|---|
| `ipc.yml` | Comandos Tauri + eventos | `src-tauri` (`#[tauri::command]`, `generate_handler!`, `app.emit`) |
| `mcp.yml` | Tools MCP | `crates/lodestar-mcp` (registro de tools JSON-RPC) |

## Reglas de mantenimiento

1. **Toda historia que toque la frontera lleva su «Delta de contrato»** en la spec
   (`requirements/`), y el YAML se actualiza en el mismo PR que el código.
2. Un delta ratificado pero aún no implementado se marca `estado: pendiente` en la entrada — el
   guardián no lo borra al sincronizar.
3. En discrepancia superficie↔YAML gana el **código real** (salvo `estado: pendiente`); en
   discrepancia de tipos gana **`core::types`**. Los nombres congelados nunca se «arreglan»
   renombrando código.
4. `/contrato --check` antes de cualquier PR que toque `core::types`, `src-tauri`,
   `lodestar-mcp` o `frontend/src/lib/ipc/`.
