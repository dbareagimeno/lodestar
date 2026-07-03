//! Fachada de escritorio (Tauri v2) — `ARCHITECTURE.md §7.1`.
//!
//! Shell **fino** sobre `lodestar-workspace`: **cero lógica OKF**. Registra la tabla de comandos
//! con los nombres congelados (§10 fila 7), empuja `BundleSnapshot` y reemite el bus `IndexEvent`
//! de la cache como evento `bundle:changed` (watcher + escrituras → UI en vivo).

// En release, sin consola en Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use lodestar_core::types::{Direction, FrontmatterPatch, RelPath};
use lodestar_store::Store;
use lodestar_workspace::{BundleSnapshot, Workspace};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

/// Estado de la app: el bundle abierto (uno por proceso, §12 lockfile).
#[derive(Default)]
struct AppState {
    ws: Mutex<Option<Workspace>>,
}

type CmdResult<T> = Result<T, String>;

/// Toma el lock del estado recuperándose del envenenamiento: un panic puntual dentro de un
/// comando no debe dejar la app permanentemente inerte.
fn lock_ws<'a>(state: &'a State<'_, AppState>) -> MutexGuard<'a, Option<Workspace>> {
    state.ws.lock().unwrap_or_else(|p| p.into_inner())
}

fn with_ws<T>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&Workspace) -> CmdResult<T>,
) -> CmdResult<T> {
    let guard = lock_ws(state);
    let ws = guard.as_ref().ok_or("no hay ningún bundle abierto")?;
    f(ws)
}

fn rp(s: &str) -> CmdResult<RelPath> {
    RelPath::new(s).map_err(|e| e.to_string())
}

/// Reemite el bus `IndexEvent` de la cache como `bundle:changed` (snapshot completo) a la webview.
///
/// Sostiene un `Weak<Store>`: si retuviera el `Arc`, el hilo mantendría vivo el `Store` viejo
/// (cuyo `Bus` contiene el `Sender`) y `rx.iter()` no terminaría jamás — cada reapertura de
/// bundle fugaría un hilo + una conexión SQLite, y los eventos encolados del bundle anterior
/// pisarían la UI con snapshots del bundle equivocado.
fn start_forwarder(app: AppHandle, store: &Arc<Store>) {
    let rx = store.subscribe();
    let weak: Weak<Store> = Arc::downgrade(store);
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            // Coalescing: drena la ráfaga pendiente (un switch/pull toca N ficheros y emite N
            // eventos) y computa UN solo snapshot en vez de N análisis completos consecutivos.
            while rx.try_recv().is_ok() {}
            // Si el bundle se cerró/reemplazó, el store ya no existe: este forwarder muere.
            let Some(store) = weak.upgrade() else { break };
            let b = store.bundle();
            let snap = BundleSnapshot {
                files: b.files().clone(),
                analysis: b.analyze().clone(),
                graph: b.graph_model(),
            };
            drop(store);
            if app.emit("bundle:changed", &snap).is_err() {
                break; // la app se está cerrando
            }
        }
    });
}

// --- comandos (nombres congelados, §7.1) -----------------------------------

