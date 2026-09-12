//! Fase roja de E35-H03: el informe debe hacer observable el trabajo del rebuild sin convertir
//! los objetivos de 1k/10k/100k en un gate de CI.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
struct SqlTraceSummary {
    build_id: String,
    prepares: usize,
    executes: usize,
    deletes: usize,
    inserts_by_table: BTreeMap<String, usize>,
    lifecycle: Vec<(u64, String, String)>,
    load_footer_seq: Option<u64>,
}

fn trace_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn read_sql_trace(mut trace: std::fs::File, path: &std::path::Path) -> SqlTraceSummary {
    let mut contents = String::new();
    trace.read_to_string(&mut contents).unwrap_or_else(|error| {
        panic!(
            "H03: no se pudo leer la traza SQL NDJSON {:?}: {error}",
            path
        )
    });
    let mut lines = contents.lines();
    let header: Value = serde_json::from_str(
        lines
            .next()
            .unwrap_or_else(|| panic!("H03: traza vacía: falta header")),
    )
    .expect("H03: header NDJSON válido");
    assert_eq!(
        header["event"].as_str(),
        Some("header"),
        "H03: primer evento header"
    );
    assert_eq!(header["seq"].as_u64(), Some(0), "H03: header seq=0");
    let build_id = header["build_id"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .expect("H03: header build_id no vacío")
        .to_owned();
    let mut summary = SqlTraceSummary {
        build_id: build_id.clone(),
        ..SqlTraceSummary::default()
    };
    let mut expected_seq = 1_u64;
    let mut footer = None;
    for (line_number, line) in lines.by_ref().enumerate() {
        let event: Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!(
                "H03: línea {} de la traza no es JSON: {error}",
                line_number + 2
            )
        });
        assert_eq!(
            event["seq"].as_u64(),
            Some(expected_seq),
            "H03: secuencia NDJSON contigua en línea {}",
            line_number + 2
        );
        assert_eq!(
            event["build_id"].as_str(),
            Some(build_id.as_str()),
            "H03: build_id estable en línea {}",
            line_number + 2
        );
        expected_seq += 1;
        let kind = event["event"]
            .as_str()
            .unwrap_or_else(|| panic!("H03: línea {} sin event", line_number + 2));
        if kind == "footer" {
            if event["complete"].as_bool() != Some(true) {
                summary.load_footer_seq = Some(expected_seq - 1);
                continue;
            }
            assert_eq!(
                event["complete"].as_bool(),
                Some(true),
                "H03: footer complete=true"
            );
            footer = Some(event);
            break;
        }
        if matches!(kind, "integrity_check" | "swap" | "publication") {
            let result = event["result"].as_str().unwrap_or_default().to_owned();
            summary
                .lifecycle
                .push((expected_seq - 1, kind.to_owned(), result));
            continue;
        }
        let sql = event["sql"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| panic!("H03: línea {} sin sql no vacío", line_number + 2));
        let table = event["table"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| panic!("H03: línea {} sin table no vacío", line_number + 2));
        if sql.trim_start().to_ascii_uppercase().starts_with("DELETE") {
            summary.deletes += 1;
        }
        match kind {
            "prepare" => summary.prepares += 1,
            "execute" => {
                summary.executes += 1;
                if sql.to_ascii_uppercase().contains("INSERT") {
                    *summary
                        .inserts_by_table
                        .entry(table.to_owned())
                        .or_default() += 1;
                }
            }
            other => panic!("H03: evento SQL desconocido {other:?}"),
        }
    }
    let footer = footer.expect("H03: falta footer NDJSON");
    assert!(lines.next().is_none(), "H03: footer debe ser último evento");
    let counts = footer["counts"].as_object().expect("H03: footer counts");
    assert_eq!(counts["prepare"].as_u64(), Some(summary.prepares as u64));
    assert_eq!(counts["execute"].as_u64(), Some(summary.executes as u64));
    assert_eq!(counts["delete"].as_u64(), Some(summary.deletes as u64));
    assert!(
        summary.prepares > 0,
        "H03: la traza debe incluir prepares reales"
    );
    assert!(
        summary.executes > 0,
        "H03: la traza debe incluir ejecuciones reales"
    );
    summary
}

