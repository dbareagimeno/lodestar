//! Reparaciones rojas de E33-H04 (adenda de lectura independiente).
//! Snapshot de alcance: `target/agent-state/e33-h04-v2-red2-repair/pre-red.json`.
//!
//! Estos tests son intencionadamente de integración: ejercitan el `App` real y el ejecutable
//! del banco. Las aserciones de adquisición/artefactos usan oráculos externos: mutación real entre
//! muestras, recibos leídos del disco, y el artefacto oficial datado.

use lodestar_app::{App, CheckScope, Profile};
use lodestar_app::{AppError, ReadServices};
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::{DocumentRef, ErrorCode, RelPath, Severity};
use lodestar_store::Store;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

// Hooks exclusivamente de test; no forman parte de la superficie CLI/MCP ratificada.
const BARRIER_ENV: &str = "LODESTAR_BENCH_TEST_BARRIER_DIR";
// Aunque conserva *_PARENT por compatibilidad del arnés, el valor es el root exacto de evidencia
// poseído por el TempDir, nunca un parent ambiguo ni una ruta de salida compartida.
const CHANGE_PARENT_ENV: &str = "LODESTAR_BENCH_TEST_CHANGE_PARENT";
const ITERATIONS_ENV: &str = "LODESTAR_BENCH_TEST_ITERATIONS";

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn child_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("child guard activo")
    }

    fn take(&mut self) -> Child {
        self.0.take().expect("child guard activo")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn bench() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"));
    command.env("RUST_BACKTRACE", "1");
    command
}

