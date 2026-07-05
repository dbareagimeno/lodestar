<script lang="ts">
  // Barra superior: marca + píldora de versiones + píldora de conformidad (con popover) + exportar /
  // tema / ayuda. Portado del <header> del prototipo (mismas clases/estructura).
  import { snapshot, current } from "../stores/bundle";
  import { toggleTheme, theme, openDialog, select, offerCreate, toast, isReserved, verOverlayOpen } from "../stores/ui";
  import { versions, pendingCount } from "../versions";
  import { dispTitle, titleFromPath, STATUS_LABEL } from "../okf";
  import type { Check, Severity } from "../ipc/types";

  let confOpen = $state(false);
  let verOpen = $state(false);

  const analysis = $derived($snapshot?.analysis ?? null);
  const files = $derived($snapshot?.files ?? {});

  // Conformidad global (misma cuenta que el prototipo: hardFail + nº de warns).
  const warnN = $derived(
    analysis ? Object.values(analysis.perFile).reduce((a, cs) => a + cs.filter((c) => c.level === "warn").length, 0) : 0,
  );
  const fail = $derived(analysis?.hardFail ?? 0);
  const confClass = $derived(fail > 0 ? "bad" : warnN > 0 ? "warn" : "ok");
  const confText = $derived(fail > 0 ? `${fail} por corregir` : warnN > 0 ? `${warnN} aviso${warnN > 1 ? "s" : ""}` : "Conforme");

  const pending = $derived($pendingCount);
  const verText = $derived(pending > 0 ? `${pending} sin guardar` : "Al día");

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
  const lastConf = $derived([...$versions].reverse().find((v) => v.conform));

  function dt(p: string): string {
    return files[p] !== undefined ? dispTitle(files, p) : titleFromPath(p);
  }

  // Modelo del popover de conformidad (port de renderConf).
  interface Row { level: Severity; code: string; msg: string; path?: string; onClick?: () => void }
  const curChecks = $derived.by<Row[]>(() => {
    const cur = $current;
    if (!analysis || !cur || !analysis.perFile[cur]) return [];
    return analysis.perFile[cur]
      .filter((c) => !(c.level === "info" && (c.code === "LINK-STUB" || c.code === "ORPHAN")))
      .map((c) => ({ level: c.level, code: c.code, msg: c.msg }));
  });
  const allRows = $derived.by<Row[]>(() => {
    if (!analysis) return [];
    const errs: Row[] = [];
    const warns: Row[] = [];
    Object.entries(analysis.perFile).forEach(([p, cs]) => {
      (cs as Check[]).forEach((c) => {
        if (c.level === "err") errs.push({ level: "err", code: c.code, msg: c.msg, path: p, onClick: () => select(p) });
        else if (c.level === "warn") warns.push({ level: "warn", code: c.code, msg: c.msg, path: p, onClick: () => select(p) });
      });
    });
    return [...errs, ...warns];
  });
  const pendingRows = $derived.by<Row[]>(() => {
    if (!analysis) return [];
    const rows: Row[] = [];
    analysis.dangling.forEach((t) => rows.push({ level: "info", code: "STUB", msg: "por escribir: " + dt(t), onClick: () => offerCreate(t) }));
    analysis.orphans.forEach((p) => rows.push({ level: "info", code: "ORPH", msg: "sin enlaces: " + dt(p), onClick: () => select(p) }));
    return rows;
  });

  function closePops() {
    confOpen = false;
    verOpen = false;
  }
  // Cerrar popovers al hacer clic fuera.
  $effect(() => {
    if (!confOpen && !verOpen) return;
    const h = (e: MouseEvent) => {
      const el = e.target as HTMLElement;
      if (!el.closest(".conf-wrap") && !el.closest(".ver-wrap")) closePops();
    };
    document.addEventListener("click", h);
    return () => document.removeEventListener("click", h);
  });

  function exportBundle() {
    toast("Exportar: usa el binario de escritorio / CLI (lodestar export).");
  }
</script>

