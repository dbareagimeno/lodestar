<script lang="ts">
  // Panel izquierdo (explorer): navegador Lista/Mapa. Port del <section class="col explorer"> del
  // prototipo. La lista agrupa por directorio con puntos de estado + etiqueta de tipo; el mapa es la
  // isla StarMap. Datos derivados del snapshot; filtro con la query del prototipo.
  import { snapshot } from "../stores/bundle";
  import { treeQuery, current } from "../stores/bundle";
  import { explorerView, graphScope, toggleRail, closeExplorerMap, openExplorerMap, select, toast, graphBigOpen, openDialog } from "../stores/ui";
  import { parseFile, basename, RESERVED, titleFromPath } from "../okf";
  import { tokenizeQuery, matchFileQuery } from "../query";
  import StarMap from "./StarMap.svelte";

  const files = $derived($snapshot?.files ?? {});
  const analysis = $derived($snapshot?.analysis ?? null);
  const graph = $derived($snapshot?.graph ?? { nodes: [], edges: [] });

  let kebab = $state(false);

  function sortPaths(a: string, b: string): number {
    return a.localeCompare(b, undefined, { numeric: true });
  }

  interface DirRow { kind: "dir"; name: string; depth: number; key: string }
  interface FileRow { kind: "file"; path: string; depth: number; label: string; type: string | null; status: string | null; reserved: boolean; lvl: "" | "warn" | "err" }
  type Row = DirRow | FileRow;

  const rows = $derived.by<Row[]>(() => {
    const all = Object.keys(files);
    const tokens = tokenizeQuery($treeQuery.trim());
    const matched = tokens.length && analysis ? all.filter((p) => matchFileQuery(p, tokens, files, analysis)) : all;
    const sorted = matched.slice().sort(sortPaths);
    const seenDir = new Set<string>();
    const out: Row[] = [];
    sorted.forEach((p) => {
      const segs = p.split("/");
      for (let i = 0; i < segs.length - 1; i++) {
        const dpath = segs.slice(0, i + 1).join("/") + "/";
        if (!seenDir.has(dpath)) {
          seenDir.add(dpath);
          out.push({ kind: "dir", name: segs[i], depth: i, key: dpath });
        }
      }
      const bn = basename(p);
      const pf = parseFile(files, p);
      const checks = analysis?.perFile[p] ?? [];
      const lvl = checks.some((c) => c.level === "err") ? "err" : checks.some((c) => c.level === "warn") ? "warn" : "";
      out.push({
        kind: "file",
        path: p,
        depth: segs.length - 1,
        label: RESERVED.has(bn) ? bn : ((pf.fm?.title as string) || titleFromPath(p)),
        type: (pf.fm?.type as string) ?? null,
        status: pf.fm?.status ? String(pf.fm.status).toLowerCase() : null,
        reserved: RESERVED.has(bn),
        lvl,
      });
    });
    return out;
  });

  const matchCount = $derived.by(() => {
    const all = Object.keys(files);
    const tokens = tokenizeQuery($treeQuery.trim());
    return tokens.length && analysis ? all.filter((p) => matchFileQuery(p, tokens, files, analysis)).length : all.length;
  });

  function genIndex() {
    kebab = false;
    toast("Generar índice: usa la CLI (lodestar index).");
  }
  function genTags() {
    kebab = false;
    toast("Índices por etiqueta: usa la CLI (lodestar tags).");
  }
</script>

