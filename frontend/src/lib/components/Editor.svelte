<script lang="ts">
  // Editor multi-escritor: carga el .md crudo del concept seleccionado, permite editarlo y guardarlo
  // por el ÚNICO escritor (comando write_concept). El core valida: si introduciría un Err, el guardado
  // se rechaza (salvo forzar). Los diagnósticos se muestran localizados (i18n keyed por código).
  import { selected, snapshot } from "../stores/bundle";
  import { readConcept, writeConcept } from "../ipc";
  import { checkTitle, severityLabel } from "../i18n";

  let raw = $state("");
  let dirty = $state(false);
  let status = $state<string | null>(null);
  let loadedFor = $state<string | null>(null);

  const checks = $derived.by(() => {
    const s = $snapshot,
      cur = $selected;
    return s && cur ? (s.analysis.perFile[cur] ?? []) : [];
  });

  $effect(() => {
    const cur = $selected;
    if (cur && cur !== loadedFor) {
      loadedFor = cur;
      dirty = false;
      status = null;
      readConcept(cur)
        .then((c) => (raw = c))
        .catch((e) => (status = String(e)));
    }
  });

  async function save(force = false) {
    if (!$selected) return;
    try {
      const outcome = await writeConcept($selected, raw, force);
      if (outcome.written) {
        dirty = false;
        status = "Guardado.";
      } else {
        status = `Rechazado: ${outcome.rejected ?? "no conforme"}`;
      }
    } catch (e) {
      status = String(e);
    }
  }
</script>

{#if !$selected}
  <div class="empty">Selecciona una página para editarla.</div>
{:else}
  <div class="ed-head">
    <span class="path">{$selected}</span>
    <div class="spacer"></div>
    {#if status}<span class="status">{status}</span>{/if}
    <button class="btn" disabled={!dirty} onclick={() => save(false)}>Guardar</button>
    {#if status?.startsWith("Rechazado")}
      <button class="btn ghost" onclick={() => save(true)} title="Guardar aunque no sea conforme">
        Forzar
      </button>
    {/if}
  </div>

  <textarea
    class="editor"
    bind:value={raw}
    oninput={() => {
      dirty = true;
      status = null;
    }}
    spellcheck="false"
  ></textarea>

  {#if checks.length}
    <div class="checks">
      {#each checks as c}
        {#if c.level !== "pass"}
          <div class="chk {c.level}">
            <span class="code">{c.code}</span>
            <span class="sev">{severityLabel(c.level)}</span>
            <span class="ttl">{checkTitle(c.code)}</span>
            <span class="msg">{c.msg}</span>
          </div>
        {/if}
      {/each}
    </div>
  {/if}
{/if}

<style>
  .ed-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-bottom: 8px;
  }
  .path {
    font-family: var(--mono);
    font-size: 13px;
    color: var(--gold);
  }
  .spacer {
    flex: 1;
  }
  .status {
    font-size: 12px;
    color: var(--muted);
  }
  .btn {
    background: var(--accent);
    color: #1a1400;
    border: none;
    border-radius: var(--radius-sm);
    padding: 4px 12px;
    font-size: 12px;
    cursor: pointer;
  }
  .btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .btn.ghost {
    background: transparent;
    color: var(--warn);
    border: 1px solid var(--line-2);
  }
  .editor {
    width: 100%;
    flex: 1;
    min-height: 240px;
    resize: none;
    background: var(--surface);
    color: var(--ink);
    border: 1px solid var(--line-2);
    border-radius: var(--radius);
    padding: 12px;
    font-family: var(--mono);
    font-size: 13px;
    line-height: 1.6;
  }
  .checks {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 8px;
    max-height: 160px;
    overflow: auto;
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
  .chk.err {
    border-color: var(--err);
  }
  .chk .code {
    font-family: var(--mono);
    color: var(--faint);
  }
  .chk.warn .sev {
    color: var(--warn);
  }
  .chk.err .sev {
    color: var(--err);
  }
  .chk .msg {
    grid-column: 1 / -1;
    color: var(--muted);
  }
  .empty {
    color: var(--muted);
    font-size: 13px;
    padding: 24px;
  }
</style>
