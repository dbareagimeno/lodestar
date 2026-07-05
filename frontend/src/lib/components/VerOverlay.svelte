<script lang="ts">
  // Overlay de historial (versiones). Port de <div class="ver-overlay">: línea de tiempo (propuestas
  // en revisión + versiones) a la izquierda, diff estructurado a la derecha. En dev usa el motor de
  // versiones portado del prototipo.
  import { snapshot, current } from "../stores/bundle";
  import { verOverlayOpen, openDialog, confirmDlg, toast, select } from "../stores/ui";
  import {
    versions, tipVersion, tipSnapshot, diffSnap, diffChips, statusLabel, pageTitleRaw,
    type Version, type SnapDiff, type FileDiff, type DiffLine,
  } from "../versions";
  import { parseFile, basename, RESERVED, titleFromPath } from "../okf";

  const files = $derived($snapshot?.files ?? {});

  let selId = $state<string>("__work");
  let filterPath = $state<string | null>(null);

  // Al abrir, apunta a "sin guardar" si hay cambios, si no a la última versión.
  $effect(() => {
    if ($verOverlayOpen) {
      const w = diffSnap(tipSnapshot(), files);
      selId = w.files.length || w.gen.length ? "__work" : (tipVersion()?.id ?? "__work");
    }
  });

  function dt(p: string): string {
    return files[p] !== undefined ? (parseFile(files, p).fm?.title as string) || titleFromPath(p) : titleFromPath(p);
  }
  function relTime(t: number): string {
    const s = (Date.now() - t) / 1000;
    if (s < 60) return "ahora mismo";
    const m = s / 60;
    if (m < 60) return "hace " + Math.floor(m) + " min";
    const h = m / 60;
    if (h < 24) return "hace " + Math.floor(h) + " h";
    const d = h / 24;
    if (d < 30) return "hace " + Math.floor(d) + " d";
    return new Date(t).toLocaleDateString("es", { day: "numeric", month: "short" });
  }
  function absTime(t: number): string {
    return new Date(t).toLocaleString("es", { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });
  }
  function confClass(v: Version): string {
    return v.conform ? (v.warns > 0 ? "warn" : "ok") : "err";
  }

  const props = $derived.by(() => {
    if (filterPath) return [];
    return Object.keys(files)
      .filter((p) => !RESERVED.has(basename(p)) && String((parseFile(files, p).fm?.status as string) || "").toLowerCase() === "review")
      .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
  });

  interface Row { id: string; work?: boolean; msg: string; sub: string; hash?: string; chips: { cls: string; t: string }[]; conf: string; muted?: boolean }
  const rows = $derived.by<Row[]>(() => {
    void $versions;
    const out: Row[] = [];
    const w = diffSnap(tipSnapshot(), files);
    const changed = w.files.length > 0 || w.gen.length > 0;
    out.push({
      id: "__work", work: true, muted: !changed,
      msg: changed ? (w.files.length > 0 ? `${w.files.length} cambio${w.files.length > 1 ? "s" : ""} sin guardar` : "Índices por regenerar") : "Al día",
      sub: changed ? "ahora · sin guardar" : "sin cambios sin guardar",
      chips: diffChips(w), conf: "",
    });
    const vs = $versions;
    for (let i = vs.length - 1; i >= 0; i--) {
      const v = vs[i];
      if (filterPath) {
        const a = vs[i - 1] ? vs[i - 1].snapshot : {};
        if ((a[filterPath] || "") === (v.snapshot[filterPath] || "")) continue;
      }
      const a = vs[i - 1] ? vs[i - 1].snapshot : {};
      out.push({ id: v.id, msg: v.msg, sub: `${v.author} · ${relTime(v.time)}`, hash: v.id, chips: diffChips(diffSnap(a, v.snapshot)), conf: confClass(v) });
    }
    return out;
  });

  // Diff del panel derecho.
  interface Panel { title: string; sub: string; badge: { cls: string; label: string }; work: boolean; id?: string; a: Record<string, string>; b: Record<string, string> }
  const panel = $derived.by<Panel | null>(() => {
    void $versions;
    if (selId === "__work") {
      const tip = tipVersion();
      const conf = confOfFiles(files);
      return { title: "Cambios sin guardar", sub: "respecto a " + (tip ? tip.id : "el inicio") + " · ahora", badge: badge(conf.errs, conf.warns), work: true, a: tipSnapshot(), b: files };
    }
    const vs = $versions;
    const i = vs.findIndex((v) => v.id === selId);
    if (i < 0) return null;
    const v = vs[i], a = vs[i - 1] ? vs[i - 1].snapshot : {};
    return { title: v.msg, sub: v.author + " · " + absTime(v.time) + " · " + v.id, badge: badge(v.errs, v.warns), work: false, id: v.id, a, b: v.snapshot };
  });
  const panelDiff = $derived.by<SnapDiff | null>(() => {
    if (!panel) return null;
    const d = diffSnap(panel.a, panel.b);
    if (filterPath) return { files: d.files.filter((f) => f.path === filterPath), gen: d.gen.filter((g) => g.path === filterPath), stats: d.stats, statusChanges: d.statusChanges.filter((s) => s.path === filterPath) };
    return d;
  });

  function badge(errs: number, warns: number): { cls: string; label: string } {
    const cls = errs > 0 ? "err" : warns > 0 ? "warn" : "ok";
    const label = errs > 0 ? `${errs} por corregir` : warns > 0 ? `${warns} aviso${warns > 1 ? "s" : ""}` : "conforme";
    return { cls, label };
  }
  // Conformidad de una instantánea (dev). Fuera de dev, se asume conforme (no hay motor local).
  let confFn: ((f: Record<string, string>) => { errs: number; warns: number }) | null = null;
  $effect(() => {
    const w = window as unknown as { __TAURI__?: unknown };
    if (!w.__TAURI__ && import.meta.env?.DEV && !confFn) import("../ipc/mock").then((m) => (confFn = m.confOfMap));
  });
  function confOfFiles(f: Record<string, string>): { errs: number; warns: number } {
    return confFn ? confFn(f) : { errs: 0, warns: 0 };
  }

  function restore(id: string) {
    const v = $versions.find((x) => x.id === id);
    if (!v) return;
    confirmDlg("Restaurar versión", `Tu espacio volverá al estado de “${v.msg}” (${v.id}). Quedará como un cambio sin guardar.`, () => {
      toast("Restaurar: usa la CLI / el binario de escritorio.");
    });
  }

  const KEY: Record<string, string> = { type: "tipo", title: "título", description: "descripción", resource: "recurso", tags: "etiquetas", timestamp: "actualizado", status: "estado" };
