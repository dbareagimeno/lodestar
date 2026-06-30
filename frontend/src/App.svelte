<script lang="ts">
  // Cáscara de la app. El port verbatim de la UI del prototipo (layout de rails, árbol, tabs, editor,
  // pill/overlay/modo Cambios, isla del grafo) es el grueso de E6; aquí queda la estructura base que
  // consume el snapshot empujado vía los stores.
  import { onMount } from "svelte";
  import { snapshot, conformancePill, treeRows } from "./lib/stores/bundle";
  import { getSnapshot } from "./lib/ipc";

  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      $snapshot = await getSnapshot();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  });
</script>

<main>
  <header class="topbar">
    <span class="brand">lodestar</span>
    <span class="pill" class:err={!$conformancePill.conform}>{$conformancePill.label}</span>
  </header>

  {#if error}
    <p class="note">{error}</p>
  {:else if $snapshot}
    <ul class="tree">
      {#each $treeRows as row (row.path)}
        <li class:invalid={row.invalid} class:orphan={row.orphan}>{row.title}</li>
      {/each}
    </ul>
  {:else}
    <p class="note">Cargando bundle…</p>
  {/if}
</main>

<style>
  main {
    height: 100%;
    padding: 12px;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 12px;
    border-bottom: 1px solid var(--line);
    padding-bottom: 8px;
  }
  .brand {
    color: var(--gold);
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  .pill {
    font-size: 12px;
    color: var(--ok);
    border: 1px solid var(--line-2);
    border-radius: var(--radius-sm);
    padding: 2px 8px;
  }
  .pill.err {
    color: var(--err);
  }
  .tree {
    list-style: none;
    padding: 8px 0;
    margin: 0;
    font-family: var(--mono);
    font-size: 13px;
  }
  .tree li {
    padding: 2px 4px;
    color: var(--ink);
  }
  .tree li.invalid {
    color: var(--err);
  }
  .tree li.orphan {
    color: var(--muted);
  }
  .note {
    color: var(--muted);
    font-size: 13px;
  }
</style>