<section class="col explorer">
  <button class="rail-stub" title="Mostrar páginas ([)" aria-label="Mostrar panel de páginas" onclick={() => toggleRail("left")}>
    <svg class="rs-ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7h18M3 12h18M3 17h12"/></svg>
    <span class="rs-tx">espacio</span>
  </button>

  <div class="col-head">
    <div class="nav-seg" role="tablist" aria-label="Vista del espacio">
      <button class:on={$explorerView === "list"} role="tab" aria-selected={$explorerView === "list"} onclick={closeExplorerMap}>Lista</button>
      <button class:on={$explorerView === "map"} role="tab" aria-selected={$explorerView === "map"} onclick={openExplorerMap}>Mapa</button>
    </div>
    <div class="spacer"></div>
    <div class="kebab-wrap">
      <button class="ghostbtn icon-only kebab-toggle" title="Más acciones" onclick={(e) => { e.stopPropagation(); kebab = !kebab; }}>
        <svg viewBox="0 0 24 24" fill="currentColor" stroke="none"><circle cx="5" cy="12" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="19" cy="12" r="1.6"/></svg>
      </button>
      <div class="kebab-menu kebab-right" class:on={kebab}>
        <button class="km-item" onclick={genIndex}>Generar índice</button>
        <button class="km-item" onclick={genTags}>Índices por etiqueta</button>
      </div>
    </div>
    <button class="rail-collapse" title="Ocultar panel ([)" aria-label="Ocultar panel de páginas" onclick={() => toggleRail("left")}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M15 6l-6 6 6 6"/></svg>
    </button>
  </div>

  <button class="btn-new" title="Nueva página (N)" onclick={() => openDialog("new")}>
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M12 5v14M5 12h14"/></svg>
    Nueva página
  </button>

  <div class="search-row">
    <svg class="s-ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="7"/><path d="M21 21l-4.3-4.3"/></svg>
    <input class="s-input" placeholder="Buscar páginas…" spellcheck="false" autocomplete="off" bind:value={$treeQuery} />
    <button class="s-clear" class:on={!!$treeQuery} title="Limpiar (Esc)" onclick={() => ($treeQuery = "")}>×</button>
  </div>
  <div class="search-info" style:display={$treeQuery.trim() ? "block" : "none"}>{matchCount} coincidencia{matchCount === 1 ? "" : "s"}</div>

  <div class="tree" id="tree">
    {#if rows.length === 0}
      {#if $treeQuery.trim()}
        <div class="empty">sin coincidencias</div>
      {:else}
        <div class="tree-empty"><p>Sin páginas todavía.</p></div>
      {/if}
    {/if}
    {#each rows as row (row.kind === "dir" ? "d:" + row.key : "f:" + row.path)}
      {#if row.kind === "dir"}
        <div class="node dir">
          <span class="indent" style:width="{row.depth * 12}px"></span>
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7h6l2 2h10v9a2 2 0 0 1-2 2H3z"/></svg>
          <span class="nm">{row.name}/</span>
        </div>
      {:else}
        <button
          class="node"
          class:sel={$current === row.path}
          class:reserved={row.reserved}
          class:lvl-warn={row.lvl === "warn"}
          class:lvl-err={row.lvl === "err"}
          title={row.path}
          onclick={() => select(row.path)}
        >
          <span class="indent" style:width="{row.depth * 12}px"></span>
          {#if row.status}<span class="st-dot st-{row.status}" title={row.status}></span>{:else}<span class="st-dot"></span>{/if}
          <span class="nm">{row.label}</span>
          {#if row.type}<span class="tag-type">{row.type}</span>{/if}
        </button>
      {/if}
    {/each}
  </div>

  <div class="tree-legend" id="treeLegend">
    <span class="lg"><span class="st-dot st-draft"></span>Borrador</span>
    <span class="lg"><span class="st-dot st-review"></span>En revisión</span>
    <span class="lg"><span class="st-dot st-accepted"></span>Aceptada</span>
    <span class="lg"><span class="st-dot st-deprecated"></span>Obsoleta</span>
  </div>

  <div class="nav-graph" id="navGraph" class:on={$explorerView === "map"}>
    <div class="ng-bar">
      <div class="seg seg-sm">
        <button class:on={$graphScope === "bundle"} onclick={() => ($graphScope = "bundle")}>Global</button>
        <button class:on={$graphScope === "neighbor"} onclick={() => ($graphScope = "neighbor")}>Vecindad</button>
      </div>
      <div class="spacer" style="flex:1"></div>
      <button class="ghostbtn icon-only" title="Pantalla completa (M)" onclick={() => ($graphBigOpen = true)}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3H5a2 2 0 0 0-2 2v3M16 3h3a2 2 0 0 1 2 2v3M21 16v3a2 2 0 0 1-2 2h-3M3 16v3a2 2 0 0 0 2 2h3"/></svg>
      </button>
    </div>
    <StarMap {graph} {analysis} {files} current={$current} query={$treeQuery} scope={$graphScope} active={$explorerView === "map" && !$graphBigOpen} />
    <div class="map-legend">
      <span><i style="background:var(--star)"></i>página</span>
      <span><i style="background:var(--accent)"></i>actual</span>
      <span><i style="background:transparent;border:1px dashed var(--ghost)"></i>por escribir</span>
    </div>
  </div>
</section>
