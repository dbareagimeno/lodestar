// Motor de versiones/historial — port del prototipo (git tras el vocabulario de lodestar). En el
// producto real estos datos vendrían de una IPC ampliada sobre git (history/diff por commit); aquí,
// para paridad de UI en dev, se computa en el cliente a partir de instantáneas (idéntico al
// prototipo). Fuera de dev el historial queda vacío.

import { writable, derived, get } from "svelte/store";
import { snapshot } from "./stores/bundle";
import { splitFront, parseFile, outLinks, basename, RESERVED, titleFromPath, STATUS_LABEL, toISOStr } from "./okf";

export interface Version {
  id: string;
  line: string;
  msg: string;
  author: string;
  time: number;
  errs: number;
  warns: number;
  conform: boolean;
  snapshot: Record<string, string>;
}
export const LINE = "principal";

export const versions = writable<Version[]>([]);

// ---- utilidades ----
function strHash(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0).toString(16).padStart(8, "0");
}
export function verId(snap: Record<string, string>, time: number): string {
  return strHash(Object.keys(snap).sort().join("|") + "#" + time).slice(0, 7);
}
export function isGenerated(p: string): boolean {
  return RESERVED.has(basename(p)) || /^tags\//.test(p);
}
function dispTitleSafe(files: Record<string, string>, p: string): string {
  return files[p] !== undefined ? (parseFile(files, p).fm?.title as string) || titleFromPath(p) : titleFromPath(p);
}
export function statusLabel(v: unknown): string {
  if (v == null || v === "") return "—";
  return STATUS_LABEL[String(v).toLowerCase()] || String(v);
}

