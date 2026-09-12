//! E35-H03 CI53 — los logs opcionales del banco no modifican un Markdown preexistente.
//!
//! Un único sandbox contiene tanto `notes.md` como los controles nuevos de `.lodestar/`. Cada
//! variable se prueba en un proceso nuevo para aislar el entorno, usando el worker SQLite que
//! ejerce rebuild, muestra RSS y cronometraje. El control confirma que el log correspondiente sí
//! produce evidencia cuando recibe un pathname nuevo del plano de control.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const SQL_TRACE: &str = "LODESTAR_H03_SQL_TRACE";
const RSS_TRACE: &str = "LODESTAR_H03_RSS_TRACE";
const TIMING_LOG: &str = "LODESTAR_BENCH_TEST_SQLITE_TIMING_LOG";
const NOTES: &[u8] = b"---\ntags: [ci53, control]\n---\n# Notes\nbytes-must-remain-identical\n";

struct LogCase {
    env: &'static str,
    safe_name: &'static str,
    evidence: fn(&[u8]) -> bool,
}

fn sql_evidence(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).ok().is_some_and(|contents| {
        contents.lines().any(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|event| event["event"].as_str().map(str::to_owned))
                .as_deref()
                == Some("header")
        })
    })
}

fn rss_evidence(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).ok().is_some_and(|contents| {
        contents
            .lines()
            .any(|line| line.starts_with("rss_sample:rebuild:"))
    })
}

fn timing_evidence(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .ok()
        .is_some_and(|contents| contents.lines().any(|line| line == "phase:rebuild:start"))
}

fn run_worker(root: &Path, env: &str, target: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lodestar-bench"))
        .args([
            "--extreme-worker",
            "--profile",
            "plano",
            "--scale",
            "1",
            "--iterations",
            "1",
            "--root",
            root.to_str().expect("CI53 root UTF-8"),
            "--worker-variant",
            "sqlite-raw",
        ])
        .env_remove(SQL_TRACE)
        .env_remove("LODESTAR_H03_SQL_TRACE_ROOT")
        .env_remove(RSS_TRACE)
        .env_remove(TIMING_LOG)
        .env(env, target)
        .output()
        .expect("CI53 ejecutar worker real del banco")
}

fn output_diagnostics(output: &Output) -> String {
    format!(
        "status={}, stdout_bytes={}, stderr={}",
        output.status,
        output.stdout.len(),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn ci53_logs_del_bench_no_modifican_markdown_preexistente_y_conservan_evidencia_nueva() {
    let sandbox = tempfile::tempdir().expect("CI53 sandbox único");
    let root = sandbox.path().join("workspace");
    let control = root.join(".lodestar");
    fs::create_dir_all(&control).expect("CI53 crear workspace y plano de control");
    let notes = root.join("notes.md");

    let cases = [
        LogCase {
            env: SQL_TRACE,
            safe_name: "ci53-sql.ndjson",
            evidence: sql_evidence,
        },
        LogCase {
            env: RSS_TRACE,
            safe_name: "ci53-rss.log",
            evidence: rss_evidence,
        },
        LogCase {
            env: TIMING_LOG,
            safe_name: "ci53-timing.log",
            evidence: timing_evidence,
        },
    ];

    let mut mutations = Vec::new();
    for case in cases {
        fs::write(&notes, NOTES).expect("CI53 restaurar bytes preexistentes");
        assert_eq!(
            fs::read(&notes).expect("CI53 leer baseline"),
            NOTES,
            "CI53 anti-vacuidad: notes.md existe con el baseline exacto antes de ejecutar {}",
            case.env
        );

        let existing_output = run_worker(&root, case.env, &notes);
        let after_existing = fs::read(&notes).expect("CI53 leer notes.md tras el worker");
        if after_existing != NOTES {
            mutations.push(format!(
                "{} cambió notes.md de {} a {} bytes; {}",
                case.env,
                NOTES.len(),
                after_existing.len(),
                output_diagnostics(&existing_output)
            ));
        }

        let safe_log = control.join(case.safe_name);
        assert!(
            !safe_log.exists(),
            "CI53 control anti-vacuidad: el log seguro debe ser nuevo para {}",
            case.env
        );
        let notes_before_safe = fs::read(&notes).expect("CI53 notes antes del control");
        let safe_output = run_worker(&root, case.env, &safe_log);
        assert!(
            safe_output.status.success(),
            "CI53 control: el worker debe aceptar un log nuevo para {}; {}",
            case.env,
            output_diagnostics(&safe_output)
        );
        assert_eq!(
            fs::read(&notes).expect("CI53 notes tras el control"),
            notes_before_safe,
            "CI53 control: el log nuevo de {} no debe tocar notes.md",
            case.env
        );
        let evidence = fs::read(&safe_log).unwrap_or_else(|error| {
            panic!(
                "CI53 control: {} no creó evidencia en {}: {error}; {}",
                case.env,
                safe_log.display(),
                output_diagnostics(&safe_output)
            )
        });
        assert!(
            !evidence.is_empty() && (case.evidence)(&evidence),
            "CI53 control anti-vacuidad: {} debe escribir evidencia reconocible; bytes={evidence:?}",
            case.env
        );
    }

    assert!(
        mutations.is_empty(),
        "rojo causal CI53: los logs opcionales modificaron un Markdown preexistente: {mutations:#?}"
    );
}
