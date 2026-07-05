// Store del snapshot (ARCHITECTURE.md §8): el `BundleSnapshot` empujado por Rust es la ÚNICA fuente;
// árbol/pill/backlinks/grafo se DERIVAN. La webview es vista fina sobre él.

import { writable, derived } from "svelte/store";
import type { BundleSnapshot } from "../ipc/types";
import { getSnapshot } from "../ipc";

/** Snapshot empujado por la fachada (evento `bundle:changed`). Única fuente de verdad. */
export const snapshot = writable<BundleSnapshot | null>(null);

/** Mapa de ficheros del snapshot (conveniencia; `{}` si no hay bundle abierto). */
export const files = derived(snapshot, ($s) => $s?.files ?? {});

/** Texto de la query de búsqueda (compartida por lista/mapa/grafo). Estado de vista. */
export const treeQuery = writable<string>("");

/** Concept seleccionado (path), o null. */
export const current = writable<string | null>(null);

/**
 * Refresca el snapshot desde la fachada. En Tauri el watcher ya empuja `bundle:changed`; esto cubre
 * el arranque y el dev-mock (que no empuja eventos) tras una escritura.
 */
export async function refreshSnapshot(): Promise<void> {
  try {
    snapshot.set(await getSnapshot());
  } catch {
    /* sin bundle abierto: se deja el estado actual */
  }
}