#[tauri::command]
async fn open_bundle(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<BundleSnapshot> {
    let root = PathBuf::from(&path);
    // Sin esta comprobación, un path arbitrario del webview crearía `.lodestar/` donde caiga
    // e indexaría medio disco.
    if !root.join("index.md").is_file() && !root.join(".lodestar").is_dir() {
        return Err(format!(
            "{path} no es un bundle lodestar (falta index.md o .lodestar/)"
        ));
    }
    let ws = Workspace::open_live(&root).map_err(|e| e.to_string())?;
    let snap = ws.snapshot().map_err(|e| e.to_string())?;
    if let Some(store) = ws.cache() {
        start_forwarder(app, store);
    }
    *lock_ws(&state) = Some(ws);
    Ok(snap)
}

#[tauri::command]
async fn get_snapshot(state: State<'_, AppState>) -> CmdResult<BundleSnapshot> {
    with_ws(&state, |ws| ws.snapshot().map_err(|e| e.to_string()))
}

#[tauri::command]
async fn list_concepts(state: State<'_, AppState>) -> CmdResult<Value> {
    with_ws(&state, |ws| {
        let rows = ws.list_concepts().map_err(|e| e.to_string())?;
        serde_json::to_value(rows).map_err(|e| e.to_string())
    })
}

#[tauri::command]
async fn read_concept(state: State<'_, AppState>, path: String) -> CmdResult<String> {
    with_ws(&state, |ws| {
        ws.read_concept(&rp(&path)?).map_err(|e| e.to_string())
    })
}

#[tauri::command]
async fn write_concept(
    state: State<'_, AppState>,
    path: String,
    content: String,
    allow_nonconformant: Option<bool>,
) -> CmdResult<Value> {
    with_ws(&state, |ws| {
        let o = ws
            .write_concept(&rp(&path)?, &content, allow_nonconformant.unwrap_or(false))
            .map_err(|e| e.to_string())?;
        Ok(outcome_json(&o))
    })
}

#[tauri::command]
async fn create_concept(
    state: State<'_, AppState>,
    path: String,
    r#type: String,
    title: Option<String>,
    body: Option<String>,
    allow_nonconformant: Option<bool>,
) -> CmdResult<Value> {
    with_ws(&state, |ws| {
        let o = ws
            .create_concept(
                &rp(&path)?,
                &r#type,
                title.as_deref(),
                body.as_deref().unwrap_or("# Resumen\n"),
                allow_nonconformant.unwrap_or(false),
            )
            .map_err(|e| e.to_string())?;
        Ok(outcome_json(&o))
    })
}

#[tauri::command]
async fn update_frontmatter(
    state: State<'_, AppState>,
    path: String,
    patch: Value,
) -> CmdResult<Value> {
    with_ws(&state, |ws| {
        let patch = parse_patch(&patch)?;
        let o = ws
            .merge_frontmatter(&rp(&path)?, patch)
            .map_err(|e| e.to_string())?;
        Ok(outcome_json(&o))
    })
}

#[tauri::command]
async fn conformance(state: State<'_, AppState>) -> CmdResult<Value> {
    with_ws(&state, |ws| {
        let a = ws.analyze().map_err(|e| e.to_string())?;
        serde_json::to_value(a).map_err(|e| e.to_string())
    })
}

#[tauri::command]
async fn query(state: State<'_, AppState>, dsl: String) -> CmdResult<Value> {
    with_ws(&state, |ws| {
        let paths = ws.query(&dsl).map_err(|e| e.to_string())?;
        serde_json::to_value(paths).map_err(|e| e.to_string())
    })
}

#[tauri::command]
async fn backlinks(state: State<'_, AppState>, path: String) -> CmdResult<Value> {
    with_ws(&state, |ws| {
        let bl = ws.backlinks(&rp(&path)?).map_err(|e| e.to_string())?;
        serde_json::to_value(bl).map_err(|e| e.to_string())
    })
}

#[tauri::command]
async fn graph_model(state: State<'_, AppState>) -> CmdResult<Value> {
    with_ws(&state, |ws| {
        let g = ws.graph_model().map_err(|e| e.to_string())?;
        serde_json::to_value(g).map_err(|e| e.to_string())
    })
}

#[tauri::command]
async fn neighborhood(
    state: State<'_, AppState>,
    path: String,
    depth: Option<u32>,
    direction: Option<String>,
) -> CmdResult<Value> {
    with_ws(&state, |ws| {
        let dir = match direction.as_deref() {
            Some("in") => Direction::In,
            Some("both") => Direction::Both,
            _ => Direction::Out,
        };
        let n = ws
            .neighborhood(&rp(&path)?, depth.unwrap_or(1), dir)
            .map_err(|e| e.to_string())?;
        serde_json::to_value(n).map_err(|e| e.to_string())
    })
}

#[tauri::command]
async fn history(state: State<'_, AppState>, limit: Option<usize>) -> CmdResult<Value> {
    with_ws(&state, |ws| {
        let rows = ws.vcs_log(limit.unwrap_or(20)).map_err(|e| e.to_string())?;
        serde_json::to_value(rows).map_err(|e| e.to_string())
    })
}

#[tauri::command]
async fn branches(state: State<'_, AppState>) -> CmdResult<Value> {
    with_ws(&state, |ws| {
        let bs = ws.branches().map_err(|e| e.to_string())?;
        serde_json::to_value(bs).map_err(|e| e.to_string())
    })
}

#[tauri::command]
async fn diff_working(state: State<'_, AppState>) -> CmdResult<Value> {
    with_ws(&state, |ws| {
        let d = ws.diff_working().map_err(|e| e.to_string())?;
        serde_json::to_value(d).map_err(|e| e.to_string())
    })
}

#[tauri::command]
async fn commit(app: AppHandle, state: State<'_, AppState>, message: String) -> CmdResult<Value> {
    let result = with_ws(&state, |ws| {
        let o = ws.commit(&message).map_err(|e| e.to_string())?;
        Ok(json!({
            "sha": o.sha.as_str(),
            "conformance": {
                "hardFail": o.conformance.hard_fail,
                "warnCount": o.conformance.warn_count,
                "conform": o.conformance.conform,
            }
        }))
    })?;
    // El contrato (`EVENTS.vcsChanged` en types.ts) promete este evento tras cambios de vcs.
    let _ = app.emit("vcs:changed", &result);
    Ok(result)
}

fn outcome_json(o: &lodestar_core::types::WriteOutcome) -> Value {
    json!({
        "path": o.path.as_str(),
        "written": o.written,
        "rejected": o.rejected,
        "checks": o.checks,
        "bundleHardFail": o.bundle_hard_fail,
    })
}

fn parse_patch(v: &Value) -> CmdResult<FrontmatterPatch> {
    let obj = v.as_object().ok_or("patch debe ser un objeto")?;
    let mut map = std::collections::BTreeMap::new();
    for (k, val) in obj {
        let yaml = if val.is_null() {
            None
        } else {
            Some(json_to_yaml(val))
        };
        map.insert(k.clone(), yaml);
    }
    Ok(FrontmatterPatch(map))
}

fn json_to_yaml(v: &Value) -> serde_yaml::Value {
    match v {
        Value::Null => serde_yaml::Value::Null,
        Value::Bool(b) => serde_yaml::Value::Bool(*b),
        Value::Number(n) => serde_yaml::from_str(&n.to_string()).unwrap_or(serde_yaml::Value::Null),
        Value::String(s) => serde_yaml::Value::String(s.clone()),
        Value::Array(a) => serde_yaml::Value::Sequence(a.iter().map(json_to_yaml).collect()),
        Value::Object(o) => {
            let mut m = serde_yaml::Mapping::new();
            for (k, val) in o {
                m.insert(serde_yaml::Value::String(k.clone()), json_to_yaml(val));
            }
            serde_yaml::Value::Mapping(m)
        }
    }
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            open_bundle,
            get_snapshot,
            list_concepts,
            read_concept,
            write_concept,
            create_concept,
            update_frontmatter,
            conformance,
            query,
            backlinks,
            graph_model,
            neighborhood,
            history,
            branches,
            diff_working,
            commit,
        ])
        .run(tauri::generate_context!())
        .expect("error al arrancar lodestar-desktop");
}
