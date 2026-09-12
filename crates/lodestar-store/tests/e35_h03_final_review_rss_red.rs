//! Reproducción independiente del C4: `peak_rss_bytes` necesita evidencia de muestreo dentro de
//! cada ventana de fase, no cuatro lecturas puntuales al final de cada fase ni un high-water global.

use lodestar_store::Store;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const STORE_SOURCE: &str = include_str!("../src/lib.rs");

fn failpoint_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct FailpointEnv;

impl Drop for FailpointEnv {
    fn drop(&mut self) {
        std::env::remove_var("LODESTAR_H03_FAILPOINT");
    }
}

fn wait_for(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "C4: no apareció el seam de inventario {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn function_scope<'a>(source: &'a str, start: &str, next: &str) -> &'a str {
    let start_at = source
        .find(start)
        .unwrap_or_else(|| panic!("C4 anti-vacuidad: falta función inicial {start}"));
    let tail = &source[start_at..];
    let end_at = tail
        .find(next)
        .unwrap_or_else(|| panic!("C4 anti-vacuidad: falta límite final {next}"));
    &tail[..end_at]
}

fn marker_position(scope: &str, marker: &str) -> Result<usize, String> {
    let mut positions = scope.match_indices(marker).map(|(position, _)| position);
    let first = positions
        .next()
        .ok_or_else(|| format!("falta marcador requerido `{marker}`"))?;
    if positions.next().is_some() {
        return Err(format!("marcador ambiguo `{marker}`"));
    }
    Ok(first)
}

/// Comprueba causalidad, no mera presencia: ambos inicios deben preceder a todo el setup/trabajo
/// que la fase afirma medir.
fn phase_starts_cover(
    scope: &str,
    duration_start: &str,
    rss_start: &str,
    covered_in_order: &[&str],
) -> Result<(), String> {
    let duration_at = marker_position(scope, duration_start)?;
    let rss_at = marker_position(scope, rss_start)?;
    let mut previous = None;
    for marker in covered_in_order {
        let current = marker_position(scope, marker)?;
        if let Some(previous) = previous {
            if current <= previous {
                return Err(format!("setup fuera de orden en `{marker}`"));
            }
        }
        if duration_at >= current {
            return Err(format!(
                "el inicio de duración `{duration_start}` queda después de `{marker}`"
            ));
        }
        if rss_at >= current {
            return Err(format!(
                "el inicio RSS `{rss_start}` queda después de `{marker}`"
            ));
        }
        previous = Some(current);
    }
    Ok(())
}

fn write(root: &std::path::Path, path: &str, contents: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(target, contents).unwrap();
}

fn markdown(index: usize) -> String {
    format!(
        "---\ntitle: doc-{index}\n---\n\n# doc-{index}\n\nRSS phase work {index} {}\n",
        "x".repeat(32 * 1024)
    )
}

fn field<'a>(phase: &'a Value, name: &str) -> Option<&'a Value> {
    phase.get(name).or_else(|| {
        phase
            .get("rss_sampling")
            .and_then(|sampling| sampling.get(name))
    })
}

fn number(phase: &Value, names: &[&str]) -> Option<u64> {
    names
        .iter()
        .find_map(|name| field(phase, name).and_then(Value::as_u64))
}

