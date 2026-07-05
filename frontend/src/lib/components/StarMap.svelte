<script lang="ts">
  // Isla imperativa del grafo (ARCHITECTURE.md §8): posee el <svg> y el loop rAF. Port de
  // buildGraphModel/simStep/paintInto/startDrag del prototipo (mismo aspecto: halos, anillos, tintes
  // estelares, aristas hot/dim, etiquetas). Svelte sólo le pasa datos por props; nunca {#each}.
  import { onMount, onDestroy } from "svelte";
  import { conceptId, starTint, basename, RESERVED } from "../okf";
  import { matchFileQuery, tokenizeQuery } from "../query";
  import { select, offerCreate } from "../stores/ui";
  import type { GraphModel, Analysis } from "../ipc/types";

  interface Props {
    graph: GraphModel;
    analysis: Analysis | null;
    files: Record<string, string>;
    current: string | null;
    query: string;
    scope: "bundle" | "neighbor";
    active: boolean; // sólo simula cuando está visible
  }
  let { graph, analysis, files, current, query, scope, active }: Props = $props();

  let svgEl: SVGSVGElement;
  interface N { id: string; ghost: boolean; type: string | null; x: number; y: number; vx: number; vy: number }
  let nodes: N[] = [];
  let links: { s: string; t: string }[] = [];
  let raf = 0;
  let alpha = 0;
  let held: N | null = null;
  let dragging = false;
  let tf = { cx: 0, cy: 0, s: 1, W: 320, H: 300 };
  const pos: Record<string, { x: number; y: number }> = {};

  // Semilla determinista (sin Math.random para reproducibilidad visual).
  function seedXY(id: string): { x: number; y: number } {
    let h = 0;
    for (let i = 0; i < id.length; i++) h = (h * 131 + id.charCodeAt(i)) >>> 0;
    const a = (h % 360) * (Math.PI / 180);
    const r = 40 + (h % 120);
    return { x: Math.cos(a) * r, y: Math.sin(a) * r };
  }

  function build() {
    const neighbor = scope === "neighbor" && current && files[current] !== undefined && !RESERVED.has(basename(current));
    let ids: Set<string>;
    if (neighbor) {
      ids = new Set([current!]);
      graph.edges.forEach((e) => {
        if (e.source === current) ids.add(e.target);
        if (e.target === current) ids.add(e.source);
      });
    } else {
      ids = new Set(graph.nodes.map((n) => n.id));
    }
    const byId = new Map(graph.nodes.map((n) => [n.id, n]));
    nodes = [...ids].map((id) => {
      const g = byId.get(id);
      const prev = pos[id] ?? seedXY(id);
      return { id, ghost: g?.ghost ?? true, type: g?.type ?? null, x: prev.x, y: prev.y, vx: 0, vy: 0 };
    });
    links = graph.edges
      .filter((e) => ids.has(e.source) && ids.has(e.target))
      .map((e) => ({ s: e.source, t: e.target }));
    nodes.forEach((n) => (pos[n.id] = { x: n.x, y: n.y }));
  }

  function step() {
    const N = nodes, k = alpha;
    for (let i = 0; i < N.length; i++)
      for (let j = i + 1; j < N.length; j++) {
        let dx = N[i].x - N[j].x, dy = N[i].y - N[j].y;
        const d2 = dx * dx + dy * dy + 0.01;
        const f = 900 / d2;
        const dist = Math.sqrt(d2), fx = (dx / dist) * f, fy = (dy / dist) * f;
        N[i].vx += fx * k; N[i].vy += fy * k; N[j].vx -= fx * k; N[j].vy -= fy * k;
      }
    const map = Object.fromEntries(N.map((n) => [n.id, n]));
    links.forEach((l) => {
      const a = map[l.s], b = map[l.t]; if (!a || !b) return;
      const dx = b.x - a.x, dy = b.y - a.y, d = Math.sqrt(dx * dx + dy * dy) + 0.01;
      const f = (d - 70) * 0.02, fx = (dx / d) * f, fy = (dy / d) * f;
      a.vx += fx * k; a.vy += fy * k; b.vx -= fx * k; b.vy -= fy * k;
    });
    N.forEach((n) => {
      if (n === held) return;
      n.vx += -n.x * 0.002 * k; n.vy += -n.y * 0.002 * k;
      n.vx *= 0.85; n.vy *= 0.85;
      n.x += n.vx; n.y += n.vy;
    });
  }

  function esc(s: string): string {
    return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
  }

  function paint() {
    const svg = svgEl;
    if (!svg) return;
    const W = svg.clientWidth || 320, H = svg.clientHeight || 300;
    let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
    nodes.forEach((n) => { minX = Math.min(minX, n.x); maxX = Math.max(maxX, n.x); minY = Math.min(minY, n.y); maxY = Math.max(maxY, n.y); });
    if (!isFinite(minX)) { minX = -100; maxX = 100; minY = -100; maxY = 100; }
    const pad = 38, sx = (W - pad * 2) / Math.max(1, maxX - minX), sy = (H - pad * 2) / Math.max(1, maxY - minY);
    const s = Math.min(sx, sy, 2.4), cx = (minX + maxX) / 2, cy = (minY + maxY) / 2;
    tf = { cx, cy, s, W, H };
    const px = (n: N) => (n.x - cx) * s + W / 2;
    const py = (n: N) => (n.y - cy) * s + H / 2;
    if (nodes.length === 0) { svg.innerHTML = `<text x="${W / 2}" y="${H / 2}" text-anchor="middle" class="nlabel">sin concepts</text>`; return; }
    const tokens = tokenizeQuery(query.trim());
    const hasQ = tokens.length > 0;
    const matched = hasQ && analysis ? new Set(nodes.filter((n) => matchFileQuery(n.id, tokens, files, analysis)).map((n) => n.id)) : null;
    const hit = (id: string) => !hasQ || matched!.has(id);
    const parts: string[] = [];
    links.forEach((l) => {
      const a = nodes.find((n) => n.id === l.s), b = nodes.find((n) => n.id === l.t); if (!a || !b) return;
      const hot = current && (l.s === current || l.t === current);
      const dim = hasQ && !(hit(l.s) && hit(l.t));
      parts.push(`<line class="edge${hot ? " hot" : ""}${dim ? " dim" : ""}" x1="${px(a)}" y1="${py(a)}" x2="${px(b)}" y2="${py(b)}"/>`);
    });
    nodes.forEach((n) => {
      const X = px(n), Y = py(n);
      const isCur = n.id === current;
      const dim = hasQ && !hit(n.id) ? " dim" : "";
      const label = conceptId(n.id).split("/").pop()!;
      const flip = X > W - 64;
      const anc = flip ? ' text-anchor="end"' : "";
      if (n.ghost) {
        parts.push(`<g class="node ghost${isCur ? " sel" : ""}${dim}" data-id="${esc(n.id)}"><circle class="core" cx="${X}" cy="${Y}" r="5"></circle><circle class="hit" cx="${X}" cy="${Y}" r="11"></circle><text class="nlabel"${anc} x="${X + (flip ? -10 : 10)}" y="${Y + 3}">${esc(label)}</text></g>`);
      } else {
        const col = isCur ? "var(--accent)" : starTint(n.type);
        const rCore = isCur ? 6 : 4.2, rHalo = isCur ? 17 : 10.5, off = isCur ? 12 : 7;
        parts.push(`<g class="node${isCur ? " sel" : ""}${dim}" data-id="${esc(n.id)}"><circle class="halo" cx="${X}" cy="${Y}" r="${rHalo}" fill="${col}"></circle><circle class="core" cx="${X}" cy="${Y}" r="${rCore}" fill="${col}"></circle>${isCur ? `<circle class="ring" cx="${X}" cy="${Y}" r="9" fill="none" stroke="var(--accent)"></circle>` : ""}<circle class="hit" cx="${X}" cy="${Y}" r="${Math.max(11, rCore + 8)}"></circle><text class="nlabel"${anc} x="${X + (flip ? -off : off)}" y="${Y + 3}">${esc(label)}</text></g>`);
      }
    });
    svg.innerHTML = parts.join("");
    svg.querySelectorAll<SVGGElement>("g.node").forEach((g) => {
      const id = g.getAttribute("data-id")!;
      const hitc = g.querySelector(".hit");
      hitc?.addEventListener("pointerdown", (e) => startDrag(e as PointerEvent, id));
      g.addEventListener("click", () => {
        const n = nodes.find((x) => x.id === id);
        if (n && !n.ghost) select(id);
        else if (n) offerCreate(id);
      });
    });
  }

  function startSim() {
    cancelAnimationFrame(raf);
    alpha = 1;
    paint();
    const tick = () => {
      step();
      nodes.forEach((n) => (pos[n.id] = { x: n.x, y: n.y }));
      paint();
      alpha *= 0.96;
      if (alpha > 0.02 || dragging) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
  }

  function startDrag(e: PointerEvent, id: string) {
    e.preventDefault();
    const n = nodes.find((x) => x.id === id); if (!n) return;
    held = n; dragging = true; n.vx = 0; n.vy = 0;
    const rect = svgEl.getBoundingClientRect();
    const move = (ev: PointerEvent) => {
      const mx = ev.clientX - rect.left, my = ev.clientY - rect.top;
      n.x = (mx - tf.W / 2) / tf.s + tf.cx; n.y = (my - tf.H / 2) / tf.s + tf.cy;
      alpha = Math.max(alpha, 0.3);
    };
    const up = () => {
      dragging = false; held = null;
      window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up);
      alpha = Math.max(alpha, 0.2); startSim();
    };
    window.addEventListener("pointermove", move); window.addEventListener("pointerup", up);
    startSim();
  }

  export function relayout() {
    nodes.forEach((n) => { const p = seedXY(n.id + "~"); n.x = p.x; n.y = p.y; n.vx = 0; n.vy = 0; });
    startSim();
  }

  // Rebuild + resimula cuando cambian datos/props relevantes y está activo.
  $effect(() => {
    void graph; void scope; void current;
    if (!active) return;
    build();
    startSim();
  });
  // Repinta (sin resimular) al cambiar la query o el tema.
  $effect(() => {
    void query;
    if (active) paint();
  });

  onMount(() => {
    const ro = new ResizeObserver(() => paint());
    ro.observe(svgEl);
    return () => ro.disconnect();
  });
  onDestroy(() => cancelAnimationFrame(raf));
</script>

<svg bind:this={svgEl} class="graph" role="img" aria-label="Mapa de conceptos"></svg>
