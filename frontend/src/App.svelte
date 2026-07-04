<script lang="ts">
  // Cáscara de la app: layout de tres columnas (páginas · centro · enlaces) + tabs de vista
  // (editor · grafo · cambios). La webview es vista fina sobre el `BundleSnapshot` empujado por Rust;
  // el grafo es una isla imperativa (§8). Aspecto portado del prototipo (variables CSS verbatim).
  import { onMount } from "svelte";
  import { snapshot, conformancePill, view, treeQuery } from "./lib/stores/bundle";
  import { getSnapshot, openBundle, createBundle, pickFolder, onBundleChanged } from "./lib/ipc";
  import Tree from "./lib/components/Tree.svelte";
  import Backlinks from "./lib/components/Backlinks.svelte";
  import Editor from "./lib/components/Editor.svelte";
  import GraphIsland from "./lib/components/GraphIsland.svelte";
  import Changes from "./lib/components/Changes.svelte";

  let error = $state<string | null>(null);
  let leftOpen = $state(true);
  let rightOpen = $state(true);
  let bundlePath = $state("");
  let busy = $state<string | null>(null);
  /// Ruta no-bundle detectada al abrir: se ofrece crear el workspace ahí mismo.
  let offerCreate = $state<string | null>(null);

  onMount(async () => {
    // Si ya hay un bundle abierto (la fachada lo abrió al arrancar), tómalo; si no, espera a abrir.
    try {
      $snapshot = await getSnapshot();
    } catch (e) {
      // «No hay bundle abierto» es el flujo normal (el usuario lo abre por ruta); cualquier
      // otro error de IPC se deja visible en consola para no depurar a ciegas.
      const msg = e instanceof Error ? e.message : String(e);
      if (!/ning[uú]n bundle|IPC no disponible/i.test(msg)) console.error("getSnapshot:", e);
    }
    try {
      await onBundleChanged((snap) => ($snapshot = snap));
    } catch {
      /* fuera de Tauri: sin eventos en vivo */
    }
  });

  function msgOf(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  async function openPath(path: string) {
    const p = path.trim();
    if (!p) return;
    error = null;
    offerCreate = null;
    busy = "Abriendo workspace…";
    try {
      $snapshot = await openBundle(p);
    } catch (e) {
      error = msgOf(e);
      // Si el directorio existe pero no es un workspace, ofrece crearlo ahí mismo.
      if (error.includes("no es un workspace")) offerCreate = p;
    } finally {
      busy = null;
    }
  }

  async function openExisting() {
    error = null;
    offerCreate = null;
    try {
      const dir = await pickFolder();
      if (!dir) return; // cancelado
      bundlePath = dir;
      await openPath(dir);
    } catch (e) {
      error = msgOf(e);
    }
  }

  async function createNew(path?: string) {
    error = null;
    offerCreate = null;
    try {
      const dir = path ?? (await pickFolder());
      if (!dir) return; // cancelado
      busy = "Creando workspace…";
      $snapshot = await createBundle(dir);
      bundlePath = dir;
    } catch (e) {
      error = msgOf(e);
    } finally {
      busy = null;
    }
  }
</script>

<div class="app">
  <header class="topbar">
    {#if $snapshot}
      <button class="rail-toggle" onclick={() => (leftOpen = !leftOpen)} title="Panel de páginas">☰</button>
    {/if}
    <span class="brand">lodestar</span>
    {#if $snapshot}
      <div class="tabs">
        <button class:on={$view === "editor"} onclick={() => ($view = "editor")}>editor</button>
        <button class:on={$view === "grafo"} onclick={() => ($view = "grafo")}>grafo</button>
        <button class:on={$view === "cambios"} onclick={() => ($view = "cambios")}>cambios</button>
      </div>
      <div class="spacer"></div>
      <span class="pill" class:err={!$conformancePill.conform}>{$conformancePill.label}</span>
      <button class="rail-toggle" onclick={() => (rightOpen = !rightOpen)} title="Panel de enlaces">⇥</button>
    {:else}
      <div class="spacer"></div>
    {/if}
  </header>

  {#if !$snapshot}
    <div class="opener">
      <div class="opener-card">
        <div class="opener-star" aria-hidden="true">✦</div>
        <h1 class="opener-title">Bienvenido a lodestar</h1>
        <p class="opener-sub">
          Tu base de conocimiento local-first: un directorio de <code>.md</code> con
          frontmatter OKF, versionado con git.
        </p>

        <div class="opener-actions">
          <button class="btn big" onclick={() => createNew()} disabled={busy !== null}>
            <span class="ico">✦</span>
            <span class="lbl">
              Crear un workspace nuevo
              <small>Elige una carpeta: se crea el índice raíz y el repositorio git</small>
            </span>
          </button>
          <button class="btn big ghost" onclick={openExisting} disabled={busy !== null}>
            <span class="ico">▸</span>
            <span class="lbl">
              Abrir un workspace existente
              <small>Busca la carpeta de un workspace ya creado</small>
            </span>
          </button>
        </div>

        <div class="opener-sep"><span>o escribe la ruta</span></div>

        <div class="row">
          <input
            bind:value={bundlePath}
            placeholder="/ruta/al/workspace"
            spellcheck="false"
            onkeydown={(e) => e.key === "Enter" && openPath(bundlePath)}
          />
          <button class="btn" onclick={() => openPath(bundlePath)} disabled={!bundlePath.trim() || busy !== null}>
            Abrir
          </button>
        </div>

        {#if busy}<p class="opener-busy">{busy}</p>{/if}
        {#if error}
          <div class="err-box">
            <p class="err-msg">{error}</p>
            {#if offerCreate}
              <button class="btn ghost-accent" onclick={() => createNew(offerCreate ?? undefined)}>
                Crear un workspace en esa carpeta
              </button>
            {/if}
          </div>
        {/if}
      </div>
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
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 32px;
    overflow: auto;
  }
  .opener-card {
    width: 100%;
    max-width: 520px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 36px 32px 28px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .opener-star {
    color: var(--accent);
    font-size: 28px;
    line-height: 1;
    text-shadow: 0 0 18px var(--accent);
  }
  .opener-title {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
    color: var(--ink);
    letter-spacing: 0.02em;
  }
  .opener-sub {
    margin: 0;
    color: var(--muted);
    font-size: 13px;
    line-height: 1.5;
  }
  .opener-actions {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 6px;
  }
  .btn.big {
    display: flex;
    align-items: center;
    gap: 12px;
    text-align: left;
    padding: 12px 14px;
    border-radius: var(--radius);
    font-size: 14px;
  }
  .btn.big .ico {
    font-size: 18px;
    line-height: 1;
  }
  .btn.big .lbl {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .btn.big .lbl small {
    font-size: 11.5px;
    font-weight: 400;
    opacity: 0.75;
  }
  .btn.big.ghost {
    background: var(--panel-2);
    color: var(--ink);
    border: 1px solid var(--line-2);
  }
  .btn.big.ghost .ico {
    color: var(--accent);
  }
  .opener-sep {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--faint);
    font-size: 11.5px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin-top: 4px;
  }
  .opener-sep::before,
  .opener-sep::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--line-2);
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
    padding: 7px 10px;
    font-family: var(--mono);
    font-size: 13px;
  }
  .opener input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .opener-busy {
    margin: 0;
    color: var(--muted);
    font-size: 13px;
  }
  .btn {
    background: var(--accent);
    color: #1a1400;
    border: none;
    border-radius: var(--radius-sm);
    padding: 6px 16px;
    cursor: pointer;
  }
  .btn:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .btn.ghost-accent {
    background: transparent;
    color: var(--accent);
    border: 1px solid var(--accent);
    align-self: flex-start;
    font-size: 12.5px;
  }
  .err-box {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border: 1px solid var(--err);
    border-radius: var(--radius-sm);
    padding: 10px 12px;
    background: color-mix(in srgb, var(--err) 8%, transparent);
  }
  .err-msg {
    margin: 0;
    color: var(--err);
    font-size: 13px;
    overflow-wrap: anywhere;
  }
</style>
