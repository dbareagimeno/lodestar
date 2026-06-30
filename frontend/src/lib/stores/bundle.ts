// Stores del bundle (ARCHITECTURE.md §8): el snapshot empujado es la ÚNICA fuente; tree/pill/backlinks
// se DERIVAN. Aquí la base; el port completo de la UI (E6) añade los derived de pill/backlinks/graph.

import { writable, derived } from "svelte/store";
import type { BundleSnapshot, ConceptSummary } from "../ipc/types";

/** El snapshot empujado por la fachada (evento `bundle:changed`). Única fuente de verdad en el front. */
export const snapshot = writable<BundleSnapshot | null>(null);

/** Texto de query del árbol (estado de vista). */
export const treeQuery = writable<string>("");

/** Píldora de conformidad derivada del snapshot. */
export const conformancePill = derived(snapshot, ($s) => {
  if (!$s) return { conform: true, hardFail: 0, warnCount: 0, label: "—" };
  const { hardFail, warnCount } = $s.analysis;
  return {
    conform: hardFail === 0,
    hardFail,
    warnCount,
    label: hardFail === 0 ? (warnCount > 0 ? `${warnCount} avisos` : "Conforme") : `${hardFail} con errores`,
  };
});

/** Filas del árbol derivadas del análisis (orphan/invalid ya resueltos en Rust). */
export const treeRows = derived(snapshot, ($s): ConceptSummary[] => {
  if (!$s) return [];
  return $s.analysis.concepts.map((path) => {
    const checks = $s.analysis.perFile[path] ?? [];
    const invalid = checks.some((c) => c.level === "err");
    const orphan = $s.analysis.orphans.includes(path);
    return { path, title: path, type: null, status: null, orphan, invalid };
  });
});
