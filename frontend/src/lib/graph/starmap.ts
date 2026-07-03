// Isla imperativa del grafo (ARCHITECTURE.md §8): posee el <svg> y el loop rAF. Svelte le pasa
// nodos/aristas por `setData` (nunca con {#each} reactivo). Port de `createStarMap`/`simStep`/
// `paintGraph` del prototipo: repulsión 900/d², muelle (d-70)*0.02, gravedad 0.002, damping 0.85,
// enfriado α*=0.96.

export interface StarNode {
  id: string;
  ghost: boolean;
  type: string | null;
}
export interface StarEdge {
  source: string;
  target: string;
}

interface Sim {
  id: string;
  ghost: boolean;
  type: string | null;
  x: number;
  y: number;
  vx: number;
  vy: number;
}

interface Opts {
  onSelect: (id: string) => void;
  onCreateGhost: (id: string) => void;
}

const ESC = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

// Tinte por tipo (estelar). Determinista por hash del tipo.
function starTint(type: string | null): string {
  if (!type) return "var(--star)";
  let h = 0;
  for (let i = 0; i < type.length; i++) h = (h * 31 + type.charCodeAt(i)) & 0xffff;
  const hue = 38 + (h % 60) - 30; // alrededor del oro
  return `hsl(${hue} 70% 62%)`;
}

