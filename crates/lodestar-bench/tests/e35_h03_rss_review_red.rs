//! Revisión independiente de la medición RSS y de las fases del rebuild E35-H03.

#[cfg(unix)]
use serde_json::Value;
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use tempfile::tempdir;

#[cfg(unix)]
fn write_rss_sampler(path: &Path, trace: &Path) {
    let script = format!(
        "#!/bin/sh\npid=${{LODESTAR_BENCH_TEST_RSS_PID:-}}\ncase \"$pid\" in ''|*[!0-9]*) exit 42;; esac\nraw=$(ps -o rss= -p \"$pid\" | awk '{{print $1 * 1024}}')\ncase \"$raw\" in ''|*[!0-9]*) exit 43;; esac\nprintf 'rss_sample:%s:%s:%s\\n' \"${{LODESTAR_BENCH_TEST_RSS_PHASE:-unknown}}\" \"$pid\" \"$raw\" >> '{}'\nprintf '%s\\n' \"$raw\"\n",
        trace.display()
    );
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn c4_rebuild_rss_es_especifico_se_reconcilia_y_separa_de_la_fase_de_lectura() {
    let support = tempdir().unwrap();
    let sampler = support.path().join("rss-sampler.sh");
    let trace = support.path().join("ordered-events.log");
    write_rss_sampler(&sampler, &trace);
    let output = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"))
        .args([
            "--extreme",
            "--profile",
            "plano",
            "--scale",
            "2",
            "--iterations",
            "1",
        ])
        .env("RUST_BACKTRACE", "1")
        .env("LODESTAR_BENCH_TEST_RSS_SAMPLER", &sampler)
        .env("LODESTAR_BENCH_TEST_SQLITE_TIMING_LOG", &trace)
        .env("LODESTAR_H03_RSS_TRACE", &trace)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "C4 review: banco extremo falló: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let targets = report["rebuild_objectives"]
        .as_object()
        .expect("C4 review: rebuild_objectives");
    let row = report["measurements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["variant"].as_str() == Some("sqlite-raw"))
        .expect("C4 review: falta fila sqlite-raw");
    let rebuild = row["rebuild"].as_object().expect("C4 review: rebuild");
    let rebuild_report = rebuild["report"].as_object().expect("C4 review: report");
    let rss = rebuild["rss"].as_object().expect(
        "C4 review: row.rebuild.rss debe medirse inmediatamente después del rebuild, antes de queries",
    );
    assert_eq!(rss["status"].as_str(), Some("available"));
    let peak = rss["absolute_bytes"]
        .as_u64()
        .expect("C4 review: RSS absoluto");
    let worker_pid = row["worker_pid"]
        .as_u64()
        .expect("C4 review: PID del worker extremo");
    assert_eq!(
        rss["worker_pid"].as_u64(),
        Some(worker_pid),
        "C4 review: row.rebuild.rss debe identificar el worker que se midió"
    );
    assert!(peak > 0, "C4 review: RSS real positivo");
    assert!(
        rss["method"]
            .as_str()
            .is_some_and(|method| !method.is_empty()),
        "C4 review: método RSS"
    );
    assert!(
        rss["scope"]
            .as_str()
            .is_some_and(|scope| scope.contains("rebuild")),
        "C4 review: scope RSS específico del rebuild"
    );
    assert!(
        peak > rebuild_report["max_live_body_bytes"].as_u64().unwrap_or(0),
        "C4 review: RSS real separado de max_live_body_bytes"
    );
    assert_eq!(
        targets["observed_peak_rss_bytes"].as_u64(),
        Some(peak),
        "C4 review: objective debe reconciliar row.rebuild.rss, no una lectura posterior"
    );
    assert_eq!(
        rebuild["rss"]["phase"].as_str(),
        Some("rebuild"),
        "C4 review: la muestra debe etiquetar la fase rebuild"
    );
    assert_eq!(
        rebuild["rss"]["captured_before_queries"].as_bool(),
        Some(true),
        "C4 review: RSS rebuild debe capturarse antes de las queries"
    );
    let events = fs::read_to_string(&trace).expect("C4 review: traza de eventos RSS/queries");
    let events: Vec<&str> = events.lines().collect();
    let rebuild_end = events
        .iter()
        .position(|event| *event == "phase:rebuild:end")
        .expect("C4 review: evento rebuild_end");
    let rss_sample = events
        .iter()
        .position(|event| event.starts_with("rss_sample:rebuild:"))
        .expect("C4 review: muestra RSS de la fase rebuild");
    let query_start = events
        .iter()
        .position(|event| event.starts_with("phase:tool:workspace_status:1:timer-start"))
        .expect("C4 review: query_start posterior al rebuild");
    assert!(
        rebuild_end < rss_sample && rss_sample < query_start,
        "C4 review: orden obligatorio rebuild_end -> rss_sample(rebuild) -> query_start; eventos={events:?}"
    );
    let sample_parts: Vec<&str> = events[rss_sample].split(':').collect();
    assert_eq!(
        sample_parts.len(),
        4,
        "C4 review: evento RSS con phase/pid/bytes"
    );
    assert_eq!(
        sample_parts[2].parse::<u64>().unwrap(),
        worker_pid,
        "C4 review: la muestra RSS debe medir el PID del worker, no el sampler"
    );
    let sample_value = sample_parts[3].parse::<u64>().unwrap();
    assert_eq!(
        sample_value, peak,
        "C4 review: RSS reportado debe ser la muestra concreta"
    );
    let phases = rebuild_report["phases"]
        .as_array()
        .expect("C4 review: phases");
    assert_eq!(phases.len(), 4);
    for name in ["inventory", "index", "validate", "swap"] {
        let phase = phases
            .iter()
            .find(|phase| phase["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("C4 review: falta fase {name}"));
        assert!(
            phase["duration_ns"]
                .as_u64()
                .is_some_and(|duration| duration > 0),
            "C4 review: duración real >0 para {name}"
        );
    }
    let row_rss = row["rss"].as_object().expect("C4 review: row.rss");
    assert_ne!(
        row_rss.get("scope"),
        rss.get("scope"),
        "C4 review: row.rss de lectura no debe sustituir row.rebuild.rss"
    );
}
