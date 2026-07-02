<script lang="ts">
  // Panel izquierdo: árbol de concepts filtrable. Deriva del snapshot; el filtro es subcadena
  // sobre título/ruta (el core es la autoridad para la query semántica del comando `query`).
  import { treeRows, treeQuery, selected } from "../stores/bundle";

  const rows = $derived(
    $treeQuery.trim()
      ? $treeRows.filter(
          (r) =>
            r.title.toLowerCase().includes($treeQuery.toLowerCase()) ||
            r.path.toLowerCase().includes($treeQuery.toLowerCase()),
        )
      : $treeRows,
  );
</script>

<div class="col-head">
  <span>páginas</span>
  <span class="hcount">{rows.length}</span>
</div>
<input class="search" placeholder="Filtrar…" bind:value={$treeQuery} />
<div class="tree">
  {#each rows as row (row.path)}
    <button
      type="button"
      class="row"
      class:invalid={row.invalid}
      class:orphan={row.orphan}
      class:sel={$selected === row.path}
      onclick={() => ($selected = row.path)}
      title={row.path}
    >
      <span class="dot" class:err={row.invalid} class:muted={row.orphan}></span>
      <span class="t">{row.title}</span>
      {#if row.type}<span class="ty">{row.type}</span>{/if}
    </button>
  {/each}
  {#if rows.length === 0}
    <p class="note">Sin páginas.</p>
  {/if}
</div>
<div class="tree-legend">
  <span><span class="dot err"></span> con error</span>
  <span><span class="dot muted"></span> huérfana</span>
</div>

<style>
  .col-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 12px;
    color: var(--muted);
    text-transform: lowercase;
    padding-bottom: 6px;
  }
  .hcount {
    color: var(--faint);
  }
  .search {
    width: 100%;
    background: var(--panel-2);
    color: var(--ink);
    border: 1px solid var(--line-2);
    border-radius: var(--radius-sm);
    padding: 5px 8px;
    font-size: 13px;
    margin-bottom: 8px;
  }
  .tree {
    display: flex;
    flex-direction: column;
    gap: 1px;
    overflow: auto;
    flex: 1;
  }
  .row {
    all: unset;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-family: var(--mono);
    font-size: 13px;
    color: var(--ink);
  }
  .row:hover {
    background: var(--panel-2);
  }
  .row.sel {
    background: var(--accent-dim);
  }
  .row.invalid .t {
    color: var(--err);
  }
  .row.orphan .t {
    color: var(--muted);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--star);
    flex: none;
  }
  .dot.err {
    background: var(--err);
  }
  .dot.muted {
    background: var(--ghost);
  }
  .t {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ty {
    font-size: 11px;
    color: var(--faint);
  }
  .tree-legend {
    display: flex;
    gap: 12px;
    font-size: 11px;
    color: var(--faint);
    padding-top: 8px;
    border-top: 1px solid var(--line);
    margin-top: 8px;
  }
  .tree-legend .dot {
    display: inline-block;
    margin-right: 4px;
  }
  .note {
    color: var(--muted);
    font-size: 13px;
  }
</style>
