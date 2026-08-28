//! E35-H03 CI47 — reproducción temprana del hijo integrado de adquisición del benchmark.
//!
//! El rojo Windows previo apareció después de que el harness Win32 aislado ya estuviera verde:
//! `lodestar-bench --probe-acquisition-root` murió antes de publicar READY al ejecutar la secuencia
//! real App::open → adquisición disco → Store::open_and_build → adquisición SQLite. Este test
//! invoca exactamente ese entrypoint público y conserva el estado de disco si vuelve a morir.

use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

fn write_fixture(root: &Path) {
    fs::write(
        root.join("control.md"),
        "---\ntags: [h04, control]\nservice: bench\n---\n# Control\nmarker-search-h04\n[child](child.md)\n[missing](missing.md)\n",
    )
    .expect("CI47 control fixture");
    fs::write(
        root.join("child.md"),
        "---\ntags: [child]\nservice: bench\n---\n# Child\nmarker-get-h04\n[leaf](leaf.md)\n",
    )
    .expect("CI47 child fixture");
    fs::write(
        root.join("leaf.md"),
        "---\ntags: [leaf]\nservice: bench\n---\n# Leaf\nmarker-impact-h04\n",
    )
    .expect("CI47 leaf fixture");
    fs::write(root.join("broken.md"), "---\ntags: [\n---\n# Broken\n")
        .expect("CI47 broken fixture");
}

fn path_diagnostics(label: &str, path: &Path) -> String {
    let size = fs::metadata(path)
        .map(|metadata| metadata.len().to_string())
        .unwrap_or_else(|error| format!("error:{error}"));
    format!(
        "{label}={{path:{}, exists:{}, size:{size}}}",
        path.display(),
        path.exists()
    )
}

fn sqlite_generation(path: &Path) -> String {
    if !path.is_file() {
        return "absent".into();
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    match rusqlite::Connection::open_with_flags(path, flags) {
        Ok(connection) => connection
            .query_row("SELECT COUNT(*) FROM documents", [], |row| {
                row.get::<_, u64>(0)
            })
            .map(|documents| format!("documents:{documents}"))
            .unwrap_or_else(|error| format!("query_error:{error}")),
        Err(error) => format!("open_error:{error}"),
    }
}

fn disk_diagnostics(root: &Path) -> String {
    let cache = root.join(lodestar_store::CACHE_DIR);
    let active = cache.join(lodestar_store::DB_FILE);
    let next = cache.join("index.db.next");
    let wal = cache.join("index.db-wal");
    let shm = cache.join("index.db-shm");
    let mut markdown_generation = fs::read_dir(root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension().and_then(|extension| extension.to_str()) == Some("md")
                })
                .map(|path| path_diagnostics("markdown", &path))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|error| vec![format!("markdown_read_dir_error:{error}")]);
    markdown_generation.sort();
    let mut cache_siblings = fs::read_dir(&cache)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| path_diagnostics("cache_sibling", &entry.path()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|error| vec![format!("cache_read_dir_error:{error}")]);
    cache_siblings.sort();
    format!(
        "cwd={}; root={}; canonical_root={:?}; markdown_generation=[{}]; {}; generation={}; {}; {}; {}; cache_siblings=[{}]",
        std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|error| format!("error:{error}")),
        root.display(),
        root.canonicalize(),
        markdown_generation.join(", "),
        path_diagnostics("active", &active),
        sqlite_generation(&active),
        path_diagnostics("next", &next),
        path_diagnostics("wal", &wal),
        path_diagnostics("shm", &shm),
        cache_siblings.join(", ")
    )
}

#[test]
fn ci47_hijo_bench_publica_ready_tras_open_and_build_integrado() {
    let root = tempfile::tempdir().expect("CI47 root exacto");
    write_fixture(root.path());

    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"))
        .args([
            "--probe-acquisition-root",
            root.path().to_str().expect("CI47 root UTF-8"),
        ])
        .env("RUST_BACKTRACE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("CI47 spawn del hijo real lodestar-bench");
    let stdout = child.stdout.take().expect("CI47 stdout del hijo");
    let mut lines = BufReader::new(stdout).lines();
    let ready_line = match lines.next() {
        Some(line) => line.expect("CI47 READY UTF-8"),
        None => {
            drop(child.stdin.take());
            let mut stderr = Vec::new();
            child
                .stderr
                .take()
                .expect("CI47 stderr del hijo")
                .read_to_end(&mut stderr)
                .expect("CI47 leer stderr antes de READY");
            let status = child.wait().expect("CI47 esperar hijo antes de READY");
            panic!(
                "rojo causal CI47: el hijo store terminó antes de READY; status={status}; stderr={}; {}",
                String::from_utf8_lossy(&stderr),
                disk_diagnostics(root.path())
            );
        }
    };
    let ready: Value = serde_json::from_str(&ready_line).unwrap_or_else(|error| {
        panic!(
            "CI47: la primera salida debe ser READY JSON: {error}; line={ready_line:?}; {}",
            disk_diagnostics(root.path())
        )
    });
    assert_eq!(
        ready.get("event").and_then(Value::as_str),
        Some("READY"),
        "CI47: Store::open_and_build y las tres adquisiciones deben completar antes de READY; ready={ready}; {}",
        disk_diagnostics(root.path())
    );

    writeln!(
        child.stdin.as_mut().expect("CI47 stdin del hijo"),
        "continue"
    )
    .expect("CI47 continuar probe sin mutación");
    drop(child.stdin.take());
    let final_stdout = lines
        .collect::<Result<Vec<_>, _>>()
        .expect("CI47 salida final UTF-8")
        .join("\n");
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .expect("CI47 stderr final")
        .read_to_end(&mut stderr)
        .expect("CI47 leer stderr final");
    let status = child.wait().expect("CI47 esperar hijo final");
    assert!(
        status.success(),
        "CI47: el refresco integrado posterior a READY también debe completar; status={status}; stderr={}; stdout={final_stdout}; {}",
        String::from_utf8_lossy(&stderr),
        disk_diagnostics(root.path())
    );
    let report: Value = serde_json::from_str(final_stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "CI47: el hijo debe cerrar con su informe JSON real: {error}; stdout={final_stdout}; stderr={}; {}",
            String::from_utf8_lossy(&stderr),
            disk_diagnostics(root.path())
        )
    });
    let sqlite_before = report["before"]["sqlite-raw"]["document_count"]
        .as_u64()
        .expect("CI47 generación SQLite antes como contador");
    let sqlite_after = report["after"]["sqlite-raw"]["document_count"]
        .as_u64()
        .expect("CI47 generación SQLite después como contador");
    assert!(sqlite_before > 0, "CI47 anti-vacuidad: generación no vacía");
    assert_eq!(
        sqlite_after,
        sqlite_before,
        "CI47: sin mutación, el refresco debe conservar la misma generación; report={report}; {}",
        disk_diagnostics(root.path())
    );
}
