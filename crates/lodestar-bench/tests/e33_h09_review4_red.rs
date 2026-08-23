//! Reparación R4 de E33-H09: oráculos externos para footprint y preflight.
//!
//! Las escalas grandes nunca se materializan: un `df` falso determinista detiene la ejecución
//! después de validar la escala. La corrida materializada usa siete Markdown y una iteración.

#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const TOOLS: [&str; 7] = [
    "workspace_status",
    "knowledge_search",
    "knowledge_get",
    "metadata_inspect",
    "graph_query",
    "impact_analyze",
    "knowledge_check",
];

fn bench() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"));
    command.env("RUST_BACKTRACE", "1");
    command
}

fn combined(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn parse_report(output: &Output, context: &str) -> Value {
    assert!(
        output.status.success(),
        "{context}: ejecución fallida: {}",
        combined(output)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{context}: stdout no es JSON: {error}"))
}

#[cfg(unix)]
fn fake_df(temp: &Path, available_blocks: u64) -> (PathBuf, PathBuf, String) {
    use std::os::unix::fs::PermissionsExt;

    let bin = temp.join("fake-bin");
    fs::create_dir_all(&bin).expect("crear bin de df falso");
    let script = bin.join("df");
    let log = temp.join("df.log");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\necho invoked >> \"$LODESTAR_H09_DF_LOG\"\necho 'Filesystem 1024-blocks Used Available Capacity Mounted on'\necho '/fake 1 0 {available_blocks} 0% /'\n"
        ),
    )
    .expect("escribir df falso");
    let mut permissions = fs::metadata(&script)
        .expect("metadata de df falso")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("hacer ejecutable df falso");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    (bin, log, path)
}

#[cfg(unix)]
fn extreme_command(temp: &Path, scale: &str) -> (Command, PathBuf) {
    let (_bin, log, path) = fake_df(temp, 10_000_000);
    let mut command = bench();
    command
        .env("PATH", path)
        .env("LODESTAR_H09_DF_LOG", &log)
        .args([
            "--extreme",
            "--profile",
            "realista",
            "--scale",
            scale,
            "--iterations",
            "1",
        ]);
    (command, log)
}

