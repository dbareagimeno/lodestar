<script lang="ts">
  // Cáscara de la app. El port verbatim y completo de la UI del prototipo (rails redimensionables,
  // tabs, editor multi-escritor, isla del grafo con createStarMap, overlay/modo Cambios) es el grueso
  // pendiente de E6 y requiere la fachada Tauri empaquetada. Aquí ya se consume el snapshot empujado:
  // árbol filtrable + selección + panel de conformidad localizado (i18n keyed por código).
  import { onMount } from "svelte";
  import { snapshot, conformancePill, treeRows, treeQuery } from "./lib/stores/bundle";
  import { getSnapshot, onBundleChanged } from "./lib/ipc";
  import { checkTitle, severityLabel } from "./lib/i18n";
  import type { RelPath } from "./lib/ipc/types";

  let error = $state<string | null>(null);
  let selected = $state<RelPath | null>(null);

  const rows = $derived(
    $treeQuery.trim()
      ? $treeRows.filter((r) => r.title.toLowerCase().includes($treeQuery.toLowerCase()))
      : $treeRows,
  );
  const checks = $derived(
    selected && $snapshot ? ($snapshot.analysis.perFile[selected] ?? []) : [],
  );

  onMount(async () => {
    try {
      $snapshot = await getSnapshot();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
    // Reconciliación en vivo: la fachada empuja el snapshot completo al cambiar el bundle.
    await onBundleChanged((snap) => {
      $snapshot = snap;
    });
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
    <div class="layout">
      <section class="rail">
        <input
          class="search"
          placeholder="Filtrar concepts…"
          bind:value={$treeQuery}
        />
        <ul class="tree">
          {#each rows as row (row.path)}
            <li
              class:invalid={row.invalid}
              class:orphan={row.orphan}
              class:sel={selected === row.path}
            >
              <button type="button" onclick={() => (selected = row.path)}>{row.title}</button>
            </li>
          {/each}
        </ul>
      </section>

      <section class="detail">
        {#if selected}
          <h2>{selected}</h2>
          {#if checks.length === 0}
            <p class="note">Sin diagnósticos.</p>
          {:else}
            <ul class="checks">
              {#each checks as c}
                <li class="chk {c.level}">
                  <span class="code">{c.code}</span>
                  <span class="sev">{severityLabel(c.level)}</span>
                  <span class="title">{checkTitle(c.code)}</span>
                  <span class="msg">{c.msg}</span>
                </li>
              {/each}
            </ul>
          {/if}
        {:else}
          <p class="note">Selecciona un concept para ver su conformidad.</p>
        {/if}
      </section>
    </div>
  {:else}
    <p class="note">Cargando bundle…</p>
  {/if}
</main>

<style>
  main {
    height: 100%;
    padding: 12px;
    display: flex;
    flex-direction: column;
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
  .layout {
    display: grid;
    grid-template-columns: 292px 1fr;
    gap: 12px;
    flex: 1;
    min-height: 0;
  }
  .rail {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-right: 1px solid var(--line);
    padding-right: 8px;
    overflow: auto;
  }
  .search {
    background: var(--bg-2, #1a1a1a);
    color: var(--ink);
    border: 1px solid var(--line-2);
    border-radius: var(--radius-sm);
    padding: 4px 8px;
    font-size: 13px;
  }
  .tree {
    list-style: none;
    padding: 0;
    margin: 0;
    font-family: var(--mono);
    font-size: 13px;
  }
  .tree li button {
    all: unset;
    display: block;
    width: 100%;
    padding: 2px 4px;
    color: var(--ink);
    cursor: pointer;
  }
  .tree li.invalid button {
    color: var(--err);
  }
  .tree li.orphan button {
    color: var(--muted);
  }
  .tree li.sel button {
    background: var(--line);
    border-radius: var(--radius-sm);
  }
  .detail {
    overflow: auto;
  }
  .detail h2 {
    font-family: var(--mono);
    font-size: 14px;
    color: var(--gold);
  }
  .checks {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .chk {
    display: grid;
    grid-template-columns: auto auto 1fr;
    gap: 8px;
    font-size: 12px;
    padding: 4px 6px;
    border: 1px solid var(--line-2);
    border-radius: var(--radius-sm);
  }
  .chk .code {
    font-family: var(--mono);
    color: var(--muted);
  }
  .chk.err {
    border-color: var(--err);
  }
  .chk.warn .sev {
    color: var(--gold);
  }
  .chk .msg {
    grid-column: 1 / -1;
    color: var(--muted);
  }
  .note {
    color: var(--muted);
    font-size: 13px;
  }
</style>
