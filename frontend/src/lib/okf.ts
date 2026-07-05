// Helpers OKF de **presentación**, portados 1:1 del prototipo (prototype/index.html).
//
// Ojo: la lógica autoritativa (conformidad, grafo, query) vive en el core Rust y llega en el
// `BundleSnapshot`. Estos helpers son SOLO para renderizar la vista (parsear frontmatter de un `.md`
// para pintar título/tipo/estado, resolver enlaces del cuerpo, formatear fechas, etc.). Mantienen la
// semántica del prototipo para que el aspecto sea idéntico.

export const RESERVED = new Set(["index.md", "log.md"]);
export const KNOWN_FM = ["type", "title", "description", "resource", "tags", "timestamp", "status"];
export const STATUS_LABEL: Record<string, string> = {
  "": "Sin estado",
  draft: "Borrador",
  review: "En revisión",
  accepted: "Aceptada",
  deprecated: "Obsoleta",
};

export type Fm = Record<string, unknown>;
export interface ParsedFile {
  reserved?: boolean;
  fm: Fm | null;
  fmErr?: string;
  fmText?: string | null;
  body: string;
  openFront?: boolean;
}

export function basename(p: string): string {
  return p.split("/").pop() ?? p;
}
export function dirOf(p: string): string {
  const i = p.lastIndexOf("/");
  return i < 0 ? "" : p.slice(0, i + 1);
}
export function conceptId(p: string): string {
  return p.replace(/\.md$/, "");
}

export function splitFront(raw: string): { fmText: string | null; body: string; openFront?: boolean } {
  if (raw.startsWith("---")) {
    const m = raw.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/);
    if (m) return { fmText: m[1], body: m[2] ?? "" };
    return { fmText: null, body: raw, openFront: true };
  }
  return { fmText: "", body: raw };
}

