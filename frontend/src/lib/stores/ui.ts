// Estado de VISTA (no de datos): modo del editor, pestañas, raíles, tema, diálogos, overlays, toast.
// El prototipo lo tenía en variables globales + DOM manual; aquí son stores para que la vista Svelte
// reaccione. Los datos (ficheros/análisis/grafo) viven en `bundle.ts` (snapshot de Rust).

import { get, writable } from "svelte/store";
import { current, refreshSnapshot } from "./bundle";
import { writeConcept, createConcept as ipcCreateConcept } from "../ipc";
import { RESERVED, basename } from "../okf";

export type Mode = "preview" | "form" | "raw" | "diff";
export type ExplorerView = "list" | "map";
export type GraphScope = "bundle" | "neighbor";
export type MobileView = "explorer" | "editor" | "inspector";
export type DialogId = "new" | "help" | "keys" | "commit" | "confirm" | null;

export const mode = writable<Mode>("preview");
export const tabs = writable<string[]>([]);
export const explorerView = writable<ExplorerView>("list");
export const graphScope = writable<GraphScope>("bundle");
export const graphBigOpen = writable(false);
export const verOverlayOpen = writable(false);

export const railLeft = writable(false); // true = colapsado
export const railRight = writable(false);
export const clW = writable(292);
export const crW = writable(336);

export const theme = writable<"light" | "dark">((document.documentElement.dataset.theme as "light" | "dark") || "dark");
export const mobileView = writable<MobileView>("editor");

export const dialog = writable<DialogId>(null);
export const confirmState = writable<{ title: string; msg: string; onYes: () => void } | null>(null);
export const toastState = writable<{ msg: string; action?: { label: string; fn: () => void } } | null>(null);

let toastT: ReturnType<typeof setTimeout> | null = null;
export function toast(msg: string, action?: { label: string; fn: () => void }) {
  if (toastT) clearTimeout(toastT);
  toastState.set({ msg, action });
  toastT = setTimeout(() => toastState.set(null), action ? 5200 : 1900);
}

/** Selección/activación de una página: la abre en una pestaña y la marca activa. */
export function select(path: string) {
  const t = get(tabs);
  if (!t.includes(path)) tabs.set([...t, path]);
  current.set(path);
  mobileView.set("editor");
  document.body.dataset.view = "editor";
}
export const activate = select;

export function closeTab(id: string) {
  const t = get(tabs);
  const idx = t.indexOf(id);
  if (idx < 0) return;
  const next = t.slice();
  next.splice(idx, 1);
  tabs.set(next);
  if (get(current) === id) {
    current.set(next[idx] || next[idx - 1] || null);
  }
}

export function setMode(m: Mode) {
  mode.set(m);
}

export function toggleRail(side: "left" | "right") {
  if (side === "left") railLeft.update((v) => !v);
  else railRight.update((v) => !v);
}

export function openExplorerMap() {
  explorerView.set("map");
  document.body.dataset.explorer = "map";
  mobileView.set("explorer");
  document.body.dataset.view = "explorer";
}
export function closeExplorerMap() {
  explorerView.set("list");
  document.body.dataset.explorer = "list";
}

export function toggleTheme() {
  const t = get(theme) === "dark" ? "light" : "dark";
  theme.set(t);
  document.documentElement.dataset.theme = t;
}

export function openDialog(id: DialogId) {
  dialog.set(id);
}
export function closeDialog() {
  dialog.set(null);
}
export function confirmDlg(title: string, msg: string, onYes: () => void) {
  confirmState.set({ title, msg, onYes });
  dialog.set("confirm");
}

/** Escribe una página por el ÚNICO escritor (write_concept) y refresca el snapshot. */
export async function writeFile(path: string, content: string): Promise<void> {
  try {
    await writeConcept(path, content, true);
  } catch (e) {
    toast("Error al guardar: " + String(e));
  }
  await refreshSnapshot();
}

/** Crea una página nueva (o abre la existente) y la selecciona. */
export async function createConcept(rawPath: string, type: string) {
  let path = rawPath.trim();
  if (!path) return;
  if (!path.endsWith(".md")) path += ".md";
  try {
    await ipcCreateConcept(path, type);
  } catch (e) {
    toast("Error al crear: " + String(e));
    return;
  }
  await refreshSnapshot();
  closeDialog();
  mode.set("raw");
  select(path);
  toast("creado · " + path);
}

export function offerCreate(targetPath: string) {
  pendingNewPath.set(targetPath);
  dialog.set("new");
}
/** Ruta pre-rellenada al abrir el diálogo "Nueva página" desde un enlace fantasma. */
export const pendingNewPath = writable<string>("");

export function isReserved(path: string | null): boolean {
  return !!path && RESERVED.has(basename(path));
}
