// Búsqueda por frontmatter — port de tokenizeQuery/matchToken del prototipo. Filtra el árbol y atenúa
// el grafo con la MISMA semántica (Y implícito, `field:sub`, `field=exact`, `-neg`, `has:`/`no:`/
// `is:`/`body:`, y texto suelto = subcadena en nombre+frontmatter+cuerpo). Quirk conservado: los
// ficheros reservados se descartan antes de negar salvo `is:reserved`.

import { RESERVED, basename, parseFile } from "./okf";
import type { Analysis } from "./ipc/types";

export interface Token { neg: boolean; field: string | null; op: string | null; val: string }

export function tokenizeQuery(q: string): Token[] {
  const out: Token[] = [];
  const n = q.length;
  let i = 0;
  while (i < n) {
    while (i < n && /\s/.test(q[i])) i++;
    if (i >= n) break;
    let neg = false;
    if (q[i] === "-") { neg = true; i++; }
    let field: string | null = null, op: string | null = null, j = i;
    while (j < n && /[\w\-]/.test(q[j])) j++;
    if (j < n && (q[j] === ":" || q[j] === "=")) { field = q.slice(i, j); op = q[j]; i = j + 1; }
    let val = "";
    if (q[i] === '"') { i++; while (i < n && q[i] !== '"') val += q[i++]; if (i < n) i++; }
    else { while (i < n && !/\s/.test(q[i])) val += q[i++]; }
    if (val[0] === "!") { neg = !neg; val = val.slice(1); }
    out.push({ neg, field, op, val });
  }
  return out;
}

function fmGet(fm: Record<string, unknown>, key: string): unknown {
  return fm[key] !== undefined ? fm[key] : fm[key.toLowerCase()];
}
function fmPresent(fm: Record<string, unknown>, key: string): boolean {
  const v = fmGet(fm, key);
  return v !== undefined && v !== "" && !(Array.isArray(v) && v.length === 0);
}
function fieldMatch(raw: unknown, value: string, op: string | null): boolean {
  if (raw === undefined || raw === null) return false;
  const val = String(value).toLowerCase();
  if (Array.isArray(raw)) return raw.some((x) => (op === "=" ? String(x).toLowerCase() === val : String(x).toLowerCase().includes(val)));
  const s = String(raw).toLowerCase();
  return op === "=" ? s === val : s.includes(val);
}
function valueIncludes(raw: unknown, val: string): boolean {
  if (raw == null) return false;
  if (Array.isArray(raw)) return raw.some((x) => String(x).toLowerCase().includes(val));
  return String(raw).toLowerCase().includes(val);
}
function isPredicate(name: string, path: string, pf: ReturnType<typeof parseFile>, A: Analysis): boolean {
  switch (name) {
    case "orphan": return (A.orphans || []).includes(path);
    case "invalid": return ((A.perFile || {})[path] || []).some((c) => c.level === "err");
    case "reserved": return RESERVED.has(basename(path));
    case "linked": return ((A.inn || {})[path] || []).length > 0;
    case "accepted": case "draft": case "review": case "deprecated":
      return !!(pf.fm && String((pf.fm.status as string) || "").toLowerCase() === name);
    default: return false;
  }
}

export function matchToken(t: Token, path: string, files: Record<string, string>, A: Analysis): boolean {
  const pf = parseFile(files, path);
  const fm = (pf.fm || {}) as Record<string, unknown>;
  const reserved = RESERVED.has(basename(path));
  const val = (t.val || "").toLowerCase();
  const fieldName = t.field ? t.field.toLowerCase() : null;
  const isFieldToken = t.field && !["has", "no", "is", "body"].includes(fieldName!);
  if (reserved && (isFieldToken || fieldName === "has" || fieldName === "no" || (fieldName === "is" && val !== "reserved"))) return false;
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

export function matchFileQuery(path: string, tokens: Token[], files: Record<string, string>, A: Analysis): boolean {
  if (!tokens.length) return true;
  return tokens.every((t) => matchToken(t, path, files, A));
}
