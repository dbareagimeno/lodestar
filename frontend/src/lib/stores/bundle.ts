// Stores del bundle (ARCHITECTURE.md §8): el snapshot empujado es la ÚNICA fuente; tree/pill/
// backlinks/grafo se DERIVAN. La webview es vista fina sobre el `BundleSnapshot` de Rust.

import { writable, derived } from "svelte/store";
import type { BundleSnapshot, ConceptSummary, RelPath } from "../ipc/types";

/** El snapshot empujado por la fachada (evento `bundle:changed`). Única fuente de verdad. */
export const snapshot = writable<BundleSnapshot | null>(null);

/** Texto de query del árbol/grafo (estado de vista). */
export const treeQuery = writable<string>("");

/** Concept seleccionado (path), o null. */
export const selected = writable<RelPath | null>(null);

/** Vista activa: editor | grafo | cambios. */
export const view = writable<"editor" | "grafo" | "cambios">("editor");

/** Píldora de conformidad derivada del snapshot (nunca obsoleta: se recalcula con cada push). */
export const conformancePill = derived(snapshot, ($s) => {
  if (!$s) return { conform: true, hardFail: 0, warnCount: 0, label: "—" };
  const { hardFail, warnCount } = $s.analysis;
  return {
    conform: hardFail === 0,
    hardFail,
    warnCount,
    label:
      hardFail === 0
        ? warnCount > 0
          ? `${warnCount} avisos`
          : "Conforme"
        : `${hardFail} con errores`,
  };
});

/** El bloque de frontmatter de un `.md` crudo, o "" si no hay. */
function fmBlock(raw: string | undefined): string {
  return raw?.match(/^---\r?\n([\s\S]*?)\r?\n---/)?.[1] ?? "";
}

/** Extrae el `title` del frontmatter de un `.md` crudo (barato; el core es la autoridad real). */
function titleOf(raw: string | undefined, path: string): string {
  const t = fmBlock(raw).match(/^\s*title:\s*(.+?)\s*$/m);
  if (t) return t[1].replace(/^["']|["']$/g, "");
  const base = path.replace(/\.md$/, "").split("/").pop() ?? path;
  return base.replace(/[-_]/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

/** Filas del árbol derivadas del análisis. `type`/`status` salen del grafo (autoritativos del
 * core); el fallback regex queda ACOTADO al frontmatter — antes escaneaba el fichero entero y
 * un `status: draft` dentro de un bloque de código del cuerpo contaminaba el árbol. */
export const treeRows = derived(snapshot, ($s): ConceptSummary[] => {
  if (!$s) return [];
  const orphans = new Set($s.analysis.orphans);
  const byId = new Map($s.graph.nodes.map((n) => [n.id, n]));
  return $s.analysis.concepts.map((path) => {
    const checks = $s.analysis.perFile[path] ?? [];
    const raw = $s.files[path];
    const node = byId.get(path);
    const fm = fmBlock(raw);
    const type =
      node?.type ?? fm.match(/^\s*type:\s*(.+?)\s*$/m)?.[1]?.replace(/^["']|["']$/g, "") ?? null;
    const status =
      node?.status ?? fm.match(/^\s*status:\s*(.+?)\s*$/m)?.[1]?.replace(/^["']|["']$/g, "") ?? null;
    return {
      path,
      title: titleOf(raw, path),
      type,
      status,
      orphan: orphans.has(path),
      invalid: checks.some((c) => c.level === "err"),
    };
  });
});
