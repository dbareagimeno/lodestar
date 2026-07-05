<script lang="ts">
  // Panel central (editor). Port de la columna editora del prototipo: tabbar, ed-head (breadcrumb,
  // punto de guardado, chip de versión, segmento Leer/Editar/Código/Cambios, kebab borrar) y ed-scroll
  // con los modos preview/form/raw/reserved/diff. La página se guarda por el ÚNICO escritor
  // (write_concept); mientras editas, un draft local evita pisadas.
  import DOMPurify from "dompurify";
  import { snapshot, current, files as filesStore } from "../stores/bundle";
  import { mode, tabs, setMode, closeTab, select, offerCreate, writeFile, confirmDlg, toast, openDialog, isReserved, verOverlayOpen } from "../stores/ui";
  import {
    parseFile, basename, dirOf, RESERVED, titleFromPath, STATUS_LABEL, resolveLink, fmtWhen, miniMd,
    buildRaw, extrasToYAML, toISOStr, KNOWN_FM,
  } from "../okf";
  import { versions, tipSnapshot, tipVersion, diffSnap, statusLabel, pageTitleRaw, type FileDiff, type DiffLine } from "../versions";

  const files = $derived($filesStore);
  const analysis = $derived($snapshot?.analysis ?? null);

  // ---- draft local del fichero abierto ----
  let draft = $state("");
  let loadedFor = $state<string | null>(null);
  let dirty = $state(false);
  let writeT: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    const cur = $current;
    if (cur && cur !== loadedFor) {
      loadedFor = cur;
      dirty = false;
      draft = files[cur] ?? "";
    } else if (!cur) {
      loadedFor = null;
      draft = "";
    }
  });
  // Reconciliación externa (watcher/otra fachada): si no editas, refresca el draft.
  $effect(() => {
    const cur = loadedFor;
    if (!cur) return;
    const onDisk = files[cur];
    if (onDisk !== undefined && !dirty && onDisk !== draft) draft = onDisk;
  });

  function scheduleWrite() {
    dirty = true;
    if (writeT) clearTimeout(writeT);
    const cur = loadedFor;
    const content = draft;
    writeT = setTimeout(async () => {
      if (!cur) return;
      await writeFile(cur, content);
      dirty = false;
    }, 220);
  }

  // Vista del fichero actual desde el draft (refleja ediciones sin guardar al instante).
  const pf = $derived.by(() => {
    const cur = $current;
    if (!cur) return null;
    return parseFile({ [cur]: draft }, cur);
  });

  // breadcrumb
  const crumb = $derived.by(() => {
    const cur = $current;
    if (!cur) return { folder: [] as string[], last: "—" };
    const segs = cur.split("/");
    const folder = segs.slice(0, -1);
    const isCode = $mode === "raw" || RESERVED.has(basename(cur));
    const last = isCode ? basename(cur) : ((pf?.fm?.title as string) || titleFromPath(cur));
    return { folder, last };
  });

  // ver-chip: pendiente vs último commit que tocó la página.
  const verChip = $derived.by(() => {
    const cur = $current;
    void $versions;
    if (!cur || files[cur] === undefined || RESERVED.has(basename(cur))) return null;
    const changed = (tipSnapshot()[cur] || "") !== (draft || files[cur] || "");
    if (changed) return { pending: true, text: "sin versionar" };
    const vs = $versions;
    for (let i = vs.length - 1; i >= 0; i--) {
      const a = vs[i - 1] ? vs[i - 1].snapshot : {};
      if ((a[cur] || "") !== (vs[i].snapshot[cur] || "")) return { pending: false, text: vs[i].id };
    }
    return { pending: false, text: "sin versión" };
  });

  // ---- preview ----
  const previewHtml = $derived.by(() => {
    const cur = $current;
    if (!cur || !pf) return "";
    const raw = miniMd(pf.body || "");
    return DOMPurify.sanitize(raw.replace(/<script[\s\S]*?<\/script>/gi, ""));
  });
  const readChips = $derived.by(() => {
    const fm = (pf?.fm || {}) as Record<string, unknown>;
    const chips: { k: string; v: string; href?: string }[] = [];
    if (fm.type) chips.push({ k: "Tipo", v: String(fm.type) });
    if (fm.status) chips.push({ k: "Estado", v: STATUS_LABEL[String(fm.status).toLowerCase()] || String(fm.status) });
    const tags = Array.isArray(fm.tags) ? fm.tags : fm.tags ? [fm.tags] : [];
    if (tags.length) chips.push({ k: "Etiquetas", v: tags.join(", ") });
    if (fm.timestamp) chips.push({ k: "Actualizado", v: fmtWhen(fm.timestamp) });
    if (fm.resource) chips.push({ k: "Recurso", v: "enlace", href: String(fm.resource) });
    return chips;
  });
  // Enlaces internos del preview: clases + navegación (imperativo, como el prototipo).
  let previewEl = $state<HTMLElement | null>(null);
  $effect(() => {
    void previewHtml;
    const el = previewEl;
    const cur = $current;
    if (!el || !cur) return;
    el.querySelectorAll("a").forEach((a) => {
      const href = a.getAttribute("href") || "";
      const t = resolveLink(href, cur);
      if (t) {
        if (files[t] !== undefined) {
          a.classList.add("clink");
          a.onclick = (e) => { e.preventDefault(); select(t); };
        } else {
          a.classList.add("clink-ghost");
          a.title = "concept por escribir";
          a.onclick = (e) => { e.preventDefault(); offerCreate(t); };
        }
      } else if (/^[a-z]+:/i.test(href)) {
        a.target = "_blank";
        a.rel = "noopener";
      }
    });
  });

  const notes = $derived.by(() => {
    const cur = $current;
    const ri = (cur && analysis?.perFile[cur]) || [];
    return ri.filter((c) => c.level === "err" || c.level === "warn");
  });

  function noteIcon(level: string): string {
    if (level === "err") return '<circle cx="12" cy="12" r="9"/><path d="M12 8v5M12 16h.01"/>';
    if (level === "warn") return '<path d="M12 3l9 16H3z"/><path d="M12 10v4M12 17h.01"/>';
    return '<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>';
  }

  // ---- form ----
  const fm = $derived((pf?.fm || {}) as Record<string, unknown>);
  const extras = $derived(Object.keys(fm).filter((k) => !KNOWN_FM.includes(k)));
  function fmField(codes: string[]) {
    const cur = $current;
    const issues = (cur && analysis?.perFile[cur]) || [];
    return issues.find((x) => codes.includes(x.code) && x.level !== "pass") ?? null;
  }
  // Reserializa el formulario al draft.
  function reserialize(patch: Partial<Record<string, string>>, bodyVal?: string) {
    const f = { ...fm } as Record<string, unknown>;
    const cur = { type: "type", title: "title", description: "description", resource: "resource", tags: "tags", timestamp: "timestamp", status: "status" };
    void cur;
    Object.entries(patch).forEach(([k, v]) => {
      if (k === "tags") {
        const arr = String(v || "").split(",").map((s) => s.trim()).filter(Boolean);
        if (arr.length) f.tags = arr; else delete f.tags;
      } else if (v == null || v === "") {
        delete f[k];
      } else f[k] = v;
    });
    const body = bodyVal ?? pf?.body ?? "";
    draft = buildRaw(f, body);
    scheduleWrite();
  }
  const tagsStr = $derived(Array.isArray(fm.tags) ? (fm.tags as string[]).join(", ") : (fm.tags as string) || "");
  const bodyIssues = $derived.by(() => {
    const cur = $current;
    const issues = (cur && analysis?.perFile[cur]) || [];
    return issues.filter((x) => ["BODY-STRUCT", "LINK-REL"].includes(x.code));
  });

  // ---- diff mode ----
  const diffModel = $derived.by(() => {
    const cur = $current;
    void $versions;
    if (!cur || RESERVED.has(basename(cur))) return { na: true } as const;
    const aRaw = tipSnapshot()[cur];
    const bRaw = draft || files[cur];
    if ((aRaw || "") === (bRaw || "")) return { empty: true, tip: tipVersion() } as const;
    const aObj = aRaw === undefined ? {} : { [cur]: aRaw };
    const d = diffSnap(aObj, { [cur]: bRaw });
    return { d } as const;
  });

  function delPage() {
    const cur = $current;
    if (!cur) return;
    confirmDlg("Borrar página", `¿Seguro que quieres borrar “${(pf?.fm?.title as string) || titleFromPath(cur)}”?`, () => {
      toast("Borrar: usa la CLI / el binario de escritorio.");
    });
  }
  let kebab = $state(false);

  function tagsFromStatusDot(s: string | null | undefined): string {
    return "st-" + (s ? String(s).toLowerCase() : "none");
  }
