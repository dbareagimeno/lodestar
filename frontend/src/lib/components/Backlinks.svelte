<script lang="ts">
  // Panel derecho: vecindad de enlaces del concept seleccionado, derivada del snapshot
  // (inn/out/dangling ya computados por el core). Click en un vecino navega.
  import { snapshot, selected } from "../stores/bundle";

  const inbound = $derived.by(() => {
    const s = $snapshot,
      cur = $selected;
    if (!s || !cur) return [];
    return s.analysis.inn[cur] ?? [];
  });
  const outbound = $derived.by(() => {
    const s = $snapshot,
      cur = $selected;
    if (!s || !cur) return [];
    return s.analysis.out[cur] ?? [];
  });
  const indexRefs = $derived.by(() => {
    const s = $snapshot,
      cur = $selected;
    if (!s || !cur) return [];
    return [...s.analysis.inIndex].includes(cur) ? ["listada en un índice"] : [];
  });
</script>

<div class="col-head">
  <span>enlaces</span>
</div>
{#if !$selected}
  <p class="note">Selecciona una página.</p>
{:else}
  <section>
    <h4>Entran ({inbound.length})</h4>
    {#each inbound as p}
      <button class="lnk" onclick={() => ($selected = p)}>{p}</button>
    {:else}
      <p class="empty">nadie enlaza aquí</p>
    {/each}
  </section>
  <section>
    <h4>Salen ({outbound.length})</h4>
    {#each outbound as p}
      <button class="lnk" onclick={() => ($selected = p)}>{p}</button>
    {:else}
      <p class="empty">sin enlaces salientes</p>
    {/each}
  </section>
  {#if indexRefs.length}
    <section><h4>Índice</h4><p class="empty">{indexRefs[0]}</p></section>
  {/if}
{/if}

<style>
  .col-head {
    font-size: 12px;
    color: var(--muted);
    text-transform: lowercase;
    padding-bottom: 6px;
  }
  section {
    margin-bottom: 12px;
  }
  h4 {
    font-size: 11px;
    color: var(--faint);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin: 0 0 4px;
  }
  .lnk {
    all: unset;
    display: block;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--accent);
    padding: 2px 4px;
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .lnk:hover {
    background: var(--panel-2);
  }
  .empty,
  .note {
    color: var(--faint);
    font-size: 12px;
  }
</style>