fn json_stdout(output: std::process::Output, context: &str) -> Value {
    assert!(
        output.status.success(),
        "{context}: banco falló: status={} stderr={} stdout={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("{context}: salida JSON inválida: {error}; stdout={stdout}"))
}

fn write_graph_fixture(root: &Path) {
    fs::create_dir_all(root).expect("fixture root");
    fs::write(
        root.join("a.md"),
        "---\ntags: [a]\nservice: bench\n---\n# A\nmarker-search-h04\n[b](b.md)\n[missing](missing.md)\n",
    )
    .expect("a");
    fs::write(
        root.join("b.md"),
        "---\ntags: [b]\nservice: bench\n---\n# B\nmarker-get-h04\n[leaf](leaf.md)\n",
    )
    .expect("b");
    fs::write(
        root.join("leaf.md"),
        "---\ntags: [leaf]\nservice: bench\n---\n# Leaf\nmarker-impact-h04\n",
    )
    .expect("leaf");
    fs::write(root.join("broken.md"), "---\ntags: [\n---\n# Broken\n").expect("diagnostic");
}

fn write_a4_fixture(root: &Path) {
    fs::create_dir_all(root).expect("fixture root");
    fs::create_dir_all(root.join(".lodestar")).expect("config fixture");
    fs::write(
        root.join(".lodestar/config.yaml"),
        "discovery:\n  exclude: [\"store-only-*.md\"]\n",
    )
    .expect("exclude store-only");
    fs::write(
        root.join("control.md"),
        "---\ntags: [h04, control]\nservice: bench\n---\n# Control\nmarker-search-h04\n[child](child.md)\n[missing](missing.md)\n",
    )
    .expect("control");
    fs::write(
        root.join("child.md"),
        "---\ntags: [child]\nservice: bench\n---\n# Child\nmarker-get-h04\n[leaf](leaf.md)\n",
    )
    .expect("child");
    fs::write(
        root.join("leaf.md"),
        "---\ntags: [leaf]\nservice: bench\n---\n# Leaf\nmarker-impact-h04\n",
    )
    .expect("leaf");
    fs::write(root.join("broken.md"), "---\ntags: [\n---\n# Broken\n").expect("diagnostic");
}

enum ReadyEvent {
    RamAcquire,
    Sample {
        variant: String,
        tool: String,
        index: usize,
    },
}

fn wait_for_next_ready(
    barrier: &Path,
    variants: &[&str],
    tools: &BTreeSet<&str>,
    handled: &BTreeSet<(String, String, usize)>,
    ram_acquired: bool,
    child: &mut ChildGuard,
    deadline: Instant,
) -> Option<(ReadyEvent, PathBuf)> {
    loop {
        let entries = fs::read_dir(barrier).expect("leer barrier A4");
        for entry in entries {
            let entry = entry.expect("entry barrier A4");
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "DONE" {
                return None;
            }
            if !name.starts_with("READY-") {
                continue;
            }
            if name == "READY-ram-memoizado-ACQUIRE" {
                assert!(!ram_acquired, "READY RAM ACQUIRE duplicado");
                return Some((ReadyEvent::RamAcquire, path));
            }
            let sample = variants.iter().find_map(|variant| {
                tools.iter().find_map(|tool| {
                    let prefix = format!("READY-{variant}-{tool}-");
                    name.strip_prefix(&prefix).and_then(|suffix| {
                        suffix
                            .parse::<usize>()
                            .ok()
                            .filter(|index| {
                                *index >= 1 && !(*variant == "ram-memoizado" && *index == 1)
                            })
                            .map(|index| ((*variant).to_owned(), (*tool).to_owned(), index))
                    })
                })
            });
            let (variant, tool, index) =
                sample.unwrap_or_else(|| panic!("READY desconocido: {name}"));
            assert!(
                !handled.contains(&(variant.clone(), tool.clone(), index)),
                "READY duplicado: {name}"
            );
            return Some((
                ReadyEvent::Sample {
                    variant,
                    tool,
                    index,
                },
                path,
            ));
        }
        if child
            .child_mut()
            .try_wait()
            .expect("esperar estado del banco A4")
            .is_some()
        {
            return None;
        }
        assert!(
            Instant::now() < deadline,
            "no se recibieron todos los READY A4; atendidos={handled:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn mutate_generation(root: &Path, generation: usize) {
    fs::write(
        root.join("child.md"),
        format!(
            "---\ntags: [child, generation-{generation}]\nservice: bench\n---\n# Child generation {generation}\nmarker-get-h04-{generation}\n[leaf](leaf.md)\n"
        ),
    )
    .expect("mutar child");
    fs::write(
        root.join(format!("generation-{generation}.md")),
        format!(
            "---\ntags: [generation-{generation}, new]\nservice: bench\n---\n# Generation {generation}\nmarker-search-h04 marker-search-h04-{generation}\n[child](child.md)\n[dangling-{generation}](missing-generation-{generation}.md)\n"
        ),
    )
    .expect("añadir generación");
    fs::write(
        root.join(format!("store-only-{generation}.md")),
        format!(
            "---\ntags: [store-only, generation-{generation}]\nservice: bench\n---\n# Store only {generation}\nmarker-search-h04 store-only-{generation}\n[child](child.md)\n[dangling-{generation}](missing-store-{generation}.md)\n"
        ),
    )
    .expect("añadir documento solo Store");
}

fn document(path: &str) -> DocumentRef {
    DocumentRef {
        path: RelPath::new(path).expect("fixture relpath"),
        id: None,
    }
}

fn encoded<T: Serialize>(result: Result<T, AppError>, context: &str) -> Value {
    serde_json::to_value(result.unwrap_or_else(|error| panic!("{context}: {error:?}")))
        .expect("resultado serializable")
}

fn seven_reads_on_snapshot(seam: &ReadServices<'_>) -> Vec<Value> {
    vec![
        encoded(
            seam.workspace_status(Profile::Standard),
            "workspace_status snapshot",
        ),
        encoded(
            seam.knowledge_search("marker-search-h04", None, None, &[], None, None),
            "knowledge_search snapshot",
        ),
        encoded(
            seam.knowledge_get(&document("b.md"), &["body".into()], None),
            "knowledge_get snapshot",
        ),
        encoded(
            seam.metadata_inspect("field", Some("tags"), None, None),
            "metadata_inspect snapshot",
        ),
        encoded(
            seam.graph_query("components", None, None, None, None, None, None),
            "graph_query snapshot",
        ),
        encoded(
            seam.impact_analyze(&document("b.md"), "delete", None),
            "impact_analyze snapshot",
        ),
        encoded(
            seam.knowledge_check(
                &CheckScope::Document {
                    r#ref: document("b.md"),
                },
                Some(Severity::Info),
                true,
                None,
                None,
            ),
            "knowledge_check Document snapshot",
        ),
        encoded(
            seam.knowledge_check(
                &CheckScope::Affected {
                    refs: vec![document("b.md")],
                    depth: 2,
                },
                Some(Severity::Info),
                true,
                None,
                None,
            ),
            "knowledge_check Affected snapshot",
        ),
    ]
}

fn expected_tool_from_seam(seam: &ReadServices<'_>, tool: &str) -> Value {
    match tool {
        "workspace_status" => encoded(
            seam.workspace_status(Profile::Standard),
            "workspace_status oráculo fresco",
        ),
        "knowledge_search" => encoded(
            seam.knowledge_search(
                "marker-search-h04",
                Some("service = \"bench\""),
                None,
                &[],
                None,
                None,
            ),
            "knowledge_search oráculo fresco",
        ),
        "knowledge_get" => encoded(
            seam.knowledge_get(
                &document("child.md"),
                &[
                    "body".into(),
                    "frontmatter".into(),
                    "outgoingLinks".into(),
                    "backlinks".into(),
                    "diagnostics".into(),
                ],
                None,
            ),
            "knowledge_get oráculo fresco",
        ),
        "metadata_inspect" => encoded(
            seam.metadata_inspect("field", Some("tags"), None, None),
            "metadata_inspect oráculo fresco",
        ),
        "graph_query" => encoded(
            seam.graph_query("components", None, None, None, None, None, None),
            "graph_query oráculo fresco",
        ),
        "impact_analyze" => encoded(
            seam.impact_analyze(&document("child.md"), "delete", None),
            "impact_analyze oráculo fresco",
        ),
        "knowledge_check" => encoded(
            seam.knowledge_check(
                &CheckScope::Workspace,
                Some(Severity::Info),
                true,
                None,
                None,
            ),
            "knowledge_check oráculo fresco",
        ),
        _ => panic!("tool A4 no permitida: {tool}"),
    }
}

fn expected_tool_from_fresh_app(root: &Path, tool: &str) -> Value {
    let app = App::open(root).expect("App fresco para oráculo A4");
    match tool {
        "workspace_status" => encoded(
            app.workspace_status(Profile::Standard),
            "workspace_status App fresco",
        ),
        "knowledge_search" => encoded(
            app.knowledge_search(
                "marker-search-h04",
                Some("service = \"bench\""),
                None,
                &[],
                None,
                None,
            ),
            "knowledge_search App fresco",
        ),
        "knowledge_get" => encoded(
            app.knowledge_get(
                &document("child.md"),
                &[
                    "body".into(),
                    "frontmatter".into(),
                    "outgoingLinks".into(),
                    "backlinks".into(),
                    "diagnostics".into(),
                ],
                None,
            ),
            "knowledge_get App fresco",
        ),
        "metadata_inspect" => encoded(
            app.metadata_inspect("field", Some("tags"), None, None),
            "metadata_inspect App fresco",
        ),
        "graph_query" => encoded(
            app.graph_query("components", None, None, None, None, None, None),
            "graph_query App fresco",
        ),
        "impact_analyze" => encoded(
            app.impact_analyze(&document("child.md"), "delete", None),
            "impact_analyze App fresco",
        ),
        "knowledge_check" => encoded(
            app.knowledge_check(
                &CheckScope::Workspace,
                Some(Severity::Info),
                true,
                None,
                None,
            ),
            "knowledge_check App fresco",
        ),
        _ => panic!("tool A4 no permitida: {tool}"),
    }
}

fn expected_tool_from_store(root: &Path, tool: &str) -> Value {
    let app = App::open(root).expect("App para oráculo SQLite A4");
    let store = Store::open(root).expect("Store para oráculo SQLite A4");
    store.rebuild().expect("rebuild externo SQLite A4");
    let store_set = store.document_set();
    let seam = app.read_services(&store_set);
    expected_tool_from_seam(&seam, tool)
}

fn expected_tools_from_ram_snapshot(
    root: &Path,
    tools: &BTreeSet<&str>,
) -> BTreeMap<String, Value> {
    let app = App::open(root).expect("App para snapshot RAM A4");
    let ram_set = app.workspace().document_set().expect("DocumentSet RAM A4");
    let seam = app.read_services(&ram_set);
    tools
        .iter()
        .map(|tool| ((*tool).to_owned(), expected_tool_from_seam(&seam, tool)))
        .collect()
}

fn expected_tools_from_fresh_app(root: &Path, tools: &BTreeSet<&str>) -> BTreeMap<String, Value> {
    tools
        .iter()
        .map(|tool| ((*tool).to_owned(), expected_tool_from_fresh_app(root, tool)))
        .collect()
}

/// BDD-H04-R1: un kind inválido es INVALID_SCHEMA antes de cualquier discovery IO.
#[test]
fn impact_analyze_valida_schema_antes_de_discovery_io() {
    let root = TempDir::new().expect("root");
    fs::write(root.path().join("ok.md"), "# ok\n").expect("ok fixture");
    fs::create_dir_all(root.path().join(".lodestar")).expect("config dir");
    fs::write(
        root.path().join(".lodestar/config.yaml"),
        "discovery:\n  include: [\"[\"]\n",
    )
    .expect("glob inválido");
    let app = App::open(root.path()).expect("App::open no debe descubrir documentos");
    // El root desaparece después de abrir App. Si impact_analyze intenta adquirir el snapshot
    // antes de validar `kind`, devuelve WORKSPACE_NOT_FOUND/IO y oculta el error accionable.
    let detached = root.path().to_path_buf();
    fs::remove_dir_all(&detached).expect("retirar root para probar el orden de validación");

    let error = app
        .impact_analyze(&document("ok.md"), "replace_document", None)
        .expect_err("kind inválido debe fallar aunque el discovery ya no sea posible");
    assert_eq!(
        error.code,
        ErrorCode::InvalidSchema,
        "H04-R1: INVALID_SCHEMA debe preceder a cualquier IO de discovery"
    );
}

/// BDD-H04-R1b: graph_query sobre un seam no debe reabrir el disco ni mezclar otro snapshot.
#[test]
fn graph_query_con_ref_usa_el_document_set_memoizado_sin_reabrir_disco() {
    let root = TempDir::new().expect("root");
    write_graph_fixture(root.path());
    let app = App::open(root.path()).expect("open");
    let (snapshot, discovery) = app
        .workspace()
        .document_set_with_discovery()
        .expect("snapshot inicial");
    let seam = app
        .read_services(&snapshot)
        .with_discovery_diagnostics(discovery);

    // El snapshot ya contiene b.md. Un seam que consulte App::resolve_ref/discovery otra vez
    // pierde ese documento y no está midiendo el mismo conjunto de documentos.
    fs::write(root.path().join("b.md"), [0xff, 0xfe, 0xfd]).expect("mutación ilegible");
    let result = seam
        .graph_query(
            "backlinks",
            Some(&document("b.md")),
            None,
            None,
            None,
            None,
            None,
        )
        .expect("graph_query debe operar sobre el snapshot memoizado");
    assert!(!result.nodes.is_empty(), "H04-R1b: nodes no vacío");
    assert!(!result.edges.is_empty(), "H04-R1b: edge a→b no vacío");
}

/// BDD-H04-R1c: impact_analyze válido también debe consumir exactamente el DocumentSet capturado.
#[test]
fn impact_analyze_valido_resuelve_desde_snapshot_sin_reabrir_disco() {
    let root = TempDir::new().expect("root");
    write_graph_fixture(root.path());
    let app = App::open(root.path()).expect("open");
    let (snapshot, discovery) = app
        .workspace()
        .document_set_with_discovery()
        .expect("snapshot inicial");
    let seam = app
        .read_services(&snapshot)
        .with_discovery_diagnostics(discovery);

    // b.md sigue en el DocumentSet capturado, aunque ya no existe en disco.
    fs::remove_file(root.path().join("b.md")).expect("eliminar fuente después del snapshot");
    let result = seam
        .impact_analyze(&document("b.md"), "delete", None)
        .expect("impact_analyze debe resolver el ref desde el snapshot");
    assert_eq!(result.summary.directly_affected, 1);
    assert_eq!(result.summary.transitively_affected, 1);
}

/// BDD-H04-R1d/A1: las siete lecturas, incluidos Document/Affected y el contexto de receipts,
/// permanecen exactamente sobre un único DocumentSet aunque el disco se retire después.
#[test]
fn siete_lecturas_integrales_conservan_resultado_sobre_snapshot_unico() {
    let root = TempDir::new().expect("root");
    write_graph_fixture(root.path());
    let app = App::open(root.path()).expect("open");
    let plan = app
        .change_plan(
            None,
            &json!([{
                "op": "replace_body",
                "path": "a.md",
                "body": "# A\n[b](b.md)\n\nreceipt-marker\n"
            }]),
            PlanPolicy {
                require_valid_result: false,
                allow_warnings: true,
            },
        )
        .expect("plan receipt context");
    app.change_apply(&plan.change_set_id, None)
        .expect("apply receipt context");
    let (snapshot, discovery) = app
        .workspace()
        .document_set_with_discovery()
        .expect("snapshot único");
    let seam = app
        .read_services(&snapshot)
        .with_discovery_diagnostics(discovery);
    let before = seven_reads_on_snapshot(&seam);
    let status = before[0].as_object().expect("workspace_status result");
    assert!(!status["receipts"].as_array().unwrap().is_empty());
    for (index, result) in before.iter().enumerate() {
        assert!(
            result.as_object().is_some_and(|object| !object.is_empty()),
            "lectura {index} no puede ser vacía"
        );
    }

    fs::remove_dir_all(root.path()).expect("retirar todo el root tras adquisición");
    let after = seven_reads_on_snapshot(&seam);
    assert_eq!(
        before, after,
        "A1: ninguna lectura puede mezclar otro snapshot"
    );
}

/// BDD-H04-R2: la medición conserva las siete métricas y separa rebuild sin inventar una forma
/// adicional de traza interna.
#[test]
fn mediciones_sqlite_registran_rebuild_separado_y_metricas_no_vacias() {
    let root = TempDir::new().expect("root");
    fs::write(
        root.path().join("control.md"),
        "---\nservice: bench\n---\n# control\n",
    )
    .expect("fixture");
    let report = json_stdout(
        bench()
            .args([
                "--smoke",
                "--seed",
                "33",
                "--root",
                root.path().to_str().unwrap(),
            ])
            .output()
            .expect("run smoke"),
        "H04-R2",
    );
    let sqlite = report["measurements"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["variant"] == "sqlite-raw"))
        .expect("fila sqlite-raw");
    let rebuild = sqlite["rebuild"].as_object().expect("rebuild separado");
    assert!(rebuild["sample_count"].as_u64().unwrap_or(0) > 0);
    assert!(rebuild["p95_ns"].as_u64().unwrap_or(0) >= rebuild["p50_ns"].as_u64().unwrap_or(1));
    let tools = sqlite["tools"].as_object().expect("tools sqlite");
    assert_eq!(
        tools.len(),
        7,
        "H04-R2: SQLite debe medir las siete lecturas"
    );
    for (tool, metric) in tools {
        assert!(
            metric["sample_count"].as_u64().unwrap_or(0) > 0,
            "H04-R2/{tool}: métrica sin muestras"
        );
        assert!(
            metric["payload_bytes"].as_u64().unwrap_or(0) > 0,
            "H04-R2/{tool}: payload vacío"
        );
    }
    let ram = report["measurements"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["variant"] == "ram-memoizado"))
        .expect("fila RAM");
    assert_eq!(ram["tools"].as_object().map(|tools| tools.len()), Some(7));
}

/// BDD-H04-R2b/A4: el probe debe demostrar adquisición real con una mutación externa entre
/// muestras; no basta con que el proceso se auto-declare instrumentado.
#[test]
fn probe_de_adquisicion_observa_mutacion_externa_por_variante() {
    let root = TempDir::new().expect("root");
    fs::write(root.path().join("one.md"), "# one\n").expect("fixture");
    let mut child = bench()
        .args(["--probe-acquisition-root", root.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn probe");
    let stdout = child.stdout.take().expect("probe stdout");
    let mut lines = BufReader::new(stdout).lines();
    let ready: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).expect("READY JSON");
    assert_eq!(ready["event"], "READY");
    // La mutación ocurre después de READY y antes de continue: el test controla el oráculo.
    let before_files = fs::read_dir(root.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
        .count();
    assert_eq!(before_files, 1);
    fs::write(root.path().join("second.md"), "# second\n").expect("mutación controlada");
    writeln!(child.stdin.as_mut().unwrap(), "continue").expect("continue");
    drop(child.stdin.take());
    let report: Value = serde_json::from_str(
        &lines
            .map(|line| line.unwrap())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .expect("probe final JSON");
    let before = report["before"].as_object().expect("before");
    let after = report["after"].as_object().expect("after");
    for variant in ["disk-reparseo", "sqlite-raw"] {
        assert!(
            after[variant]["document_count"].as_u64().unwrap()
                > before[variant]["document_count"].as_u64().unwrap(),
            "H04-R2b/{variant}: adquisición posterior debe ver second.md"
        );
    }
    assert_eq!(
        after["ram-memoizado"]["document_count"], before["ram-memoizado"]["document_count"],
        "H04-R2b/RAM: el DocumentSet memoizado no debe cambiar"
    );
    assert!(report["rebuild"]["sample_count"].as_u64().unwrap_or(0) > 0);
    assert!(child.wait().expect("probe wait").success());
}

/// BDD-A4 normal smoke: el control es privado del banco, pero la corrida que se mide sigue siendo
/// la ruta normal. Disk/SQLite barreran cada muestra (incluida la 1), RAM fija un snapshot previo
/// y solo barrera sus muestras posteriores; el oráculo distingue App, Store y RAM.
#[test]
fn smoke_normal_honra_barrera_y_muestra_adquisicion_por_variante() {
    let root = TempDir::new().expect("corpus root");
    let barrier = TempDir::new().expect("barrier root");
    let parent = TempDir::new().expect("cycle parent");
    write_a4_fixture(root.path());
    let tools: BTreeSet<_> = [
        "workspace_status",
        "knowledge_search",
        "knowledge_get",
        "metadata_inspect",
        "graph_query",
        "impact_analyze",
        "knowledge_check",
    ]
    .into_iter()
    .collect();
    let baseline_app = expected_tools_from_fresh_app(root.path(), &tools);
    let mut command = bench();
    command
        .args([
            "--smoke",
            "--seed",
            "33",
            "--root",
            root.path().to_str().unwrap(),
        ])
        .env(BARRIER_ENV, barrier.path())
        .env(CHANGE_PARENT_ENV, parent.path())
        .env(ITERATIONS_ENV, "3");
    let child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn smoke normal instrumentado");
    let mut child = ChildGuard::new(child);
    let stdout = child
        .child_mut()
        .stdout
        .take()
        .expect("stdout solo diagnóstico final");
    drop(child.child_mut().stdin.take());
    let variants = ["disk-reparseo", "sqlite-raw", "ram-memoizado"];
    let mut handled = BTreeSet::new();
    let mut ram_acquired = false;
    let mut ram_expected: Option<BTreeMap<String, Value>> = None;
    let mut sample_expected: BTreeMap<(String, String, usize), Value> = BTreeMap::new();
    let mut last_expected: BTreeMap<(String, String), Value> = BTreeMap::new();
    let mut generation = 0;
    let deadline = Instant::now() + Duration::from_secs(20);
    while let Some((event, ready)) = wait_for_next_ready(
        barrier.path(),
        &variants,
        &tools,
        &handled,
        ram_acquired,
        &mut child,
        deadline,
    ) {
        fs::remove_file(&ready).expect("retirar READY atendido");
        match event {
            ReadyEvent::RamAcquire => {
                ram_expected = Some(expected_tools_from_ram_snapshot(root.path(), &tools));
                ram_acquired = true;
                fs::write(
                    barrier.path().join("CONTINUE-ram-memoizado-ACQUIRE"),
                    "snapshot=R\n",
                )
                .expect("CONTINUE RAM ACQUIRE");
            }
            ReadyEvent::Sample {
                variant,
                tool,
                index,
            } => {
                assert!(
                    variant != "ram-memoizado" || ram_acquired,
                    "RAM sample antes de ACQUIRE"
                );
                generation += 1;
                let state = generation * 2;
                let key = (variant.clone(), tool.clone(), index);
                match variant.as_str() {
                    "disk-reparseo" => {
                        mutate_generation(root.path(), state);
                        let fresh = expected_tool_from_fresh_app(root.path(), &tool);
                        if let Some(previous) =
                            last_expected.insert((variant.clone(), tool.clone()), fresh.clone())
                        {
                            assert_ne!(
                                fresh, previous,
                                "disk/{tool}/{index}: mutación no cambia el oráculo"
                            );
                        } else {
                            assert_ne!(
                                &fresh,
                                baseline_app.get(&tool).expect("baseline App por tool"),
                                "disk/{tool}/1: mutación no cambia el oráculo inicial"
                            );
                        }
                        sample_expected.insert(key, fresh);
                    }
                    "sqlite-raw" => {
                        mutate_generation(root.path(), state);
                        let store = expected_tool_from_store(root.path(), &tool);
                        mutate_generation(root.path(), state + 1);
                        let disk = expected_tool_from_fresh_app(root.path(), &tool);
                        assert_ne!(
                            store, disk,
                            "sqlite/{tool}/{index}: Store y disco no divergen sin rebuild"
                        );
                        if let Some(previous) =
                            last_expected.insert((variant.clone(), tool.clone()), store.clone())
                        {
                            assert_ne!(
                                store, previous,
                                "sqlite/{tool}/{index}: mutación no cambia Store"
                            );
                        } else {
                            assert_ne!(
                                &store,
                                baseline_app.get(&tool).expect("baseline App por tool"),
                                "sqlite/{tool}/1: mutación no cambia Store inicial"
                            );
                        }
                        sample_expected.insert(key, store);
                    }
                    "ram-memoizado" => {
                        mutate_generation(root.path(), state);
                    }
                    _ => panic!("variante A4 desconocida: {variant}"),
                }
                fs::write(
                    barrier
                        .path()
                        .join(format!("CONTINUE-{variant}-{tool}-{index}")),
                    format!("generation={state}\n"),
                )
                .expect("CONTINUE sample A4");
                assert!(handled.insert((variant, tool, index)), "sample duplicada");
            }
        }
    }
    let report_text = BufReader::new(stdout)
        .lines()
        .map(|line| line.expect("salida final UTF-8"))
        .collect::<Vec<_>>()
        .join("\n");
    let output = child.take().wait_with_output().expect("smoke wait");
    assert!(
        output.status.success(),
        "smoke normal instrumentado: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(ram_acquired, "debe atenderse READY RAM ACQUIRE");
    assert!(
        !handled.is_empty(),
        "debe atenderse al menos una muestra A4"
    );
    let report: Value = serde_json::from_str(report_text.trim()).expect("informe smoke final");

    let trace = report["acquisition_trace"]
        .as_object()
        .expect("traza privada de adquisición normal");
    for variant in variants {
        let variant_trace = trace[variant].as_object().expect("traza variante");
        assert_eq!(
            variant_trace
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            tools
        );
        let row = report["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["variant"] == variant)
            .expect("fila variante");
        for tool in &tools {
            let samples = variant_trace[*tool].as_array().expect("muestras tool");
            let sample_count = row["tools"][*tool]["sample_count"]
                .as_u64()
                .expect("sample_count métrica");
            assert_eq!(sample_count, 3, "{variant}/{tool}: N test-only debe ser 3");
            assert_eq!(samples.len(), sample_count as usize);
            let observed_indices: BTreeSet<_> = handled
                .iter()
                .filter(|(seen_variant, seen_tool, _)| {
                    seen_variant == variant && seen_tool == *tool
                })
                .map(|(_, _, index)| *index)
                .collect();
            let expected_indices: BTreeSet<_> = if variant == "ram-memoizado" {
                (2..=sample_count as usize).collect()
            } else {
                (1..=sample_count as usize).collect()
            };
            assert_eq!(
                observed_indices, expected_indices,
                "{variant}/{tool}: faltan/sobran barreras por sample_count"
            );
            if variant == "ram-memoizado" {
                let ram = ram_expected
                    .as_ref()
                    .expect("snapshot RAM antes de samples")
                    .get(*tool)
                    .expect("oráculo RAM por tool");
                for sample in samples {
                    assert_eq!(
                        &sample["result"], ram,
                        "RAM/{tool}: snapshot debe ser estable"
                    );
                }
            } else {
                for index in 1..=sample_count as usize {
                    let oracle = sample_expected
                        .get(&(variant.to_owned(), (*tool).to_owned(), index))
                        .expect("oráculo backend por sample");
                    assert_eq!(
                        &samples[index - 1]["result"],
                        oracle,
                        "{variant}/{tool}/{index}: resultado backend divergente"
                    );
                    if index == 1 {
                        assert_ne!(
                            &samples[0]["result"],
                            baseline_app.get(*tool).expect("baseline App por tool"),
                            "{variant}/{tool}/1: mutación no observable"
                        );
                    } else {
                        assert_ne!(
                            &samples[index - 2]["result"],
                            oracle,
                            "{variant}/{tool}/{index}: mutación no observable"
                        );
                    }
                }
            }
        }
    }
    let sqlite_row = report["measurements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["variant"] == "sqlite-raw")
        .expect("fila SQLite");
    let rebuild = sqlite_row["rebuild"].as_object().expect("rebuild separado");
    assert!(rebuild["sample_count"].as_u64().unwrap_or(0) > 0);
    assert!(sqlite_row["tools"]
        .as_object()
        .unwrap()
        .values()
        .all(|metric| metric.get("rebuild").is_none()));
}

/// BDD-A5 normal smoke: N aplica reales, receipts abiertos desde el parent poseído y ningún
/// hermano preexistente tocado.
#[test]
fn ciclo_smoke_normal_hace_applies_reales_bajo_parent_poseido() {
    let root = TempDir::new().expect("corpus root");
    let holder = TempDir::new().expect("holder de ownership");
    let exact_root = holder.path().join("owned-cycle-root");
    fs::create_dir(&exact_root).expect("root exacto de ciclo");
    fs::write(root.path().join("control.md"), "# before\n").expect("fixture");
    let internal_marker = exact_root.join("ownership-marker");
    fs::write(&internal_marker, "intacto dentro del root exacto\n").expect("marker interno");
    let sentinel = holder.path().join("sibling-sentinel.md");
    fs::write(&sentinel, "no tocar\n").expect("sentinel");
    let exact_root = fs::canonicalize(&exact_root).expect("canonical root exacto");
    let mut command = bench();
    command
        .args([
            "--smoke",
            "--seed",
            "33",
            "--root",
            root.path().to_str().unwrap(),
        ])
        .env(CHANGE_PARENT_ENV, &exact_root);
    let report = json_stdout(
        command.output().expect("run smoke normal con parent"),
        "A5 smoke cycle",
    );
    assert_eq!(report["change_cycle"]["source"], "app/disk");
    let cycle = &report["change_cycle"];
    let iterations = cycle["iterations"].as_u64().expect("iterations");
    assert!(iterations >= 2);
    let metric = cycle["metric"].as_object().expect("metric ciclo");
    assert_eq!(metric["sample_count"].as_u64(), Some(iterations));
    let p50 = metric["p50_ns"].as_u64().expect("p50 ciclo");
    let p95 = metric["p95_ns"].as_u64().expect("p95 ciclo");
    assert!(p50 > 0, "p50 ciclo no positivo");
    assert!(p95 > 0, "p95 ciclo no positivo");
    assert!(p95 >= p50, "p95 ciclo < p50");
    assert!(metric["payload_bytes"].as_u64().unwrap_or(0) > 0);
    let receipts = cycle["receipts"].as_array().expect("receipts reales");
    assert_eq!(receipts.len(), iterations as usize);
    let mut receipt_ids = BTreeSet::new();
    for receipt in receipts {
        assert_eq!(receipt["apply"]["applied"], true, "ApplyResult real");
        let apply_id = receipt["apply"]["receiptId"]
            .as_str()
            .expect("ApplyResult receiptId");
        assert!(receipt_ids.insert(apply_id), "receiptId reutilizado");
        let raw_path = receipt["receipt_path"].as_str().expect("receipt_path");
        let raw = Path::new(raw_path);
        assert!(raw
            .components()
            .all(|component| matches!(component, Component::Normal(_))));
        let receipt_path = exact_root.join(raw);
        let receipt_path = fs::canonicalize(&receipt_path)
            .unwrap_or_else(|_| panic!("receipt inexistente: {raw_path}"));
        assert!(receipt_path.starts_with(&exact_root));
        assert!(receipt_path.is_file(), "receipt inexistente: {raw_path}");
        let on_disk: Value = serde_json::from_str(&fs::read_to_string(&receipt_path).unwrap())
            .expect("receipt JSON");
        assert_eq!(on_disk["id"].as_str(), Some(apply_id));
        assert_eq!(receipt["changed_paths"], json!(["control.md"]));
        let body_changes = on_disk["semanticDiff"]["bodyChanges"]
            .as_array()
            .expect("bodyChanges del receipt real");
        assert_eq!(
            body_changes.len(),
            1,
            "un cambio de contenido por iteración"
        );
        assert_eq!(body_changes[0], "control.md");
    }
    assert_eq!(
        fs::read(&internal_marker).unwrap(),
        b"intacto dentro del root exacto\n"
    );
    assert_eq!(fs::read(&sentinel).unwrap(), b"no tocar\n");
}

/// BDD-H04-R3b: todo fixture y estado de ciclo debe permanecer dentro del TempDir poseído por el
/// test; el proceso debe terminar correctamente y dejar un recibo real.
#[test]
fn ciclo_de_cambio_respeta_ownership_del_tempdir_y_produce_recibo() {
    let root = TempDir::new().expect("root");
    fs::write(root.path().join("control.md"), "# before\n").expect("fixture");
    let output = bench()
        .args(["--probe-change-root", root.path().to_str().unwrap()])
        .output()
        .expect("run smoke");
    let report = json_stdout(output, "H04-R3b");
    let receipt_path = root.path().join(report["receipt_path"].as_str().unwrap());
    assert!(receipt_path.starts_with(root.path()));
    assert!(
        receipt_path.is_file(),
        "H04-R3b: ciclo sin receipt dentro del TempDir"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("control.md"))
            .unwrap()
            .matches("after-state")
            .count(),
        1
    );
}

/// BDD-H04-R4: abre el receipt real, y cruza su id con el ApplyResult real y su ruta del informe.
#[test]
fn receipt_se_cruza_con_apply_receipt_id_y_no_es_fabricable() {
    let root = TempDir::new().expect("root");
    fs::write(root.path().join("control.md"), "# before\n").expect("fixture");
    let report = json_stdout(
        bench()
            .args(["--probe-change-root", root.path().to_str().unwrap()])
            .output()
            .expect("run smoke"),
        "H04-R4",
    );
    let apply = report["apply"].as_object().expect("apply completo");
    let apply_id = apply
        .get("receiptId")
        .and_then(Value::as_str)
        .expect("apply.receiptId: el ciclo debe cruzar el recibo real");
    let path = report["receipt_path"].as_str().expect("receipt_path");
    let receipt_path = root.path().join(path);
    assert!(receipt_path.is_file(), "receipt_path debe existir en disco");
    let receipt: Value = serde_json::from_str(&fs::read_to_string(&receipt_path).unwrap())
        .expect("receipt JSON real");
    assert_eq!(receipt["id"].as_str(), Some(apply_id));
    assert_eq!(
        receipt["id"].as_str(),
        Path::new(path).file_stem().and_then(|s| s.to_str())
    );
}

/// Estructural-A6/R5 smoke: la calibración se marca pendiente hasta ejecutar el arnés wire real.
#[test]
fn smoke_no_fabrica_calibracion_wire() {
    let root = TempDir::new().expect("root");
    fs::write(root.path().join("control.md"), "# control\n").expect("fixture");
    let report = json_stdout(
        bench()
            .args([
                "--smoke",
                "--seed",
                "33",
                "--root",
                root.path().to_str().unwrap(),
            ])
            .output()
            .expect("run smoke"),
        "H04-R5",
    );
    let wire = report["wire_calibration"].as_object().expect("wire");
    assert_eq!(wire["status"], "pending");
    assert!(wire["results"].as_array().unwrap().is_empty());
}

/// Estructural-A6/R5 full: el artefacto oficial datado prueba la matriz completa y una calibración
/// wire estructurada. Es un guard protector: no fabrica rojo si el artefacto ya es válido.
#[test]
fn artefacto_full_oficial_prueba_matriz_y_calibracion_real() {
    // La adenda de retención deja el bruto fuera de Git: el formato se valida con un fixture pequeño.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let json_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/e33_h04_full_format.json");
    let md_path = root.join("docs/qa/e33-h04-banco-rendimiento-2026-08-22.md");
    let report: Value =
        serde_json::from_str(&fs::read_to_string(json_path).expect("fixture H04 pequeña"))
            .expect("fixture H04 válida");

    assert_eq!(report["profiles"], serde_json::json!(["plano", "realista"]));
    assert_eq!(report["scales"], serde_json::json!([100, 1000, 10000]));
    let runs = report["runs"].as_array().expect("runs");
    assert_eq!(runs.len(), 6, "2 perfiles × 3 escalas");
    let expected_variants: BTreeSet<_> = ["disk-reparseo", "sqlite-raw", "ram-memoizado"]
        .into_iter()
        .collect();
    let expected_tools: BTreeSet<_> = [
        "workspace_status",
        "knowledge_search",
        "knowledge_get",
        "metadata_inspect",
        "graph_query",
        "impact_analyze",
        "knowledge_check",
    ]
    .into_iter()
    .collect();

    for run in runs {
        let scale = run["scale"].as_u64().expect("scale");
        let measurements = run["measurements"].as_array().expect("measurements");
        assert_eq!(measurements.len(), 3, "3 variantes por corrida");
        let variants: BTreeSet<_> = measurements
            .iter()
            .filter_map(|row| row["variant"].as_str())
            .collect();
        assert_eq!(variants, expected_variants);
        for row in measurements {
            assert_eq!(row["document_count"], scale);
            let cold = row["cold_open"].as_object().expect("cold_open");
            assert!(cold["sample_count"].as_u64().unwrap_or(0) > 0);
            assert!(cold["p50_ns"].as_u64().unwrap_or(0) > 0);
            assert!(cold["p95_ns"].as_u64().unwrap_or(0) >= cold["p50_ns"].as_u64().unwrap_or(0));
            assert!(cold["payload_bytes"].as_u64().unwrap_or(0) > 0);
            assert_eq!(cold["result"]["counts"]["documents"], scale);
            let tools = row["tools"].as_object().expect("tools");
            assert_eq!(
                tools.keys().map(String::as_str).collect::<BTreeSet<_>>(),
                expected_tools
            );
            for (tool, metric) in tools {
                assert!(
                    metric["sample_count"].as_u64().unwrap_or(0) > 0,
                    "{tool}: muestras"
                );
                assert!(metric["p50_ns"].as_u64().unwrap_or(0) > 0, "{tool}: p50");
                assert!(
                    metric["p95_ns"].as_u64().unwrap_or(0)
                        >= metric["p50_ns"].as_u64().unwrap_or(0),
                    "{tool}: p95"
                );
                assert!(
                    metric["payload_bytes"].as_u64().unwrap_or(0) > 0,
                    "{tool}: payload"
                );
                assert!(!metric["result"].is_null(), "{tool}: result");
            }
            if row["variant"] == "sqlite-raw" {
                assert!(row["rebuild"]["p95_ns"].as_u64().unwrap_or(0) > 0);
            } else {
                assert!(row.get("rebuild").is_none());
            }
        }
        let cycle = run["change_cycle"].as_object().expect("change_cycle");
        assert_eq!(cycle["source"], "app/disk");
        assert!(cycle["metric"]["p95_ns"].as_u64().unwrap_or(0) > 0);
    }

    let wire = report["wire_calibration"].as_object().expect("wire");
    assert_eq!(wire["status"], "complete");
    assert_eq!(wire["profile"], "realista");
    assert_eq!(wire["scale"], 10000);
    assert_eq!(wire["transport"], "JSON-RPC/stdio");
    assert_eq!(wire["binary"], "lodestar-mcp");
    let results = wire["results"].as_array().expect("resultados wire");
    assert_eq!(results.len(), 2);
    for result in results {
        assert!(result["sample_count"].as_u64().unwrap_or(0) > 0);
        assert!(result["payload_bytes"].as_u64().unwrap_or(0) > 0);
        assert!(result["p95_seconds"].as_f64().unwrap_or(0.0) > 0.0);
    }
    assert_eq!(report["commit"], report["provenance"]["commit"]);
    assert!(report["provenance"]["working_tree_clean"].is_boolean());

    let markdown = fs::read_to_string(md_path).expect("resumen H04 existente");
    let markdown_lower = markdown.to_ascii_lowercase();
    assert!(markdown.contains("| Perfil | Escala | Variante | Tool |"));
    assert!(markdown_lower.contains("rebuild"));
    assert!(markdown_lower.contains("sqlite-raw"));
    assert!(markdown_lower.contains("from_store"));
    for tool in expected_tools {
        assert!(
            markdown.contains(&format!("| {tool} |")),
            "MD sin tool {tool}"
        );
    }
}

/// Estructural-A6: una dependencia externa sin features no puede importar el seam; el segundo
/// check habilita únicamente `bench-internal` para demostrar que el negativo no es ambiental.
#[test]
fn api_default_no_expone_read_services_y_feature_explicita_si_compila() {
    let workspace = TempDir::new().expect("workspace crate externo");
    let target = TempDir::new().expect("target externo poseído");
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo");
    let app_path = repo.join("crates/lodestar-app");
    let app_path_literal =
        serde_json::to_string(&app_path.to_string_lossy()).expect("ruta app como literal TOML");
    let manifest_path = workspace.path().join("Cargo.toml");
    let source_path = workspace.path().join("src/main.rs");
    fs::create_dir_all(workspace.path().join("src")).expect("src externo");
    let manifest = |features: Option<&str>| {
        let feature_line = features
            .map(|value| format!(", features = [\"{value}\"]"))
            .unwrap_or_default();
        format!(
            "[package]\nname = \"lodestar-api-guard\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\nlodestar-app = {{ path = {app_path_literal}{feature_line} }}\n",
        )
    };
    fs::write(
        &source_path,
        "use lodestar_app::ReadServices;\nfn main() {}\n",
    )
    .expect("source externo");
    fs::write(&manifest_path, manifest(None)).expect("manifest default");
    let default_check = Command::new("cargo")
        .args(["check", "--offline", "--manifest-path"])
        .arg(&manifest_path)
        .env("CARGO_TARGET_DIR", target.path())
        .output()
        .expect("cargo check default");
    assert!(
        !default_check.status.success(),
        "ReadServices no es API default"
    );
    let default_stderr = String::from_utf8_lossy(&default_check.stderr);
    assert!(
        default_stderr.contains("ReadServices")
            && (default_stderr.contains("unresolved import")
                || default_stderr.contains("no `ReadServices`")),
        "fallo debe ser específicamente el import: {default_stderr}"
    );
    fs::write(&manifest_path, manifest(Some("bench-internal"))).expect("manifest feature");
    let feature_check = Command::new("cargo")
        .args(["check", "--offline", "--manifest-path"])
        .arg(&manifest_path)
        .env("CARGO_TARGET_DIR", target.path())
        .output()
        .expect("cargo check feature");
    assert!(
        feature_check.status.success(),
        "bench-internal debe habilitar el seam explícitamente: {}",
        String::from_utf8_lossy(&feature_check.stderr)
    );
}

/// Estructural-A6/R6: una corrida en árbol sucio no puede afirmar un commit limpio.
#[test]
fn provenance_no_afirma_commit_limpio_en_worktree_dirty() {
    let root = TempDir::new().expect("root");
    fs::write(root.path().join("control.md"), "# control\n").expect("fixture");
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo)
        .output()
        .expect("git status");
    assert!(
        status.status.success(),
        "git status debe ser un oráculo válido"
    );
    let dirty = !status.stdout.is_empty();
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .expect("git rev-parse");
    assert!(head.status.success(), "git rev-parse debe funcionar");
    let head = String::from_utf8(head.stdout).unwrap().trim().to_owned();
    let report = json_stdout(
        bench()
            .args([
                "--smoke",
                "--seed",
                "33",
                "--root",
                root.path().to_str().unwrap(),
            ])
            .output()
            .expect("run smoke"),
        "H04-R6",
    );
    assert_eq!(report["commit"].as_str(), Some(head.as_str()));
    let provenance = report["provenance"]
        .as_object()
        .expect("provenance explícita");
    let clean = provenance
        .get("working_tree_clean")
        .or_else(|| provenance.get("clean"))
        .and_then(Value::as_bool)
        .expect("working_tree_clean/clean boolean explícito");
    assert_eq!(clean, !dirty);
    assert_eq!(provenance["commit"], report["commit"]);
}