</script>

<section class="col editor">
  <div class="tabbar" class:empty={$tabs.length === 0}>
    {#each $tabs as id (id)}
      <div class="tab" class:on={id === $current} title={id} onclick={() => select(id)} role="button" tabindex="0">
        <span class="st-dot {tagsFromStatusDot((parseFile(files, id).fm?.status as string) ?? null)}"></span>
        <span class="tab-nm">{files[id] !== undefined ? (parseFile(files, id).fm?.title as string) || titleFromPath(id) : titleFromPath(id)}</span>
        <button class="tab-x" title="Cerrar (Ctrl/⌘+W)" onclick={(e) => { e.stopPropagation(); closeTab(id); }}>×</button>
      </div>
    {/each}
  </div>

  <div class="ed-head" id="edHeadDoc">
    <div class="crumb">{crumb.folder.map((f) => f + " / ").join("")}<b>{crumb.last}</b></div>
    <div class="spacer" style="flex:1"></div>
    <span class="save-stat">
      {#if $current}
        {#if dirty}<span class="save-dot dirty" title="Sin guardar"></span>{:else}<span class="save-dot clean" title="Guardado"></span>{/if}
      {/if}
    </span>
    {#if verChip}
      <button class="ver-chip" class:pending={verChip.pending} onclick={() => ($verOverlayOpen = true)}>
        {#if verChip.pending}<span class="cd"></span>sin versionar{:else}<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 3v5h5"/><path d="M3.05 13A9 9 0 1 0 6 5.3L3 8"/><path d="M12 7v5l3 1.5"/></svg><span class="cm">{verChip.text}</span>{/if}
      </button>
    {/if}
    {#if $current && !isReserved($current)}
      <div class="seg">
        <button class:on={$mode === "preview"} onclick={() => setMode("preview")}>Leer</button>
        <button class:on={$mode === "form"} onclick={() => setMode("form")}>Editar</button>
        <button class:on={$mode === "raw"} onclick={() => setMode("raw")}>Código</button>
        <button class:on={$mode === "diff"} onclick={() => setMode("diff")}>Cambios</button>
      </div>
    {/if}
    <div class="kebab-wrap">
      <button class="ghostbtn icon-only kebab-toggle" title="Más" onclick={(e) => { e.stopPropagation(); kebab = !kebab; }}>
        <svg viewBox="0 0 24 24" fill="currentColor" stroke="none"><circle cx="5" cy="12" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="19" cy="12" r="1.6"/></svg>
      </button>
      <div class="kebab-menu kebab-right" class:on={kebab}>
        <button class="km-item danger" onclick={() => { kebab = false; delPage(); }}>Borrar página</button>
      </div>
    </div>
  </div>

  <div class="ed-scroll" id="edScroll">
    {#if !$current || files[$current] === undefined}
      <div class="doc-empty">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4"><path d="M12 2.5l2.4 6.7 6.6 2.3-6.6 2.3L12 21.5l-2.4-6.7L3 12.5l6.6-2.3z"/></svg>
        <h2>{Object.keys(files).length ? "Selecciona una página" : "Tu espacio está vacío"}</h2>
        <p>{Object.keys(files).length ? "Elige una página del navegador de la izquierda, o el mapa de relaciones, para abrirla aquí." : "Crea la primera página para empezar tu base de conocimiento."}</p>
        <button class="de-cta" onclick={() => openDialog("new")}><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M12 5v14M5 12h14"/></svg>Nueva página</button>
      </div>
    {:else if isReserved($current)}
      {@render reservedView()}
    {:else if $mode === "raw"}
      <textarea class="raw-edit" spellcheck="false" style="margin:14px 16px;width:calc(100% - 32px)" bind:value={draft} oninput={scheduleWrite}></textarea>
      {@render noteBlock("Revisión", notes)}
    {:else if $mode === "form"}
      {@render formView()}
    {:else if $mode === "diff"}
      {@render diffView()}
    {:else}
      {@render previewView()}
    {/if}
  </div>
</section>

{#snippet previewView()}
  <div class="doc-read">
    <h1 class="doc-read-title">{(pf?.fm?.title as string) || titleFromPath($current!)}</h1>
    {#if pf?.fm?.description}<p class="doc-read-sub">{pf.fm.description}</p>{/if}
    {#if readChips.length}
      <div class="read-meta">
        {#each readChips as c}
          <span class="rmeta"><span class="rk">{c.k}</span>{#if c.href}<a href={c.href} target="_blank" rel="noopener">{c.v}</a>{:else}{c.v}{/if}</span>
        {/each}
      </div>
    {/if}
  </div>
  <div class="preview" bind:this={previewEl}>{@html previewHtml}</div>
  {@render noteBlock("Revisión", notes)}
{/snippet}

{#snippet noteBlock(title: string, list: typeof notes)}
  {#if list.length}
    <div class="doc-notes">
      <div class="doc-notes-h">{title}</div>
      {#each list as c}
        <div class="fld-note {c.level}"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">{@html noteIcon(c.level)}</svg><span>{c.msg}</span></div>
      {/each}
    </div>
  {/if}
{/snippet}

{#snippet formView()}
  <div>
    <div class="doc-head">
      <input class="doc-title" class:need={!fm.title} value={(fm.title as string) || ""} placeholder="Título de la página" oninput={(e) => reserialize({ title: e.currentTarget.value })} />
      {#if fmField(["REC-TITLE"])}{@render inlineNote(fmField(["REC-TITLE"]))}{/if}
      <textarea class="doc-sub" rows="1" placeholder="Añade una breve descripción…" spellcheck="false" value={(fm.description as string) || ""} oninput={(e) => reserialize({ description: e.currentTarget.value })}></textarea>
      {#if fmField(["REC-DESC"])}{@render inlineNote(fmField(["REC-DESC"]))}{/if}
      <div class="meta-grid">
        <span class="meta-k">Tipo</span>
        <input class="meta-v" class:need={!fm.type} value={(fm.type as string) || ""} placeholder="Página" spellcheck="false" oninput={(e) => reserialize({ type: e.currentTarget.value })} />
        {#if fmField(["OKF-TYPE"])}{@render inlineNote(fmField(["OKF-TYPE"]))}{/if}
        <span class="meta-k">Estado</span>
        <select class="meta-v" value={(fm.status as string) || ""} onchange={(e) => reserialize({ status: e.currentTarget.value })}>
          {#each ["", "draft", "review", "accepted", "deprecated"] as s}<option value={s}>{STATUS_LABEL[s]}</option>{/each}
        </select>
        <span class="meta-k">Etiquetas</span>
        <input class="meta-v" value={tagsStr} placeholder="añadir…" oninput={(e) => reserialize({ tags: e.currentTarget.value })} />
        {#if fmField(["FMT-TAGS"])}{@render inlineNote(fmField(["FMT-TAGS"]))}{/if}
        <span class="meta-k">Actualizado</span>
        <div class="meta-when">
          <input class="meta-v" value={toISOStr(fm.timestamp)} placeholder="—" spellcheck="false" oninput={(e) => reserialize({ timestamp: e.currentTarget.value })} />
          <button class="mini" onclick={() => reserialize({ timestamp: new Date().toISOString().replace(/\.\d+Z$/, "Z") })}>hoy</button>
        </div>
        {#if fmField(["FMT-TS"])}{@render inlineNote(fmField(["FMT-TS"]))}{/if}
        <span class="meta-k">Recurso</span>
        <input class="meta-v" value={(fm.resource as string) || ""} placeholder="enlace (opcional)" spellcheck="false" oninput={(e) => reserialize({ resource: e.currentTarget.value })} />
      </div>
      <details class="fm-extra wiki" open={extras.length > 0}>
        <summary>Campos adicionales</summary>
        <textarea class="fld" rows={Math.max(2, extras.length + 1)} spellcheck="false" placeholder="clave: valor" value={extrasToYAML(fm)}></textarea>
      </details>
    </div>
    <div class="body-wrap">
      <textarea class="body-edit" spellcheck="false" placeholder="# Cuerpo markdown…" value={pf?.body ?? ""} oninput={(e) => reserialize({}, e.currentTarget.value)}></textarea>
    </div>
    {@render noteBlock("Sugerencias", bodyIssues)}
  </div>
{/snippet}

{#snippet inlineNote(c: { level: string; msg: string } | null)}
  {#if c}<div class="fld-note {c.level}"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">{@html noteIcon(c.level)}</svg><span>{c.msg}</span></div>{/if}
{/snippet}

{#snippet reservedView()}
  {@const cur = $current!}
  {@const bn = basename(cur)}
  <div>
    <div class="reserved-note">
      {#if bn === "index.md" && /^tags\//.test(cur)}
        <b>{cur}</b> — índice por tag, generado escaneando el frontmatter. No lo edites a mano: se regenera desde los <span class="mono">tags</span> de los concepts.
        <div class="acts"><button class="btn-ghost" onclick={() => toast("Regenerar índices por tag: usa la CLI (lodestar tags).")}>Regenerar índices por tag</button></div>
      {:else if bn === "index.md"}
        <b>index.md</b> — listado del directorio <b>{dirOf(cur) || "raíz"}</b> para divulgación progresiva (§6). No lleva frontmatter.
        <div class="acts"><button class="btn-ghost" onclick={() => toast("Regenerar índice: usa la CLI (lodestar index).")}>Regenerar desde los concepts</button></div>
      {:else}
        <b>log.md</b> — historial de cambios (§7). Entradas agrupadas por fecha ISO, más recientes primero.
        <div class="acts"><button class="btn-ghost" onclick={() => toast("Añadir entrada: edita log.md directamente.")}>Añadir entrada (hoy)</button></div>
      {/if}
    </div>
    <textarea class="raw-edit" spellcheck="false" style="margin:0 16px;width:calc(100% - 32px)" bind:value={draft} oninput={scheduleWrite}></textarea>
  </div>
{/snippet}

{#snippet diffView()}
  <div class="diffmode">
    {#if "na" in diffModel}
      <div class="vd-empty">El modo Cambios no aplica a esta página.</div>
    {:else if "empty" in diffModel}
      <div class="vd-empty"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M20 6L9 17l-5-5"/></svg>Sin cambios desde la última versión{#if diffModel.tip}{" · "}<span class="mono">{diffModel.tip.id}</span>{/if}.<br /><span style="color:var(--faint);font-size:12px">Edita la página y vuelve aquí para ver el diff antes de guardar.</span></div>
    {:else}
      <div class="vd-summary"><span>Cambios sin guardar en esta página, respecto a la última versión.</span></div>
      {#if diffModel.d.statusChanges.length}
        <div class="vd-statusline">
          {#each diffModel.d.statusChanges as s}{@render lifecycle(s)}{/each}
        </div>
      {/if}
      {#if diffModel.d.files[0]}{@render diffCard(diffModel.d.files[0])}{/if}
    {/if}
  </div>
{/snippet}

{#snippet lifecycle(s: { path: string; from: string | null; to: string | null })}
  <div class="vd-lc">
    <span style="color:var(--faint);font-size:11px;text-transform:uppercase;letter-spacing:.06em;font-weight:600">Ciclo de vida</span>
    {#if s.from}<span class="stp"><span class="st-dot st-{String(s.from).toLowerCase()}"></span>{statusLabel(s.from)}</span>{:else}<span class="stp" style="color:var(--faint);font-weight:600">página nueva</span>{/if}
    <span class="arrow">→</span>
    {#if s.to}<span class="stp"><span class="st-dot st-{String(s.to).toLowerCase()}"></span>{statusLabel(s.to)}</span>{:else}<span class="stp" style="color:var(--faint);font-weight:600">página nueva</span>{/if}
    <span style="margin-left:auto;color:var(--muted)">{(parseFile(files, s.path).fm?.title as string) || titleFromPath(s.path)}</span>
  </div>
{/snippet}

{#snippet diffCard(f: FileDiff)}
  <div class="vd-card">
    <div class="vd-card-h"><span class="nm">{pageTitleRaw(f)}</span><span class="pth">{f.path}</span><span class="vd-kind {f.kind}">{f.kind === "add" ? "nueva" : f.kind === "mod" ? "modificada" : "eliminada"}</span></div>
    {#if f.fm.length}
      <div class="vd-sect"><div class="vd-sect-h">Metadatos</div>
        {#each f.fm as c}{@render fieldRow(c)}{/each}
      </div>
    {/if}
    {#if f.linksAdd.length || f.linksRem.length}
      <div class="vd-sect"><div class="vd-sect-h">Enlaces · impacto en el grafo</div>
        {#each f.linksAdd as t}<div class="vd-link add"><span class="pm">+</span><span>enlaza a <b>{files[t] !== undefined ? (parseFile(files, t).fm?.title as string) || titleFromPath(t) : titleFromPath(t)}</b></span></div>{/each}
        {#each f.linksRem as t}<div class="vd-link rem"><span class="pm">−</span><span>ya no enlaza a <b>{files[t] !== undefined ? (parseFile(files, t).fm?.title as string) || titleFromPath(t) : titleFromPath(t)}</b></span></div>{/each}
      </div>
    {/if}
    {#if f.body.some((r) => r.t === "+" || r.t === "-")}
      <div class="vd-sect"><div class="vd-sect-h">Cuerpo</div>
        <div class="vd-body">
          {#each f.body as r}
            {#if r.t === "gap"}<div class="vd-ln gap">⋯ {r.n} línea{r.n! > 1 ? "s" : ""} sin cambios</div>
            {:else}<div class="vd-ln {r.t === '+' ? 'add' : r.t === '-' ? 'rem' : ''}"><span class="g">{r.t === "+" ? "+" : r.t === "-" ? "−" : ""}</span><span>{r.s || " "}</span></div>{/if}
          {/each}
        </div>
      </div>
    {/if}
  </div>
{/snippet}

{#snippet fieldRow(c: { key: string; from: string | null; to: string | null })}
  {@const KEY = { type: "tipo", title: "título", description: "descripción", resource: "recurso", tags: "etiquetas", timestamp: "actualizado", status: "estado" }}
  {@const k = (KEY as Record<string, string>)[c.key] || c.key}
  {@const f = (v: string | null) => (c.key === "status" ? statusLabel(v) : v)}
  <div class="vd-field"><span class="fk">{k}</span>
    {#if c.from == null}<span class="vd-new">{f(c.to)}</span>
    {:else if c.to == null}<span class="vd-old">{f(c.from)}</span>
    {:else}<span class="vd-old">{f(c.from)}</span><span class="vd-arrow">→</span><span class="vd-new">{f(c.to)}</span>{/if}
  </div>
{/snippet}
