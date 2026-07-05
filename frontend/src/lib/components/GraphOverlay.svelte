<script lang="ts">
  // Overlay del grafo a pantalla completa. Port de <div class="graph-overlay">. Reusa la isla StarMap.
  import { snapshot, current, treeQuery } from "../stores/bundle";
  import { graphBigOpen, graphScope } from "../stores/ui";
  import { tokenizeQuery, matchFileQuery } from "../query";
  import StarMap from "./StarMap.svelte";

  const files = $derived($snapshot?.files ?? {});
  const analysis = $derived($snapshot?.analysis ?? null);
  const graph = $derived($snapshot?.graph ?? { nodes: [], edges: [] });

  let starmap = $state<StarMap | null>(null);

  const matchInfo = $derived.by(() => {
    const tokens = tokenizeQuery($treeQuery.trim());
    if (!tokens.length || !analysis) return "";
    const n = graph.nodes.filter((x) => !x.ghost && matchFileQuery(x.id, tokens, files, analysis)).length;
    return n + " coincidencia" + (n === 1 ? "" : "s");
  });
</script>

<div class="graph-overlay" class:on={$graphBigOpen}>
  <div class="go-bar">
    <span class="go-title">mapa · <span>{$graphScope === "neighbor" ? "vecindad" : "espacio"}</span></span>
    <div class="seg seg-sm">
      <button class:on={$graphScope === "bundle"} onclick={() => ($graphScope = "bundle")}>Global</button>
      <button class:on={$graphScope === "neighbor"} onclick={() => ($graphScope = "neighbor")}>Vecindad</button>
    </div>
    <button class="mini" onclick={() => starmap?.relayout()}>Reorganizar</button>
    <div class="go-search">
      <svg class="s-ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="7"/><path d="M21 21l-4.3-4.3"/></svg>
      <input class="s-input" placeholder="filtrar: type:Spec  tags:auth  is:orphan" spellcheck="false" autocomplete="off" bind:value={$treeQuery} />
      <button class="s-clear" class:on={!!$treeQuery} onclick={() => ($treeQuery = "")}>×</button>
      <span class="go-count">{matchInfo}</span>
    </div>
    <div class="spacer" style="flex:1"></div>
    <div class="legend">
      <span><i style="background:var(--star)"></i>página</span>
      <span><i style="background:var(--accent)"></i>actual</span>
      <span><i style="background:transparent;border:1px dashed var(--ghost)"></i>por escribir</span>
    </div>
    <button class="tbtn icon-only" title="Minimizar (Esc)" onclick={() => ($graphBigOpen = false)}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M19 9h-4V5M15 9l5-5M5 15h4v4M9 15l-5 5"/></svg>
    </button>
  </div>
  <StarMap bind:this={starmap} {graph} {analysis} {files} current={$current} query={$treeQuery} scope={$graphScope} active={$graphBigOpen} />
</div>