#[test]
fn c4_peak_rss_exige_muestras_por_ventana_y_no_una_lectura_final_por_fase() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..24 {
        write(root.path(), &format!("docs/{index:02}.md"), markdown(index));
    }

    let store = Store::open(root.path()).unwrap();
    let report = store.rebuild().unwrap();
    let phases = report["phases"]
        .as_array()
        .expect("C4: phases debe ser un array");
    assert_eq!(
        phases.len(),
        4,
        "C4: el rebuild debe exponer las cuatro fases"
    );

    let mut phase_windows = Vec::new();
    for phase in phases {
        let name = phase["name"].as_str().expect("C4: nombre de fase");
        let phase_started = phase["phase_started_monotonic_ns"]
            .as_u64()
            .expect("C4: inicio monotónico de fase");
        let phase_finished = phase["phase_finished_monotonic_ns"]
            .as_u64()
            .expect("C4: fin monotónico de fase");
        let sample_count = number(phase, &["sample_count", "rss_sample_count"])
            .unwrap_or_else(|| panic!("C4: falta sample_count observable para la fase {name}"));
        let window_started = number(
            phase,
            &[
                "sample_window_started_monotonic_ns",
                "rss_sample_window_started_monotonic_ns",
                "window_started_monotonic_ns",
            ],
        )
        .unwrap_or_else(|| panic!("C4: falta inicio de ventana RSS para la fase {name}"));
        let window_finished = number(
            phase,
            &[
                "sample_window_finished_monotonic_ns",
                "rss_sample_window_finished_monotonic_ns",
                "window_finished_monotonic_ns",
            ],
        )
        .unwrap_or_else(|| panic!("C4: falta fin de ventana RSS para la fase {name}"));
        let peak = phase["peak_rss_bytes"]
            .as_u64()
            .unwrap_or_else(|| panic!("C4: falta peak_rss_bytes para la fase {name}"));
        assert!(peak > 0, "C4: RSS positiva para la fase {name}");
        assert!(
            sample_count > 0,
            "C4: al menos una muestra en la fase {name}"
        );
        assert!(
            window_started <= window_finished,
            "C4: ventana RSS válida para {name}: {window_started}..{window_finished}"
        );
        assert!(
            phase_started <= window_started && window_finished <= phase_finished,
            "C4: la ventana RSS debe estar contenida en la fase {name}: fase={phase_started}..{phase_finished}, ventana={window_started}..{window_finished}"
        );
        if name == "index" {
            assert!(
                phase["counters"]["documents_read"].as_u64().unwrap_or(0) > 0,
                "C4: la fase index debe contener trabajo real"
            );
            assert!(
                sample_count > 1,
                "C4: una fase indexada con trabajo no puede respaldar peak_rss_bytes con una única lectura final"
            );
        }
        phase_windows.push((phase_started, phase_finished));
    }
    for pair in phase_windows.windows(2) {
        assert!(
            pair[0].1 <= pair[1].0,
            "C4: ventanas de fases no solapadas: {phase_windows:?}"
        );
    }
}

#[test]
fn c4_index_duration_y_rss_cubren_limpieza_trace_apertura_y_authorizer() {
    let rebuild = function_scope(STORE_SOURCE, "fn rebuild_iter<I>(", "    fn swap_active(");
    phase_starts_cover(
        rebuild,
        "let index_started = Instant::now();",
        "let index_rss_window = RssWindow::new()?;",
        &[
            "remove_cache_files(&next)?;",
            "let mut trace = SqlTrace::new(&next)?;",
            "let mut next_conn = schema::open_build_connection(&next)?;",
            "next_conn.authorizer(Some(sql_audit.authorizer()));",
        ],
    )
    .unwrap_or_else(|error| panic!("C4 fase index incompleta: {error}"));
}

#[test]
fn c4_validate_duration_y_rss_cubren_snapshot_corrupcion_y_apertura() {
    let rebuild = function_scope(STORE_SOURCE, "fn rebuild_iter<I>(", "    fn swap_active(");
    let validation = function_scope(
        rebuild,
        "// Revalidate the complete canonical snapshot",
        "if failpoint_for(&self.root, \"pause_before_swap\")",
    );
    phase_starts_cover(
        validation,
        "let validate_start = Instant::now();",
        "let validate_rss_window = RssWindow::new()?;",
        &[
            "verify_rebuild_snapshot(&snapshot)?;",
            "if failpoint_for(&self.root, \"corrupt_next_before_integrity\")",
            "let validation_conn = schema::open_validation_connection(&next)?;",
        ],
    )
    .unwrap_or_else(|error| panic!("C4 fase validate incompleta: {error}"));
}

