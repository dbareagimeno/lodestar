//! Regresión conductual Windows de repair16 sobre el informe público del benchmark E35-H03.

#![cfg(windows)]

use std::process::Command;

use serde_json::Value;

/// C8 — una corrida mínima real publica RSS Windows disponible, positivo y con la procedencia del
/// sistema operativo. La prueba ejecuta el mismo binario que produce la evidencia, sin seams.
#[test]
fn c8_benchmark_minimo_publica_rss_windows_real_y_disponible() {
    let output = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"))
        .args([
            "--extreme",
            "--profile",
            "plano",
            "--scale",
            "1",
            "--iterations",
            "1",
        ])
        .env_remove("LODESTAR_BENCH_TEST_RSS_SAMPLER")
        .output()
        .expect("C8: ejecutar benchmark mínimo en Windows");
    assert!(
        output.status.success(),
        "C8: benchmark falló: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("C8: informe JSON válido");
    assert_eq!(
        report["scale"].as_u64(),
        Some(1),
        "anti-vacuidad: escala real"
    );
    let sqlite = report["measurements"]
        .as_array()
        .expect("C8: measurements")
        .iter()
        .find(|row| row["variant"].as_str() == Some("sqlite-raw"))
        .expect("C8: variante sqlite-raw ejecutada");
    assert!(
        sqlite["document_count"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "anti-vacuidad: el corpus medido contiene documentos"
    );
    let rss = &sqlite["rebuild"]["rss"];
    assert_eq!(rss["status"].as_str(), Some("available"));
    assert!(
        rss["absolute_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > 0),
        "C8: el pico RSS Windows publicado debe ser positivo"
    );
    assert_eq!(rss["raw_units"].as_str(), Some("bytes"));
    assert!(
        rss["method"]
            .as_str()
            .is_some_and(|method| method.contains("GetProcessMemoryInfo")
                && method.contains("PeakWorkingSetSize")),
        "C8: la procedencia debe identificar la medición nativa de pico Windows"
    );
    assert!(
        rss["platform"]
            .as_str()
            .is_some_and(|platform| platform.starts_with("windows/")),
        "C8: el informe debe atribuir la muestra a Windows"
    );
}
