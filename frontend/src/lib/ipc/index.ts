// Envoltorio tipado del IPC con Tauri (ARCHITECTURE.md §8). Mata la deriva de nombres Rust↔TS.
//
// Importa los tipos del contrato (generados desde Rust en producción). Cada función invoca UN comando
// de la Workspace. La fachada Tauri (E6) registra estos comandos; aquí está el lado del cliente.

import { COMMANDS, EVENTS } from "./types";
import type { Analysis, BundleSnapshot, ConceptSummary, RelPath } from "./types";

// `invoke`/`listen` reales vienen de @tauri-apps/api en la app empaquetada. Para que el frontend
// compile y se pueda probar fuera de Tauri, se resuelven de forma perezosa con un fallback claro.
type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

async function getInvoke(): Promise<Invoke> {
  const w = window as unknown as { __TAURI__?: { core?: { invoke?: Invoke } } };
  const invoke = w.__TAURI__?.core?.invoke;
  if (!invoke) {
    throw new Error("IPC no disponible: la app no corre dentro de Tauri (E6 pendiente de empaquetar).");
  }
  return invoke;
}

export async function getSnapshot(): Promise<BundleSnapshot> {
  const invoke = await getInvoke();
  return (await invoke(COMMANDS.getSnapshot)) as BundleSnapshot;
}

export async function conformance(): Promise<Analysis> {
  const invoke = await getInvoke();
  return (await invoke(COMMANDS.conformance)) as Analysis;
}

export async function listConcepts(): Promise<ConceptSummary[]> {
  const invoke = await getInvoke();
  return (await invoke(COMMANDS.listConcepts)) as ConceptSummary[];
}

export async function query(dsl: string): Promise<RelPath[]> {
  const invoke = await getInvoke();
  return (await invoke(COMMANDS.query, { dsl })) as RelPath[];
}

export { COMMANDS, EVENTS };
