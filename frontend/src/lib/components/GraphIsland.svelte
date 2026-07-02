<script lang="ts">
  // Isla imperativa del grafo (ARCHITECTURE.md §8): el módulo `createStarMap` posee el <svg> y el
  // loop rAF; aquí SOLO le pasamos nodos/aristas/selección por métodos en $effect. NUNCA {#each}.
  import { onMount, onDestroy } from "svelte";
  import { snapshot, selected, treeQuery } from "../stores/bundle";
  import { createStarMap, type StarNode, type StarEdge } from "../graph/starmap";

  let svgEl: SVGSVGElement;
  let map: ReturnType<typeof createStarMap> | null = null;

  onMount(() => {
    map = createStarMap(svgEl, {
      onSelect: (id) => ($selected = id),
      onCreateGhost: (id) => ($selected = id),
    });
  });
  onDestroy(() => map?.destroy());

  // Empuja el modelo del grafo al módulo imperativo cuando cambia el snapshot o la selección.
  $effect(() => {
    const s = $snapshot;
    const cur = $selected;
    if (!map || !s) return;
    const nodes: StarNode[] = s.graph.nodes.map((n) => ({ id: n.id, ghost: n.ghost, type: n.type }));
    const edges: StarEdge[] = s.graph.edges.map((e) => ({ source: e.source, target: e.target }));
    map.setData(nodes, edges, cur);
  });

  // Resalta los nodos que casan con la query (subcadena sobre id); null = sin filtro.
  $effect(() => {
    const q = $treeQuery.trim().toLowerCase();
    const s = $snapshot;
    if (!map || !s) return;
    map.setQuery(q ? new Set(s.graph.nodes.map((n) => n.id).filter((id) => id.toLowerCase().includes(q))) : null);
  });
</script>

<svg bind:this={svgEl} class="starmap" role="img" aria-label="Mapa de conceptos"></svg>

<style>
  .starmap {
    width: 100%;
    height: 100%;
    display: block;
    background: radial-gradient(circle at 50% 40%, rgba(243, 198, 89, 0.04), transparent 70%);
  }
  .starmap :global(.edge) {
    stroke: var(--line-2);
    stroke-width: 1;
  }
  .starmap :global(.edge.hot) {
    stroke: var(--accent);
    stroke-width: 1.5;
  }
  .starmap :global(.edge.dim) {
    opacity: 0.15;
  }
  .starmap :global(.node .core) {
    stroke: rgba(0, 0, 0, 0.4);
    stroke-width: 0.5;
  }
  .starmap :global(.node .halo) {
    opacity: 0.18;
  }
  .starmap :global(.node.ghost .core) {
    fill: var(--ghost);
  }
  .starmap :global(.node.dim) {
    opacity: 0.25;
  }
  .starmap :global(.node .hit) {
    fill: transparent;
    cursor: pointer;
  }
  .starmap :global(.nlabel) {
    fill: var(--muted);
    font-family: var(--mono);
    font-size: 10px;
  }
  .starmap :global(.node.sel .nlabel) {
    fill: var(--ink);
  }
</style>
