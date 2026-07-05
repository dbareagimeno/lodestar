// Fuente de autocompletado de enlaces internos. Se activa con el cursor dentro de `](…` y
// ofrece las páginas del bundle como rutas absolutas `/ruta.md` — la forma recomendada que
// `resolveLink` (okf.ts) resuelve desde cualquier página. Se registra como language-data de
// markdownLanguage, así NO se dispara dentro del frontmatter YAML del modo Código.
// Alcance v1: solo enlaces; el filtrado fuzzy por defecto de CM sobre `label` hace el resto.
import type { CompletionContext, CompletionResult } from "@codemirror/autocomplete";

export type PageEntry = { path: string; title: string };

// `getPages` es una closure: CM lee la lista fresca en cada activación sin que el wrapper
// tenga que recrear extensiones cuando cambian los ficheros del bundle.
export function linkCompletion(getPages: () => PageEntry[]) {
  return (ctx: CompletionContext): CompletionResult | null => {
    const line = ctx.state.doc.lineAt(ctx.pos);
    const before = line.text.slice(0, ctx.pos - line.from);
    const m = /\]\(([^()\s]*)$/.exec(before); // cursor dentro de `](…`
    if (!m) return null;
    return {
      from: ctx.pos - m[1].length,
      options: getPages().map((p) => ({
        label: "/" + p.path, // forma absoluta recomendada por resolveLink
        detail: p.title,
        type: "text",
        // Los índices generados rara vez son el destino que buscas: al fondo de la lista.
        boost: p.path.endsWith("index.md") ? -1 : 0,
      })),
      validFor: /^[^()\s]*$/, // filtra en cliente mientras tecleas, sin re-consultar
    };
  };
}
