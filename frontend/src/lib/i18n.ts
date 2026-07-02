// i18n de conformidad **keyed por código** (ARCHITECTURE.md §12, E8-H03).
//
// El core emite `code` + `targets` (estables); la UI localiza a partir del código. Aquí el catálogo
// español (locale por defecto; el usuario es hispanohablante). Añadir un locale = añadir un objeto
// con las mismas claves `CheckCode` — sin tocar el core ni las fachadas.

import type { CheckCode, Severity } from "./ipc/types";

/** Títulos cortos y estables por código OKF (para tooltips, filtros y agrupación en la UI). */
export const CHECK_TITLES_ES: Record<CheckCode, string> = {
  "OKF-FM01": "Faltan metadatos",
  "OKF-FM02": "Metadatos sin cerrar",
  "OKF-FM03": "Metadatos mal formados",
  "OKF-TYPE": "Falta el tipo",
  "REC-TITLE": "Sin título",
  "REC-DESC": "Sin descripción",
  "FMT-TAGS": "Etiquetas mal formateadas",
  "FMT-TS": "Fecha no estándar",
  "LINK-STUB": "Enlaces a páginas inexistentes",
  "LINK-REL": "Enlaces relativos",
  ORPHAN: "Página huérfana",
  "BODY-STRUCT": "Cuerpo sin apartados",
  "OKF-IDX": "Índice con metadatos",
  "OKF-LOG": "Fechas del historial",
  "OKF-CONFLICT": "Conflicto de merge sin resolver",
};

/** Etiqueta legible de una severidad. */
export const SEVERITY_LABELS_ES: Record<Severity, string> = {
  pass: "OK",
  info: "Sugerencia",
  warn: "Aviso",
  err: "Error",
};

/** Título localizado de un código (fallback: el propio código). */
export function checkTitle(code: CheckCode): string {
  return CHECK_TITLES_ES[code] ?? code;
}

/** Etiqueta localizada de una severidad. */
export function severityLabel(level: Severity): string {
  return SEVERITY_LABELS_ES[level] ?? level;
}
