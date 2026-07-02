// Arnés diferencial JS-vs-Rust (E1-H18, ARCHITECTURE.md §12). Copia VERBATIM las funciones puras del
// prototipo (`prototype/index.html`), adaptando SOLO las dependencias globales (`window.jsyaml` → js-yaml,
// el `files` global → un módulo). El test Rust `tests/differential.rs` ejecuta esto sobre fixtures
// compartidas y compara su salida normalizada con `lodestar-core` — la red de seguridad de la paridad.
//
// Estas funciones son el ORÁCULO: definen el comportamiento esperado. Si Rust difiere, gana el prototipo.

import yaml from "js-yaml";

// --- globales del prototipo (los asigna analyzeFixture) -----------------------
let files = {};
let analysis = null;

const RESERVED = new Set(["index.md", "log.md"]);
const LINK_RE = /\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g;

// --- modelo (verbatim, con jsyaml→yaml) ---------------------------------------
function splitFront(raw) {
  if (raw.startsWith("---")) {
    const m = raw.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/);
    if (m) return { fmText: m[1], body: m[2] ?? "" };
    return { fmText: null, body: raw, openFront: true };
  }
  return { fmText: "", body: raw };
}
function parseYAML(text) {
  if (text == null) return { ok: false, data: null, err: "frontmatter sin cierre" };
  if (text.trim() === "") return { ok: true, data: {} };
  try {
    const d = yaml.load(text);
    return { ok: true, data: d && typeof d === "object" ? d : {} };
  } catch (e) {
    return { ok: false, data: null, err: e.reason || e.message || "YAML inválido" };
  }
}
function basename(p) {
  return p.split("/").pop();
}
function dirOf(p) {
  const i = p.lastIndexOf("/");
  return i < 0 ? "" : p.slice(0, i + 1);
}
function conceptId(p) {
  return p.replace(/\.md$/, "");
}
function parseFile(path) {
  const raw = files[path] ?? "";
  const { fmText, body, openFront } = splitFront(raw);
  if (RESERVED.has(basename(path))) {
    return { reserved: true, body: raw, fm: null };
  }
  if (openFront) return { fm: null, fmErr: "frontmatter sin cierre", body };
  if (fmText === "") return { fm: null, fmErr: "sin frontmatter", body };
  const p = parseYAML(fmText);
  if (!p.ok) return { fm: null, fmErr: p.err, body, fmText };
  return { fm: p.data, body, fmText };
}
function resolveLink(href, fromPath) {
  if (/^[a-z]+:/i.test(href)) return null;
  if (href.startsWith("#")) return null;
  let h = href.split("#")[0].split("?")[0];
  if (!h) return null;
  if (h.endsWith("/")) h += "index.md";
  if (!/\.md$/.test(h)) return null;
  let target;
  if (h.startsWith("/")) target = h.slice(1);
  else {
    const base = dirOf(fromPath);
    target = normalize(base + h);
  }
  return target;
}
function normalize(p) {
  const parts = [];
  p.split("/").forEach((seg) => {
    if (seg === "." || seg === "") return;
    if (seg === "..") {
      parts.pop();
      return;
    }
    parts.push(seg);
  });
  return parts.join("/");
}
function outLinks(path, body) {
  const set = new Set();
  let m;
  LINK_RE.lastIndex = 0;
  while ((m = LINK_RE.exec(body))) {
    const t = resolveLink(m[1], path);
    if (t && t !== path) set.add(t);
  }
  return [...set];
}
function rawRelLinks(path) {
  const body = splitFront(files[path]).body || "";
  const res = [];
  let m;
  LINK_RE.lastIndex = 0;
  while ((m = LINK_RE.exec(body))) {
    const h = m[1];
    if (/^\.{1,2}\//.test(h) && /\.md/.test(h)) res.push(h);
  }
  return res;
}
function isISO(s) {
  if (s instanceof Date) return !isNaN(s.getTime());
  return typeof s === "string" && !isNaN(Date.parse(s)) && /\d{4}-\d{2}-\d{2}/.test(s);
}
function titleFromPath(p) {
  return conceptId(basename(p))
    .replace(/[-_]/g, " ")
    .replace(/\b\w/g, (c) => c.toUpperCase());
}
function dispTitle(path) {
  const pf = parseFile(path);
  return (pf.fm && pf.fm.title) || titleFromPath(path);
}
function splitFront2(raw) {
  const s = splitFront(raw);
  return { fmText: s.fmText, body: s.body };
}
function chk(level, code, msg, targets) {
  return { level, code, msg, targets };
}

// --- conformidad + analyze (verbatim) -----------------------------------------
function analyzeBundle() {
  const concepts = [];
  const out = {};
  const inn = {};
  const dangling = new Set();
  const inIndex = new Set();
  for (const path of Object.keys(files)) {
    const bn = basename(path);
    const { body } = splitFront(files[path]);
    if (bn === "index.md") {
      outLinks(path, files[path]).forEach((t) => inIndex.add(t));
      continue;
    }
    if (bn === "log.md") continue;
    concepts.push(path);
    out[path] = outLinks(path, body);
  }
  concepts.forEach((p) => {
    inn[p] = inn[p] || [];
  });
  for (const p of concepts) {
    for (const t of out[p]) {
      if (files[t] !== undefined && !RESERVED.has(basename(t))) {
        (inn[t] = inn[t] || []).push(p);
      } else if (files[t] !== undefined && basename(t) === "index.md") {
        // enlaces a index.md no cuentan
      } else {
        dangling.add(t);
      }
    }
  }
  const perFile = {};
  let hardFail = 0;
  for (const path of Object.keys(files)) {
    perFile[path] = validateFile(path, { concepts, out, inn, inIndex, dangling });
    if (perFile[path].some((c) => c.level === "err")) hardFail++;
  }
  const orphans = concepts.filter(
    (p) => (inn[p] || []).length === 0 && !inIndex.has(p) && (out[p] || []).length >= 0,
  );
  analysis = { concepts, out, inn, dangling: [...dangling], inIndex, perFile, hardFail, orphans };
  return analysis;
}
function validateFile(path, ctx) {
  const bn = basename(path);
  const out = [];
  const pf = parseFile(path);
  if (bn === "index.md") {
    const { fmText } = splitFront2(files[path]);
    if (fmText && fmText.trim() !== "") {
      const isRoot = dirOf(path) === "";
      const okFM =
        isRoot && /^\s*okf_version\s*:/.test(fmText) && fmText.trim().split(/\r?\n/).length === 1;
      if (!okFM)
        out.push(chk("warn", "OKF-IDX", "Esta página de índice no debería llevar metadatos al inicio."));
    }
    return out;
  }
  if (bn === "log.md") {
    const bad = (files[path].match(/^##\s+(.+)$/gm) || []).filter(
      (h) => !/^##\s+\d{4}-\d{2}-\d{2}\s*$/.test(h),
    );
    if (bad.length)
      out.push(chk("warn", "OKF-LOG", "Las fechas del historial deben escribirse como AAAA-MM-DD."));
    return out;
  }
  if (pf.fmErr === "sin frontmatter") {
    out.push(chk("err", "OKF-FM01", "Falta el bloque de metadatos al inicio de la página."));
    return out;
  }
  if (pf.fmErr === "frontmatter sin cierre") {
    out.push(chk("err", "OKF-FM02", "El bloque de metadatos no está cerrado."));
    return out;
  }
  if (pf.fmErr) {
    out.push(chk("err", "OKF-FM03", "Los metadatos tienen un error de formato: " + pf.fmErr));
    return out;
  }
  const fm = pf.fm || {};
  if (!fm.type || String(fm.type).trim() === "") {
    out.push(chk("err", "OKF-TYPE", "Falta indicar de qué tipo es esta página."));
  } else out.push(chk("pass", "OKF-TYPE", "Es una página de tipo " + fm.type + "."));
  if (!fm.title) out.push(chk("info", "REC-TITLE", "Sin título: ponle un nombre legible."));
  if (!fm.description)
    out.push(chk("info", "REC-DESC", "Sin descripción: ayuda a encontrarla y a previsualizarla."));
  if (fm.tags && !Array.isArray(fm.tags))
    out.push(chk("warn", "FMT-TAGS", "Las etiquetas deberían ir como una lista."));
  if (fm.timestamp && !isISO(fm.timestamp))
    out.push(chk("warn", "FMT-TS", "La fecha no está en el formato estándar."));
  const o = ctx.out[path] || [];
  const dang = o.filter((t) => files[t] === undefined);
  if (dang.length)
    out.push(
      chk(
        "info",
        "LINK-STUB",
        dang.length + (dang.length === 1 ? " enlace" : " enlaces") + " a páginas inexistentes",
        dang,
      ),
    );
  const rel = rawRelLinks(path);
  if (rel.length) out.push(chk("info", "LINK-REL", "Hay enlaces relativos."));
  if ((ctx.inn[path] || []).length === 0 && !ctx.inIndex.has(path)) {
    out.push(chk("info", "ORPHAN", "Ninguna otra página enlaza a esta."));
  }
  if (!/^#{1,6}\s/m.test(pf.body || ""))
    out.push(chk("info", "BODY-STRUCT", "El cuerpo no tiene apartados."));
  return out;
}

// --- query (verbatim) ---------------------------------------------------------
function tokenizeQuery(q) {
  const out = [];
  const n = q.length;
  let i = 0;
  while (i < n) {
    while (i < n && /\s/.test(q[i])) i++;
    if (i >= n) break;
    let neg = false;
    if (q[i] === "-") {
      neg = true;
      i++;
    }
    let field = null,
      op = null,
      j = i;
    while (j < n && /[\w\-]/.test(q[j])) j++;
    if (j < n && (q[j] === ":" || q[j] === "=")) {
      field = q.slice(i, j);
      op = q[j];
      i = j + 1;
    }
    let val = "";
    if (q[i] === '"') {
      i++;
      while (i < n && q[i] !== '"') val += q[i++];
      if (i < n) i++;
    } else {
      while (i < n && !/\s/.test(q[i])) val += q[i++];
    }
    if (val[0] === "!") {
      neg = !neg;
      val = val.slice(1);
    }
    out.push({ neg, field, op, val });
  }
  return out;
}
function matchFileQuery(path, tokens, A) {
  if (!tokens.length) return true;
  const pf = parseFile(path);
  return tokens.every((t) => matchToken(t, path, pf, A));
}
function matchToken(t, path, pf, A) {
  const fm = pf.fm || {};
  const reserved = RESERVED.has(basename(path));
  const val = (t.val || "").toLowerCase();
  const fieldName = t.field ? t.field.toLowerCase() : null;
  const isFieldToken = t.field && !["has", "no", "is", "body"].includes(fieldName);
  if (
    reserved &&
    (isFieldToken ||
      fieldName === "has" ||
      fieldName === "no" ||
      (fieldName === "is" && val !== "reserved"))
  )
    return false;
  let res = false;
  if (t.field) {
    if (fieldName === "has") res = fmPresent(fm, t.val);
    else if (fieldName === "no") res = !fmPresent(fm, t.val);
    else if (fieldName === "is") res = isPredicate(val, path, pf, A);
    else if (fieldName === "body") res = (pf.body || "").toLowerCase().includes(val);
    else res = fieldMatch(fmGet(fm, t.field), t.val, t.op);
  } else {
    if (basename(path).toLowerCase().includes(val)) res = true;
    else if (Object.values(fm).some((v) => valueIncludes(v, val))) res = true;
    else res = (pf.body || "").toLowerCase().includes(val);
  }
  return t.neg ? !res : res;
}
function fmGet(fm, key) {
  return fm[key] !== undefined ? fm[key] : fm[key.toLowerCase()];
}
function fmPresent(fm, key) {
  const v = fmGet(fm, key);
  return v !== undefined && v !== "" && !(Array.isArray(v) && v.length === 0);
}
function fieldMatch(raw, value, op) {
  if (raw === undefined || raw === null) return false;
  const val = String(value).toLowerCase();
  if (Array.isArray(raw))
    return raw.some((x) =>
      op === "=" ? String(x).toLowerCase() === val : String(x).toLowerCase().includes(val),
    );
  const s = String(raw).toLowerCase();
  return op === "=" ? s === val : s.includes(val);
}
function valueIncludes(raw, val) {
  if (raw == null) return false;
  if (Array.isArray(raw)) return raw.some((x) => String(x).toLowerCase().includes(val));
  return String(raw).toLowerCase().includes(val);
}
function isPredicate(name, path, pf, A) {
  switch (name) {
    case "orphan":
      return (A.orphans || []).includes(path);
    case "invalid":
      return ((A.perFile || {})[path] || []).some((c) => c.level === "err");
    case "reserved":
      return RESERVED.has(basename(path));
    case "linked":
      return ((A.inn || {})[path] || []).length > 0;
    case "accepted":
    case "draft":
    case "review":
    case "deprecated":
      return !!(pf.fm && String(pf.fm.status || "").toLowerCase() === name);
    default:
      return false;
  }
}

// --- generadores (parte pura, verbatim) ---------------------------------------
function sortPaths(a, b) {
  return a.localeCompare(b, undefined, { numeric: true });
}
function slugifyTag(t) {
  let s = String(t).toLowerCase().trim().normalize("NFC");
  s = s.replace(/[\/\\'"]+/g, "-").replace(/\s+/g, "-");
  s = s.replace(/[^\p{L}\p{N}._-]+/gu, "-");
  s = s.replace(/-+/g, "-").replace(/^[-.]+|[-.]+$/g, "");
  return s || "tag";
}
function genIndex(dir) {
  const here = Object.keys(files).filter(
    (p) => dirOf(p) === dir && !RESERVED.has(basename(p)),
  );
  const subdirs = new Set();
  Object.keys(files).forEach((p) => {
    if (p.startsWith(dir) && p !== dir) {
      const rest = p.slice(dir.length);
      const seg = rest.split("/")[0];
      if (rest.includes("/")) subdirs.add(seg + "/");
    }
  });
  let out = `# ${dir ? dir.replace(/\/$/, "") : "Bundle"}\n\n`;
  if (dir === "") out = `---\nokf_version: "0.1"\n---\n\n# Bundle\n\n`;
  const byType = {};
  here.sort().forEach((p) => {
    const pf = parseFile(p);
    const t = (pf.fm && pf.fm.type) || "Concept";
    (byType[t] = byType[t] || []).push(p);
  });
  Object.keys(byType)
    .sort()
    .forEach((t) => {
      out += `# ${t}\n\n`;
      byType[t].forEach((p) => {
        const pf = parseFile(p);
        const ti = (pf.fm && pf.fm.title) || conceptId(basename(p));
        const de = (pf.fm && pf.fm.description) || "";
        out += `* [${ti}](${basename(p)})${de ? ` - ${de}` : ""}\n`;
      });
      out += "\n";
    });
  if ([...subdirs].length) {
    out += `# Subdirectorios\n\n`;
    [...subdirs].sort().forEach((s) => {
      out += `* [${s}](${s})\n`;
    });
    out += "\n";
  }
  return out.replace(/\n+$/, "\n");
}
function genTagIndexes() {
  const a = analyzeBundle();
  const tagMap = {};
  a.concepts.forEach((p) => {
    const pf = parseFile(p);
    const tg = pf.fm && pf.fm.tags;
    if (!tg) return;
    const arr = Array.isArray(tg) ? tg : [tg];
    arr.forEach((raw) => {
      const t = String(raw).trim();
      if (!t) return;
      (tagMap[t] = tagMap[t] || new Set()).add(p);
    });
  });
  const tags = Object.keys(tagMap).sort((x, y) => x.localeCompare(y));
  const existing = Object.keys(files).filter(
    (p) => p === "tags/index.md" || /^tags\/[^/]+\/index\.md$/.test(p),
  );
  const writes = {};
  if (tags.length === 0) {
    return { writes, deletes: existing.sort() };
  }
  const slugByTag = {};
  const used = new Set();
  tags.forEach((t) => {
    let b = slugifyTag(t),
      s = b,
      i = 2;
    while (used.has(s)) {
      s = b + "-" + i;
      i++;
    }
    used.add(s);
    slugByTag[t] = s;
  });
  tags.forEach((t) => {
    const slug = slugByTag[t];
    const items = [...tagMap[t]].sort(sortPaths).map((p) => {
      const pf = parseFile(p);
      const ti = (pf.fm && pf.fm.title) || conceptId(basename(p));
      const de = (pf.fm && pf.fm.description) || "";
      return `* [${ti}](/${p})${de ? ` - ${de}` : ""}`;
    });
    writes[`tags/${slug}/index.md`] = `# ${t}\n\n${items.join("\n")}\n`;
  });
  const rootItems = tags.map((t) => {
    const n = tagMap[t].size;
    return `* [${t}](${slugByTag[t]}/) - ${n} concept${n !== 1 ? "s" : ""}`;
  });
  writes["tags/index.md"] = `# Tags\n\n${rootItems.join("\n")}\n`;
  const deletes = existing.filter((p) => !(p in writes)).sort();
  return { writes, deletes };
}

// --- grafo (modelo, sin física) -----------------------------------------------
function graphModel() {
  const a = analysis || analyzeBundle();
  const nodeSet = new Map(); // id -> ghost
  const add = (id, ghost) => {
    if (RESERVED.has(basename(id))) return;
    if (!nodeSet.has(id)) nodeSet.set(id, ghost);
  };
  a.concepts.forEach((p) => add(p, false));
  const edges = [];
  a.concepts.forEach((p) => {
    (a.out[p] || []).forEach((t) => {
      if (RESERVED.has(basename(t))) return;
      const ghost = files[t] === undefined;
      add(t, ghost);
      edges.push({ source: p, target: t, dangling: ghost });
    });
  });
  const nodes = [...nodeSet.entries()].map(([id, ghost]) => ({ id, ghost }));
  return { nodes, edges };
}

// --- driver: salida normalizada para comparar con Rust ------------------------
function sortStrs(arr) {
  return [...arr].sort();
}

export function analyzeFixture(input) {
  files = input.files;
  const queries = input.queries || [];
  const a = analyzeBundle();

  const out = {};
  for (const p of Object.keys(a.out)) out[p] = sortStrs(a.out[p]);
  const inn = {};
  for (const p of Object.keys(a.inn)) inn[p] = sortStrs(a.inn[p]);
  const perFile = {};
  for (const p of Object.keys(a.perFile)) {
    perFile[p] = a.perFile[p]
      // OKF-CONFLICT no existe en el prototipo (es una adición ratificada del core); fuera de la comparación.
      .filter((c) => c.code !== "OKF-CONFLICT")
      .map((c) => `${c.level}:${c.code}`)
      .sort();
  }
  // warnCount = total de checks 'warn' (todos locales), igual que el core.
  let warnCount = 0;
  for (const p of Object.keys(a.perFile))
    warnCount += a.perFile[p].filter((c) => c.level === "warn").length;

  const query = {};
  for (const q of queries) {
    const tokens = tokenizeQuery(q.trim());
    query[q] = sortStrs(Object.keys(files).filter((p) => matchFileQuery(p, tokens, a)));
  }

  return {
    concepts: sortStrs(a.concepts),
    out,
    inn,
    inIndex: sortStrs([...a.inIndex]),
    dangling: sortStrs(a.dangling),
    orphans: sortStrs(a.orphans),
    perFile,
    hardFail: a.hardFail,
    warnCount,
    query,
    genIndexRoot: genIndex(""),
    genTagIndexes: genTagIndexes(),
    graph: (() => {
      const g = graphModel();
      return {
        nodes: g.nodes.slice().sort((x, y) => (x.id < y.id ? -1 : x.id > y.id ? 1 : 0)),
        edges: g.edges
          .slice()
          .sort((x, y) =>
            x.source !== y.source
              ? x.source < y.source
                ? -1
                : 1
              : x.target < y.target
                ? -1
                : x.target > y.target
                  ? 1
                  : 0,
          ),
      };
    })(),
  };
}