fn wait_for_file(path: &Path, child: &mut Child, context: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !path.is_file() {
        if let Some(status) = child.try_wait().expect("consultar proceso extremo") {
            panic!(
                "{context}: el proceso terminó antes de publicar {}: {status}",
                path.display()
            );
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("{context}: timeout esperando {}", path.display());
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn continue_barrier(directory: &Path, child: &mut Child, label: &str) {
    wait_for_file(&directory.join(format!("READY-{label}")), child, label);
    fs::write(directory.join(format!("CONTINUE-{label}")), b"continue\n").expect("liberar barrera");
}

fn independent_markdown_size(root: &Path) -> (u64, u64) {
    fn walk(path: &Path, count: &mut u64, bytes: &mut u64) {
        for entry in fs::read_dir(path).expect("listar corpus") {
            let entry = entry.expect("entrada de corpus");
            let entry_path = entry.path();
            let file_type = entry.file_type().expect("tipo de entrada");
            if file_type.is_dir() && entry.file_name() != ".lodestar" {
                walk(&entry_path, count, bytes);
            } else if file_type.is_file()
                && entry_path.extension().and_then(|value| value.to_str()) == Some("md")
            {
                *count += 1;
                *bytes += entry.metadata().expect("metadata Markdown").len();
            }
        }
    }

    let mut count = 0;
    let mut bytes = 0;
    walk(root, &mut count, &mut bytes);
    (count, bytes)
}

fn file_bytes(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn independent_sqlite_size(root: &Path) -> [u64; 5] {
    let cache = root.join(lodestar_store::CACHE_DIR);
    let main = file_bytes(&cache.join(lodestar_store::DB_FILE));
    let wal = file_bytes(&cache.join("index.db-wal"));
    let shm = file_bytes(&cache.join("index.db-shm"));
    let auxiliary = wal.checked_add(shm).expect("bytes auxiliares SQLite");
    let total = main.checked_add(auxiliary).expect("bytes totales SQLite");
    [main, wal, shm, auxiliary, total]
}

#[cfg(unix)]
#[test]
fn corrida_pequena_contrasta_con_filesystem_conteo_corpus_y_bytes_sqlite() {
    let temp = tempfile::tempdir().expect("directorio temporal");
    let root = temp.path().join("extreme-root");
    let barriers = temp.path().join("barriers");
    let (mut command, log) = extreme_command(temp.path(), "3");
    command
        .env("LODESTAR_BENCH_TEST_BARRIER_DIR", &barriers)
        .args(["--root", root.to_str().expect("root UTF-8")])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("iniciar corrida pequeña");

    let first_disk = "disk-reparseo-workspace_status-1";
    wait_for_file(
        &barriers.join(format!("READY-{first_disk}")),
        &mut child,
        "corpus materializado",
    );
    let observed_corpus = independent_markdown_size(&root);
    assert_eq!(observed_corpus.0, 7, "anti-vacuidad del fixture pequeño");
    assert!(observed_corpus.1 > 0, "el corpus debe ocupar bytes reales");
    fs::write(
        barriers.join(format!("CONTINUE-{first_disk}")),
        b"continue\n",
    )
    .expect("liberar primera barrera");
    for tool in TOOLS.into_iter().skip(1) {
        continue_barrier(&barriers, &mut child, &format!("disk-reparseo-{tool}-1"));
    }
    for tool in TOOLS {
        continue_barrier(&barriers, &mut child, &format!("sqlite-raw-{tool}-1"));
    }

    wait_for_file(
        &barriers.join("READY-ram-memoizado-ACQUIRE"),
        &mut child,
        "SQLite finalizado",
    );
    let observed_sqlite = independent_sqlite_size(&root);
    assert!(observed_sqlite[0] > 0, "SQLite main debe existir");
    assert!(observed_sqlite[4] > 0, "SQLite total debe ser no vacío");
    fs::write(
        barriers.join("CONTINUE-ram-memoizado-ACQUIRE"),
        b"continue\n",
    )
    .expect("liberar worker RAM");

    let output = child.wait_with_output().expect("terminar corrida pequeña");
    let report = parse_report(&output, "contraste externo de footprint");
    assert_eq!(report["corpus"]["document_count"], observed_corpus.0);
    assert_eq!(report["corpus"]["bytes"], observed_corpus.1);
    for (field, expected) in [
        ("main_bytes", observed_sqlite[0]),
        ("wal_bytes", observed_sqlite[1]),
        ("shm_bytes", observed_sqlite[2]),
        ("auxiliary_bytes", observed_sqlite[3]),
        ("total_bytes", observed_sqlite[4]),
    ] {
        assert_eq!(report["sqlite"][field], expected, "SQLite {field}");
    }
    assert!(!root.exists(), "el guard RAII debe retirar el root");
    assert_eq!(
        fs::read_to_string(log).expect("log de df").lines().count(),
        1,
        "el preflight debe medir espacio exactamente una vez"
    );
}

#[cfg(unix)]
#[test]
fn rss_getrusage_declara_pico_absoluto_base_y_delta_reconciliables() {
    let temp = tempfile::tempdir().expect("directorio temporal");
    let (mut command, _log) = extreme_command(temp.path(), "3");
    let report = parse_report(
        &command.output().expect("ejecutar medición RSS"),
        "medición RSS",
    );
    let rows = report["measurements"]
        .as_array()
        .expect("tres mediciones por variante");
    assert_eq!(rows.len(), 3, "anti-vacuidad: tres workers");
    for row in rows {
        let variant = row["variant"].as_str().unwrap_or("<sin variante>");
        let rss = &row["rss"];
        if rss["status"] == "unavailable" {
            assert!(
                rss["reason"]
                    .as_str()
                    .is_some_and(|reason| !reason.is_empty()),
                "{variant}: RSS no disponible exige motivo"
            );
            continue;
        }
        assert_eq!(rss["status"], "available", "{variant}: RSS disponible");
        assert!(rss["method"]
            .as_str()
            .is_some_and(|method| method.contains("getrusage(RUSAGE_SELF).ru_maxrss")));
        assert_eq!(rss["units"], "bytes");
        assert!(rss["platform"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        assert!(rss["scope"]
            .as_str()
            .is_some_and(|value| value.contains("pico")));

        let absolute = rss["absolute_bytes"]
            .as_u64()
            .unwrap_or_else(|| panic!("{variant}: falta pico absoluto medido"));
        let baseline = rss["baseline_bytes"]
            .as_u64()
            .unwrap_or_else(|| panic!("{variant}: falta base RSS medida, no una constante"));
        let delta = rss["delta_bytes"]
            .as_u64()
            .unwrap_or_else(|| panic!("{variant}: falta delta RSS medido"));
        assert!(absolute > 0 && baseline > 0, "{variant}: anti-vacuidad RSS");
        assert_eq!(
            baseline.checked_add(delta),
            Some(absolute),
            "{variant}: base + delta debe reconciliar con el pico absoluto"
        );
        assert!(
            !rss.to_string().to_ascii_lowercase().contains("estimate"),
            "{variant}: una estimación no puede disfrazarse de getrusage"
        );
    }
}

#[cfg(unix)]
#[test]
fn preflight_declara_memoria_y_rss_como_no_verificados_en_estado_estructurado() {
    let temp = tempfile::tempdir().expect("directorio temporal");
    let (mut command, _log) = extreme_command(temp.path(), "3");
    let report = parse_report(
        &command.output().expect("ejecutar preflight pequeño"),
        "declaración de memoria del preflight",
    );
    assert_eq!(report["preflight"]["status"], "checked");
    let memory = report["preflight"]["memory_verification"]
        .as_object()
        .expect("memory_verification debe ser un estado estructurado, no un rótulo ambiguo");
    assert_eq!(memory["status"], "unverified");
    let reason = memory["reason"]
        .as_str()
        .expect("motivo explícito de memoria/RSS no verificados")
        .to_ascii_lowercase();
    assert!(reason.contains("rss"), "el motivo debe nombrar RSS");
    assert!(
        reason.contains("memoria") || reason.contains("memory"),
        "el motivo debe nombrar memoria"
    );
    assert!(
        reason.contains("no verific") || reason.contains("unverified"),
        "el motivo debe declarar que el preflight no la verificó"
    );
}

#[cfg(unix)]
#[test]
fn scale_un_millon_con_df_suficiente_exige_confirmacion_antes_de_crear_root() {
    let temp = tempfile::tempdir().expect("directorio temporal");
    let root = temp.path().join("million-root");
    let (_bin, log, path) = fake_df(temp.path(), 100_000_000_000);
    let mut command = bench();
    command
        .env("PATH", path)
        .env("LODESTAR_H09_DF_LOG", &log)
        .args([
            "--extreme",
            "--profile",
            "realista",
            "--scale",
            "1000000",
            "--iterations",
            "1",
            "--root",
            root.to_str().expect("root UTF-8"),
        ]);
    let output = command.output().expect("validar confirmación extrema");
    let text = combined(&output);
    assert!(!output.status.success(), "1M sin confirmación debe fallar");
    assert!(
        text.contains("confirmación explícita") && text.contains("--confirm-extreme"),
        "el fallo debe ser inequívocamente de confirmación: {text}"
    );
    assert!(
        !text.contains("espacio insuficiente"),
        "el df conocido era suficiente: {text}"
    );
    assert!(!root.exists(), "el preflight no puede crear el root");
    assert_eq!(
        fs::read_to_string(log)
            .expect("df debe haberse ejecutado")
            .lines()
            .count(),
        1,
        "la confirmación se exige después de consultar df"
    );
}

#[cfg(unix)]
#[test]
fn escalas_positivas_no_historicas_y_grandes_llegan_a_df_sin_whitelist() {
    let temp = tempfile::tempdir().expect("directorio temporal");
    let (_bin, log, path) = fake_df(temp.path(), 1);
    let scales = ["17", "257", "562949953413119"];
    for scale in scales {
        let root = temp.path().join(format!("scale-{scale}"));
        let mut command = bench();
        command
            .env("PATH", &path)
            .env("LODESTAR_H09_DF_LOG", &log)
            .args([
                "--extreme",
                "--profile",
                "realista",
                "--scale",
                scale,
                "--iterations",
                "1",
                "--root",
                root.to_str().expect("root UTF-8"),
            ]);
        let output = command.output().expect("validar escala abierta");
        let text = combined(&output);
        assert!(
            !output.status.success(),
            "df insuficiente debe detener {scale}"
        );
        assert!(
            text.contains("preflight extremo: espacio insuficiente")
                && text.contains(&format!("scale={scale}")),
            "{scale} debe superar validación y llegar a df: {text}"
        );
        assert!(
            !text.to_ascii_lowercase().contains("whitelist"),
            "{scale} no puede rechazarse por lista: {text}"
        );
        assert!(!root.exists(), "{scale}: df falso evita materialización");
    }
    assert_eq!(
        fs::read_to_string(log)
            .expect("invocaciones de df")
            .lines()
            .count(),
        scales.len(),
        "cada escala positiva debe llegar al preflight de disco"
    );
}