</script>

<div class="ver-overlay" class:on={$verOverlayOpen}>
  <div class="vo-bar">
    <span class="vo-title"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 3v5h5"/><path d="M3.05 13A9 9 0 1 0 6 5.3L3 8"/><path d="M12 7v5l3 1.5"/></svg>historial · <span class="ln">principal</span></span>
    {#if filterPath}
      <span class="vo-filter">sólo <b>{dt(filterPath)}</b><button title="Quitar filtro" onclick={() => (filterPath = null)}>×</button></span>
    {/if}
    {#if $versions.some((v) => v.conform)}
      <button class="vp-btn ghost" style="height:30px;padding:0 12px;width:auto" onclick={() => toast("Restaurar última conforme: usa la CLI (lodestar last-conforming).")}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7v6h6"/><path d="M3.5 13a9 9 0 1 0 2.5-7.7L3 8"/></svg>Restaurar última conforme
      </button>
    {/if}
    <div class="spacer" style="flex:1"></div>
    <div class="vo-legend">
      <span><i style="background:var(--ok)"></i>conforme</span>
      <span><i style="background:var(--err)"></i>no conforme</span>
      <span><i style="background:transparent;border:2px dashed var(--accent)"></i>sin guardar</span>
    </div>
    <button class="tbtn icon-only" title="Cerrar (Esc)" onclick={() => ($verOverlayOpen = false)}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M19 9h-4V5M15 9l5-5M5 15h4v4M9 15l-5 5"/></svg>
    </button>
  </div>

  <div class="vo-body">
    <div class="vo-timeline">
      {#if props.length}
        <div class="vo-sec">Propuestas en revisión · {props.length}</div>
        <div class="propose">
          {#each props as p}
            <button class="prop-item" onclick={() => { $verOverlayOpen = false; select(p); }}><span class="st-dot"></span><span>{dt(p)}</span><span class="pi-tag">en revisión</span></button>
          {/each}
        </div>
      {/if}
      <div class="vo-sec">Línea principal{filterPath ? " · historial de la página" : ""}</div>
      {#each rows as r}
        <button class="vrow" class:work={r.work} class:on={r.id === selId} onclick={() => (selId = r.id)}>
          <div class="vlane"><span class="lane-dot {r.work ? '' : r.conf || 'ok'}"></span></div>
          <div class="vcard" style:opacity={r.muted ? "0.66" : undefined}>
            <div class="vc-top"><span class="vc-msg">{r.msg}</span>{#if r.hash}<span class="vc-hash">{r.hash}</span>{/if}</div>
            {#if r.sub}<div class="vc-sub">{r.sub}</div>{/if}
            {#if r.chips.length}<div class="vc-chips">{#each r.chips as c}<span class="vchip {c.cls}">{c.t}</span>{/each}</div>{/if}
          </div>
        </button>
      {/each}
    </div>

    <div class="vo-diff">
      {#if panel && panelDiff}
        <div class="vd-meta">
          <div style="min-width:0;flex:1">
            <h3 class="vd-h">{panel.title}</h3>
            <div class="vd-sub"><span class="vd-badge {panel.badge.cls}"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" style="width:13px;height:13px">{@html panel.badge.cls === "ok" ? '<path d="M20 6L9 17l-5-5"/>' : panel.badge.cls === "warn" ? '<path d="M12 3l9 16H3z"/><path d="M12 10v4M12 17h.01"/>' : '<circle cx="12" cy="12" r="9"/><path d="M12 8v5M12 16h.01"/>'}</svg>{panel.badge.label}</span><span>{panel.sub}</span></div>
          </div>
          <div class="vd-acts">
            {#if panel.work}
              <button class="vp-btn" style="height:31px;padding:0 13px;width:auto" onclick={() => openDialog("commit")}><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"/></svg>Guardar versión</button>
            {:else}
              <button class="btn-ghost" onclick={() => (selId = "__work")}>Comparar con ahora</button>
              <button class="btn-primary" onclick={() => restore(panel!.id!)}>Restaurar</button>
            {/if}
          </div>
        </div>

        {#if panelDiff.files.length === 0 && panelDiff.gen.length === 0}
          <div class="vd-empty"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M20 6L9 17l-5-5"/></svg>Sin diferencias en esta selección.</div>
        {:else}
          {#if panelDiff.statusChanges.length}
            <div class="vd-statusline">
              {#each panelDiff.statusChanges as s}
                {#if s.from || s.to}
                  <div class="vd-lc">
                    <span style="color:var(--faint);font-size:11px;text-transform:uppercase;letter-spacing:.06em;font-weight:600">Ciclo de vida</span>
                    {#if s.from}<span class="stp"><span class="st-dot st-{String(s.from).toLowerCase()}"></span>{statusLabel(s.from)}</span>{:else}<span class="stp" style="color:var(--faint);font-weight:600">página nueva</span>{/if}
                    <span class="arrow">→</span>
                    {#if s.to}<span class="stp"><span class="st-dot st-{String(s.to).toLowerCase()}"></span>{statusLabel(s.to)}</span>{:else}<span class="stp" style="color:var(--faint);font-weight:600">página nueva</span>{/if}
                    <span style="margin-left:auto;color:var(--muted)">{dt(s.path)}</span>
                  </div>
                {/if}
              {/each}
            </div>
          {/if}
          {#each panelDiff.files as f}
            {@render diffCard(f)}
          {/each}
          {#if panelDiff.gen.length}
            <div class="vd-gen"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 7V4h16v3M9 20h6M12 4v16"/></svg>{panelDiff.gen.length} fichero{panelDiff.gen.length > 1 ? "s" : ""} generado{panelDiff.gen.length > 1 ? "s" : ""} (índices y registro) — se regeneran solos, no son ediciones manuales</div>
          {/if}
        {/if}
      {/if}
    </div>
  </div>
</div>

{#snippet diffCard(f: FileDiff)}
  <div class="vd-card">
    <div class="vd-card-h"><span class="nm">{pageTitleRaw(f)}</span><span class="pth">{f.path}</span><span class="vd-kind {f.kind}">{f.kind === "add" ? "nueva" : f.kind === "mod" ? "modificada" : "eliminada"}</span></div>
    {#if f.fm.length}
      <div class="vd-sect"><div class="vd-sect-h">Metadatos</div>
        {#each f.fm as c}
          {@const k = KEY[c.key] || c.key}
          {@const fmt = (v: string | null) => (c.key === "status" ? statusLabel(v) : v)}
          <div class="vd-field"><span class="fk">{k}</span>
            {#if c.from == null}<span class="vd-new">{fmt(c.to)}</span>
            {:else if c.to == null}<span class="vd-old">{fmt(c.from)}</span>
            {:else}<span class="vd-old">{fmt(c.from)}</span><span class="vd-arrow">→</span><span class="vd-new">{fmt(c.to)}</span>{/if}
          </div>
        {/each}
      </div>
    {/if}
    {#if f.linksAdd.length || f.linksRem.length}
      <div class="vd-sect"><div class="vd-sect-h">Enlaces · impacto en el grafo</div>
        {#each f.linksAdd as t}<div class="vd-link add"><span class="pm">+</span><span>enlaza a <b>{dt(t)}</b></span></div>{/each}
        {#each f.linksRem as t}<div class="vd-link rem"><span class="pm">−</span><span>ya no enlaza a <b>{dt(t)}</b></span></div>{/each}
      </div>
    {/if}
    {#if f.body.some((r) => r.t === "+" || r.t === "-")}
      <div class="vd-sect"><div class="vd-sect-h">Cuerpo</div>
        <div class="vd-body">
          {#each f.body as r}
            {#if r.t === "gap"}<div class="vd-ln gap">⋯ {r.n} línea{r.n! > 1 ? "s" : ""} sin cambios</div>
            {:else}<div class="vd-ln {r.t === '+' ? 'add' : r.t === '-' ? 'rem' : ''}"><span class="g">{r.t === "+" ? "+" : r.t === "-" ? "−" : ""}</span><span>{r.s || " "}</span></div>{/if}
          {/each}
        </div>
      </div>
    {/if}
  </div>
{/snippet}