#[test]
fn c4_swap_duration_y_rss_cubren_pausa_ultima_verificacion_y_failpoint() {
    let rebuild = function_scope(STORE_SOURCE, "fn rebuild_iter<I>(", "    fn swap_active(");
    let swap = function_scope(
        rebuild,
        "let swap_started = Instant::now();",
        "trace.lifecycle(\"swap\", \"ok\");",
    );
    phase_starts_cover(
        swap,
        "let swap_started = Instant::now();",
        "let swap_rss_window = RssWindow::new()?;",
        &[
            "if failpoint_for(&self.root, \"pause_before_swap\")",
            "verify_rebuild_snapshot(&snapshot)?;",
            "if failpoint_for(&self.root, \"before_swap\")",
        ],
    )
    .unwrap_or_else(|error| panic!("C4 fase swap incompleta: {error}"));
}

#[test]
fn c4_duration_total_del_rebuild_incluye_la_ventana_de_inventario() {
    let _env_lock = failpoint_env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "doc.md",
        "---\ntitle: duration\n---\n\n# duration\n",
    );
    let store = Store::open(root.path()).unwrap();

    std::env::set_var(
        "LODESTAR_H03_FAILPOINT",
        format!("{}:after_snapshot_before_read", root.path().display()),
    );
    let _cleanup = FailpointEnv;
    let rebuilding = std::thread::spawn(move || store.rebuild());
    let pause = root
        .path()
        .join(".lodestar/h03-pause-after-snapshot-before-read");
    wait_for(&pause);
    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(
        root.path()
            .join(".lodestar/h03-release-after-snapshot-before-read"),
        b"release\n",
    )
    .unwrap();

    let report = rebuilding.join().unwrap().unwrap();
    let phases = report["phases"].as_array().expect("C4: phases array");
    let first_started = phases
        .first()
        .and_then(|phase| phase["phase_started_monotonic_ns"].as_u64())
        .expect("C4: inicio de inventory");
    let last_finished = phases
        .last()
        .and_then(|phase| phase["phase_finished_monotonic_ns"].as_u64())
        .expect("C4: fin de swap");
    let end_to_end_ns = last_finished.saturating_sub(first_started);
    let duration_ns = report["duration_ns"]
        .as_u64()
        .expect("C4: duration_ns global");
    assert!(
        duration_ns >= end_to_end_ns,
        "C4: duration_ns debe incluir inventory y abarcar el intervalo completo; reportado={duration_ns}, intervalo={end_to_end_ns}"
    );
}

#[test]
fn c4_guardas_contrafactuales_rechazan_inicios_movidos_despues_del_setup() {
    let valid = "DURATION\nRSS\nREMOVE\nTRACE\nOPEN\nAUTH\n";
    phase_starts_cover(
        valid,
        "DURATION",
        "RSS",
        &["REMOVE", "TRACE", "OPEN", "AUTH"],
    )
    .expect("C4 anti-vacuidad: el orden completo de referencia debe ser aceptado");

    for (name, counterfactual) in [
        (
            "duration tras limpieza",
            "REMOVE\nDURATION\nRSS\nTRACE\nOPEN\nAUTH\n",
        ),
        (
            "RSS tras trace",
            "DURATION\nREMOVE\nTRACE\nRSS\nOPEN\nAUTH\n",
        ),
        (
            "duration tras apertura",
            "RSS\nREMOVE\nTRACE\nOPEN\nDURATION\nAUTH\n",
        ),
        (
            "RSS tras authorizer",
            "DURATION\nREMOVE\nTRACE\nOPEN\nAUTH\nRSS\n",
        ),
    ] {
        assert!(
            phase_starts_cover(
                counterfactual,
                "DURATION",
                "RSS",
                &["REMOVE", "TRACE", "OPEN", "AUTH"],
            )
            .is_err(),
            "C4 anti-vacuidad: debe rechazarse el contrafactual {name}"
        );
    }
}
