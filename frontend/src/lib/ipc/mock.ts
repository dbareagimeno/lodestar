// Mock IPC **solo para desarrollo en navegador** (`vite dev` fuera de Tauri). Siembra el mismo bundle
// que el prototipo y computa un análisis equivalente, para poder ver y comparar la UI sin el backend
// Rust. NO se usa en producción: `ipc/index.ts` sólo cae aquí cuando `import.meta.env.DEV` y no hay
// Tauri. La lógica autoritativa real vive en el core Rust; esto es un doble de pruebas fiel al
// prototipo (analyzeBundle/validateFile portados 1:1).

import {
  RESERVED,
  basename,
  dirOf,
  splitFront,
  parseFile,
  outLinks,
  isISO,
  titleFromPath,
} from "../okf";
import type {
  Analysis,
  BundleSnapshot,
  Check,
  CheckCode,
  Edge,
  GraphModel,
  GraphNode,
  RelPath,
  Severity,
  WriteOutcome,
} from "./types";

let files: Record<string, string> = {};

function chk(level: Severity, code: CheckCode, msg: string, targets: RelPath[] = []): Check {
  return { level, code, msg, targets };
}

function validateFile(map: Record<string, string>, path: string, ctx: { out: Record<string, string[]>; inn: Record<string, string[]>; inIndex: Set<string> }): Check[] {
  const bn = basename(path);
  const out: Check[] = [];
  const pf = parseFile(map, path);
  if (bn === "index.md") {
    const { fmText } = splitFront(map[path]);
    if (fmText && fmText.trim() !== "") {
      const isRoot = dirOf(path) === "";
      const okFM = isRoot && /^\s*okf_version\s*:/.test(fmText) && fmText.trim().split(/\r?\n/).length === 1;
      if (!okFM) out.push(chk("warn", "OKF-IDX", "Esta página de índice no debería llevar metadatos al inicio."));
    }
    return out;
  }
  if (bn === "log.md") {
    const bad = (map[path].match(/^##\s+(.+)$/gm) || []).filter((h) => !/^##\s+\d{4}-\d{2}-\d{2}\s*$/.test(h));
    if (bad.length) out.push(chk("warn", "OKF-LOG", "Las fechas del historial deben escribirse como AAAA-MM-DD."));
    return out;
  }
  if (pf.fmErr === "sin frontmatter") return [chk("err", "OKF-FM01", "Falta el bloque de metadatos al inicio de la página.")];
  if (pf.fmErr === "frontmatter sin cierre") return [chk("err", "OKF-FM02", "El bloque de metadatos no está cerrado.")];
  if (pf.fmErr) return [chk("err", "OKF-FM03", "Los metadatos tienen un error de formato: " + pf.fmErr)];
  const fm = pf.fm || {};
  if (!fm.type || String(fm.type).trim() === "") out.push(chk("err", "OKF-TYPE", "Falta indicar de qué tipo es esta página."));
  else out.push(chk("pass", "OKF-TYPE", "Es una página de tipo “" + fm.type + "”."));
  if (!fm.title) out.push(chk("info", "REC-TITLE", "Sin título: ponle un nombre legible."));
  if (!fm.description) out.push(chk("info", "REC-DESC", "Sin descripción: ayuda a encontrarla y a previsualizarla."));
  if (fm.tags && !Array.isArray(fm.tags)) out.push(chk("warn", "FMT-TAGS", "Las etiquetas deberían ir como una lista."));
  if (fm.timestamp && !isISO(fm.timestamp)) out.push(chk("warn", "FMT-TS", "La fecha no está en el formato estándar."));
  const o = ctx.out[path] || [];
  const dang = o.filter((t) => map[t] === undefined);
  if (dang.length)
    out.push(
      chk(
        "info",
        "LINK-STUB",
        dang.length + (dang.length === 1 ? " enlace lleva" : " enlaces llevan") + " a páginas que aún no existen: " + dang.map((t) => dispTitle(map, t)).join(", "),
        dang,
      ),
    );
  const rel = rawRelLinks(map, path);
  if (rel.length) out.push(chk("info", "LINK-REL", "Hay enlaces relativos; es mejor usar la ruta completa /…"));
  if ((ctx.inn[path] || []).length === 0 && !ctx.inIndex.has(path)) out.push(chk("info", "ORPHAN", "Ninguna otra página enlaza a esta."));
  if (!/^#{1,6}\s/m.test(pf.body || "")) out.push(chk("info", "BODY-STRUCT", "El cuerpo no tiene apartados; añade encabezados para organizarlo."));
  return out;
}

function rawRelLinks(map: Record<string, string>, path: string): string[] {
  const body = splitFront(map[path]).body || "";
  const res: string[] = [];
  const re = /\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(body))) {
    const h = m[1];
    if (/^\.{1,2}\//.test(h) && /\.md/.test(h)) res.push(h);
  }
  return res;
}

function dispTitle(map: Record<string, string>, path: string): string {
  const pf = parseFile(map, path);
  return (pf.fm && (pf.fm.title as string)) || titleFromPath(path);
}

export function analyzeMap(files: Record<string, string>): { analysis: Analysis; graph: GraphModel } {
  const concepts: string[] = [];
  const out: Record<string, string[]> = {};
  const inn: Record<string, string[]> = {};
  const dangling = new Set<string>();
  const inIndex = new Set<string>();
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
  concepts.forEach((p) => (inn[p] = inn[p] || []));
  for (const p of concepts) {
    for (const t of out[p]) {
      if (files[t] !== undefined && !RESERVED.has(basename(t))) (inn[t] = inn[t] || []).push(p);
      else if (files[t] !== undefined && basename(t) === "index.md") {
        /* index target: not a backlink */
      } else dangling.add(t);
    }
  }
  const perFile: Record<string, Check[]> = {};
  let hardFail = 0;
  let warnCount = 0;
  for (const path of Object.keys(files)) {
    perFile[path] = validateFile(files, path, { out, inn, inIndex });
    if (perFile[path].some((c) => c.level === "err")) hardFail++;
    warnCount += perFile[path].filter((c) => c.level === "warn").length;
  }
  const orphans = concepts.filter((p) => (inn[p] || []).length === 0 && !inIndex.has(p));

  // grafo
  const nodeMap = new Map<string, GraphNode>();
  const addNode = (id: string, ghost: boolean) => {
    if (RESERVED.has(basename(id))) return;
    if (!nodeMap.has(id)) {
      const pf = ghost ? null : parseFile(files, id);
      nodeMap.set(id, {
        id,
        ghost,
        type: (pf && (pf.fm?.type as string)) || null,
        status: (pf && (pf.fm?.status as string)) || null,
      });
    }
  };
  concepts.forEach((p) => addNode(p, false));
  const edges: Edge[] = [];
  concepts.forEach((p) => {
    (out[p] || []).forEach((t) => {
      if (RESERVED.has(basename(t))) return;
      const ghost = files[t] === undefined;
      addNode(t, ghost);
      edges.push({ source: p, target: t, dangling: ghost });
    });
  });

  const okfMatch = (files["index.md"] || "").match(/okf_version:\s*["']?([^"'\n]+)/);
  const analysis: Analysis = {
    concepts,
    out,
    inn,
    inIndex: [...inIndex],
    dangling: [...dangling],
    orphans,
    perFile,
    hardFail,
    warnCount,
    okfVersion: okfMatch ? okfMatch[1] : null,
  };
  return { analysis, graph: { nodes: [...nodeMap.values()], edges } };
}

function analyze(): { analysis: Analysis; graph: GraphModel } {
  return analyzeMap(files);
}

/** Conformidad de una instantánea arbitraria (para el motor de versiones). */
export function confOfMap(map: Record<string, string>): { errs: number; warns: number; conform: boolean } {
  const { analysis } = analyzeMap(map);
  return { errs: analysis.hardFail, warns: analysis.warnCount, conform: analysis.hardFail === 0 };
}

function snapshot(): BundleSnapshot {
  const { analysis, graph } = analyze();
  return { files: { ...files }, analysis, graph };
}

// ---- SEED (idéntico al prototipo) ----
export function SEED(): Record<string, string> {
  return {
    "index.md": `---
okf_version: "0.1"
---

# Bundle de ejemplo

Specs del servicio de autenticación. Edita, valida y exporta.

# Spec

* [Login de usuario](specs/auth-login.md) - Flujo y reglas del inicio de sesión.
* [Token de sesión](specs/session-token.md) - Formato y caducidad del token.

# API Endpoint

* [POST /login](api/login-endpoint.md) - Endpoint de autenticación.
`,
    "specs/auth-login.md": `---
type: Spec
title: Login de usuario
description: Flujo y reglas del inicio de sesión por contraseña.
tags: [auth, login]
status: accepted
timestamp: 2026-06-15T10:00:00Z
---

# Resumen

El usuario se autentica con email y contraseña contra
[POST /login](/api/login-endpoint.md), que emite un
[token de sesión](/specs/session-token.md).

# Reglas

* Tras 5 intentos fallidos se aplica [rate limiting](/specs/rate-limit.md).
* La contraseña nunca se registra en logs.

# Criterios de aceptación

- [ ] Credenciales válidas devuelven 200 y un token.
- [ ] Credenciales inválidas devuelven 401 sin filtrar cuál falló.
`,
    "specs/session-token.md": `---
type: Spec
title: Token de sesión
description: Formato, firma y caducidad del token emitido en el login.
tags: [auth, token]
status: review
timestamp: 2026-06-16T09:00:00Z
---

# Formato

Token opaco de 32 bytes, base64url. Lo consume
[POST /login](/api/login-endpoint.md) y lo valida el middleware.

# Caducidad

24 horas. Renovable mientras siga activo.
`,
    "api/login-endpoint.md": `---
type: API Endpoint
title: POST /login
description: Endpoint de autenticación por contraseña.
resource: https://api.example.com/login
tags: [api, auth]
status: draft
timestamp: 2026-06-16T09:30:00Z
---

# Resumen

Implementa el [login de usuario](/specs/auth-login.md) y emite un
[token de sesión](/specs/session-token.md).

# Schema

| Campo      | Tipo   | Descripción                |
|------------|--------|----------------------------|
| email      | string | Email del usuario.         |
| password   | string | Contraseña en claro (TLS). |

# Citations

[1] [Spec de login](/specs/auth-login.md)
`,
    "metrics/login-success-rate.md": `---
type: Metric
title: Tasa de éxito de login
description: Porcentaje de intentos de login que devuelven 200.
tags: [metric, auth]
status: draft
timestamp: 2026-06-14T08:00:00Z
---

# Definición

logins_200 / logins_total en una ventana de 5 minutos.

_Nota: este concept es huérfano (nadie lo enlaza) — el inspector lo marca._
`,
    "log.md": `# Update Log

## 2026-06-16
* **Update**: Añadido [token de sesión](/specs/session-token.md).
* **Creation**: Creado [POST /login](/api/login-endpoint.md).

## 2026-06-15
* **Initialization**: Bundle inicial con el [login de usuario](/specs/auth-login.md).
`,
  };
}

function ensureSeeded() {
  if (Object.keys(files).length === 0) files = SEED();
}

export function isMockActive(): boolean {
  // Sólo en dev y fuera de Tauri.
  const w = window as unknown as { __TAURI__?: unknown };
  return Boolean(import.meta.env?.DEV) && !w.__TAURI__;
}

export async function mockInvoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  ensureSeeded();
  switch (cmd) {
    case "get_snapshot":
    case "open_bundle":
    case "create_bundle":
      return snapshot();
    case "pick_folder":
      return "/ruta/al/workspace";
    case "read_concept":
      return files[args!.path as string] ?? "";
    case "write_concept": {
      const path = args!.path as string;
      files[path] = args!.content as string;
      const { analysis } = analyze();
      const outcome: WriteOutcome = {
        path,
        written: true,
        rejected: null,
        checks: analysis.perFile[path] ?? [],
        bundleHardFail: analysis.hardFail,
      };
      return outcome;
    }
    case "create_concept": {
      const path = args!.path as string;
      const type = (args!.type as string) ?? "";
      const title = (args!.title as string) || titleFromPath(path);
      const heading = type ? `# ${type} - ${title}` : `# ${title}`;
      files[path] =
        (args!.body as string) || `---\ntype: ${type}\n---\n\n${heading}\n`;
      return { path, written: true, rejected: null, checks: [], bundleHardFail: 0 } as WriteOutcome;
    }
    default:
      return snapshot();
  }
}

/** Estado en memoria para que la UI pueda leer/escribir directamente en dev. */
export function mockFiles(): Record<string, string> {
  ensureSeeded();
  return files;
}
