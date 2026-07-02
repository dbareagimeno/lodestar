<script lang="ts">
  // Cáscara de la app: layout de tres columnas (páginas · centro · enlaces) + tabs de vista
  // (editor · grafo · cambios). La webview es vista fina sobre el `BundleSnapshot` empujado por Rust;
  // el grafo es una isla imperativa (§8). Aspecto portado del prototipo (variables CSS verbatim).
  import { onMount } from "svelte";
  import { snapshot, conformancePill, view, treeQuery } from "./lib/stores/bundle";
  import { getSnapshot, openBundle, onBundleChanged } from "./lib/ipc";
  import Tree from "./lib/components/Tree.svelte";
  import Backlinks from "./lib/components/Backlinks.svelte";
  import Editor from "./lib/components/Editor.svelte";
  import GraphIsland from "./lib/components/GraphIsland.svelte";
  import Changes from "./lib/components/Changes.svelte";

  let error = $state<string | null>(null);
  let leftOpen = $state(true);
  let rightOpen = $state(true);
  let bundlePath = $state("");

  onMount(async () => {
    // Si ya hay un bundle abierto (la fachada lo abrió al arrancar), tómalo; si no, espera a abrir.
    try {
      $snapshot = await getSnapshot();
    } catch {
      // sin bundle abierto todavía; el usuario lo abre por ruta
    }
    try {
      await onBundleChanged((snap) => ($snapshot = snap));
    } catch {
      /* fuera de Tauri: sin eventos en vivo */
    }
  });

  async function open() {
    error = null;
    try {
      $snapshot = await openBundle(bundlePath);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
</script>

<div class="app">
  <header class="topbar">
    <button class="rail-toggle" onclick={() => (leftOpen = !leftOpen)} title="Panel de páginas">☰</button>
    <span class="brand">lodestar</span>
    <div class="tabs">
      <button class:on={$view === "editor"} onclick={() => ($view = "editor")}>editor</button>
      <button class:on={$view === "grafo"} onclick={() => ($view = "grafo")}>grafo</button>
      <button class:on={$view === "cambios"} onclick={() => ($view = "cambios")}>cambios</button>
    </div>
    <div class="spacer"></div>
    <span class="pill" class:err={!$conformancePill.conform}>{$conformancePill.label}</span>
    <button class="rail-toggle" onclick={() => (rightOpen = !rightOpen)} title="Panel de enlaces">⇥</button>
  </header>

  {#if !$snapshot}
    <div class="opener">
      <p class="note">Abre un bundle (directorio de <code>.md</code> con frontmatter OKF):</p>
      <div class="row">
        <input bind:value={bundlePath} placeholder="/ruta/al/bundle" onkeydown={(e) => e.key === "Enter" && open()} />
        <button class="btn" onclick={open}>Abrir</button>
      </div>
      {#if error}<p class="err-msg">{error}</p>{/if}
    </div>
  {:else}
    <div class="layout" class:no-left={!leftOpen} class:no-right={!rightOpen}>
      {#if leftOpen}
        <aside class="rail left"><Tree /></aside>
      {/if}
      <main class="center">
        {#if $view === "editor"}
          <Editor />
        {:else if $view === "grafo"}
          <div class="graph-wrap">
            <input class="gq" placeholder="Resaltar en el mapa…" bind:value={$treeQuery} />
            <div class="graph-host"><GraphIsland /></div>
          </div>
        {:else}
          <Changes />
        {/if}
      </main>
      {#if rightOpen}
        <aside class="rail right"><Backlinks /></aside>
      {/if}
    </div>
  {/if}
</div>

<style>
  .app {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 12px;
    border-bottom: 1px solid var(--line);
    padding: 8px 12px;
    background: var(--surface);
  }
  .brand {
    color: var(--gold);
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  .tabs {
    display: flex;
    gap: 2px;
  }
  .tabs button {
    all: unset;
    padding: 4px 12px;
    font-size: 12px;
    color: var(--muted);
    cursor: pointer;
    border-radius: var(--radius-sm);
  }
  .tabs button.on {
    color: var(--ink);
    background: var(--panel-2);
  }
  .spacer {
    flex: 1;
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
    border-color: var(--err);
  }
  .rail-toggle {
    all: unset;
    cursor: pointer;
    color: var(--muted);
    padding: 2px 6px;
  }
  .layout {
    flex: 1;
    display: grid;
    grid-template-columns: 292px 1fr 320px;
    min-height: 0;
  }
  .layout.no-left {
    grid-template-columns: 1fr 320px;
  }
  .layout.no-right {
    grid-template-columns: 292px 1fr;
  }
  .layout.no-left.no-right {
    grid-template-columns: 1fr;
  }
  .rail {
    display: flex;
    flex-direction: column;
    padding: 12px;
    overflow: hidden;
    background: var(--panel);
  }
  .rail.left {
    border-right: 1px solid var(--line);
  }
  .rail.right {
    border-left: 1px solid var(--line);
  }
  .center {
    display: flex;
    flex-direction: column;
    padding: 12px;
    min-width: 0;
    overflow: hidden;
  }
  .graph-wrap {
    display: flex;
    flex-direction: column;
    height: 100%;
    gap: 8px;
  }
  .gq {
    background: var(--panel-2);
    color: var(--ink);
    border: 1px solid var(--line-2);
    border-radius: var(--radius-sm);
    padding: 5px 8px;
    font-size: 13px;
  }
  .graph-host {
    flex: 1;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .opener {
    padding: 48px;
    max-width: 560px;
  }
  .opener .row {
    display: flex;
    gap: 8px;
  }
  .opener input {
    flex: 1;
    background: var(--panel-2);
    color: var(--ink);
    border: 1px solid var(--line-2);
    border-radius: var(--radius-sm);
    padding: 6px 8px;
    font-family: var(--mono);
  }
  .btn {
    background: var(--accent);
    color: #1a1400;
    border: none;
    border-radius: var(--radius-sm);
    padding: 6px 16px;
    cursor: pointer;
  }
  .note {
    color: var(--muted);
    font-size: 13px;
  }
  .err-msg {
    color: var(--err);
    font-size: 13px;
  }
</style>