// ---- diff consciente de OKF ----
function fmObj(raw: string): Record<string, unknown> {
  const s = splitFront(raw || "");
  if (s.fmText == null || s.fmText === "") return {};
  const p = parseFile({ x: raw }, "x");
  return p.fm || {};
}
function fmFmt(v: unknown): string {
  if (v == null) return "";
  if (Array.isArray(v)) return v.join(", ");
  return String(v);
}
export interface FieldChange { key: string; from: string | null; to: string | null }
export function fmDiff(aRaw: string, bRaw: string): FieldChange[] {
  const A = fmObj(aRaw), B = fmObj(bRaw);
  const keys = [...new Set([...Object.keys(A), ...Object.keys(B)])];
  const out: FieldChange[] = [];
  keys.forEach((k) => {
    const af = fmFmt(A[k]), bf = fmFmt(B[k]);
    if (af === bf) return;
    out.push({ key: k, from: A[k] === undefined ? null : af, to: B[k] === undefined ? null : bf });
  });
  const ord = ["status", "type", "title", "description", "tags", "timestamp", "resource"];
  out.sort((x, y) => ((ord.indexOf(x.key) + 1) || 99) - ((ord.indexOf(y.key) + 1) || 99));
  return out;
}
export interface DiffLine { t: " " | "+" | "-" | "gap"; s?: string; n?: number }
export function lineDiff(a: string, b: string): DiffLine[] {
  const A = String(a).split("\n"), B = String(b).split("\n");
  const n = A.length, m = B.length;
  const dp = Array.from({ length: n + 1 }, () => new Int32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) dp[i][j] = A[i] === B[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const out: DiffLine[] = [];
  let i = 0, j = 0;
  while (i < n && j < m) {
    if (A[i] === B[j]) { out.push({ t: " ", s: A[i] }); i++; j++; }
    else if (dp[i + 1][j] >= dp[i][j + 1]) { out.push({ t: "-", s: A[i] }); i++; }
    else { out.push({ t: "+", s: B[j] }); j++; }
  }
  while (i < n) out.push({ t: "-", s: A[i++] });
  while (j < m) out.push({ t: "+", s: B[j++] });
  return out;
}
export function collapseDiff(rows: DiffLine[]): DiffLine[] {
  const out: DiffLine[] = [];
  let i = 0;
  while (i < rows.length) {
    if (rows[i].t === " ") {
      let j = i;
      while (j < rows.length && rows[j].t === " ") j++;
      const run = j - i, keepTop = i > 0 ? 2 : 0, keepBot = j < rows.length ? 2 : 0;
      if (run > 4 && run - keepTop - keepBot > 0) {
        for (let k = 0; k < keepTop; k++) out.push(rows[i + k]);
        out.push({ t: "gap", n: run - keepTop - keepBot });
        for (let k = keepBot; k > 0; k--) out.push(rows[j - k]);
      } else for (let k = i; k < j; k++) out.push(rows[k]);
      i = j;
    } else { out.push(rows[i]); i++; }
  }
  return out;
}
function sortPaths(a: string, b: string): number {
  return a.localeCompare(b, undefined, { numeric: true });
}
export interface FileDiff {
  path: string; kind: "add" | "mod" | "remove"; fm: FieldChange[]; body: DiffLine[];
  linksAdd: string[]; linksRem: string[]; aRaw?: string; bRaw?: string;
}
export interface SnapDiff {
  files: FileDiff[]; gen: { path: string; kind: string }[];
  stats: { added: number; modified: number; removed: number };
  statusChanges: { path: string; from: string | null; to: string | null; kind: string }[];
}
export function diffSnap(a: Record<string, string>, b: Record<string, string>): SnapDiff {
  const keys = [...new Set([...Object.keys(a), ...Object.keys(b)])].sort(sortPaths);
  const fileList: FileDiff[] = [], gen: { path: string; kind: string }[] = [];
  let added = 0, modified = 0, removed = 0;
  const statusChanges: SnapDiff["statusChanges"] = [];
  keys.forEach((p) => {
    const av = a[p], bv = b[p];
    if (av === bv) return;
    const kind = av === undefined ? "add" : bv === undefined ? "remove" : "mod";
    if (isGenerated(p)) { gen.push({ path: p, kind }); return; }
    if (kind === "add") added++; else if (kind === "remove") removed++; else modified++;
    const fm = fmDiff(av || "", bv || "");
    const sc = fm.find((c) => c.key === "status"); if (sc) statusChanges.push({ path: p, from: sc.from, to: sc.to, kind });
    const body = collapseDiff(lineDiff(splitFront(av || "").body || "", splitFront(bv || "").body || ""));
    const la = av !== undefined ? outLinks(p, splitFront(av).body) : [];
    const lb = bv !== undefined ? outLinks(p, splitFront(bv).body) : [];
    fileList.push({ path: p, kind: kind as FileDiff["kind"], fm, body, linksAdd: lb.filter((x) => !la.includes(x)), linksRem: la.filter((x) => !lb.includes(x)), aRaw: av, bRaw: bv });
  });
  return { files: fileList, gen, stats: { added, modified, removed }, statusChanges };
}
export function diffChips(d: SnapDiff): { cls: string; t: string }[] {
  const c: { cls: string; t: string }[] = [];
  if (d.stats.added) c.push({ cls: "add", t: "+" + d.stats.added });
  if (d.stats.modified) c.push({ cls: "mod", t: "~" + d.stats.modified });
  if (d.stats.removed) c.push({ cls: "rem", t: "−" + d.stats.removed });
  d.statusChanges.forEach((s) => {
    if (s.from && s.to && s.from !== s.to) c.push({ cls: "st", t: statusLabel(s.from) + "→" + statusLabel(s.to) });
  });
  return c;
}
export function pageTitleRaw(f: FileDiff): string {
  const fm = fmObj(f.bRaw || f.aRaw || "");
  return (fm.title as string) || titleFromPath(f.path);
}
export function suggestMsg(d: SnapDiff, files: Record<string, string>): string {
  if (d.stats.added === 1 && d.stats.modified === 0 && d.stats.removed === 0) {
    const f = d.files.find((x) => x.kind === "add");
    return "Añade " + (f ? pageTitleRaw(f) : "una página");
  }
  if (d.statusChanges.length === 1 && d.statusChanges[0].to) {
    const s = d.statusChanges[0];
    return statusLabel(s.to) + ": " + dispTitleSafe(files, s.path);
  }
  const parts: string[] = [];
  if (d.stats.added) parts.push(d.stats.added + " nueva" + (d.stats.added > 1 ? "s" : ""));
  if (d.stats.modified) parts.push(d.stats.modified + " modificada" + (d.stats.modified > 1 ? "s" : ""));
  if (d.stats.removed) parts.push(d.stats.removed + " eliminada" + (d.stats.removed > 1 ? "s" : ""));
  return "Actualiza el espacio" + (parts.length ? " (" + parts.join(", ") + ")" : "");
}

export function tipVersion(): Version | null {
  const vs = get(versions);
  return vs.length ? vs[vs.length - 1] : null;
}
export function tipSnapshot(): Record<string, string> {
  const t = tipVersion();
  return t ? t.snapshot : {};
}

/** Nº de páginas (no generadas) con cambios sin guardar respecto a la última versión. */
export const pendingCount = derived([snapshot, versions], ([$s]) => {
  const files = $s?.files ?? {};
  const d = diffSnap(tipSnapshot(), files);
  return d.files.length;
});

// ---- carga inicial (dev: siembra idéntica al prototipo) ----
export async function loadVersions(): Promise<void> {
  const w = window as unknown as { __TAURI__?: unknown };
  if (get(versions).length) return;
  if (w.__TAURI__ || !import.meta.env?.DEV) return;
  const { SEED, confOfMap } = await import("./ipc/mock");
  const mkVer = (snap: Record<string, string>, meta: { author: string; t: string | number; msg: string }): Version => {
    const time = typeof meta.t === "number" ? meta.t : Date.parse(meta.t);
    const c = confOfMap(snap);
    return { id: verId(snap, time), line: LINE, msg: meta.msg, author: meta.author, time, errs: c.errs, warns: c.warns, conform: c.conform, snapshot: snap };
  };
  const S = SEED();
  const idxFull = S["index.md"];
  const idxNoApi = idxFull.replace("\n# API Endpoint\n\n* [POST /login](api/login-endpoint.md) - Endpoint de autenticación.\n", "");
  const idxLoginOnly = idxNoApi.replace("* [Token de sesión](specs/session-token.md) - Formato y caducidad del token.\n", "");
  const logFull = S["log.md"];
  const logNoApi = logFull.replace("* **Creation**: Creado [POST /login](/api/login-endpoint.md).\n", "");
  const logInit = logFull.replace(/## 2026-06-16[\s\S]*?\n\n/, "");
  const aLogin = "---\ntype: Spec\ntitle: Login de usuario\ndescription: Flujo y reglas del inicio de sesión por contraseña.\ntags: [auth, login]\nstatus: draft\ntimestamp: 2026-06-15T10:00:00Z\n---\n\n# Resumen\n\nEl usuario se autentica con email y contraseña.\n\n# Criterios de aceptación\n\n- [ ] Credenciales válidas devuelven 200 y un token.\n";
  const cLogin = S["specs/auth-login.md"].replace("status: accepted", "status: draft").replace("\n# Reglas\n\n* Tras 5 intentos fallidos se aplica [rate limiting](/specs/rate-limit.md).\n* La contraseña nunca se registra en logs.\n", "");
  const cEndpoint = "---\ntitle: POST /login\ndescription: Endpoint de autenticación por contraseña.\nresource: https://api.example.com/login\ntags: [api, auth]\nstatus: draft\ntimestamp: 2026-06-16T09:30:00Z\n---\n\n# Resumen\n\nImplementa el [login de usuario](/specs/auth-login.md).\n";
  const A = { "index.md": idxLoginOnly, "specs/auth-login.md": aLogin, "log.md": logInit };
  const B = { ...A, "index.md": idxNoApi, "specs/session-token.md": S["specs/session-token.md"], "log.md": logNoApi };
  const C = { ...B, "index.md": idxFull, "specs/auth-login.md": cLogin, "api/login-endpoint.md": cEndpoint, "log.md": logFull };
  const D = { ...S };
  versions.set([
    mkVer(A, { author: "Marta", t: "2026-06-15T10:05:00Z", msg: "Bundle inicial: login de usuario" }),
    mkVer(B, { author: "Dani", t: "2026-06-16T09:10:00Z", msg: "Especifica el token de sesión" }),
    mkVer(C, { author: "Dani", t: "2026-06-16T09:35:00Z", msg: "Borrador del endpoint POST /login" }),
    mkVer(D, { author: "Tú", t: "2026-06-16T12:10:00Z", msg: "Pone al día: endpoint conforme, login aceptado y métrica" }),
  ]);
}

// ---- guardar versión (commit) ----
function appendVersionLog(raw: string, msg: string): string {
  const today = new Date().toISOString().slice(0, 10);
  const bullet = "* **Versión**: " + msg;
  raw = raw || "# Update Log\n";
  if (raw.indexOf("## " + today) >= 0) return raw.replace("## " + today + "\n", "## " + today + "\n" + bullet + "\n");
  const m = raw.match(/^(#\s+.+\n+)([\s\S]*)$/);
  const section = "## " + today + "\n" + bullet + "\n\n";
  if (m) return m[1] + section + m[2];
  return "# Update Log\n\n" + section + raw;
}

export async function commitVersion(msg: string, opts?: { log?: boolean }): Promise<void> {
  const { toast } = await import("./stores/ui");
  const { refreshSnapshot } = await import("./stores/bundle");
  const { writeFile } = await import("./stores/ui");
  let files = get(snapshot)?.files ?? {};
  const d = diffSnap(tipSnapshot(), files);
  if (!d.files.length && !d.gen.length) {
    toast("no hay cambios que guardar");
    return;
  }
  const message = (msg || "").trim() || suggestMsg(d, files);
  if (opts?.log) {
    await writeFile("log.md", appendVersionLog(files["log.md"] || "", message));
    files = get(snapshot)?.files ?? files;
  }
  const w = window as unknown as { __TAURI__?: unknown };
  let conf = { errs: 0, warns: 0, conform: true };
  if (!w.__TAURI__ && import.meta.env?.DEV) {
    const { confOfMap } = await import("./ipc/mock");
    conf = confOfMap(files);
  }
  const snap = { ...files };
  const time = Date.parse("2026-06-16T12:20:00Z") + get(versions).length; // determinista en dev
  const v: Version = { id: verId(snap, time), line: LINE, msg: message, author: "Tú", time, errs: conf.errs, warns: conf.warns, conform: conf.conform, snapshot: snap };
  versions.update((vs) => [...vs, v]);
  await refreshSnapshot();
  toast("versión guardada · " + v.id + (v.conform ? "" : " · ⚠ no conforme"));
}

export { toISOStr };
