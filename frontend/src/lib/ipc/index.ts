// Envoltorio tipado del IPC con Tauri (ARCHITECTURE.md §8). Mata la deriva de nombres Rust↔TS.
//
// Cada función invoca UN comando registrado por la fachada Tauri (src-tauri). Fuera de Tauri
// (p. ej. `vite dev` en navegador) `invoke` lanza un error claro y la UI muestra el aviso.

import { COMMANDS, EVENTS } from "./types";
import type {
  Analysis,
  Backlinks,
  BundleSnapshot,
  ConceptSummary,
  GraphModel,
  RelPath,
  WriteOutcome,
} from "./types";

type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

async function getInvoke(): Promise<Invoke> {
  const w = window as unknown as { __TAURI__?: { core?: { invoke?: Invoke } } };
  const invoke = w.__TAURI__?.core?.invoke;
  if (!invoke) {
    throw new Error("IPC no disponible: la app no corre dentro de Tauri (usa el binario de escritorio).");
  }
  return invoke;
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const invoke = await getInvoke();
  return (await invoke(cmd, args)) as T;
}

export const openBundle = (path: string) => call<BundleSnapshot>(COMMANDS.openBundle, { path });
/** Diálogo nativo de carpetas; `null` si el usuario cancela. */
export const pickFolder = () => call<string | null>(COMMANDS.pickFolder);
/** Crea un workspace nuevo (scaffold + git) en `path` y lo abre. */
export const createBundle = (path: string) => call<BundleSnapshot>(COMMANDS.createBundle, { path });
export const getSnapshot = () => call<BundleSnapshot>(COMMANDS.getSnapshot);
export const listConcepts = () => call<ConceptSummary[]>(COMMANDS.listConcepts);
export const readConcept = (path: RelPath) => call<string>(COMMANDS.readConcept, { path });
export const writeConcept = (path: RelPath, content: string, allowNonconformant = false) =>
  call<WriteOutcome>(COMMANDS.writeConcept, { path, content, allowNonconformant });
export const createConcept = (
  path: RelPath,
  type: string,
  title?: string,
  body?: string,
  allowNonconformant = false,
) => call<WriteOutcome>(COMMANDS.createConcept, { path, type, title, body, allowNonconformant });
export const conformance = () => call<Analysis>(COMMANDS.conformance);
export const query = (dsl: string) => call<RelPath[]>(COMMANDS.query, { dsl });
export const backlinks = (path: RelPath) => call<Backlinks>(COMMANDS.backlinks, { path });
export const graphModel = () => call<GraphModel>(COMMANDS.graphModel);
export const history = (limit = 20) => call<CommitRow[]>(COMMANDS.history, { limit });
export const diffWorking = () => call<OkfDiff>(COMMANDS.diffWorking);
export const commit = (message: string) => call<CommitResult>(COMMANDS.commit, { message });

// Tipos auxiliares de comandos que no viven en el snapshot.
export interface CommitRow {
  id: string;
  short: string;
  message: string;
  author: { name: string; email: string };
  timeUnix: number;
  parents: string[];
  conformance: { hardFail: number; warnCount: number; conform: boolean } | null;
}
export interface CommitResult {
  sha: string;
  conformance: { hardFail: number; warnCount: number; conform: boolean };
}
// El OkfDiff se renderiza tal cual llega; se tipa laxo para no duplicar el contrato completo.
export type OkfDiff = Record<string, unknown>;

// Suscripción a eventos empujados por la fachada (`bundle:changed`).
type Listen = (event: string, cb: (e: { payload: unknown }) => void) => Promise<() => void>;

export async function onBundleChanged(cb: (snap: BundleSnapshot) => void): Promise<() => void> {
  const w = window as unknown as { __TAURI__?: { event?: { listen?: Listen } } };
  const listen = w.__TAURI__?.event?.listen;
  if (!listen) return () => {};
  return listen(EVENTS.bundleChanged, (e) => cb(e.payload as BundleSnapshot));
}

export { COMMANDS, EVENTS };
