//! Reproducción independiente del C4: `peak_rss_bytes` necesita evidencia de muestreo dentro de
//! cada ventana de fase, no cuatro lecturas puntuales al final de cada fase ni un high-water global.

use lodestar_store::Store;
use serde_json::Value;

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