fn report_for(profile: &str, scale: u64) -> Value {
    let _env_lock = trace_env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let support = tempfile::tempdir().unwrap();
    let root = support.path().join("workspace");
    let trace_path = root.join(".lodestar/rebuild.ndjson");
    assert!(!root.exists(), "H03: --root debe empezar inexistente");
    assert!(!trace_path.exists(), "H03: la traza debe ser nueva");
    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"))
        .args([
            "--extreme",
            "--profile",
            profile,
            "--scale",
            &scale.to_string(),
            "--iterations",
            "1",
        ])
        .arg("--root")
        .arg(&root)
        .env("RUST_BACKTRACE", "1")
        .env("LODESTAR_H03_SQL_TRACE", &trace_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let trace = loop {
        match std::fs::File::open(&trace_path) {
            Ok(trace) => break Some(trace),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("H03: no se pudo abrir la traza nueva {trace_path:?}: {error}"),
        }
        if child.try_wait().expect("H03: consultar banco").is_some() {
            break None;
        }
        assert!(
            Instant::now() < deadline,
            "H03: el banco no creó la traza dentro de <root>/.lodestar"
        );
        std::thread::sleep(Duration::from_millis(1));
    };
    let output: Output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "banco H03 falló: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let mut report: Value = serde_json::from_slice(&output.stdout).unwrap();
    // `--extreme` limpia incluso una raíz explícita. El descriptor abierto mantiene viva la
    // evidencia hasta leerla después de que el proceso haya terminado.
    let trace = read_sql_trace(
        trace.unwrap_or_else(|| {
            panic!(
                "H03: el banco terminó sin crear la traza nueva dentro de <root>/.lodestar: {}",
                trace_path.display()
            )
        }),
        &trace_path,
    );
    report["h03_sql_trace"] = serde_json::json!({
        "build_id": trace.build_id,
        "prepares": trace.prepares,
        "executes": trace.executes,
        "deletes": trace.deletes,
        "inserts_by_table": trace.inserts_by_table,
        "lifecycle": trace.lifecycle,
        "load_footer_seq": trace.load_footer_seq,
    });
    report
}

fn report(scale: u64) -> Value {
    report_for("plano", scale)
}

/// C7/C8: la evidencia pública del rebuild no puede depender de activar seams de diagnóstico.
/// El binario debe publicar el informe completo y su RSS específico aun en una invocación normal.
#[test]
fn c7_evidencia_rebuild_publica_incluye_fases_rss_y_objetivo_sin_seams() {
    let output = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"))
        .args([
            "--extreme",
            "--profile",
            "plano",
            "--scale",
            "3",
            "--iterations",
            "1",
        ])
        .env("RUST_BACKTRACE", "1")
        .env_remove("LODESTAR_H03_SQL_TRACE")
        .env_remove("LODESTAR_H03_RSS_TRACE")
        .env_remove("LODESTAR_BENCH_TEST_RSS_SAMPLER")
        .env_remove("LODESTAR_BENCH_TEST_SQLITE_TIMING_LOG")
        .output()
        .expect("C7 ejecutar banco sin seams H03");
    assert!(
        output.status.success(),
        "C7 banco normal falló: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout.is_empty(), "C7 salida JSON no vacía");
    let report: Value = serde_json::from_slice(&output.stdout).expect("C7 JSON público válido");
    assert_eq!(report["scale"].as_u64(), Some(3), "C7 escala observada");
    let sqlite = report["measurements"]
        .as_array()
        .expect("C7 measurements público")
        .iter()
        .find(|row| row["variant"].as_str() == Some("sqlite-raw"))
        .expect("C7 falta variante sqlite-raw");
    assert!(
        sqlite["document_count"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "C7 corpus no vacuo"
    );

    let rebuild = sqlite["rebuild"]
        .as_object()
        .expect("C7 rebuild público como objeto");
    let rebuild_report = rebuild["report"]
        .as_object()
        .expect("C7 rebuild.report debe ser público sin seams");
    assert!(
        rebuild_report["documents_read"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "C7 contador documents_read no vacuo"
    );
    assert!(
        rebuild_report["rows_written"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "C7 contador rows_written no vacuo"
    );
    let phases = rebuild_report["phases"]
        .as_array()
        .expect("C7 fases del rebuild públicas");
    assert_eq!(phases.len(), 4, "C7 exactamente cuatro fases");
    assert_eq!(
        phases
            .iter()
            .map(|phase| phase["name"].as_str().expect("C7 nombre de fase"))
            .collect::<Vec<_>>(),
        ["inventory", "index", "validate", "swap"],
        "C7 fases completas y ordenadas"
    );
    let mut positive_phase_duration = false;
    for phase in phases {
        if phase["duration_ns"]
            .as_u64()
            .is_some_and(|duration| duration > 0)
        {
            positive_phase_duration = true;
        }
        assert!(phase["peak_rss_bytes"].as_u64().is_some(), "C7 RSS de fase");
        let counters = phase["counters"]
            .as_object()
            .expect("C7 contadores de fase");
        for field in [
            "documents_read",
            "relational_inserts",
            "fts_inserts",
            "delete_statements",
            "prepared_statement_count",
        ] {
            assert!(
                counters[field].as_u64().is_some(),
                "C7 falta contador {field} en fase {}",
                phase["name"]
            );
        }
    }
    assert!(
        positive_phase_duration,
        "C7 alguna fase debe medir trabajo real"
    );

    let rss = rebuild["rss"]
        .as_object()
        .expect("C7 rebuild.rss debe ser público sin sampler");
    assert_eq!(
        rss["status"].as_str(),
        Some("available"),
        "C7 RSS disponible"
    );
    let absolute_rss = rss["absolute_bytes"]
        .as_u64()
        .expect("C7 RSS absoluto")
        .max(1);
    assert!(absolute_rss > 1, "C7 RSS positivo y no centinela");
    assert_eq!(rss["phase"].as_str(), Some("rebuild"), "C7 fase RSS");
    assert_eq!(
        rss["captured_before_queries"].as_bool(),
        Some(true),
        "C7 RSS capturado antes de queries"
    );

    let objectives = report["rebuild_objectives"]
        .as_object()
        .expect("C8 objetivos públicos");
    assert_eq!(objectives["gate"].as_bool(), Some(false), "C8 gate=false");
    assert_eq!(
        objectives["observed_peak_rss_bytes"].as_u64(),
        Some(absolute_rss),
        "C8 objetivo debe reconciliar RSS del rebuild"
    );
}

/// C4: solo sqlite-raw reconstruye; cada fase conserva contadores ligados a la escala observada
/// y la ausencia de rebuild en disco/RAM evita confundir lecturas con construcción.
#[test]
fn c4_informe_rebuild_sqlite_raw_separa_fases_y_liga_contadores_a_escala() {
    let mut sqlite_reports = Vec::new();
    let mut trace_prepare_counts = BTreeSet::new();
    for scale in [2, 3] {
        let report = report(scale);
        let trace = report["h03_sql_trace"]
            .as_object()
            .expect("C4 traza SQL NDJSON observable");
        assert!(
            trace["executes"].as_u64().is_some_and(|count| count > 0),
            "C4 la traza debe incluir ejecuciones reales"
        );
        assert_eq!(
            trace["deletes"].as_u64(),
            Some(0),
            "C4 la traza no puede incluir DELETE"
        );
        let trace_prepares = trace["prepares"]
            .as_u64()
            .expect("C4 prepares de la traza SQL");
        assert!(
            trace_prepares > 0,
            "C4 la traza debe incluir prepares reales"
        );
        trace_prepare_counts.insert(trace_prepares);
        assert_eq!(report["scale"].as_u64(), Some(scale), "C4 escala no vacua");
        let rows = report["measurements"].as_array().expect("C4 measurements");
        let row = rows
            .iter()
            .find(|row| row["variant"].as_str() == Some("sqlite-raw"))
            .expect("C4 falta sqlite-raw")
            .clone();
        sqlite_reports.push((report, row));
    }
    let mut previous_documents = 0;
    for (report, row) in &sqlite_reports {
        let variant = row["variant"].as_str().expect("C4 variant");
        match variant {
            "sqlite-raw" => {
                let rebuild = row["rebuild"].as_object().expect("C4 rebuild como objeto");
                let rebuild_report = rebuild["report"]
                    .as_object()
                    .expect("C4 RebuildReport serializado");
                assert_eq!(
                    rebuild_report["delete_statements"].as_u64(),
                    Some(0),
                    "C4 rebuild insert-only"
                );
                assert!(
                    rebuild_report["prepared_statement_count"]
                        .as_u64()
                        .is_some_and(|count| count > 0),
                    "C4 preparaciones reutilizadas"
                );
                assert_eq!(
                    rebuild_report["documents_read"].as_u64(),
                    Some(row["document_count"].as_u64().expect("C4 document_count")),
                    "C4 documentos leídos"
                );
                assert!(
                    rebuild_report["rows_written"]
                        .as_u64()
                        .is_some_and(|rows| rows > 0),
                    "C4 filas escritas"
                );
                let phases = rebuild_report["phases"]
                    .as_array()
                    .expect("C4 fases del rebuild sqlite");
                assert_eq!(
                    phases
                        .iter()
                        .filter_map(|phase| phase["name"].as_str())
                        .collect::<Vec<_>>(),
                    vec!["inventory", "index", "validate", "swap"],
                    "C4 fases nombradas y ordenadas"
                );
                let mut docs = 0;
                let mut relational = 0;
                let mut fts = 0;
                let mut deletes = 0;
                let mut prepared = None;
                let expected_documents = row["document_count"]
                    .as_u64()
                    .expect("C4 document_count de la variante");
                assert_eq!(
                    rebuild_report["documents_read"].as_u64(),
                    Some(expected_documents),
                    "C4 documents_read debe ser exactamente N"
                );
                for phase in phases {
                    assert!(
                        phase["duration_ns"].as_u64().is_some(),
                        "C4 duración de fase"
                    );
                    assert!(phase["peak_rss_bytes"].as_u64().is_some(), "C4 RSS de fase");
                    let counters = phase["counters"]
                        .as_object()
                        .expect("C4 contadores de fase");
                    docs += counters["documents_read"]
                        .as_u64()
                        .expect("C4 documentos leídos");
                    relational += counters["relational_inserts"]
                        .as_u64()
                        .expect("C4 inserts relacionales");
                    fts += counters["fts_inserts"].as_u64().expect("C4 inserts FTS");
                    deletes += counters["delete_statements"]
                        .as_u64()
                        .expect("C4 deletes observables");
                    let count = counters["prepared_statement_count"]
                        .as_u64()
                        .expect("C4 preparaciones observables");
                    if let Some(first) = prepared {
                        assert_eq!(count, first, "C4 preparaciones constantes entre fases");
                    } else {
                        prepared = Some(count);
                    }
                }
                assert_eq!(
                    docs, expected_documents,
                    "C4 documentos leídos exactamente una vez por el corpus admitido"
                );
                assert_eq!(
                    rebuild_report["relational_inserts"].as_u64(),
                    Some(relational),
                    "C4 relational_inserts debe reconciliar las filas por fase"
                );
                assert_eq!(
                    rebuild_report["fts_inserts"].as_u64(),
                    Some(fts),
                    "C4 fts_inserts debe reconciliar las filas por fase"
                );
                assert_eq!(fts, expected_documents, "C4 FTS debe ser exactamente N");
                assert_eq!(
                    rebuild_report["rows_written"].as_u64(),
                    Some(relational + fts),
                    "C4 rows_written debe ser la suma exacta de proyecciones"
                );
                assert_eq!(deletes, 0, "C4 rebuild insert-only no ejecuta DELETE");
                assert!(
                    rebuild_report["max_live_body_bytes"].as_u64().is_some(),
                    "C4 memoria viva observable"
                );
                assert!(
                    docs > previous_documents,
                    "C4 segunda escala debe hacer crecer N"
                );
                let trace = report["h03_sql_trace"].as_object().unwrap();
                let inserts = trace["inserts_by_table"].as_object().unwrap();
                assert_eq!(
                    inserts["documents"].as_u64(),
                    Some(expected_documents),
                    "C4 traza documents exacta por build"
                );
                assert_eq!(
                    inserts["documents_fts"].as_u64(),
                    Some(expected_documents),
                    "C4 traza FTS exacta por build"
                );
                previous_documents = docs;
            }
            _ => unreachable!("C4 solo se seleccionó sqlite-raw"),
        }
    }
    assert_eq!(
        trace_prepare_counts.len(),
        1,
        "C4 las preparaciones observadas no pueden crecer con N"
    );
}

/// C8: los límites son objetivos de ingeniería no bloqueantes, con medidas observadas y
/// procedencia suficiente para reproducir la corrida; no basta con repetir los números en texto.
#[test]
fn c8_informe_objetivos_rebuild_no_gate_con_medida_y_procedencia() {
    let report = report(3);
    assert_eq!(report["profile"].as_str(), Some("plano"));
    assert_eq!(report["scale"].as_u64(), Some(3));
    assert_eq!(
        report["iterations"].as_u64(),
        Some(1),
        "C8 iterations top-level debe ser exactamente la corrida solicitada"
    );
    let targets = report["rebuild_objectives"]
        .as_object()
        .expect("C8 rebuild_objectives");
    assert_eq!(targets["max_duration_seconds"].as_u64(), Some(60));
    assert_eq!(
        targets["max_peak_rss_bytes"].as_u64(),
        Some(512 * 1024 * 1024)
    );
    assert_eq!(
        targets["gate"].as_bool(),
        Some(false),
        "C8 nunca es gate prematuro"
    );
    assert!(
        targets["observed_duration_seconds"]
            .as_f64()
            .is_some_and(|value| value > 0.0),
        "C8 medida de duración"
    );
    assert!(
        targets["observed_peak_rss_bytes"]
            .as_u64()
            .is_some_and(|value| value > 0),
        "C8 medida RSS"
    );
    let provenance = targets["provenance"].as_object().expect("C8 procedencia");
    for key in ["profile", "scale", "iterations", "machine", "commit"] {
        assert!(provenance.contains_key(key), "C8 procedencia falta {key}");
    }
    assert_eq!(provenance["profile"].as_str(), Some("plano"));
    assert_eq!(provenance["scale"].as_u64(), Some(3));
    assert_eq!(
        provenance["iterations"].as_u64(),
        Some(1),
        "C8 iterations debe reconciliar con --iterations 1"
    );
    for key in ["machine", "commit"] {
        assert!(
            provenance[key]
                .as_str()
                .is_some_and(|value| !value.trim().is_empty()),
            "C8 procedencia {key} no vacía"
        );
    }

    let sqlite = report["measurements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["variant"].as_str() == Some("sqlite-raw"))
        .expect("C8 sqlite-raw");
    let rebuild = sqlite["rebuild"].as_object().expect("C8 rebuild");
    let rebuild_report = rebuild["report"].as_object().expect("C8 RebuildReport");
    let phases = rebuild_report["phases"]
        .as_array()
        .expect("C8 fases SQLite");
    let phase_duration: u64 = phases
        .iter()
        .map(|phase| phase["duration_ns"].as_u64().expect("C8 duración por fase"))
        .sum();
    let phase_peak = phases
        .iter()
        .map(|phase| phase["peak_rss_bytes"].as_u64().expect("C8 RSS por fase"))
        .max()
        .expect("C8 al menos una fase");
    let observed_ns = targets["observed_duration_seconds"].as_f64().unwrap() * 1_000_000_000.0;
    assert!(
        observed_ns >= phase_duration as f64,
        "C8 duración observada debe reconciliar las fases"
    );
    assert!(
        targets["observed_peak_rss_bytes"].as_u64().unwrap() >= phase_peak,
        "C8 RSS observado debe cubrir las fases"
    );
    assert!(
        targets["command"]
            .as_str()
            .is_some_and(|command| !command.trim().is_empty()),
        "C8 comando reproducible"
    );
    assert!(
        report["machine"]
            .as_str()
            .is_some_and(|machine| !machine.trim().is_empty()),
        "C8 machine no vacío"
    );
    assert!(
        report["commit"]
            .as_str()
            .is_some_and(|commit| !commit.trim().is_empty()),
        "C8 commit no vacío"
    );
}

/// Matriz Realista 1k/10k/100k: es evidencia opt-in y no un gate rutinario de CI.
#[test]
#[ignore = "evidencia extrema opt-in; ejecutar explícitamente para actualizar la medición"]
fn c8_evidencia_extrema_realista_1k_10k_100k_es_reproducible() {
    for scale in [1_000, 10_000, 100_000] {
        let report = report_for("realista", scale);
        assert_eq!(report["profile"].as_str(), Some("realista"));
        assert_eq!(report["scale"].as_u64(), Some(scale));
        assert!(report["machine"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        assert!(report["commit"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        let sqlite = report["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["variant"].as_str() == Some("sqlite-raw"))
            .expect("evidencia extrema SQLite");
        let rebuild = sqlite["rebuild"].as_object().expect("evidencia rebuild");
        let rebuild_report = rebuild["report"].as_object().expect("evidencia informe");
        assert_eq!(rebuild_report["delete_statements"].as_u64(), Some(0));
        assert!(rebuild_report["documents_read"].as_u64().is_some());
        assert!(rebuild_report["rows_written"].as_u64().is_some());
        assert_eq!(
            report["rebuild_objectives"]["gate"].as_bool(),
            Some(false),
            "los objetivos no son gates"
        );
    }
}