<header>
  <div class="brand">
    <svg class="logo-star" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 1.5l2.1 7.9 7.9 2.1-7.9 2.1-2.1 7.9-2.1-7.9L2 11.5l7.9-2.1z"/></svg>
    <b>lodestar</b>
  </div>
  <div class="spacer"></div>

  <div class="ver-wrap">
    <button
      class="pill ver"
      class:quiet={pending === 0}
      class:pending={pending > 0}
      title="Versiones del espacio (V)"
      onclick={(e) => { e.stopPropagation(); const w = !verOpen; closePops(); verOpen = w; }}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 3v5h5"/><path d="M3.05 13A9 9 0 1 0 6 5.3L3 8"/><path d="M12 7v5l3 1.5"/></svg>
      <span class="vtx">{verText}</span>
    </button>
    <div class="ver-pop" class:on={verOpen}>
      <div class="vp-state {pending > 0 ? 'pending' : 'clean'}">
        {#if pending > 0}
          <svg class="vs-ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 8v5M12 16h.01"/><circle cx="12" cy="12" r="9"/></svg>
          <span>{pending} página{pending > 1 ? "s" : ""} con cambios sin guardar</span>
        {:else}
          <svg class="vs-ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M20 6L9 17l-5-5"/></svg>
          <span>Todo guardado · el espacio está al día</span>
        {/if}
      </div>
      <div class="vp-actions">
        <button class="vp-btn" disabled={pending === 0} onclick={() => { closePops(); openDialog("commit"); }}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"/></svg>Guardar versión
        </button>
        <button class="vp-btn ghost" onclick={() => { closePops(); $verOverlayOpen = true; }}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 3v5h5"/><path d="M3.05 13A9 9 0 1 0 6 5.3L3 8"/><path d="M12 7v5l3 1.5"/></svg>Ver historial completo
        </button>
      </div>
      <div class="vp-line">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="6" cy="6" r="2.4"/><circle cx="6" cy="18" r="2.4"/><path d="M6 8.4v7.2"/><circle cx="18" cy="6" r="2.4"/><path d="M18 8.4c0 4-3 5.6-6 5.6"/></svg>
        línea principal · {$versions.length} {$versions.length === 1 ? "versión" : "versiones"}
      </div>
      <div class="vp-h">Recientes</div>
      <div class="vp-list">
        {#each [...$versions].slice(-4).reverse() as v}
          <button class="vp-item" onclick={() => { closePops(); $verOverlayOpen = true; }}><span class="vp-dot {v.conform ? (v.warns > 0 ? 'warn' : 'ok') : 'err'}"></span><span class="vp-msg">{v.msg}</span><span class="vp-meta">{v.id} · {relTime(v.time)}</span></button>
        {/each}
      </div>
      {#if lastConf && pending > 0}
        <div class="vp-foot"><button onclick={() => { closePops(); toast("Restaurar última conforme: usa la CLI (lodestar last-conforming)."); }}>↺ Restaurar última versión conforme ({lastConf.id})</button></div>
      {/if}
    </div>
  </div>

  <div class="conf-wrap">
    <button
      class="pill {confClass === 'ok' ? 'ok' : confClass === 'bad' ? 'bad' : 'warn'}"
      title="Revisión del espacio"
      onclick={(e) => { e.stopPropagation(); const w = !confOpen; closePops(); confOpen = w; }}
    >
      <span class="dot"></span><span>{confText}</span>
    </button>
    <div class="conf-pop" class:on={confOpen}>
      <div class="conf-banner {confClass}">
        {#if fail > 0}
          <svg class="big" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="9"/><path d="M12 8v5M12 16h.01"/></svg>
          {fail} página{fail > 1 ? "s" : ""} por corregir
        {:else if warnN > 0}
          <svg class="big" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 3l9 16H3z"/><path d="M12 10v4M12 17h.01"/></svg>
          {warnN} aviso{warnN > 1 ? "s" : ""} — nada que bloquee
        {:else}
          <svg class="big" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M20 6L9 17l-5-5"/></svg>
          Todo en orden
        {/if}
      </div>

      {#if $current && analysis?.perFile[$current] && !isReserved($current)}
        <div class="scope-h">Esta página</div>
        {#if curChecks.length === 0}
          <div class="empty">sin problemas en esta página</div>
        {/if}
        {#each curChecks as c}
          {@render checkRow(c)}
        {/each}
      {/if}

      <div class="scope-h">Todo el espacio</div>
      {#if allRows.length === 0}
        {@render checkRow({ level: "pass", code: "ALL", msg: "Sin errores ni avisos." })}
      {/if}
      {#each allRows as c}
        {@render checkRow(c)}
      {/each}

      <div class="scope-h">Por escribir · en el espacio</div>
      {#if pendingRows.length === 0}
        <div class="empty">nada pendiente</div>
      {/if}
      {#each pendingRows as c}
        {@render checkRow(c)}
      {/each}
    </div>
  </div>

  <button class="tbtn icon-only" title="Exportar bundle (.zip)" onclick={exportBundle}>
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3"/></svg>
  </button>
  <button class="tbtn icon-only" title="Tema claro/oscuro" onclick={toggleTheme}>
    {#if $theme === "dark"}
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"/></svg>
    {:else}
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4 12H2M22 12h-2M5 5l1.5 1.5M17.5 17.5L19 19M19 5l-1.5 1.5M6.5 17.5L5 19"/></svg>
    {/if}
  </button>
  <button class="tbtn icon-only" title="Qué es OKF / atajos" onclick={() => openDialog("help")}>
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="9"/><path d="M9.5 9a2.5 2.5 0 1 1 3.5 2.3c-.8.4-1 .9-1 1.7M12 17h.01"/></svg>
  </button>
</header>

{#snippet checkRow(c: Row)}
  <div class="check {c.level}" class:clickable={!!c.onClick} title={c.code} onclick={c.onClick} role={c.onClick ? "button" : undefined} tabindex={c.onClick ? 0 : undefined}>
    <svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      {#if c.level === "err"}<path d="M12 8v5M12 16h.01"/><circle cx="12" cy="12" r="9"/>
      {:else if c.level === "warn"}<path d="M12 3l9 16H3z"/><path d="M12 10v4M12 17h.01"/>
      {:else if c.level === "info"}<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>
      {:else}<path d="M20 6L9 17l-5-5"/>{/if}
    </svg>
    <div class="tx">{#if c.path}<span style="color:var(--accent);font-weight:500">{dt(c.path)}</span> · {/if}{c.msg}</div>
  </div>
{/snippet}
