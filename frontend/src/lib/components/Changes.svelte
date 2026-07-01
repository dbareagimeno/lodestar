<script lang="ts">
  // Modo «Cambios»: diff semántico del working tree vs HEAD (OkfDiff perezoso del core) + commit.
  import { onMount } from "svelte";
  import { commit, diffWorking } from "../ipc";

  interface FieldChange { key: string; from: string | null; to: string | null }
  interface BodyHunk { t: "context" | "add" | "remove" | "gap"; v: string | number }
  interface FileDiff {
    path: string;
    kind: "add" | "mod" | "remove";
    fm: FieldChange[];
    body: BodyHunk[];
    linksAdded: string[];
    linksRemoved: string[];
  }
  interface OkfDiff {
    files: FileDiff[];
    generated: { path: string; kind: string }[];
    stats: { added: number; modified: number; removed: number };
    statusChanges: { path: string; from: string | null; to: string | null }[];
    suggested: Record<string, unknown>;
  }

  let diff = $state<OkfDiff | null>(null);
  let msg = $state("");
  let status = $state<string | null>(null);

  async function load() {
    status = null;
    try {
      diff = (await diffWorking()) as unknown as OkfDiff;
      msg = suggestMessage(diff);
    } catch (e) {
      status = String(e);
    }
  }
  onMount(load);

  function suggestMessage(d: OkfDiff): string {
    const s = d.suggested as { kind?: string; title?: string; to?: string };
    if (s?.kind === "AddSingle") return `Añade ${s.title}`;
    if (s?.kind === "StatusSingle") return `${s.title}: ${s.to}`;
    const { added, modified, removed } = d.stats;
    return `Actualiza el bundle (+${added} ~${modified} -${removed})`;
  }

  async function doCommit() {
    if (!msg.trim()) return;
    try {
      const r = await commit(msg);
      status = r.conformance.conform
        ? `Commit ${r.sha.slice(0, 7)} · conforme`
        : `Commit ${r.sha.slice(0, 7)} · NO conforme (${r.conformance.hardFail})`;
      await load();
    } catch (e) {
      status = String(e);
    }
  }
</script>

<div class="wrap">
  {#if status}<p class="status">{status}</p>{/if}
  {#if !diff}
    <p class="note">Cargando diff…</p>
  {:else if diff.files.length === 0 && diff.generated.length === 0}
    <p class="note">Sin cambios sin commitear.</p>
  {:else}
    <div class="commit">
      <input bind:value={msg} placeholder="Mensaje de commit…" />
      <button class="btn" onclick={doCommit} disabled={!msg.trim()}>Commit</button>
    </div>
    <p class="stats">
      +{diff.stats.added} añadidos · ~{diff.stats.modified} modificados · −{diff.stats.removed} eliminados
    </p>
    {#each diff.files as f (f.path)}
      <div class="file">
        <div class="fh">
          <span class="k {f.kind}">{f.kind}</span>
          <span class="p">{f.path}</span>
        </div>
        {#each f.fm as fc}
          <div class="fm">
            <span class="key">{fc.key}</span>
            <span class="from">{fc.from ?? "∅"}</span>
            <span class="arr">→</span>
            <span class="to">{fc.to ?? "∅"}</span>
          </div>
        {/each}
        {#each f.body as h}
          {#if h.t === "add"}<pre class="add">+ {h.v}</pre>
          {:else if h.t === "remove"}<pre class="rem">- {h.v}</pre>
          {:else if h.t === "gap"}<pre class="gap">⋯ {h.v} líneas</pre>
          {:else}<pre class="ctx">  {h.v}</pre>{/if}
        {/each}
      </div>
    {/each}
    {#if diff.generated.length}
      <div class="gen">
        <h4>Artefactos regenerados</h4>
        {#each diff.generated as g}<span class="gp">{g.kind} {g.path}</span>{/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .wrap {
    overflow: auto;
    height: 100%;
    padding: 4px;
  }
  .commit {
    display: flex;
    gap: 8px;
    margin-bottom: 8px;
  }
  .commit input {
    flex: 1;
    background: var(--panel-2);
    color: var(--ink);
    border: 1px solid var(--line-2);
    border-radius: var(--radius-sm);
    padding: 6px 8px;
    font-size: 13px;
  }
  .btn {
    background: var(--accent);
    color: #1a1400;
    border: none;
    border-radius: var(--radius-sm);
    padding: 4px 14px;
    cursor: pointer;
  }
  .btn:disabled {
    opacity: 0.4;
  }
  .stats {
    font-size: 12px;
    color: var(--muted);
  }
  .file {
    border: 1px solid var(--line);
    border-radius: var(--radius);
    margin-bottom: 10px;
    padding: 8px;
  }
  .fh {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-bottom: 4px;
  }
  .k {
    font-size: 10px;
    text-transform: uppercase;
    padding: 1px 5px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--line-2);
  }
  .k.add {
    color: var(--ok);
  }
  .k.mod {
    color: var(--warn);
  }
  .k.remove {
    color: var(--err);
  }
  .p {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--gold);
  }
  .fm {
    display: flex;
    gap: 6px;
    font-family: var(--mono);
    font-size: 12px;
  }
  .fm .key {
    color: var(--faint);
  }
  .fm .from {
    color: var(--err);
  }
  .fm .to {
    color: var(--ok);
  }
  pre {
    margin: 0;
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
    line-height: 1.5;
  }
  pre.add {
    color: var(--ok);
  }
  pre.rem {
    color: var(--err);
  }
  pre.gap {
    color: var(--faint);
  }
  pre.ctx {
    color: var(--muted);
  }
  .gen h4 {
    font-size: 11px;
    color: var(--faint);
    text-transform: uppercase;
  }
  .gp {
    display: block;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--muted);
  }
  .note {
    color: var(--muted);
    font-size: 13px;
  }
  .status {
    font-size: 12px;
    color: var(--accent);
  }
</style>
