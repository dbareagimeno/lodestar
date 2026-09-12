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
use std::process::{Command, Stdio};
#[cfg(unix)]
use tempfile::tempdir;

#[cfg(unix)]
fn make_executable(path: &Path, script: String) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn write_value_sampler(path: &Path, sample_bytes: u64) {
    let script = format!(
        "#!/bin/sh\npid=${{LODESTAR_BENCH_TEST_RSS_PID:-}}\ncase \"$pid\" in ''|*[!0-9]*) exit 42;; esac\nprintf '%s\\n' '{sample_bytes}'\n"
    );
    make_executable(path, script);
}

#[cfg(unix)]
fn write_order_sampler(path: &Path, timing_log: &Path, rss_log: &Path, sample_bytes: u64) {
    let script = format!(
        "#!/bin/sh\npid=${{LODESTAR_BENCH_TEST_RSS_PID:-}}\ncase \"$pid\" in ''|*[!0-9]*) exit 42;; esac\nphase=${{LODESTAR_BENCH_TEST_RSS_PHASE:-unknown}}\nsample={sample_bytes}\nif [ \"$phase\" = rebuild ]; then\n  grep -Fx 'phase:rebuild:end' '{}' >/dev/null || exit 45\n  printf 'rss_sample:%s:%s:%s:after-rebuild-end\\n' \"$phase\" \"$pid\" \"$sample\" >> '{}'\nfi\nprintf '%s\\n' \"$sample\"\n",
        timing_log.display(),
        rss_log.display()
    );
    make_executable(path, script);
}

#[cfg(unix)]
fn extreme_report(sample_bytes: u64) -> (Value, Value, u32, Vec<String>, Vec<String>) {
    let support = tempdir().unwrap();
    let value_sampler = support.path().join("rss-value-sampler.sh");
    write_value_sampler(&value_sampler, sample_bytes);
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
        .env("LODESTAR_BENCH_TEST_RSS_SAMPLER", &value_sampler)
        .env_remove("LODESTAR_BENCH_TEST_SQLITE_TIMING_LOG")
        .env_remove("LODESTAR_H03_RSS_TRACE")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "C4 review: banco extremo falló: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = serde_json::from_slice(&output.stdout).unwrap();

    let root = support.path().join("persistent-worker-root");
    let control = root.join(".lodestar");
    fs::create_dir_all(&control).expect("C4 crear root persistente y plano de control");
    fs::write(
        root.join("control.md"),
        "---\nservice: bench\ntags: [rss-review]\n---\n# RSS review\nmarker-rss-review\n",
    )
    .expect("C4 escribir Markdown de control");
    let timing_log = control.join("rss-review-timing.log");
    let rss_log = control.join("rss-review-sample.log");
    for target in [&timing_log, &rss_log] {
        assert!(
            !target.exists(),
            "C4 control: cada destino debe ser nuevo dentro de <root>/.lodestar: {}",
            target.display()
        );
    }
    let order_sampler = support.path().join("rss-order-sampler.sh");
    write_order_sampler(&order_sampler, &timing_log, &rss_log, sample_bytes);
    let child = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"))
        .args([
            "--extreme-worker",
            "--profile",
            "plano",
            "--scale",
            "1",
            "--iterations",
            "1",
            "--worker-variant",
            "sqlite-raw",
        ])
        .arg("--root")
        .arg(&root)
        .env("RUST_BACKTRACE", "1")
        .env("LODESTAR_BENCH_TEST_RSS_SAMPLER", &order_sampler)
        .env("LODESTAR_BENCH_TEST_SQLITE_TIMING_LOG", &timing_log)
        .env("LODESTAR_H03_RSS_TRACE", &rss_log)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let worker_pid = child.id();
    let worker_output = child.wait_with_output().unwrap();
    assert!(
        worker_output.status.success(),
        "C4 review: worker persistente falló: stdout={} stderr={}",
        String::from_utf8_lossy(&worker_output.stdout),
        String::from_utf8_lossy(&worker_output.stderr)
    );
    let worker_report = serde_json::from_slice(&worker_output.stdout).unwrap();
    let timing_events = fs::read_to_string(&timing_log)
        .expect("C4 leer timing dentro de <root>/.lodestar")
        .lines()
        .map(str::to_owned)
        .collect();
    let rss_events = fs::read_to_string(&rss_log)
        .expect("C4 leer RSS dentro de <root>/.lodestar")
        .lines()
        .map(str::to_owned)
        .collect();
    (report, worker_report, worker_pid, timing_events, rss_events)
}

