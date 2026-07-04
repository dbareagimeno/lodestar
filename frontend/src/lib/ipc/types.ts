// Contrato de tipos del IPC — espejo de `lodestar_core::types` (ARCHITECTURE.md §4.1, §8).
//
// NOTA: en producción este fichero se GENERA desde Rust con ts-rs/specta (E0-H04/E6-H03) y se marca
// "NO EDITAR". Aquí se mantiene a mano como contrato de referencia hasta que el generador esté cableado.

export type RelPath = string;

export type Severity = "pass" | "info" | "warn" | "err";

export type CheckCode =
  | "OKF-FM01" | "OKF-FM02" | "OKF-FM03" | "OKF-TYPE"
  | "REC-TITLE" | "REC-DESC" | "FMT-TAGS" | "FMT-TS"
  | "LINK-STUB" | "LINK-REL" | "ORPHAN" | "BODY-STRUCT"
  | "OKF-IDX" | "OKF-LOG" | "OKF-CONFLICT";

export interface Check {
  level: Severity;
  code: CheckCode;
  msg: string;
  targets: RelPath[];
}

export interface Analysis {
  concepts: RelPath[];
  out: Record<RelPath, RelPath[]>;
  inn: Record<RelPath, RelPath[]>;
  inIndex: RelPath[];
  dangling: RelPath[];
  orphans: RelPath[];
  perFile: Record<RelPath, Check[]>;
  hardFail: number;
  warnCount: number;
  okfVersion: string | null;
}

export interface GraphNode {
  id: RelPath;
  ghost: boolean;
  type: string | null;
  status: string | null;
}

export interface Edge {
  source: RelPath;
  target: RelPath;
  dangling: boolean;
}

export interface GraphModel {
  nodes: GraphNode[];
  edges: Edge[];
}

export interface ConceptSummary {
  path: RelPath;
  title: string;
  type: string | null;
  status: string | null;
  orphan: boolean;
  invalid: boolean;
}

export interface BundleSnapshot {
  files: Record<RelPath, string>;
  analysis: Analysis;
  graph: GraphModel;
}

export interface LinkRef {
  path: RelPath;
  href: string;
}

export interface Backlinks {
  inbound: LinkRef[];
  indexRefs: RelPath[];
  out: RelPath[];
  dangling: string[];
}

export interface WriteOutcome {
  path: RelPath;
  written: boolean;
  rejected: string | null;
  checks: Check[];
  bundleHardFail: number;
}

export interface Neighborhood {
  root: RelPath;
  nodes: GraphNode[];
  edges: Edge[];
}

export interface Branch {
  name: string;
  isHead: boolean;
  upstream: string | null;
  ahead: number;
  behind: number;
}

export interface CommitConformance {
  hardFail: number;
  warnCount: number;
  conform: boolean;
}

// Nombres de comando/evento congelados (§7.1, §10 fila 7). Fuente única compartida con Rust.
export const COMMANDS = {
  openBundle: "open_bundle",
  getSnapshot: "get_snapshot",
  listConcepts: "list_concepts",
  readConcept: "read_concept",
  writeConcept: "write_concept",
  createConcept: "create_concept",
  updateFrontmatter: "update_frontmatter",
  conformance: "conformance",
  query: "query",
  backlinks: "backlinks",
  graphModel: "graph_model",
  neighborhood: "neighborhood",
  history: "history",
  branches: "branches",
  diffWorking: "diff_working",
  commit: "commit",
} as const;

export const EVENTS = {
  bundleChanged: "bundle:changed",
  vcsChanged: "vcs:changed",
} as const;
