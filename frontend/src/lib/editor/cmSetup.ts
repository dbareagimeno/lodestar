// Tema y highlight compartidos por las superficies CodeMirror del editor. Todo se tematiza con
// las variables CSS del tema (`--ink`, `--panel`, `--accent`…): así los modos claro/oscuro
// (`html[data-theme]`) quedan cubiertos gratis, sin duplicar paletas aquí.
import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";

// Aspecto base: fondo transparente (el contenedor `.cm-host` pone panel/borde cuando toca),
// caret y selección con el acento, y el tooltip de autocompletado con el aspecto de los
// paneles flotantes de la app (mismo borde, radio y sombra que los kebab/menus).
export const baseTheme = EditorView.theme({
  "&": { backgroundColor: "transparent", color: "var(--ink)" },
  "&.cm-focused": { outline: "none" },
  ".cm-content": { caretColor: "var(--ink)", padding: "0" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--ink)" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground":
    { backgroundColor: "var(--accent-dim)" },
  ".cm-placeholder": { color: "var(--faint)" },
  ".cm-tooltip": {
    backgroundColor: "var(--panel)",
    border: "1px solid var(--line-2)",
    borderRadius: "var(--radius)",
    boxShadow: "var(--shadow)",
    color: "var(--ink)",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul": {
    fontFamily: "var(--mono)",
    fontSize: "12px",
    maxHeight: "16em",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li": { padding: "4px 8px" },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]": {
    backgroundColor: "var(--panel-2)",
    color: "var(--accent)",
  },
  ".cm-completionDetail": { color: "var(--faint)", fontStyle: "normal", marginLeft: "0.8em" },
  ".cm-completionMatchedText": { textDecoration: "none", color: "var(--accent)" },
});

// Tipografías por superficie: replican `.raw-edit` (mono 13px/1.65) y `.body-edit`
// (sans 15px/1.75) para que el cambio textarea→CM no altere el aspecto del prototipo.
export const rawTypography = EditorView.theme({
  ".cm-content": { fontFamily: "var(--mono)", fontSize: "13px", lineHeight: "1.65" },
});
export const bodyTypography = EditorView.theme({
  ".cm-content": { fontFamily: "var(--sans)", fontSize: "15px", lineHeight: "1.75" },
});

// Highlight discreto con la paleta de la app: nada de arcoíris, solo acentuar la estructura
// (headings/enlaces con el acento, código/urls con el tono "estrella", marcas casi invisibles).
// Cubre también el frontmatter YAML del modo Código (propertyName con el acento).
export const mdHighlight = syntaxHighlighting(
  HighlightStyle.define([
    { tag: t.heading, color: "var(--accent)", fontWeight: "700" },
    { tag: t.link, color: "var(--accent)" },
    { tag: t.url, color: "var(--star)" },
    { tag: t.monospace, color: "var(--star)" },
    { tag: t.number, color: "var(--star)" },
    { tag: t.quote, color: "var(--muted)", fontStyle: "italic" },
    { tag: t.list, color: "var(--muted)" },
    { tag: t.emphasis, fontStyle: "italic" },
    { tag: t.strong, fontWeight: "700" },
    { tag: t.strikethrough, textDecoration: "line-through" },
    { tag: t.processingInstruction, color: "var(--faint)" },
    { tag: t.meta, color: "var(--faint)" },
    { tag: t.contentSeparator, color: "var(--faint)" },
    // YAML del frontmatter (modo Código).
    { tag: t.propertyName, color: "var(--accent)" },
    { tag: t.string, color: "var(--ink)" },
    { tag: t.bool, color: "var(--star)" },
    { tag: t.null, color: "var(--faint)" },
  ]),
);