#[cfg(unix)]
fn assert_rss_case(
    sample_bytes: u64,
    report: &Value,
    worker_report: &Value,
    observed_worker_pid: u32,
    timing_events: &[String],
    rss_events: &[String],
    reconciled: bool,
) {
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
    assert_eq!(
        rss["raw_value"].as_u64(),
        Some(sample_bytes),
        "C4 review: el sampler controlable debe conservar su muestra raw"
    );
    let absolute = rss["absolute_bytes"]
        .as_u64()
        .expect("C4 review: RSS absoluto");
    let phase_peak = rebuild_report["peak_rss_bytes"]
        .as_u64()
        .expect("C4 review: peak RSS de RebuildReport");
    let worker_pid = row["worker_pid"]
        .as_u64()
        .expect("C4 review: PID del worker extremo");
    assert_eq!(
        rss["worker_pid"].as_u64(),
        Some(worker_pid),
        "C4 review: row.rebuild.rss debe identificar el worker que se midió"
    );
    assert!(absolute > 0, "C4 review: RSS reconciliado positivo");
    assert_eq!(
        targets["observed_peak_rss_bytes"].as_u64(),
        Some(absolute),
        "C8 review: objective debe reconciliar row.rebuild.rss, no una lectura posterior"
    );
    assert_eq!(rss["phase"].as_str(), Some("rebuild"));
    assert_eq!(rss["captured_before_queries"].as_bool(), Some(true));
    assert!(
        rss["scope"]
            .as_str()
            .is_some_and(|scope| scope.contains("rebuild")),
        "C4 review: scope RSS específico del rebuild"
    );
    assert!(
        absolute > rebuild_report["max_live_body_bytes"].as_u64().unwrap_or(0),
        "C4 review: RSS reconciliado separado de max_live_body_bytes"
    );

    if reconciled {
        assert!(
            phase_peak > sample_bytes,
            "C4 control bajo: RebuildReport debe ganar a la muestra controlada"
        );
        assert_eq!(absolute, phase_peak);
        assert_eq!(rss["sampled_absolute_bytes"].as_u64(), Some(sample_bytes));
        assert_eq!(rss["phase_peak_rss_bytes"].as_u64(), Some(phase_peak));
        assert_eq!(
            rss["method"].as_str(),
            Some("max(LODESTAR_BENCH_TEST_RSS_SAMPLER stdout u64, RebuildReport.peak_rss_bytes)"),
            "C4: reconciliación explícita y auditable"
        );
        assert_ne!(
            rss["raw_value"], rss["absolute_bytes"],
            "C4: raw no puede fingirse absoluto cuando gana el pico de fase"
        );
    } else {
        assert!(
            sample_bytes > phase_peak,
            "C4 control alto: la muestra externa debe ganar con margen seguro"
        );
        assert_eq!(absolute, sample_bytes);
        assert_eq!(rss.get("sampled_absolute_bytes"), None);
        assert_eq!(rss.get("phase_peak_rss_bytes"), None);
        assert_eq!(
            rss["method"].as_str(),
            Some("LODESTAR_BENCH_TEST_RSS_SAMPLER stdout u64")
        );
        assert_eq!(
            rss["raw_value"], rss["absolute_bytes"],
            "C4: raw==absolute solo cuando gana la muestra externa"
        );
    }

    let direct_rebuild = worker_report["rebuild"]
        .as_object()
        .expect("C4 review: rebuild del worker persistente");
    let direct_report = direct_rebuild["report"]
        .as_object()
        .expect("C4 review: RebuildReport del worker persistente");
    let direct_rss = direct_rebuild["rss"]
        .as_object()
        .expect("C4 review: RSS del worker persistente");
    let direct_absolute = direct_rss["absolute_bytes"]
        .as_u64()
        .expect("C4 review: RSS absoluto del worker persistente");
    let direct_phase_peak = direct_report["peak_rss_bytes"]
        .as_u64()
        .expect("C4 review: peak de fase del worker persistente");
    assert_eq!(direct_rss["raw_value"].as_u64(), Some(sample_bytes));
    if reconciled {
        assert_eq!(direct_absolute, direct_phase_peak);
        assert_eq!(
            direct_rss["sampled_absolute_bytes"].as_u64(),
            Some(sample_bytes)
        );
        assert_eq!(
            direct_rss["phase_peak_rss_bytes"].as_u64(),
            Some(direct_phase_peak)
        );
    } else {
        assert_eq!(direct_absolute, sample_bytes);
        assert_eq!(direct_rss.get("sampled_absolute_bytes"), None);
        assert_eq!(direct_rss.get("phase_peak_rss_bytes"), None);
    }

    let rebuild_end = timing_events
        .iter()
        .position(|event| event == "phase:rebuild:end")
        .expect("C4 review: evento rebuild_end");
    let query_start = timing_events
        .iter()
        .position(|event| event.starts_with("phase:tool:workspace_status:1:timer-start"))
        .expect("C4 review: query_start posterior al rebuild");
    assert!(
        rebuild_end < query_start,
        "C4 review: rebuild_end debe preceder query_start; timing={timing_events:?}"
    );
    assert_eq!(
        rss_events.len(),
        1,
        "C4 review: una única muestra rebuild en el destino RSS separado"
    );
    let sample_parts: Vec<&str> = rss_events[0].split(':').collect();
    assert_eq!(sample_parts.len(), 5);
    assert_eq!(
        sample_parts[2].parse::<u64>().unwrap(),
        u64::from(observed_worker_pid),
        "C4 review: la muestra debe medir el worker, no el sampler"
    );
    assert_eq!(
        sample_parts[3].parse::<u64>().unwrap(),
        sample_bytes,
        "C4 review: la traza conserva la muestra externa raw antes de reconciliar"
    );
    assert_eq!(
        sample_parts[4], "after-rebuild-end",
        "C4 review: el sampler autentica que observó rebuild_end antes de publicar la muestra; la llamada síncrona precede query_start"
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

#[cfg(unix)]
#[test]
fn c4_rebuild_rss_es_especifico_se_reconcilia_y_separa_de_la_fase_de_lectura() {
    let low_sample = 4_096;
    let (low_report, low_worker, low_pid, low_timing, low_rss) = extreme_report(low_sample);
    assert_rss_case(
        low_sample,
        &low_report,
        &low_worker,
        low_pid,
        &low_timing,
        &low_rss,
        true,
    );

    // No asigna memoria: solo es un u64 emitido por el sampler. El margen evita depender del RSS
    // concreto del runner y fuerza determinísticamente la rama donde gana la muestra externa.
    let high_sample = 1_u64 << 50;
    let (high_report, high_worker, high_pid, high_timing, high_rss) = extreme_report(high_sample);
    assert_rss_case(
        high_sample,
        &high_report,
        &high_worker,
        high_pid,
        &high_timing,
        &high_rss,
        false,
    );
}
