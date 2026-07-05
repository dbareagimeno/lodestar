<script lang="ts">
  // Isla imperativa de CodeMirror (mismo precedente que el grafo, ARCHITECTURE.md §8): el
  // componente POSEE el EditorView; Svelte solo pasa props. El contrato con el padre es el de
  // un textarea "no controlado mientras editas": cada tecla sale por `onChange` y el `value`
  // externo solo se aplica cuando difiere del doc (cambio de fichero o reconciliación del
  // watcher, que el padre ya condiciona a `!dirty`) — así el sync nunca pisa lo tecleado.
  import { untrack } from "svelte";
  import { EditorState, type Extension } from "@codemirror/state";
  import { EditorView, keymap, drawSelection, placeholder as cmPlaceholder } from "@codemirror/view";
  import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
  import { autocompletion, completionKeymap } from "@codemirror/autocomplete";
  import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
  import { yamlFrontmatter } from "@codemirror/lang-yaml";
  import { baseTheme, rawTypography, bodyTypography, mdHighlight } from "../editor/cmSetup";
  import { linkCompletion, type PageEntry } from "../editor/linkCompletion";

  interface Props {
    value: string;
    onChange: (v: string) => void;
    variant: "raw" | "body";
    placeholder?: string;
    getPages?: () => PageEntry[];
    autocomplete?: boolean;
  }
  let { value, onChange, variant, placeholder, getPages, autocomplete = true }: Props = $props();

  let host: HTMLDivElement;
  let view: EditorView | null = null;

  function buildExtensions(): Extension[] {
    // El autocompletado se registra como language-data de markdownLanguage: en el modo Código
    // (yamlFrontmatter) el frontmatter parsea como YAML y la fuente NO se dispara ahí dentro.
    const mdExtensions =
      autocomplete && getPages ? [markdownLanguage.data.of({ autocomplete: linkCompletion(getPages) })] : [];
    const lang =
      variant === "raw"
        ? yamlFrontmatter({ content: markdown({ base: markdownLanguage }) })
        : markdown({ base: markdownLanguage });
    const exts: Extension[] = [
      lang,
      ...mdExtensions,
      history(),
      drawSelection(),
      EditorView.lineWrapping,
      EditorView.contentAttributes.of({ spellcheck: "false" }),
      // SIN indentWithTab: Tab sigue moviendo el foco (accesibilidad, como los textareas).
      keymap.of([...defaultKeymap, ...historyKeymap, ...completionKeymap]),
      baseTheme,
      variant === "raw" ? rawTypography : bodyTypography,
      mdHighlight,
      EditorView.updateListener.of((u) => {
        if (u.docChanged) onChange(u.state.doc.toString());
      }),
    ];
    if (autocomplete && getPages) exts.push(autocompletion());
    if (placeholder) exts.push(cmPlaceholder(placeholder));
    return exts;
  }

  // Montaje/destroy: debe correr UNA sola vez. Las props de $props() son reactivas, así que
  // leer `value`/`variant`/… aquí las convertiría en dependencias y cada tecla (onChange →
  // draft → prop `value`) destruiría y recrearía el EditorView, perdiendo foco/caret/undo.
  // `untrack` corta esas dependencias: los cambios de `value` llegan por el efecto de sync de
  // abajo, y un fichero nuevo llega vía {#key loadedFor} del padre, que recrea el componente.
  $effect(() => {
    view = untrack(
      () =>
        new EditorView({
          state: EditorState.create({ doc: value, extensions: buildExtensions() }),
          parent: host,
        }),
    );
    return () => {
      view?.destroy();
      view = null;
    };
  });

  // Sincronización externa anti-eco: si el doc ya iguala `value` (el caso de cada tecla, que
  // volvió por onChange), no hay nada que hacer; si difiere, reemplazo completo con el caret
  // acotado — equivalente exacto del patrón "uncontrolled mientras editas" de los textareas.
  $effect(() => {
    const v = value;
    if (!view) return;
    if (view.state.doc.toString() === v) return;
    const anchor = Math.min(view.state.selection.main.anchor, v.length);
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: v },
      selection: { anchor },
    });
  });
</script>

<div bind:this={host} class="cm-host {variant === 'raw' ? 'cm-raw' : 'cm-body'}"></div>
