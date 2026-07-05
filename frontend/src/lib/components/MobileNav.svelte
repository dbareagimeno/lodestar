<script lang="ts">
  import { mobileView } from "../stores/ui";
  function go(v: "explorer" | "editor" | "inspector") {
    $mobileView = v;
    document.body.dataset.view = v;
  }
</script>

<nav class="mobile-nav">
  <button class:on={$mobileView === "explorer"} onclick={() => go("explorer")}>
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7h18M3 12h18M3 17h12"/></svg>páginas
  </button>
  <button class:on={$mobileView === "editor"} onclick={() => go("editor")}>
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 4H4v16h16v-7M18.5 2.5a2.1 2.1 0 0 1 3 3L12 15l-4 1 1-4z"/></svg>editor
  </button>
  <button class:on={$mobileView === "inspector"} onclick={() => go("inspector")}>
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M10 13a5 5 0 0 0 7 0l2-2a5 5 0 0 0-7-7l-1 1M14 11a5 5 0 0 0-7 0l-2 2a5 5 0 0 0 7 7l1-1"/></svg>enlaces
  </button>
</nav>
