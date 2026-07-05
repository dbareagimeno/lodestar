<script lang="ts">
  // Cáscara de la app: header + main de 3 columnas (explorer · editor · inspector) + overlays (grafo,
  // historial) + diálogos + toast + nav móvil. Port del <body> del prototipo (mismas clases/IDs/atributos
  // data-*). La webview es vista fina sobre el BundleSnapshot empujado por Rust.
  import { onMount } from "svelte";
  import { snapshot, current, refreshSnapshot } from "./lib/stores/bundle";
  import { onBundleChanged } from "./lib/ipc";
  import { loadVersions } from "./lib/versions";
  import {
    mode, tabs, explorerView, railLeft, railRight, clW, crW, mobileView,
    dialog, graphBigOpen, verOverlayOpen,
    setMode, toggleRail, openExplorerMap, closeExplorerMap, openDialog, closeDialog,
  } from "./lib/stores/ui";
  import Header from "./lib/components/Header.svelte";
  import Explorer from "./lib/components/Explorer.svelte";
  import Editor from "./lib/components/Editor.svelte";
  import Inspector from "./lib/components/Inspector.svelte";
  import GraphOverlay from "./lib/components/GraphOverlay.svelte";
  import VerOverlay from "./lib/components/VerOverlay.svelte";
  import Dialogs from "./lib/components/Dialogs.svelte";
  import Toast from "./lib/components/Toast.svelte";
  import MobileNav from "./lib/components/MobileNav.svelte";

  let mainEl: HTMLElement;
  let gutterL: HTMLElement;
  let gutterR: HTMLElement;

  onMount(async () => {
    await refreshSnapshot();
    await loadVersions();
    // Selección inicial: como el prototipo, abre la spec de login si existe (paridad de la vista por
    // defecto). En producción con bundle vacío, se queda en el estado vacío.
    const s = $snapshot;
    if (s && !$current && s.files["specs/auth-login.md"] !== undefined) {
      tabs.set(["specs/auth-login.md"]);
      current.set("specs/auth-login.md");
      mode.set("preview");
    }
    try {
      await onBundleChanged((snap) => snapshot.set(snap));
    } catch {
      /* fuera de Tauri: sin eventos en vivo (el mock refresca al escribir) */
    }
  });

  // Atributos data-* en <body> (los usan selectores globales del prototipo).
  $effect(() => {
    document.body.dataset.railLeft = $railLeft ? "off" : "on";
    document.body.dataset.railRight = $railRight ? "off" : "on";
    document.body.dataset.explorer = $explorerView;
    document.body.dataset.view = $mobileView;
  });

  // Layout de columnas (port de applyLayout): raíles colapsados = 40px; mapa ensancha el izquierdo.
  $effect(() => {
    void $railLeft; void $railRight; void $clW; void $crW; void $explorerView;
    if (!mainEl) return;
    if (window.innerWidth <= 900) {
      mainEl.style.gridTemplateColumns = "";
      if (gutterL) gutterL.style.display = "none";
      if (gutterR) gutterR.style.display = "none";
      return;
    }
    const L = $railLeft ? 40 : $explorerView === "map" ? Math.max($clW, 400) : $clW;
    const R = $railRight ? 40 : $crW;
    mainEl.style.gridTemplateColumns = `${L}px minmax(0,1fr) ${R}px`;
    if (gutterL) { gutterL.style.display = $railLeft ? "none" : ""; gutterL.style.left = `${L}px`; }
    if (gutterR) { gutterR.style.display = $railRight ? "none" : ""; gutterR.style.right = `${R}px`; }
  });

  function startRailDrag(side: "left" | "right", e: PointerEvent) {
    if (window.innerWidth <= 900) return;
    e.preventDefault();
    const g = e.currentTarget as HTMLElement;
    g.classList.add("dragging");
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    const move = (ev: PointerEvent) => {
      if (side === "left") clW.set(Math.max(220, Math.min(480, ev.clientX)));
      else crW.set(Math.max(240, Math.min(520, window.innerWidth - ev.clientX)));
    };
    const up = () => {
      document.removeEventListener("pointermove", move);
      document.removeEventListener("pointerup", up);
      g.classList.remove("dragging");
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    document.addEventListener("pointermove", move);
    document.addEventListener("pointerup", up);
  }

  function onKey(e: KeyboardEvent) {
    const tag = (e.target as HTMLElement).tagName;
    if ((e.metaKey || e.ctrlKey) && (e.key === "w" || e.key === "W")) {
      if ($current) { e.preventDefault(); import("./lib/stores/ui").then((m) => m.closeTab($current!)); }
      return;
    }
    if ((e.metaKey || e.ctrlKey) && (e.key === "s" || e.key === "S")) { e.preventDefault(); openDialog("commit"); return; }
    if (/input|textarea|select/i.test(tag)) return;
    if (e.key === "/") { e.preventDefault(); (document.querySelector(".search-row .s-input") as HTMLInputElement)?.focus(); return; }
    if (e.key === "n" || e.key === "N") openDialog("new");
    if (e.key === "m" || e.key === "M") {
      if ($graphBigOpen) $graphBigOpen = false;
      else if ($explorerView === "map") closeExplorerMap();
      else openExplorerMap();
    }
    if (e.key === "[") { e.preventDefault(); toggleRail("left"); }
    if (e.key === "]") { e.preventDefault(); toggleRail("right"); }
    if (e.key === "?" || (e.shiftKey && e.key === "/")) { e.preventDefault(); openDialog("keys"); }
    if (e.key === "1") setMode("preview");
    if (e.key === "2") setMode("form");
    if (e.key === "3") setMode("raw");
    if (e.key === "4") setMode("diff");
    if (e.key === "v" || e.key === "V") $verOverlayOpen = !$verOverlayOpen;
    if (e.key === "Escape") {
      if ($verOverlayOpen) $verOverlayOpen = false;
      else if ($graphBigOpen) $graphBigOpen = false;
      else if ($dialog) closeDialog();
    }
  }
</script>

<svelte:window on:keydown={onKey} />

<Header />

<main bind:this={mainEl}>
  <Explorer />
  <Editor />
  <Inspector />
  <div class="gutter l" bind:this={gutterL} aria-hidden="true" onpointerdown={(e) => startRailDrag("left", e)}></div>
  <div class="gutter r" bind:this={gutterR} aria-hidden="true" onpointerdown={(e) => startRailDrag("right", e)}></div>
</main>

<GraphOverlay />
<VerOverlay />
<MobileNav />
<Dialogs />
<Toast />