// mini-YAML del prototipo (fallback sin js-yaml): suficiente para el frontmatter OKF plano.
export function miniYAML(t: string): Fm {
  const o: Fm = {};
  t.split(/\r?\n/).forEach((line) => {
    const m = line.match(/^([A-Za-z0-9_\-]+):\s*(.*)$/);
    if (!m) return;
    let v: unknown = m[2].trim();
    if (/^\[.*\]$/.test(v as string)) {
      v = (v as string)
        .slice(1, -1)
        .split(",")
        .map((s) => s.trim().replace(/^["']|["']$/g, ""))
        .filter(Boolean);
    } else v = (v as string).replace(/^["']|["']$/g, "");
    o[m[1]] = v;
  });
  return o;
}

export function parseYAML(text: string | null): { ok: boolean; data: Fm | null; err?: string } {
  if (text == null) return { ok: false, data: null, err: "frontmatter sin cierre" };
  if (text.trim() === "") return { ok: true, data: {} };
  try {
    return { ok: true, data: miniYAML(text) };
  } catch {
    return { ok: false, data: null, err: "YAML inválido" };
  }
}

export function parseFile(files: Record<string, string>, path: string): ParsedFile {
  const raw = files[path] ?? "";
  const { fmText, body, openFront } = splitFront(raw);
  if (RESERVED.has(basename(path))) return { reserved: true, body: raw, fm: null };
  if (openFront) return { fm: null, fmErr: "frontmatter sin cierre", body };
  if (fmText === "") return { fm: null, fmErr: "sin frontmatter", body };
  const p = parseYAML(fmText);
  if (!p.ok) return { fm: null, fmErr: p.err, body, fmText };
  return { fm: p.data, body, fmText };
}

export function dumpYAML(obj: Fm): string {
  return Object.entries(obj)
    .map(([k, v]) => {
      if (Array.isArray(v)) return `${k}: [${v.join(", ")}]`;
      return `${k}: ${v}`;
    })
    .join("\n");
}

export function buildRaw(fm: Fm, body: string): string {
  const ordered: Fm = {};
  KNOWN_FM.forEach((k) => {
    if (fm[k] !== undefined && fm[k] !== "" && !(Array.isArray(fm[k]) && (fm[k] as unknown[]).length === 0)) ordered[k] = fm[k];
  });
  Object.keys(fm).forEach((k) => {
    if (!(k in ordered) && fm[k] !== undefined && fm[k] !== "") ordered[k] = fm[k];
  });
  const y = dumpYAML(ordered);
  return `---\n${y}\n---\n\n${body.replace(/^\n+/, "")}`;
}

export function extrasToYAML(fm: Fm): string {
  const ex: Fm = {};
  Object.keys(fm).forEach((k) => {
    if (!KNOWN_FM.includes(k)) ex[k] = fm[k];
  });
  if (Object.keys(ex).length === 0) return "";
  return dumpYAML(ex);
}

export function titleFromPath(p: string): string {
  return conceptId(basename(p))
    .replace(/[-_]/g, " ")
    .replace(/\b\w/g, (c) => c.toUpperCase());
}

export function dispTitle(files: Record<string, string>, path: string): string {
  const pf = parseFile(files, path);
  return (pf.fm && (pf.fm.title as string)) || titleFromPath(path);
}

function normalize(p: string): string {
  const parts: string[] = [];
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

export function resolveLink(href: string, fromPath: string): string | null {
  if (/^[a-z]+:/i.test(href)) return null;
  if (href.startsWith("#")) return null;
  let h = href.split("#")[0].split("?")[0];
  if (!h) return null;
  if (h.endsWith("/")) h += "index.md";
  if (!/\.md$/.test(h)) return null;
  if (h.startsWith("/")) return h.slice(1);
  return normalize(dirOf(fromPath) + h);
}

const LINK_RE = /\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g;
export function outLinks(path: string, body: string): string[] {
  const set = new Set<string>();
  let m: RegExpExecArray | null;
  LINK_RE.lastIndex = 0;
  while ((m = LINK_RE.exec(body))) {
    const t = resolveLink(m[1], path);
    if (t && t !== path) set.add(t);
  }
  return [...set];
}

export function isISO(s: unknown): boolean {
  if (s instanceof Date) return !isNaN(s.getTime());
  return typeof s === "string" && !isNaN(Date.parse(s)) && /\d{4}-\d{2}-\d{2}/.test(s);
}
export function toISOStr(v: unknown): string {
  if (v instanceof Date) return isNaN(v.getTime()) ? "" : v.toISOString().replace(/\.\d+Z$/, "Z");
  return v == null ? "" : String(v);
}
export function fmtWhen(v: unknown): string {
  const d = v instanceof Date ? v : new Date(v as string);
  if (isNaN(d.getTime())) return String(v);
  return d.toLocaleDateString("es", { day: "numeric", month: "short", year: "numeric" });
}

// tinte estelar determinista por tipo (idéntico al prototipo)
export function starTint(t: string | null | undefined): string {
  if (!t) return "#d9cfa6";
  let h = 0;
  for (let i = 0; i < t.length; i++) h = (h * 31 + t.charCodeAt(i)) % 360;
  return `hsl(${h} 30% 78%)`;
}

// markdown mínimo del prototipo (miniMd) — el prototipo usa `marked` si está, con fallback a esto.
function esc(s: string): string {
  return String(s == null ? "" : s).replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]!);
}
export function miniMd(md: string): string {
  let h = esc(md);
  h = h
    .replace(/^### (.*)$/gm, "<h3>$1</h3>")
    .replace(/^## (.*)$/gm, "<h2>$1</h2>")
    .replace(/^# (.*)$/gm, "<h1>$1</h1>");
  h = h.replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>").replace(/`([^`]+)`/g, "<code>$1</code>");
  h = h.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2">$1</a>');
  h = h.replace(/^\s*[-*] (.*)$/gm, "<li>$1</li>");
  // agrupa <li> contiguos en <ul>
  h = h.replace(/(?:<li>.*<\/li>\n?)+/g, (m) => `<ul>${m.replace(/\n/g, "")}</ul>`);
  h = h
    .split(/\n{2,}/)
    .map((b) => (/^<(h\d|ul|li|table|blockquote|pre)/.test(b.trim()) ? b : `<p>${b.replace(/\n/g, "<br>")}</p>`))
    .join("\n");
  return h;
}