export function createStarMap(svg: SVGSVGElement, opts: Opts) {
  let nodes: Sim[] = [];
  let links: StarEdge[] = [];
  let current: string | null = null;
  let matched: Set<string> | null = null;
  const pos = new Map<string, { x: number; y: number }>();
  let raf = 0;
  let alpha = 0;
  let held: Sim | null = null;
  let dragging = false;
  let tf = { cx: 0, cy: 0, s: 1, W: 0, H: 0 };
  let destroyed = false;
  let seed = 1;
  const rnd = () => {
    // PRNG determinista (evita saltos entre renders sin depender de Math.random global).
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    return (seed / 0x7fffffff) * 300 - 150;
  };

  function setData(n: StarNode[], e: StarEdge[], cur: string | null) {
    current = cur;
    const ids = new Set(n.map((x) => x.id));
    nodes = n.map((x) => {
      const prev = pos.get(x.id);
      return {
        id: x.id,
        ghost: x.ghost,
        type: x.type,
        x: prev ? prev.x : rnd(),
        y: prev ? prev.y : rnd(),
        vx: 0,
        vy: 0,
      };
    });
    links = e.filter((l) => ids.has(l.source) && ids.has(l.target));
    start();
  }

  function setQuery(m: Set<string> | null) {
    matched = m;
    paint();
  }

  function start() {
    cancelAnimationFrame(raf);
    alpha = 1;
    paint();
    const tick = () => {
      if (destroyed) return;
      step();
      nodes.forEach((nn) => pos.set(nn.id, { x: nn.x, y: nn.y }));
      paint();
      alpha *= 0.96;
      if (alpha > 0.02 || dragging) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
  }

  function step() {
    const N = nodes;
    const k = alpha;
    for (let i = 0; i < N.length; i++) {
      for (let j = i + 1; j < N.length; j++) {
        const dx = N[i].x - N[j].x,
          dy = N[i].y - N[j].y;
        const d2 = dx * dx + dy * dy + 0.01;
        const f = 900 / d2;
        const dist = Math.sqrt(d2);
        const fx = (dx / dist) * f,
          fy = (dy / dist) * f;
        N[i].vx += fx * k;
        N[i].vy += fy * k;
        N[j].vx -= fx * k;
        N[j].vy -= fy * k;
      }
    }
    const map: Record<string, Sim> = {};
    N.forEach((n) => (map[n.id] = n));
    links.forEach((l) => {
      const a = map[l.source],
        b = map[l.target];
      if (!a || !b) return;
      const dx = b.x - a.x,
        dy = b.y - a.y;
      const d = Math.sqrt(dx * dx + dy * dy) + 0.01;
      const f = (d - 70) * 0.02;
      const fx = (dx / d) * f,
        fy = (dy / d) * f;
      a.vx += fx * k;
      a.vy += fy * k;
      b.vx -= fx * k;
      b.vy -= fy * k;
    });
    N.forEach((n) => {
      if (n === held) {
        // El nodo agarrado no integra NI acumula: sin esto, la repulsión/muelle de arriba
        // seguía sumando a vx/vy durante todo el drag y al soltar salía disparado.
        n.vx = 0;
        n.vy = 0;
        return;
      }
      n.vx += -n.x * 0.002 * k;
      n.vy += -n.y * 0.002 * k;
      n.vx *= 0.85;
      n.vy *= 0.85;
      n.x += n.vx;
      n.y += n.vy;
    });
  }

  function paint() {
    const W = svg.clientWidth || 320,
      H = svg.clientHeight || 300;
    let minX = Infinity,
      maxX = -Infinity,
      minY = Infinity,
      maxY = -Infinity;
    nodes.forEach((n) => {
      minX = Math.min(minX, n.x);
      maxX = Math.max(maxX, n.x);
      minY = Math.min(minY, n.y);
      maxY = Math.max(maxY, n.y);
    });
    if (!isFinite(minX)) {
      minX = -100;
      maxX = 100;
      minY = -100;
      maxY = 100;
    }
    const pad = 38;
    const sx = (W - pad * 2) / Math.max(1, maxX - minX);
    const sy = (H - pad * 2) / Math.max(1, maxY - minY);
    const s = Math.min(sx, sy, 2.4);
    const cx = (minX + maxX) / 2,
      cy = (minY + maxY) / 2;
    tf = { cx, cy, s, W, H };
    const px = (n: Sim) => (n.x - cx) * s + W / 2;
    const py = (n: Sim) => (n.y - cy) * s + H / 2;
    if (nodes.length === 0) {
      svg.innerHTML = `<text x="${W / 2}" y="${H / 2}" text-anchor="middle" class="nlabel">sin concepts</text>`;
      return;
    }
    const hasQ = matched !== null;
    const hit = (id: string) => !hasQ || matched!.has(id);
    const parts: string[] = [];
    const byId: Record<string, Sim> = {};
    nodes.forEach((n) => (byId[n.id] = n));
    links.forEach((l) => {
      const a = byId[l.source],
        b = byId[l.target];
      if (!a || !b) return;
      const hot = current && (l.source === current || l.target === current);
      const dim = hasQ && !(hit(l.source) && hit(l.target));
      parts.push(
        `<line class="edge${hot ? " hot" : ""}${dim ? " dim" : ""}" x1="${px(a)}" y1="${py(a)}" x2="${px(b)}" y2="${py(b)}"/>`,
      );
    });
    nodes.forEach((n) => {
      const X = px(n),
        Y = py(n);
      const isCur = n.id === current;
      const dim = hasQ && !hit(n.id) ? " dim" : "";
      const label = n.id.replace(/\.md$/, "").split("/").pop() ?? n.id;
      const flip = X > W - 64;
      const anc = flip ? ' text-anchor="end"' : "";
      if (n.ghost) {
        parts.push(
          `<g class="node ghost${isCur ? " sel" : ""}${dim}" data-id="${ESC(n.id)}">` +
            `<circle class="core" cx="${X}" cy="${Y}" r="5"></circle>` +
            `<circle class="hit" cx="${X}" cy="${Y}" r="11"></circle>` +
            `<text class="nlabel"${anc} x="${X + (flip ? -10 : 10)}" y="${Y + 3}">${ESC(label)}</text></g>`,
        );
      } else {
        const col = isCur ? "var(--accent)" : starTint(n.type);
        const rCore = isCur ? 6 : 4.2;
        const rHalo = isCur ? 17 : 10.5;
        const off = isCur ? 12 : 7;
        parts.push(
          `<g class="node${isCur ? " sel" : ""}${dim}" data-id="${ESC(n.id)}">` +
            `<circle class="halo" cx="${X}" cy="${Y}" r="${rHalo}" fill="${col}"></circle>` +
            `<circle class="core" cx="${X}" cy="${Y}" r="${rCore}" fill="${col}"></circle>` +
            (isCur ? `<circle class="ring" cx="${X}" cy="${Y}" r="9" fill="none" stroke="var(--accent)"></circle>` : "") +
            `<circle class="hit" cx="${X}" cy="${Y}" r="${Math.max(11, rCore + 8)}"></circle>` +
            `<text class="nlabel"${anc} x="${X + (flip ? -off : off)}" y="${Y + 3}">${ESC(label)}</text></g>`,
        );
      }
    });
    svg.innerHTML = parts.join("");
    // Solo pointerdown: el `click` del navegador NUNCA llegaba a los <g> — el repintado
    // síncrono de `start()` los saca del DOM entre pointerdown y pointerup, y el click se
    // despacha al ancestro común (el svg, sin handler). La selección se resuelve en el
    // pointerup de startDrag con umbral de movimiento (tap = seleccionar, drag = arrastrar).
    svg.querySelectorAll<SVGGElement>("g.node").forEach((g) => {
      const id = g.getAttribute("data-id")!;
      g.querySelector(".hit")?.addEventListener("pointerdown", (e) => startDrag(e as PointerEvent, id));
    });
  }

  function startDrag(e: PointerEvent, id: string) {
    e.preventDefault();
    const n = nodes.find((x) => x.id === id);
    if (!n) return;
    held = n;
    dragging = true;
    n.vx = 0;
    n.vy = 0;
    const startX = e.clientX,
      startY = e.clientY;
    let moved = false;
    const move = (ev: PointerEvent) => {
      if (Math.abs(ev.clientX - startX) + Math.abs(ev.clientY - startY) > 4) moved = true;
      if (!moved) return;
      // El rect se re-lee por evento: si el layout cambió a mitad del drag, unas coordenadas
      // capturadas al inicio desplazarían el nodo respecto al puntero.
      const rect = svg.getBoundingClientRect();
      const mx = ev.clientX - rect.left,
        my = ev.clientY - rect.top;
      n.x = (mx - tf.W / 2) / tf.s + tf.cx;
      n.y = (my - tf.H / 2) / tf.s + tf.cy;
      n.vx = 0;
      n.vy = 0;
      alpha = Math.max(alpha, 0.3);
    };
    const up = () => {
      dragging = false;
      held = null;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      if (!moved) {
        // Tap sin arrastre = selección (sustituye al listener de `click` roto).
        if (!n.ghost) opts.onSelect(id);
        else opts.onCreateGhost(id);
      }
      alpha = Math.max(alpha, 0.2);
      start();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    start();
  }

  // Repinta al redimensionar: con la simulación fría (alpha ≤ 0.02) nada volvía a pintar y el
  // grafo quedaba mal escalado hasta el siguiente push de datos.
  const resizeObserver = typeof ResizeObserver !== "undefined" ? new ResizeObserver(() => paint()) : null;
  resizeObserver?.observe(svg);

  function destroy() {
    destroyed = true;
    resizeObserver?.disconnect();
    cancelAnimationFrame(raf);
    svg.innerHTML = "";
  }

  return { setData, setQuery, destroy };
}
