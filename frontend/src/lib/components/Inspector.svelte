<script lang="ts">
  // Panel derecho (inspector). Port del <section class="col inspector">: mini-mapa de vecindad
  // (radial estático, renderInspMap) + panel de enlaces (citado por / enlaza a / por escribir).
  import { snapshot, current } from "../stores/bundle";
  import { toggleRail, select, offerCreate, graphScope, graphBigOpen, isReserved } from "../stores/ui";
  import { parseFile, basename, conceptId, RESERVED, titleFromPath, starTint } from "../okf";

  const files = $derived($snapshot?.files ?? {});
  const analysis = $derived($snapshot?.analysis ?? null);

  function dt(p: string): string {
    return files[p] !== undefined ? (parseFile(files, p).fm?.title as string) || titleFromPath(p) : titleFromPath(p);
  }

  // ---- backlinks ----
  const bl = $derived.by(() => {
    const cur = $current;
    if (!cur || !analysis || RESERVED.has(basename(cur))) return null;
    const inn = analysis.inn[cur] || [];
    const out = analysis.out[cur] || [];
    const outReal = out.filter((t) => files[t] !== undefined && !RESERVED.has(basename(t)));
    const outDang = out.filter((t) => files[t] === undefined);
    return { inn, outReal, outDang, count: inn.length + out.length };
  });

  // ---- mini-mapa radial (port de renderInspMap) ----
  interface MiniNode { id: string; ghost: boolean; x: number; y: number; col: string; label: string; anchorEnd: boolean; lx: number; ly: number }
  const W = 300, H = 190, ccx = W / 2, ccy = H / 2;
  const mini = $derived.by(() => {
    const cur = $current;
    if (!cur || !analysis || files[cur] === undefined || RESERVED.has(basename(cur))) return null;
    const seen = new Map<string, { ghost: boolean }>();
    (analysis.out[cur] || []).forEach((t) => { if (!RESERVED.has(basename(t))) seen.set(t, { ghost: files[t] === undefined }); });
    (analysis.inn[cur] || []).forEach((s) => { if (!RESERVED.has(basename(s)) && !seen.has(s)) seen.set(s, { ghost: false }); });
    const neigh = [...seen.entries()];
    const n = neigh.length;
    const Rx = Math.max(38, Math.min((W - 176) / 2, 92));
    const Ry = Math.max(34, (H - 66) / 2);
    const nodes: MiniNode[] = neigh.map(([id, m], i) => {
      const a = -Math.PI / 2 + (i / Math.max(1, n)) * Math.PI * 2;
      const x = ccx + Math.cos(a) * Rx, y = ccy + Math.sin(a) * Ry;
      let label = conceptId(id).split("/").pop()!;
      if (label.length > 10) label = label.slice(0, 9) + "…";
      const right = x >= ccx - 0.5;
      const pf = m.ghost ? null : parseFile(files, id);
      return { id, ghost: m.ghost, x, y, col: m.ghost ? "" : starTint((pf?.fm?.type as string) || ""), label, anchorEnd: !right, lx: x + (right ? 8 : -8), ly: y + 3 };
    });
    return { nodes, n };
  });

  function clickNode(id: string) {
    if (files[id] !== undefined) select(id);
    else offerCreate(id);
  }
</script>

<section class="col inspector">
  <button class="rail-stub" title="Mostrar enlaces (])" aria-label="Mostrar panel de enlaces" onclick={() => toggleRail("right")}>
    <svg class="rs-ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M10 13a5 5 0 0 0 7 0l2-2a5 5 0 0 0-7-7l-1 1M14 11a5 5 0 0 0-7 0l-2 2a5 5 0 0 0 7 7l1-1"/></svg>
    <span class="rs-tx">enlaces</span>
  </button>

  <div class="col-head">
    <button class="rail-collapse" title="Ocultar panel (])" aria-label="Ocultar panel de enlaces" onclick={() => toggleRail("right")}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M9 6l6 6-6 6"/></svg>
    </button>
    <span>enlaces</span>
    <div class="spacer"></div>
    <span class="hcount">{bl && bl.count ? bl.count : ""}</span>
  </div>

  <div class="insp-body">
    {#if mini}
      <div class="insp-map">
        <button class="insp-expand" title="Mapa del espacio (M)" onclick={() => { $graphScope = "neighbor"; $graphBigOpen = true; }}>ampliar ↗</button>
        <svg class="graph mini" id="graphMini" viewBox="0 0 {W} {H}">
          {#each mini.nodes as nb}
            <line class="edge hot" x1={ccx} y1={ccy} x2={nb.x.toFixed(1)} y2={nb.y.toFixed(1)} />
          {/each}
          {#each mini.nodes as nb}
            <g class="node" class:ghost={nb.ghost} data-id={nb.id} onclick={() => clickNode(nb.id)} role="button" tabindex="0">
              <title>{conceptId(nb.id)}</title>
              {#if nb.ghost}
                <circle class="core" cx={nb.x.toFixed(1)} cy={nb.y.toFixed(1)} r="4.5"></circle>
              {:else}
                <circle class="halo" cx={nb.x.toFixed(1)} cy={nb.y.toFixed(1)} r="9" fill={nb.col}></circle>
                <circle class="core" cx={nb.x.toFixed(1)} cy={nb.y.toFixed(1)} r="4" fill={nb.col}></circle>
              {/if}
              <circle class="hit" cx={nb.x.toFixed(1)} cy={nb.y.toFixed(1)} r="13"></circle>
              <text class="nlabel" text-anchor={nb.anchorEnd ? "end" : "start"} x={nb.lx.toFixed(1)} y={nb.ly.toFixed(1)}>{nb.label}</text>
            </g>
          {/each}
          <g class="node sel">
            <title>{$current ? conceptId($current) : ""}</title>
            <circle class="halo" cx={ccx} cy={ccy} r="15" fill="var(--accent)"></circle>
            <circle class="core" cx={ccx} cy={ccy} r="5.5" fill="var(--accent)"></circle>
            <circle class="ring" cx={ccx} cy={ccy} r="9.5" fill="none" stroke="var(--accent)"></circle>
          </g>
          {#if mini.n === 0}
            <text x={ccx} y={ccy + 28} text-anchor="middle" class="nlabel" style="opacity:.45">sin conexiones</text>
          {/if}
        </svg>
      </div>
    {/if}

    <div class="insp-pane on" id="paneLinks">
      {#if !$current || isReserved($current) || !bl}
        <div class="empty">selecciona una página</div>
      {:else}
        <div class="bl-h">citado por <span class="n">{bl.inn.length}</span></div>
        {#if bl.inn.length === 0}<div class="empty">nadie lo enlaza</div>{/if}
        {#each bl.inn as p}
          <button class="bl-item" onclick={() => select(p)}><span class="arrow">←</span><span>{dt(p)}</span></button>
        {/each}

        <div class="bl-h">enlaza a <span class="n">{bl.outReal.length}</span></div>
        {#if bl.outReal.length === 0}<div class="empty">sin enlaces salientes</div>{/if}
        {#each bl.outReal as p}
          <button class="bl-item" onclick={() => select(p)}><span class="arrow">→</span><span>{dt(p)}</span></button>
        {/each}

        {#if bl.outDang.length}
          <div class="bl-h">por escribir <span class="n">{bl.outDang.length}</span></div>
          {#each bl.outDang as t}
            <button class="bl-item ghost" onclick={() => offerCreate(t)}><span class="arrow">+</span><span>{dt(t)}</span><span class="bl-create">crear</span></button>
          {/each}
        {/if}
      {/if}
    </div>
  </div>
</section>
