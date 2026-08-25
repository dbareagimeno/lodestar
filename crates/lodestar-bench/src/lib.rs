//! Banco interno de rendimiento de Lodestar (E33-H04).

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use lodestar_app::{App, AppError, CheckScope, Profile, ReadServices};
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::{DocumentRef, RelPath, Severity};
use lodestar_fixtures::escala::{self, Perfil};
use lodestar_store::Store;
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const TOOLS: [&str; 7] = [
    "workspace_status",
    "knowledge_search",
    "knowledge_get",
    "metadata_inspect",
    "graph_query",
    "impact_analyze",
    "knowledge_check",
];
const SEARCH_WHERE: &str = r#"service = "bench""#;
const BARRIER_ENV: &str = "LODESTAR_BENCH_TEST_BARRIER_DIR";
const CHANGE_PARENT_ENV: &str = "LODESTAR_BENCH_TEST_CHANGE_PARENT";
const ITERATIONS_ENV: &str = "LODESTAR_BENCH_TEST_ITERATIONS";
const RSS_SAMPLER_ENV: &str = "LODESTAR_BENCH_TEST_RSS_SAMPLER";
const SQLITE_TIMING_TRACE_ENV: &str = "LODESTAR_BENCH_TEST_SQLITE_TIMING_TRACE";
const A6_REPO_ROOT_ENV: &str = "LODESTAR_BENCH_TEST_A6_REPO_ROOT";
const BENCH_VARIANTS: [&str; 3] = ["disk-reparseo", "sqlite-raw", "ram-memoizado"];
const FULL_PROFILE_NAMES: [&str; 2] = ["plano", "realista"];
const FULL_PROFILES: [(&str, Perfil); 2] =
    [("plano", Perfil::Plano), ("realista", Perfil::Realista)];
const FULL_SCALES: [usize; 3] = [100, 1_000, 10_000];
const FULL_ITERATIONS: usize = 10;

#[derive(Debug, Parser)]
#[command(
    name = "lodestar-bench",
    about = "Banco reproducible de evidencia de Lodestar"
)]
struct Args {
    #[arg(long)]
    smoke: bool,
    /// Run the opt-in parametrized footprint probe. This mode never participates in H05/CI.
    #[arg(long)]
    extreme: bool,
    /// Corpus profile for --extreme (currently plano or realista).
    #[arg(long)]
    profile: Option<String>,
    /// Positive document scale for --extreme; any positive u64 is accepted.
    #[arg(long, allow_hyphen_values = true)]
    scale: Option<u64>,
    /// Positive number of samples for --extreme.
    #[arg(long)]
    iterations: Option<u64>,
    /// Explicitly acknowledge the resource cost of an extreme run at one million documents or more.
    #[arg(long)]
    confirm_extreme: bool,
    /// Internal isolated worker used by --extreme; not part of the supported CLI.
    #[arg(long, hide = true)]
    extreme_worker: bool,
    #[arg(long, hide = true)]
    worker_variant: Option<String>,
    /// Internal test-only observation of the official full matrix; never part of the public CLI.
    #[arg(long, hide = true)]
    internal_test_full_plan: bool,
    /// Evaluate a versioned report against ratified limits and a machine baseline.
    #[arg(long)]
    gate: bool,
    #[arg(long)]
    report: Option<PathBuf>,
    #[arg(long)]
    thresholds: Option<PathBuf>,
    #[arg(long)]
    baseline: Option<PathBuf>,
    #[arg(long)]
    machine_id: Option<String>,
    #[arg(long, default_value_t = 33)]
    seed: u64,
    #[arg(long)]
    root: Option<PathBuf>,
    #[arg(long)]
    probe_acquisition_root: Option<PathBuf>,
    #[arg(long)]
    probe_change_root: Option<PathBuf>,
    #[arg(long)]
    json_output: Option<PathBuf>,
    #[arg(long)]
    markdown_output: Option<PathBuf>,
    /// Re-render an existing JSON report with the canonical Markdown renderer.
    #[arg(long)]
    render_report: Option<PathBuf>,
    /// JSON wire-calibration block produced by the real MCP harness. Internal bench input only.
    #[arg(long)]
    wire_calibration_input: Option<PathBuf>,
    /// Verify the versioned raw wire evidence against the official calibration block.
    #[arg(long)]
    validate_wire_calibration_chain: bool,
    #[arg(long)]
    wire_evidence: Option<PathBuf>,
    #[arg(long)]
    official_report: Option<PathBuf>,
    /// Check lodestar-app's manifest, or an explicit fixture, for a direct lodestar-store dependency.
    #[arg(long)]
    check_a6_dependencies: bool,
    #[arg(long)]
    manifest: Option<PathBuf>,
}

struct OwnedExtremeRoot {
    path: PathBuf,
    _temp: Option<tempfile::TempDir>,
}

impl Drop for OwnedExtremeRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub fn run_from_args<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Args::parse_from(args);
    if args.internal_test_full_plan {
        println!("{}", serde_json::to_string(&full_plan())?);
        return Ok(());
    }
    if args.extreme_worker {
        let profile = args
            .profile
            .as_deref()
            .ok_or_else(|| anyhow!("worker extremo requiere --profile"))?;
        if !matches!(profile, "plano" | "realista") {
            return Err(anyhow!("worker extremo: perfil desconocido {profile:?}"));
        }
        let root = args
            .root
            .as_deref()
            .ok_or_else(|| anyhow!("worker extremo requiere --root"))?;
        let variant = args
            .worker_variant
            .as_deref()
            .ok_or_else(|| anyhow!("worker extremo requiere --worker-variant"))?;
        if !matches!(variant, "disk-reparseo" | "sqlite-raw" | "ram-memoizado") {
            return Err(anyhow!("worker extremo: variante desconocida {variant:?}"));
        }
        let iterations = args
            .iterations
            .ok_or_else(|| anyhow!("worker extremo requiere --iterations"))?;
        let iterations = usize::try_from(iterations)
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| anyhow!("worker extremo: --iterations debe ser positivo"))?;
        run_extreme_worker(root, variant, iterations)?;
        return Ok(());
    }
    if args.extreme {
        if args.smoke || args.gate {
            return Err(anyhow!("--extreme es incompatible con --smoke y --gate"));
        }
        if args.report.is_some()
            || args.thresholds.is_some()
            || args.baseline.is_some()
            || args.machine_id.is_some()
            || args.render_report.is_some()
            || args.wire_calibration_input.is_some()
            || args.validate_wire_calibration_chain
            || args.wire_evidence.is_some()
            || args.official_report.is_some()
            || args.check_a6_dependencies
            || args.manifest.is_some()
            || args.probe_acquisition_root.is_some()
            || args.probe_change_root.is_some()
        {
            return Err(anyhow!(
                "--extreme solo acepta --profile --scale --iterations, --root y salidas JSON/Markdown"
            ));
        }
        let profile = args
            .profile
            .as_deref()
            .ok_or_else(|| anyhow!("--extreme requiere --profile (plano o realista)"))?;
        let perfil = match profile {
            "plano" => Perfil::Plano,
            "realista" => Perfil::Realista,
            _ => return Err(anyhow!("--extreme: perfil desconocido {profile:?}")),
        };
        let scale = args
            .scale
            .ok_or_else(|| anyhow!("--extreme requiere --scale N"))?;
        if scale == 0 {
            return Err(anyhow!("--extreme: --scale debe ser un entero positivo"));
        }
        let scale_usize = usize::try_from(scale)
            .map_err(|_| anyhow!("--extreme: --scale no cabe en la plataforma actual"))?;
        let iterations = args
            .iterations
            .ok_or_else(|| anyhow!("--extreme requiere --iterations M"))?;
        if iterations == 0 {
            return Err(anyhow!(
                "--extreme: --iterations debe ser un entero positivo"
            ));
        }
        let iterations_usize = usize::try_from(iterations)
            .map_err(|_| anyhow!("--extreme: --iterations no cabe en la plataforma actual"))?;
        let preflight = preflight_extreme(scale, args.root.as_deref(), args.confirm_extreme)?;
        if scale >= 1_000_000 && !args.confirm_extreme {
            return Err(anyhow!(
                "--extreme: scale >= 1000000 requiere confirmación explícita con --confirm-extreme"
            ));
        }
        let (owned_root, root) = if let Some(root) = args.root {
            if root.exists() {
                return Err(anyhow!(
                    "--extreme exige un --root inicialmente inexistente para poder limpiarlo: {}",
                    root.display()
                ));
            }
            std::fs::create_dir_all(&root)?;
            let owner = OwnedExtremeRoot {
                path: root.clone(),
                _temp: None,
            };
            let canonical_root = std::fs::canonicalize(&root)?;
            for output in [args.json_output.as_deref(), args.markdown_output.as_deref()]
                .into_iter()
                .flatten()
            {
                if path_is_within(output, &canonical_root) {
                    drop(owner);
                    return Err(anyhow!(
                        "--extreme no admite salidas dentro del --root autolimpiable: {}",
                        output.display()
                    ));
                }
            }
            (Some(owner), root)
        } else {
            let temp = tempfile::TempDir::new().context("crear root temporal extremo")?;
            let root = temp.path().to_path_buf();
            (
                Some(OwnedExtremeRoot {
                    path: root.clone(),
                    _temp: Some(temp),
                }),
                root,
            )
        };
        escala::genera(&root, perfil, scale_usize, args.seed)
            .with_context(|| format!("generar corpus extremo {profile}/{scale}"))?;
        overlay_control(&root).context("generar control extremo")?;
        let mut report = extreme_report(
            &root,
            args.seed,
            profile,
            scale,
            iterations_usize,
            preflight,
        )?;
        if let Value::Object(object) = &mut report {
            object.insert("confirmed".into(), json!(args.confirm_extreme));
        }
        // The report is a portable artifact: service results may contain the acquisition root,
        // but no ephemeral/private path is allowed to escape into versioned evidence.
        normalize_logical_root(&mut report);
        recompute_payload_bytes(&mut report);
        let json_text = serde_json::to_string_pretty(&report)?;
        if let Some(path) = args.json_output {
            std::fs::write(path, &json_text).context("escribir salida JSON extrema")?;
        }
        if let Some(path) = args.markdown_output {
            std::fs::write(path, render_extreme_markdown(&report))
                .context("escribir salida Markdown extrema")?;
        }
        println!("{json_text}");
        drop(owned_root);
        return Ok(());
    }
    if args.confirm_extreme {
        return Err(anyhow!("--confirm-extreme solo es válido con --extreme"));
    }
    if args.gate {
        if args.smoke
            || args.seed != 33
            || args.root.is_some()
            || args.probe_acquisition_root.is_some()
            || args.probe_change_root.is_some()
            || args.json_output.is_some()
            || args.markdown_output.is_some()
            || args.render_report.is_some()
            || args.wire_calibration_input.is_some()
            || args.validate_wire_calibration_chain
            || args.wire_evidence.is_some()
            || args.official_report.is_some()
            || args.check_a6_dependencies
            || args.manifest.is_some()
        {
            return Err(anyhow!(
                "--gate solo acepta --report PATH --thresholds PATH --baseline PATH --machine-id ID"
            ));
        }
        let report = args
            .report
            .as_deref()
            .ok_or_else(|| anyhow!("--gate requiere --report PATH"))?;
        let thresholds = args
            .thresholds
            .as_deref()
            .ok_or_else(|| anyhow!("--gate requiere --thresholds PATH"))?;
        let baseline = args
            .baseline
            .as_deref()
            .ok_or_else(|| anyhow!("--gate requiere --baseline PATH"))?;
        let machine_id = args
            .machine_id
            .as_deref()
            .ok_or_else(|| anyhow!("--gate requiere --machine-id ID"))?;
        if machine_id.trim().is_empty() {
            return Err(anyhow!("--machine-id ID no puede estar vacío"));
        }
        return run_gate(report, thresholds, baseline, machine_id);
    }
    if let Some(report_path) = args.render_report.as_deref() {
        if args.markdown_output.is_none() {
            return Err(anyhow!(
                "--render-report exige --markdown-output y no genera otras salidas"
            ));
        }
        if args.smoke
            || args.root.is_some()
            || args.probe_acquisition_root.is_some()
            || args.probe_change_root.is_some()
            || args.json_output.is_some()
            || args.wire_calibration_input.is_some()
            || args.validate_wire_calibration_chain
            || args.wire_evidence.is_some()
            || args.official_report.is_some()
            || args.check_a6_dependencies
            || args.manifest.is_some()
        {
            return Err(anyhow!(
                "--render-report es incompatible con los modos de ejecución del banco"
            ));
        }
        let report: Value = serde_json::from_str(
            &std::fs::read_to_string(report_path)
                .with_context(|| format!("leer informe JSON {}", report_path.display()))?,
        )
        .with_context(|| format!("parsear informe JSON {}", report_path.display()))?;
        let markdown_path = args
            .markdown_output
            .as_deref()
            .expect("validado arriba: markdown output requerido");
        std::fs::write(markdown_path, render_markdown(&report))
            .with_context(|| format!("escribir salida Markdown {}", markdown_path.display()))?;
        return Ok(());
    }
    if args.validate_wire_calibration_chain {
        let evidence = args
            .wire_evidence
            .as_deref()
            .ok_or_else(|| anyhow!("--wire-evidence es obligatorio con la guarda wire"))?;
        let official = args
            .official_report
            .as_deref()
            .ok_or_else(|| anyhow!("--official-report es obligatorio con la guarda wire"))?;
        validate_wire_calibration_chain(evidence, official)?;
        return Ok(());
    }
    if args.check_a6_dependencies {
        let implicit_manifest = args.manifest.is_none();
        let manifest = args.manifest.unwrap_or_else(|| {
            repo_root()
                .join("crates")
                .join("lodestar-app")
                .join("Cargo.toml")
        });
        check_a6_dependencies(&manifest)?;
        if implicit_manifest {
            check_a6_contract(&a6_repo_root())?;
        }
        let manifest_label = if implicit_manifest {
            // The implicit check must be useful from any cwd without leaking the build machine's
            // absolute checkout path into command output.
            "crates/lodestar-app/Cargo.toml".to_owned()
        } else {
            manifest.to_string_lossy().into_owned()
        };
        println!(
            "{}",
            json!({
                "check": "a6-dependencies",
                "manifest": manifest_label,
                "direct_lodestar_store": false,
                "status": "ok",
            })
        );
        return Ok(());
    }
    if let Some(root) = args.probe_acquisition_root {
        return acquisition_probe(&root);
    }
    if let Some(root) = args.probe_change_root {
        return change_probe(&root);
    }
    if args.smoke && args.wire_calibration_input.is_some() {
        return Err(anyhow!(
            "--wire-calibration-input solo está disponible en la corrida full; smoke mantiene wire_calibration=pending"
        ));
    }
    let report = if args.smoke {
        // A caller-provided root is evidence input, not a scratch directory.  The barrier probes
        // are the one explicit exception: they deliberately hand ownership to the test so it can
        // mutate the source between acquisitions.  Normal smoke runs use a private recursive
        // copy; this also keeps Store's SQLite runtime out of the supplied workspace.
        let supplied_root = args
            .root
            .unwrap_or_else(|| std::env::temp_dir().join("lodestar-bench"));
        let owned_root = if std::env::var_os(BARRIER_ENV).is_none() {
            let temp = tempfile::TempDir::new().context("crear workspace privado del banco")?;
            let target = temp.path().join("root");
            std::fs::create_dir_all(&target)?;
            if supplied_root.is_dir() {
                copy_smoke_root(&supplied_root, &target)?;
            }
            Some((temp, target))
        } else {
            None
        };
        let root = owned_root
            .as_ref()
            .map(|(_, root)| root.as_path())
            .unwrap_or(supplied_root.as_path());
        if std::fs::read_dir(root)
            .map(|entries| entries.filter_map(Result::ok).next().is_none())
            .unwrap_or(true)
        {
            escala::genera(root, Perfil::Plano, 100, args.seed).context("generar corpus plano")?;
            overlay_control(root).context("generar control H04")?;
        }
        let mut report = report(root, args.seed, true)?;
        if std::env::var_os(BARRIER_ENV).is_none() {
            // The filesystem is private, but its OS temp name is intentionally not part of the
            // stable smoke schema. Keep a truthful logical marker in serialized service results.
            normalize_logical_root(&mut report);
            recompute_payload_bytes(&mut report);
        }
        drop(owned_root);
        report
    } else {
        full_report(args.seed, args.wire_calibration_input.as_deref())?
    };
    let json_text = serde_json::to_string_pretty(&report)?;
    if let Some(path) = args.json_output {
        std::fs::write(path, &json_text).context("escribir salida JSON")?;
    }
    if let Some(path) = args.markdown_output {
        std::fs::write(path, render_markdown(&report)).context("escribir salida Markdown")?;
    }
    println!("{json_text}");
    Ok(())
}

struct Calls {
    child: DocumentRef,
    missing: DocumentRef,
    check: CheckScope,
}

fn calls() -> Calls {
    Calls {
        child: DocumentRef {
            path: RelPath::new("child.md").expect("path fixture"),
            id: None,
        },
        missing: DocumentRef {
            path: RelPath::new("missing.md").expect("path fixture"),
            id: None,
        },
        check: CheckScope::Workspace,
    }
}

fn app_result<T: Serialize>(result: Result<T, AppError>) -> Value {
    match result {
        Ok(value) => serde_json::to_value(value).expect("resultado serializable"),
        Err(error) => json!({"error": {"code": error.code.as_str(), "message": error.message}}),
    }
}

fn seven_from_app(app: &App, c: &Calls) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    out.insert(
        "workspace_status".into(),
        app_result(app.workspace_status(Profile::Standard)),
    );
    out.insert(
        "knowledge_search".into(),
        app_result(app.knowledge_search(
            "marker-search-h04",
            Some(SEARCH_WHERE),
            None,
            &[],
            None,
            None,
        )),
    );
    out.insert(
        "knowledge_get".into(),
        app_result(app.knowledge_get(
            &c.child,
            &[
                "body".into(),
                "frontmatter".into(),
                "outgoingLinks".into(),
                "backlinks".into(),
                "diagnostics".into(),
            ],
            None,
        )),
    );
    out.insert(
        "metadata_inspect".into(),
        app_result(app.metadata_inspect("field", Some("tags"), None, None)),
    );
    out.insert(
        "graph_query".into(),
        app_result(app.graph_query("components", None, None, None, None, None, None)),
    );
    out.insert(
        "impact_analyze".into(),
        app_result(app.impact_analyze(&c.child, "delete", None)),
    );
    out.insert(
        "knowledge_check".into(),
        app_result(app.knowledge_check(&c.check, Some(Severity::Info), true, None, None)),
    );
    out
}

fn seven_from_seam(seam: &ReadServices<'_>, c: &Calls) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    out.insert(
        "workspace_status".into(),
        app_result(seam.workspace_status(Profile::Standard)),
    );
    out.insert(
        "knowledge_search".into(),
        app_result(seam.knowledge_search(
            "marker-search-h04",
            Some(SEARCH_WHERE),
            None,
            &[],
            None,
            None,
        )),
    );
    out.insert(
        "knowledge_get".into(),
        app_result(seam.knowledge_get(
            &c.child,
            &[
                "body".into(),
                "frontmatter".into(),
                "outgoingLinks".into(),
                "backlinks".into(),
                "diagnostics".into(),
            ],
            None,
        )),
    );
    out.insert(
        "metadata_inspect".into(),
        app_result(seam.metadata_inspect("field", Some("tags"), None, None)),
    );
    out.insert(
        "graph_query".into(),
        app_result(seam.graph_query("components", None, None, None, None, None, None)),
    );
    out.insert(
        "impact_analyze".into(),
        app_result(seam.impact_analyze(&c.child, "delete", None)),
    );
    out.insert(
        "knowledge_check".into(),
        app_result(seam.knowledge_check(&c.check, Some(Severity::Info), true, None, None)),
    );
    out
}

fn timed_app(app: &App, c: &Calls, tool: &str) -> (Duration, Value) {
    let start = Instant::now();
    let value = match tool {
        "workspace_status" => app_result(app.workspace_status(Profile::Standard)),
        "knowledge_search" => app_result(app.knowledge_search(
            "marker-search-h04",
            Some(SEARCH_WHERE),
            None,
            &[],
            None,
            None,
        )),
        "knowledge_get" => app_result(app.knowledge_get(
            &c.child,
            &[
                "body".into(),
                "frontmatter".into(),
                "outgoingLinks".into(),
                "backlinks".into(),
                "diagnostics".into(),
            ],
            None,
        )),
        "metadata_inspect" => app_result(app.metadata_inspect("field", Some("tags"), None, None)),
        "graph_query" => {
            app_result(app.graph_query("components", None, None, None, None, None, None))
        }
        "impact_analyze" => app_result(app.impact_analyze(&c.child, "delete", None)),
        "knowledge_check" => {
            app_result(app.knowledge_check(&c.check, Some(Severity::Info), true, None, None))
        }
        _ => panic!("tool desconocida: {tool}"),
    };
    (start.elapsed(), value)
}

fn timed_seam(seam: &ReadServices<'_>, c: &Calls, tool: &str) -> (Duration, Value) {
    timed_seam_from(Instant::now(), seam, c, tool)
}

fn timed_seam_from(
    start: Instant,
    seam: &ReadServices<'_>,
    c: &Calls,
    tool: &str,
) -> (Duration, Value) {
    let value = match tool {
        "workspace_status" => app_result(seam.workspace_status(Profile::Standard)),
        "knowledge_search" => app_result(seam.knowledge_search(
            "marker-search-h04",
            Some(SEARCH_WHERE),
            None,
            &[],
            None,
            None,
        )),
        "knowledge_get" => app_result(seam.knowledge_get(
            &c.child,
            &[
                "body".into(),
                "frontmatter".into(),
                "outgoingLinks".into(),
                "backlinks".into(),
                "diagnostics".into(),
            ],
            None,
        )),
        "metadata_inspect" => app_result(seam.metadata_inspect("field", Some("tags"), None, None)),
        "graph_query" => {
            app_result(seam.graph_query("components", None, None, None, None, None, None))
        }
        "impact_analyze" => app_result(seam.impact_analyze(&c.child, "delete", None)),
        "knowledge_check" => {
            app_result(seam.knowledge_check(&c.check, Some(Severity::Info), true, None, None))
        }
        _ => panic!("tool desconocida: {tool}"),
    };
    (start.elapsed(), value)
}

fn metric(samples: &[(Duration, Value)]) -> Value {
    assert!(!samples.is_empty(), "muestras");
    // Keep acquisition order in the evidence. Percentiles use a scratch vector so the result and
    // trace remain the final chronological sample rather than an accidental timing order.
    let sample_elapsed_ns: Vec<u64> = samples
        .iter()
        .map(|(duration, _)| duration.as_nanos() as u64)
        .collect();
    let mut sorted = sample_elapsed_ns.clone();
    sorted.sort_unstable();
    let p50 = sorted[sorted.len() / 2];
    let p95 = sorted[((sorted.len() * 95).saturating_sub(1) / 100).min(sorted.len() - 1)];
    let result = samples.last().expect("muestras").1.clone();
    let payload_bytes = serde_json::to_vec(&result)
        .expect("payload serializable")
        .len();
    json!({"sample_count": samples.len(), "sample_elapsed_ns": sample_elapsed_ns, "p50_ns": p50, "p95_ns": p95, "payload_bytes": payload_bytes, "result": result})
}

fn normalize_logical_root(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(root) = object.get_mut("root") {
                if root.is_string() {
                    *root = Value::String("<private-root>".into());
                }
            }
            for child in object.values_mut() {
                normalize_logical_root(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                normalize_logical_root(child);
            }
        }
        _ => {}
    }
}

fn recompute_payload_bytes(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let (Some(result), Some(payload)) = (
                object.get("result").cloned(),
                object.get_mut("payload_bytes"),
            ) {
                *payload = Value::from(
                    serde_json::to_vec(&result)
                        .expect("resultado serializable")
                        .len() as u64,
                );
            }
            for child in object.values_mut() {
                recompute_payload_bytes(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                recompute_payload_bytes(child);
            }
        }
        _ => {}
    }
}

fn test_iterations(default: usize) -> usize {
    std::env::var(ITERATIONS_ENV)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn await_test_barrier(label: &str) {
    let Some(directory) = std::env::var_os(BARRIER_ENV).map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&directory).expect("crear barrera de test");
    let ready = directory.join(format!("READY-{label}"));
    let continue_file = directory.join(format!("CONTINUE-{label}"));
    std::fs::write(&ready, b"ready\n").expect("publicar READY de test");
    loop {
        if continue_file.is_file() {
            let _ = std::fs::remove_file(&continue_file);
            // El consumidor retira READY para que el protocolo detecte duplicados.
            let _ = std::fs::remove_file(&ready);
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn finish_test_barrier() {
    if let Some(directory) = std::env::var_os(BARRIER_ENV).map(PathBuf::from) {
        let _ = std::fs::write(directory.join("DONE"), b"done\n");
    }
}

fn trace_sample(
    trace: &mut Map<String, Value>,
    tool: &str,
    index: usize,
    source: &str,
    elapsed: Duration,
    result: &Value,
) {
    let entry = json!({
        "sample_index": index,
        "source": source,
        "elapsed_ns": elapsed.as_nanos(),
        "result": result,
        "acquisition": {"source": source}
    });
    trace
        .entry(tool.to_owned())
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .expect("traza de tool")
        .push(entry);
}

fn status_count(value: &Value) -> u64 {
    value
        .get("counts")
        .and_then(|counts| counts.get("documents"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn cold_open(root: &Path, variant: &str) -> Value {
    let start = Instant::now();
    let result = match variant {
        "disk-reparseo" => match App::open(root) {
            Ok(app) => app_result(app.workspace_status(Profile::Standard)),
            Err(error) => json!({"error": {"code": "open", "message": error.to_string()}}),
        },
        "sqlite-raw" => match Store::open(root) {
            Ok(store) => {
                let doc_set = store.document_set();
                match App::open(root) {
                    Ok(app) => app_result(
                        app.read_services(&doc_set)
                            .workspace_status(Profile::Standard),
                    ),
                    Err(error) => json!({"error": {"code": "open", "message": error.to_string()}}),
                }
            }
            Err(error) => json!({"error": {"code": "open", "message": error.to_string()}}),
        },
        "ram-memoizado" => match App::open(root) {
            Ok(app) => match app.workspace().document_set() {
                Ok(doc_set) => app_result(
                    app.read_services(&doc_set)
                        .workspace_status(Profile::Standard),
                ),
                Err(error) => json!({"error": {"code": "open", "message": error.to_string()}}),
            },
            Err(error) => json!({"error": {"code": "open", "message": error.to_string()}}),
        },
        _ => json!({"error": {"code": "variant", "message": variant}}),
    };
    let samples = vec![(start.elapsed(), result)];
    metric(&samples)
}

fn cold_open_samples_with_timing(
    root: &Path,
    variant: &str,
    iterations: usize,
    elapsed_ns: Option<&[u64]>,
    timing_log: bool,
) -> Value {
    let samples = (0..iterations)
        .map(|index| {
            if timing_log {
                sqlite_timing_log(&format!("phase:cold-open:{}:timer-start", index + 1));
            }
            let measured = cold_open(root, variant);
            if timing_log {
                sqlite_timing_log(&format!("phase:cold-open:{}:timer-end", index + 1));
            }
            let elapsed = elapsed_ns
                .map(|values| values[index])
                .or_else(|| {
                    measured
                        .get("sample_elapsed_ns")
                        .and_then(Value::as_array)
                        .and_then(|samples| samples.first())
                        .and_then(Value::as_u64)
                })
                .unwrap_or(1);
            if timing_log {
                sqlite_timing_log(&format!("consume:cold-open:{}:{}", index + 1, elapsed));
            }
            (
                Duration::from_nanos(elapsed),
                measured.get("result").cloned().unwrap_or(Value::Null),
            )
        })
        .collect::<Vec<_>>();
    metric(&samples)
}

#[derive(Debug)]
struct SqliteTimingTrace {
    rebuild_elapsed_ns: u64,
    tool_elapsed_ns: BTreeMap<String, Vec<u64>>,
    cold_open_elapsed_ns: Vec<u64>,
}

fn sqlite_timing_trace_from_env(
    iterations: usize,
    cold_iterations: usize,
) -> Result<Option<SqliteTimingTrace>> {
    let Some(path) = std::env::var_os(SQLITE_TIMING_TRACE_ENV) else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(&path))
            .with_context(|| format!("leer traza SQLite {}", Path::new(&path).display()))?,
    )
    .context("parsear traza SQLite JSON")?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("traza SQLite debe ser un objeto JSON"))?;
    let expected_keys = [
        "rebuild_elapsed_ns",
        "tool_elapsed_ns",
        "cold_open_elapsed_ns",
    ];
    if object.len() != expected_keys.len()
        || expected_keys.iter().any(|key| !object.contains_key(*key))
    {
        return Err(anyhow!(
            "traza SQLite debe contener exactamente rebuild_elapsed_ns, tool_elapsed_ns y cold_open_elapsed_ns"
        ));
    }
    let read_array = |value: &Value, label: &str, expected_len: usize| -> Result<Vec<u64>> {
        let values = value
            .as_array()
            .ok_or_else(|| anyhow!("traza SQLite {label} debe ser un array"))?;
        if values.len() != expected_len {
            return Err(anyhow!(
                "traza SQLite {label} requiere {expected_len} muestras, recibió {}",
                values.len()
            ));
        }
        values
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| anyhow!("traza SQLite {label} contiene una duración no u64"))
            })
            .collect()
    };
    let rebuild = read_array(
        object
            .get("rebuild_elapsed_ns")
            .expect("clave validada arriba"),
        "rebuild_elapsed_ns",
        1,
    )?;
    let tool_object = object
        .get("tool_elapsed_ns")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("traza SQLite tool_elapsed_ns debe ser un objeto"))?;
    if tool_object.len() != TOOLS.len() || TOOLS.iter().any(|tool| !tool_object.contains_key(*tool))
    {
        return Err(anyhow!(
            "traza SQLite tool_elapsed_ns debe contener exactamente las siete tools"
        ));
    }
    let mut tool_elapsed_ns = BTreeMap::new();
    for tool in TOOLS {
        tool_elapsed_ns.insert(
            tool.to_owned(),
            read_array(
                tool_object.get(tool).expect("tool validada arriba"),
                tool,
                iterations,
            )?,
        );
    }
    let cold_open_elapsed_ns = read_array(
        object
            .get("cold_open_elapsed_ns")
            .expect("clave validada arriba"),
        "cold_open_elapsed_ns",
        cold_iterations,
    )?;
    Ok(Some(SqliteTimingTrace {
        rebuild_elapsed_ns: rebuild[0],
        tool_elapsed_ns,
        cold_open_elapsed_ns,
    }))
}

fn sqlite_timing_log_enabled() -> bool {
    std::env::var_os("LODESTAR_BENCH_TEST_SQLITE_TIMING_LOG").is_some()
}

fn sqlite_timing_log(line: &str) {
    let Some(path) = std::env::var_os("LODESTAR_BENCH_TEST_SQLITE_TIMING_LOG") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

fn variant_row(
    app: &App,
    root: &Path,
    variant: &str,
    iterations: usize,
    c: &Calls,
    acquisition_trace: &mut Map<String, Value>,
) -> Result<Value> {
    variant_row_with_cold_iterations(
        app,
        root,
        variant,
        iterations,
        1,
        c,
        acquisition_trace,
        None,
    )
}

// The worker keeps the benchmark phases explicit; the optional trace is an internal
// deterministic-test seam and is intentionally not part of the product surface.
#[allow(clippy::too_many_arguments)]
fn variant_row_with_cold_iterations(
    app: &App,
    root: &Path,
    variant: &str,
    iterations: usize,
    cold_iterations: usize,
    c: &Calls,
    acquisition_trace: &mut Map<String, Value>,
    timing_trace: Option<&SqliteTimingTrace>,
) -> Result<Value> {
    let mut measurements: BTreeMap<String, Vec<(Duration, Value)>> = TOOLS
        .iter()
        .map(|tool| ((*tool).to_string(), Vec::with_capacity(iterations)))
        .collect();
    let mut rebuild_samples: Vec<(Duration, Value)> = Vec::new();
    let mut trace_tools = Map::new();
    let last: BTreeMap<String, Value> = match variant {
        "disk-reparseo" => {
            for index in 1..=iterations {
                for tool in TOOLS {
                    await_test_barrier(&format!("{variant}-{tool}-{index}"));
                    let (elapsed, value) = timed_app(app, c, tool);
                    trace_sample(&mut trace_tools, tool, index, variant, elapsed, &value);
                    measurements.get_mut(tool).unwrap().push((elapsed, value));
                }
            }
            seven_from_app(app, c)
        }
        "sqlite-raw" => {
            // Rebuild es una fase separada de las lecturas. En el protocolo de prueba el proceso
            // externo puede reconstruir el mismo SQLite entre READY y CONTINUE; por eso cada
            // muestra abre una conexión fresca y solo mide `document_set` + servicio.
            let initial_store = Store::open(root).context("abrir store SQLite")?;
            let timing_log = sqlite_timing_log_enabled();
            if timing_log {
                sqlite_timing_log("phase:rebuild:start");
            }
            let rebuild_start = Instant::now();
            initial_store.rebuild().context("rebuild SQLite")?;
            if timing_log {
                sqlite_timing_log("phase:rebuild:end");
            }
            let rebuild_elapsed = timing_trace
                .map(|trace| Duration::from_nanos(trace.rebuild_elapsed_ns))
                .unwrap_or_else(|| rebuild_start.elapsed());
            if timing_log {
                sqlite_timing_log(&format!("consume:rebuild:{}", rebuild_elapsed.as_nanos()));
            }
            rebuild_samples.push((rebuild_elapsed, json!(true)));
            for index in 1..=iterations {
                for tool in TOOLS {
                    await_test_barrier(&format!("{variant}-{tool}-{index}"));
                    // Abrir una conexión fresca observa el SQLite reconstruido por la fase previa
                    // (o por el oráculo externo del protocolo); rebuild no entra en el percentile.
                    let store = Store::open(root).context("abrir store SQLite")?;
                    if timing_log {
                        sqlite_timing_log(&format!("phase:tool:{tool}:{index}:timer-start"));
                    }
                    let start = Instant::now();
                    let doc_set = store.document_set();
                    let seam = app.read_services(&doc_set);
                    // La adquisición SQLite forma parte de cada muestra de cada tool. El reloj
                    // arranca antes de `document_set`; la construcción del seam queda dentro de
                    // la muestra para no excluir trabajo derivado de esa adquisición.
                    let (measured_elapsed, value) = timed_seam_from(start, &seam, c, tool);
                    let elapsed = timing_trace
                        .map(|trace| Duration::from_nanos(trace.tool_elapsed_ns[tool][index - 1]))
                        .unwrap_or(measured_elapsed);
                    if timing_log {
                        sqlite_timing_log(&format!("phase:tool:{tool}:{index}:timer-end"));
                        sqlite_timing_log(&format!(
                            "consume:tool:{tool}:{index}:{}",
                            elapsed.as_nanos()
                        ));
                    }
                    trace_sample(&mut trace_tools, tool, index, variant, elapsed, &value);
                    measurements.get_mut(tool).unwrap().push((elapsed, value));
                }
            }
            let store = Store::open(root).context("abrir store SQLite final")?;
            let doc_set = store.document_set();
            seven_from_seam(&app.read_services(&doc_set), c)
        }
        "ram-memoizado" => {
            await_test_barrier("ram-memoizado-ACQUIRE");
            let doc_set = app
                .workspace()
                .document_set()
                .context("adquirir DocumentSet RAM")?;
            let seam = app.read_services(&doc_set);
            for index in 1..=iterations {
                for tool in TOOLS {
                    // Sample 1 pertenece al snapshot adquirido; desde la 2 el test puede mutar
                    // la fuente antes de continuar, demostrando que RAM no vuelve a adquirir.
                    if index > 1 {
                        await_test_barrier(&format!("{variant}-{tool}-{index}"));
                    }
                    let (elapsed, value) = timed_seam(&seam, c, tool);
                    trace_sample(&mut trace_tools, tool, index, variant, elapsed, &value);
                    measurements.get_mut(tool).unwrap().push((elapsed, value));
                }
            }
            seven_from_seam(&seam, c)
        }
        _ => return Err(anyhow!("variante desconocida: {variant}")),
    };
    let mut row = Map::new();
    row.insert("variant".into(), json!(variant));
    row.insert(
        "document_count".into(),
        json!(status_count(last.get("workspace_status").unwrap())),
    );
    let mut tool_metrics = Map::new();
    for tool in TOOLS {
        tool_metrics.insert(tool.into(), metric(measurements.get_mut(tool).unwrap()));
    }
    row.insert("tools".into(), Value::Object(tool_metrics));
    row.insert(
        "cold_open".into(),
        cold_open_samples_with_timing(
            root,
            variant,
            cold_iterations,
            timing_trace.map(|trace| trace.cold_open_elapsed_ns.as_slice()),
            variant == "sqlite-raw" && sqlite_timing_log_enabled(),
        ),
    );
    if !rebuild_samples.is_empty() {
        row.insert("rebuild".into(), metric(&rebuild_samples));
        row.insert("percentiles_includes_rebuild".into(), json!(false));
    }
    acquisition_trace.insert(variant.to_owned(), Value::Object(trace_tools));
    Ok(Value::Object(row))
}

/// Conservative lower-bound accounting performed before the generator writes its first file.
/// The estimate intentionally includes both Markdown and SQLite headroom; the result is a safety
/// gate for the opt-in probe, not a claim about the eventual footprint.
fn preflight_extreme(scale: u64, requested_root: Option<&Path>, confirmed: bool) -> Result<Value> {
    // H04 measured ~10.4 KiB/Markdown document at Realista/10k. 32 KiB/document leaves room for
    // the generator's four control files, SQLite's main/WAL/SHM files and temporary copy-on-write
    // overhead. This is a safety budget, never a claim about the eventual measured footprint.
    let bytes_per_document = 32 * 1024_u64;
    let fixed_headroom = 256 * 1024 * 1024_u64;
    let required = scale
        .checked_mul(bytes_per_document)
        .and_then(|value| value.checked_add(fixed_headroom))
        .ok_or_else(|| anyhow!("preflight extremo: espacio requerido desborda el entero"))?;
    let probe_path = requested_root
        .and_then(Path::parent)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let available = available_space_bytes(probe_path);
    if let Some(available) = available {
        if required > available {
            return Err(anyhow!(
                "preflight extremo: espacio insuficiente; estimador=heuristica-disco-v1; disponible={} bytes, requerido={} bytes, scale={scale}",
                available, required
            ));
        }
        return Ok(json!({
            "status": "checked",
            "confirmed": confirmed,
            "available_bytes": available,
            "required_bytes": required,
            "resource_scope": "espacio de disco; memoria/RSS no se verifica en preflight",
            "memory_verification": {
                "status": "unverified",
                "reason": "el preflight no verifica memoria/RSS; se mide después en el worker aislado con getrusage"
            }
        }));
    }
    if !confirmed {
        return Err(anyhow!(
            "preflight extremo: df no verificable para scale={scale}; estimador=heuristica-disco-v1; requiere --confirm-extreme; requerido={} bytes, disponible=unavailable",
            required
        ));
    }
    Ok(json!({
        "status": "unverified",
        "confirmed": true,
        "available_bytes": Value::Null,
        "required_bytes": required,
        "resource_scope": "espacio de disco no verificable por df; memoria/RSS no se verifica en preflight",
        "memory_verification": {
            "status": "unverified",
            "reason": "el preflight no verifica memoria/RSS; se mide después en el worker aislado con getrusage"
        }
    }))
}

fn available_space_bytes(path: &Path) -> Option<u64> {
    let output = Command::new("df")
        .args(["-Pk", path.to_str()?])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().last()?;
    let blocks = line.split_whitespace().nth(3)?.parse::<u64>().ok()?;
    blocks.checked_mul(1024)
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    let candidate = path.canonicalize().ok().or_else(|| {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let name = path.file_name()?;
        Some(parent.canonicalize().ok()?.join(name))
    });
    candidate.is_some_and(|candidate| candidate.starts_with(root))
}

fn rss_sampler_report(path: &OsStr, phase: Option<&str>) -> Value {
    let platform = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
    let mut command = Command::new(path);
    if let Some(phase) = phase {
        command.env("LODESTAR_BENCH_TEST_RSS_PHASE", phase);
    }
    let output = match command.output() {
        Ok(output) if output.status.success() => output,
        Ok(_output) => {
            return json!({
                "status": "unavailable",
                "reason": "sampler RSS interno terminó con error",
                "method": "LODESTAR_BENCH_TEST_RSS_SAMPLER stdout u64",
                "units": "bytes",
                "scope": "proceso worker aislado por variante",
                "platform": platform
            });
        }
        Err(error) => {
            return json!({
                "status": "unavailable",
                "reason": format!("no se pudo ejecutar sampler RSS interno: {error}"),
                "method": "LODESTAR_BENCH_TEST_RSS_SAMPLER stdout u64",
                "units": "bytes",
                "scope": "proceso worker aislado por variante",
                "platform": platform
            });
        }
    };
    let raw = match String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
    {
        Ok(value) if value > 0 => value,
        _ => {
            return json!({
                "status": "unavailable",
                "reason": "sampler RSS interno no devolvió un u64 positivo por stdout",
                "method": "LODESTAR_BENCH_TEST_RSS_SAMPLER stdout u64",
                "units": "bytes",
                "scope": "proceso worker aislado por variante",
                "platform": platform
            });
        }
    };
    json!({
        "status": "available",
        "raw_value": raw,
        "raw_units": "bytes",
        "absolute_bytes": raw,
        "method": "LODESTAR_BENCH_TEST_RSS_SAMPLER stdout u64",
        "units": "bytes",
        "scope": "pico absoluto del proceso worker aislado por variante",
        "platform": platform
    })
}

#[allow(clippy::needless_return)]
fn rss_peak_report(test_sampler: Option<&OsStr>, phase: Option<&str>) -> Value {
    if let Some(path) = test_sampler {
        return rss_sampler_report(path, phase);
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        // SAFETY: getrusage initializes the caller-provided rusage structure on success.
        let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        if status != 0 {
            return json!({
                "status": "unavailable",
                "reason": "getrusage(RUSAGE_SELF) terminó con error",
                "method": "getrusage(RUSAGE_SELF).ru_maxrss",
                "units": "bytes",
                "scope": "proceso worker aislado por variante",
                "platform": format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
            });
        }
        // SAFETY: successful getrusage initialized usage above.
        let usage = unsafe { usage.assume_init() };
        let raw = u64::try_from(usage.ru_maxrss).unwrap_or(0);
        let (raw_units, absolute_bytes) = if cfg!(target_os = "macos") {
            ("bytes", raw)
        } else {
            ("KiB", raw.saturating_mul(1024))
        };
        if absolute_bytes == 0 {
            return json!({
                "status": "unavailable",
                "reason": "getrusage devolvió ru_maxrss=0",
                "method": "getrusage(RUSAGE_SELF).ru_maxrss",
                "units": "bytes",
                "scope": "proceso worker aislado por variante",
                "platform": format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
            });
        }
        return json!({
            "status": "available",
            "raw_value": raw,
            "raw_units": raw_units,
            "absolute_bytes": absolute_bytes,
            "method": "getrusage(RUSAGE_SELF).ru_maxrss",
            "units": "bytes",
            "scope": "pico absoluto del proceso worker aislado por variante",
            "platform": format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
        });
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    json!({
        "status": "unavailable",
        "reason": "plataforma sin getrusage(RUSAGE_SELF).ru_maxrss soportado por el banco",
        "method": "unavailable",
        "units": "bytes",
        "scope": "proceso worker aislado por variante",
        "platform": format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
    })
}

fn rss_baseline_report(test_sampler: Option<&OsStr>) -> Value {
    let mut report = rss_peak_report(test_sampler, Some("baseline"));
    if let Value::Object(object) = &mut report {
        if object.get("status").and_then(Value::as_str) == Some("available") {
            object.insert(
                "scope".into(),
                json!("ru_maxrss acumulado antes de cargar el corpus en el worker aislado"),
            );
        }
    }
    report
}

fn rss_with_baseline(mut report: Value, baseline: &Value) -> Value {
    let baseline_bytes = baseline
        .get("absolute_bytes")
        .and_then(Value::as_u64)
        .filter(|_| baseline["status"] == "available");
    let absolute_bytes = report
        .get("absolute_bytes")
        .and_then(Value::as_u64)
        .filter(|_| report["status"] == "available");
    let Some((baseline_bytes, absolute_bytes)) = baseline_bytes.zip(absolute_bytes) else {
        if let Value::Object(object) = &mut report {
            object.insert("status".into(), json!("unavailable"));
            object.insert(
                "reason".into(),
                json!("no se pudo medir baseline y pico RSS con getrusage de forma reconciliable"),
            );
            object.remove("absolute_bytes");
            object.remove("raw_value");
            object.remove("raw_units");
        }
        return report;
    };
    let Some(delta_bytes) = absolute_bytes.checked_sub(baseline_bytes) else {
        if let Value::Object(object) = &mut report {
            object.insert("status".into(), json!("unavailable"));
            object.insert(
                "reason".into(),
                json!("ru_maxrss final fue menor que el baseline; delta RSS no reconciliable"),
            );
            object.remove("absolute_bytes");
            object.remove("raw_value");
            object.remove("raw_units");
        }
        return report;
    };
    if let Value::Object(object) = &mut report {
        object.insert("baseline_bytes".into(), json!(baseline_bytes));
        object.insert("delta_bytes".into(), json!(delta_bytes));
    }
    report
}

fn corpus_size_report(root: &Path) -> Result<Value> {
    fn walk(path: &Path, documents: &mut u64, bytes: &mut u64) -> Result<()> {
        for entry in std::fs::read_dir(path).with_context(|| format!("leer {}", path.display()))? {
            let entry = entry?;
            let entry_path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                if entry.file_name() == ".lodestar" {
                    continue;
                }
                walk(&entry_path, documents, bytes)?;
            } else if file_type.is_file()
                && entry_path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("md")
            {
                *documents += 1;
                *bytes = bytes.saturating_add(entry.metadata()?.len());
            }
        }
        Ok(())
    }
    let mut documents = 0;
    let mut bytes = 0;
    walk(root, &mut documents, &mut bytes)?;
    Ok(json!({"document_count": documents, "bytes": bytes, "unit": "bytes de Markdown en disco"}))
}

fn sqlite_size_report(root: &Path) -> Value {
    let directory = root.join(lodestar_store::CACHE_DIR);
    let file_size = |name: &str| {
        std::fs::metadata(directory.join(name))
            .map(|metadata| metadata.len())
            .unwrap_or(0)
    };
    let main_bytes = file_size(lodestar_store::DB_FILE);
    let wal_bytes = file_size("index.db-wal");
    let shm_bytes = file_size("index.db-shm");
    let auxiliary_bytes = wal_bytes.saturating_add(shm_bytes);
    let dbstat = Store::open(root)
        .and_then(|store| store.dbstat_report())
        .unwrap_or_else(|error| json!({"error": error.to_string(), "main_bytes": 0, "objects": [], "unattributed_bytes": 0}));
    json!({
        "main_bytes": main_bytes,
        "wal_bytes": wal_bytes,
        "shm_bytes": shm_bytes,
        "auxiliary_bytes": auxiliary_bytes,
        "total_bytes": main_bytes.saturating_add(auxiliary_bytes),
        "dbstat": dbstat,
        "unit": "bytes",
        "scope": "<root>/.lodestar/index.db y auxiliares WAL/SHM; medido al finalizar"
    })
}

fn extreme_report(
    root: &Path,
    seed: u64,
    profile: &str,
    scale: u64,
    iterations: usize,
    preflight: Value,
) -> Result<Value> {
    let mut rows = Vec::with_capacity(3);
    for variant in BENCH_VARIANTS {
        rows.push(spawn_extreme_worker(root, profile, variant, iterations)?);
    }
    let baseline_results = rows
        .first()
        .and_then(|row| row.get("tools"))
        .and_then(Value::as_object)
        .map(|tools| {
            TOOLS
                .iter()
                .filter_map(|tool| {
                    tools
                        .get(*tool)
                        .and_then(|metric| metric.get("result"))
                        .map(|result| ((*tool).to_owned(), result.clone()))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let functional_equivalence = rows.iter().all(|row| {
        row.get("tools")
            .and_then(Value::as_object)
            .map(|tools| {
                TOOLS.iter().all(|tool| {
                    tools.get(*tool).and_then(|metric| metric.get("result"))
                        == baseline_results.get(*tool)
                })
            })
            .unwrap_or(false)
    });
    let mut divergences = Vec::new();
    for row in &rows {
        let Some(variant) = row.get("variant").and_then(Value::as_str) else {
            continue;
        };
        let Some(tools) = row.get("tools").and_then(Value::as_object) else {
            continue;
        };
        for tool in TOOLS {
            let actual = tools.get(tool).and_then(|metric| metric.get("result"));
            let expected = baseline_results.get(tool);
            if actual != expected {
                divergences.push(json!({"variant": variant, "tool": tool}));
            }
        }
    }
    if !divergences.is_empty() {
        return Err(anyhow!(
            "equivalencia funcional extrema divergió en variante/tool: {}",
            serde_json::to_string(&divergences)?
        ));
    }
    let corpus = corpus_size_report(root)?;
    let sqlite = sqlite_size_report(root);
    let commit = git_commit().unwrap_or_else(|| "unknown".into());
    Ok(json!({
        "schema_version": "e33-h09-v1",
        "mode": "extreme",
        "profile": profile,
        "corpus_profile": profile,
        "scale": scale,
        "iterations": iterations,
        "seed": seed,
        "coordinator_pid": std::process::id(),
        "machine": machine_label(),
        "binary": binary_label(),
        "build_profile": build_profile(),
        "commit": commit,
        "provenance": {"commit": commit, "working_tree_clean": git_working_tree_clean()},
        "variants": BENCH_VARIANTS,
        "tools": TOOLS,
        "measurements": rows,
        "functional_equivalence": functional_equivalence,
        "equivalence_divergences": divergences,
        "captured_at": captured_at(),
        "platform": {"os": std::env::consts::OS, "arch": std::env::consts::ARCH},
        "corpus": corpus,
        "sqlite": sqlite,
        "footprint": {
            "objective": {"max_ratio": 2.5, "gate": false},
            "read_default": false
        },
        "preflight": preflight,
        "wire_calibration": {"status": "not_applicable", "reason": "modo extremo excluye wire"},
        "change_cycle": {"status": "not_applicable", "reason": "modo extremo excluye escritura"}
    }))
}

fn spawn_extreme_worker(
    root: &Path,
    profile: &str,
    variant: &str,
    iterations: usize,
) -> Result<Value> {
    let executable = std::env::current_exe().context("resolver ejecutable del worker extremo")?;
    let child = Command::new(executable)
        .args([
            "--extreme-worker",
            "--profile",
            profile,
            "--scale",
            "1",
            "--iterations",
            &iterations.to_string(),
            "--root",
            root.to_str().ok_or_else(|| anyhow!("root no UTF-8"))?,
            "--worker-variant",
            variant,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("ejecutar worker extremo {variant}"))?;
    let worker_pid = child.id();
    let output = child
        .wait_with_output()
        .with_context(|| format!("esperar worker extremo {variant}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "worker extremo {variant} falló: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let mut row: Value = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("parsear informe del worker extremo {variant}"))?;
    if let Value::Object(object) = &mut row {
        object.insert("worker_pid".into(), json!(worker_pid));
    }
    Ok(row)
}

fn run_extreme_worker(root: &Path, variant: &str, iterations: usize) -> Result<Value> {
    let test_sampler = std::env::var_os(RSS_SAMPLER_ENV);
    let baseline_rss = rss_baseline_report(test_sampler.as_deref());
    let timing_trace = if variant == "sqlite-raw" {
        sqlite_timing_trace_from_env(iterations, iterations)?
    } else {
        None
    };
    if let Some(path) = test_sampler.as_deref() {
        let _ = rss_sampler_report(path, Some("app-open-start"));
    }
    let app = App::open(root).context("abrir App en worker extremo")?;
    if let Some(path) = test_sampler.as_deref() {
        let _ = rss_sampler_report(path, Some("app-open-end"));
        let _ = rss_sampler_report(path, Some("load-start"));
    }
    let calls = calls();
    let mut trace = Map::new();
    let mut row = variant_row_with_cold_iterations(
        &app,
        root,
        variant,
        iterations,
        iterations,
        &calls,
        &mut trace,
        timing_trace.as_ref(),
    )?;
    if let Some(path) = test_sampler.as_deref() {
        let _ = rss_sampler_report(path, Some("load-end"));
    }
    if let Value::Object(object) = &mut row {
        let mut rss = rss_with_baseline(
            rss_peak_report(test_sampler.as_deref(), Some("peak")),
            &baseline_rss,
        );
        if let Value::Object(rss_object) = &mut rss {
            rss_object.insert("worker_isolated".into(), json!(true));
        }
        object.insert("rss".into(), rss);
        object.insert("worker_isolated".into(), json!(true));
    }
    println!("{}", serde_json::to_string(&row)?);
    Ok(row)
}

fn report(root: &Path, seed: u64, smoke: bool) -> Result<Value> {
    let iterations = test_iterations(if smoke { 2 } else { 10 });
    report_with_iterations(root, seed, smoke, iterations)
}

fn report_with_iterations(root: &Path, seed: u64, smoke: bool, iterations: usize) -> Result<Value> {
    let app = App::open(root).context("abrir App")?;
    let c = calls();
    let mut acquisition_trace = Map::new();
    let rows = vec![
        variant_row(
            &app,
            root,
            "disk-reparseo",
            iterations,
            &c,
            &mut acquisition_trace,
        )?,
        variant_row(
            &app,
            root,
            "sqlite-raw",
            iterations,
            &c,
            &mut acquisition_trace,
        )?,
        variant_row(
            &app,
            root,
            "ram-memoizado",
            iterations,
            &c,
            &mut acquisition_trace,
        )?,
    ];
    let (disk_set, discovery) = app
        .workspace()
        .document_set_with_discovery()
        .context("adquirir seam de disco")?;
    let app_values = seven_from_app(&app, &c);
    let seam = app
        .read_services(&disk_set)
        .with_discovery_diagnostics(discovery);
    let seam_values = seven_from_seam(&seam, &c);
    let app_results: Map<String, Value> = TOOLS
        .iter()
        .map(|tool| ((*tool).into(), app_values[*tool].clone()))
        .collect();
    let seam_results: Map<String, Value> = TOOLS
        .iter()
        .map(|tool| ((*tool).into(), seam_values[*tool].clone()))
        .collect();
    let negative_results = negative_results(&app, &c, &disk_set);
    let change_cycle = change_cycle(root, iterations)?;
    let commit = git_commit().unwrap_or_else(|| "unknown".into());
    let working_tree_clean = git_working_tree_clean();
    finish_test_barrier();
    Ok(json!({
        "schema_version": "e33-h04-v2", "seed": seed, "machine": machine_label(),
        "binary": binary_label(), "build_profile": build_profile(),
        "runtime_profile": "standard",
        "commit": commit,
        "provenance": {"commit": git_commit().unwrap_or_else(|| "unknown".into()), "working_tree_clean": working_tree_clean},
        "variants": BENCH_VARIANTS, "tools": TOOLS,
        "measurements": rows, "app_results": app_results, "seam_results": seam_results,
        "negative_results": negative_results, "change_cycle": change_cycle,
        "acquisition_trace": acquisition_trace,
        "sqlite_raw_note": "Store::document_set evita walk+IO, pero DocumentSet::from_store vuelve a parsear raw; rebuild se registra separado.",
        "wire_calibration": wire_calibration_pending(),
        "markdown_summary": format!("# Lodestar H04\n\nseed: {seed}\nmode: {}\n", if smoke {"smoke"} else {"full"}),
    }))
}

#[derive(Debug, Clone)]
struct FullExecutionConfig {
    schema_version: &'static str,
    runtime_profile: &'static str,
    wire_calibration: &'static str,
    output_formats: [&'static str; 3],
    iterations: usize,
    equivalence: &'static str,
    cold_open: &'static str,
    sqlite_rebuild: &'static str,
}

fn full_execution_config() -> FullExecutionConfig {
    FullExecutionConfig {
        schema_version: "e33-h04-v2-full",
        runtime_profile: "standard",
        wire_calibration: "full-only",
        output_formats: ["stdout-json", "json-file", "markdown-file"],
        iterations: test_iterations(FULL_ITERATIONS),
        equivalence: "exact-normalized-results",
        cold_open: "app-open-plus-first-read",
        sqlite_rebuild: "separate-from-read-percentiles",
    }
}

fn stable_fingerprint(value: &Value) -> String {
    // This is an identification digest, not a security boundary. FNV-1a keeps the seam
    // dependency-free while remaining stable across platforms and process executions.
    let bytes = serde_json::to_vec(value).expect("configuración full serializable");
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn full_execution_config_value(config: &FullExecutionConfig) -> Value {
    let mut value = json!({
        "schema_version": config.schema_version,
        "runtime_profile": config.runtime_profile,
        "wire_calibration": config.wire_calibration,
        "output_formats": config.output_formats,
        "iterations": config.iterations,
        "equivalence": config.equivalence,
        "cold_open": config.cold_open,
        "sqlite_rebuild": config.sqlite_rebuild,
    });
    let fingerprint = stable_fingerprint(&value);
    value
        .as_object_mut()
        .expect("configuración full objeto")
        .insert("fingerprint".into(), Value::String(fingerprint));
    value
}

fn full_plan() -> Value {
    let config = full_execution_config();
    let config_value = full_execution_config_value(&config);
    let jobs = FULL_PROFILES
        .iter()
        .flat_map(|(profile, _)| {
            FULL_SCALES.iter().map(move |scale| {
                json!({
                    "mode": "full",
                    "profile": profile,
                    "scale": scale,
                    "iterations": config.iterations,
                    "variants": BENCH_VARIANTS,
                    "tools": TOOLS
                })
            })
        })
        .collect::<Vec<_>>();
    json!({
        "mode": "full",
        "schema_version": config.schema_version,
        "iterations": config.iterations,
        "profiles": FULL_PROFILE_NAMES,
        "scales": FULL_SCALES,
        "variants": BENCH_VARIANTS,
        "tools": TOOLS,
        "jobs": jobs,
        "output_formats": config.output_formats,
        "runtime_profile": config.runtime_profile,
        "equivalence": config.equivalence,
        "cold_open": config.cold_open,
        "sqlite_rebuild": config.sqlite_rebuild,
        "wire_calibration": config.wire_calibration,
        "full_execution_config": config_value
    })
}

fn full_report(seed: u64, wire_input: Option<&Path>) -> Result<Value> {
    let config = full_execution_config();
    let config_value = full_execution_config_value(&config);
    let base = tempfile::TempDir::new().context("crear root temporal full")?;
    let base_path = base.path();
    let mut runs = Vec::new();
    for (profile, perfil) in FULL_PROFILES {
        for scale in FULL_SCALES {
            let root = base_path.join(profile).join(scale.to_string());
            std::fs::create_dir_all(&root)?;
            escala::genera(&root, perfil, scale, seed)
                .with_context(|| format!("generar corpus {profile}/{scale}"))?;
            overlay_control(&root)?;
            let mut run = report_with_iterations(&root, seed, false, config.iterations)?;
            if let Value::Object(ref mut object) = run {
                object.remove("wire_calibration");
                object.insert("profile".into(), json!(profile));
                object.insert("corpus_profile".into(), json!(profile));
                object.insert("runtime_profile".into(), json!("standard"));
                object.insert("scale".into(), json!(scale));
            }
            runs.push(run);
        }
    }
    let mut wire_calibration = wire_input
        .map(wire_calibration_from_input)
        .transpose()?
        .unwrap_or_else(wire_calibration_pending);
    if let Value::Object(ref mut object) = wire_calibration {
        object.insert("profile".into(), json!("realista"));
        object.insert("corpus_profile".into(), json!("realista"));
        // El arnés conserva su regla dura: un `--root` real solo se abre en readonly. El wire
        // sigue acotando framing, arranque y adquisición, sin fingir igualdad del payload de
        // capacidades con el banco interno (`standard`).
        object.insert("runtime_profile".into(), json!("readonly"));
    }
    Ok(json!({
        "schema_version": config.schema_version,
        "seed": seed,
        "machine": machine_label(),
        "binary": binary_label(),
        "build_profile": build_profile(),
        "runtime_profile": config.runtime_profile,
        "commit": git_commit().unwrap_or_else(|| "unknown".into()),
        "provenance": {"commit": git_commit().unwrap_or_else(|| "unknown".into()), "working_tree_clean": git_working_tree_clean()},
        "profiles": FULL_PROFILE_NAMES,
        "scales": FULL_SCALES,
        "iterations": config.iterations,
        "full_execution_config": config_value,
        "runs": runs,
        "wire_calibration": wire_calibration,
    }))
}

const GATE_TOOLS: [&str; 7] = [
    "workspace_status",
    "knowledge_search",
    "knowledge_get",
    "metadata_inspect",
    "graph_query",
    "impact_analyze",
    "knowledge_check",
];

#[derive(Debug, Clone, Copy)]
struct GateMetrics {
    tools: [&'static str; 7],
    p95_ns: [u64; 7],
    cold_open_ns: u64,
}

#[derive(Debug, Clone, Copy)]
struct GateSelection<'a> {
    profile: &'a str,
    metrics: GateMetrics,
}

fn read_gate_json(path: &Path, label: &str) -> Result<Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("leer {label} {}", path.display()))?;
    reject_duplicate_tools(&text, label)?;
    serde_json::from_str(&text)
        .with_context(|| format!("parsear JSON de {label} {}", path.display()))
}

struct ScanSeed<'a> {
    label: &'a str,
}

struct ScanVisitor<'a> {
    label: &'a str,
}

impl<'de, 'a> DeserializeSeed<'de> for ScanSeed<'a> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ScanVisitor { label: self.label })
    }
}

impl<'de, 'a> Visitor<'de> for ScanVisitor<'a> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("JSON value")
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_string<E>(self, _value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence
            .next_element_seed(ScanSeed { label: self.label })?
            .is_some()
        {}
        Ok(())
    }

    fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        scan_map(map, self.label)
    }
}

struct ToolsVisitor<'a> {
    label: &'a str,
}

impl<'de, 'a> Visitor<'de> for ToolsVisitor<'a> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("tools JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut seen = std::collections::HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(de::Error::custom(format!(
                    "{}: tools contiene tool duplicada: {key}",
                    self.label
                )));
            }
            map.next_value::<Value>()?;
        }
        Ok(())
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence
            .next_element_seed(ScanSeed { label: self.label })?
            .is_some()
        {}
        Ok(())
    }
}

fn scan_map<'de, A>(mut map: A, label: &str) -> Result<(), A::Error>
where
    A: MapAccess<'de>,
{
    while let Some(key) = map.next_key::<String>()? {
        if key == "tools" {
            map.next_value_seed(ToolsSeed { label })?;
        } else {
            map.next_value_seed(ScanSeed { label })?;
        }
    }
    Ok(())
}

struct ToolsSeed<'a> {
    label: &'a str,
}

impl<'de, 'a> DeserializeSeed<'de> for ToolsSeed<'a> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ToolsVisitor { label: self.label })
    }
}

fn reject_duplicate_tools(text: &str, label: &str) -> Result<()> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    deserializer
        .deserialize_any(ScanVisitor { label })
        .with_context(|| format!("validar tools de {label}"))?;
    deserializer
        .end()
        .with_context(|| format!("validar JSON completo de {label}"))
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<&'a str> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("{context}: {key} debe ser una cadena no vacía"))?;
    Ok(value)
}

fn required_u64(object: &Map<String, Value>, key: &str, context: &str) -> Result<u64> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| anyhow!("{context}: {key} debe ser un entero positivo"))
}

fn gate_metric(value: &Value, context: &str) -> Result<u64> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("{context}: la métrica debe ser un objeto JSON"))?;
    required_u64(object, "p95_ns", context)
}

fn gate_measurement(value: &Value, context: &str) -> Result<GateMetrics> {
    let measurement = value
        .as_object()
        .ok_or_else(|| anyhow!("{context}: measurement debe ser un objeto JSON"))?;
    if required_string(measurement, "variant", context)? != "disk-reparseo" {
        return Err(anyhow!(
            "{context}: solo se puede juzgar variant=disk-reparseo"
        ));
    }
    let document_count = measurement
        .get("document_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("{context}: falta document_count entero"))?;
    if document_count < 10_000 {
        return Err(anyhow!(
            "{context}: solo se puede juzgar document_count>=10000 (medido {document_count})"
        ));
    }
    let tools = measurement
        .get("tools")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("{context}: falta tools como objeto"))?;
    let missing = GATE_TOOLS
        .iter()
        .filter(|tool| !tools.contains_key(**tool))
        .copied()
        .collect::<Vec<_>>();
    let unexpected = tools
        .keys()
        .filter(|tool| !GATE_TOOLS.contains(&tool.as_str()))
        .map(String::as_str)
        .collect::<Vec<_>>();
    if !missing.is_empty() || !unexpected.is_empty() {
        return Err(anyhow!(
            "{context}: tools debe contener exactamente las siete tools requeridas una vez; faltan={missing:?}, sobrantes={unexpected:?}"
        ));
    }
    let mut p95_ns = [0_u64; 7];
    for (index, tool) in GATE_TOOLS.iter().enumerate() {
        let metric = tools
            .get(*tool)
            .ok_or_else(|| anyhow!("{context}: falta tools.{tool}"))?;
        p95_ns[index] = gate_metric(metric, &format!("{context}.tools.{tool}"))?;
    }
    let cold_open_ns = gate_metric(
        measurement
            .get("cold_open")
            .ok_or_else(|| anyhow!("{context}: falta cold_open"))?,
        &format!("{context}.cold_open"),
    )?;
    Ok(GateMetrics {
        tools: GATE_TOOLS,
        p95_ns,
        cold_open_ns,
    })
}

fn gate_report_metrics_for_profile<'a>(
    report: &'a Value,
    expected_profile: Option<&str>,
    label: &str,
) -> Result<GateSelection<'a>> {
    let object = report
        .as_object()
        .ok_or_else(|| anyhow!("{label}: el informe debe ser un objeto JSON"))?;
    if required_string(object, "schema_version", label)? != "e33-h04-v2-full" {
        return Err(anyhow!("{label}: schema_version debe ser e33-h04-v2-full"));
    }
    let runs = object
        .get("runs")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("{label}: falta runs como array"))?;
    let mut candidates = Vec::new();
    for (run_index, run) in runs.iter().enumerate() {
        let run_object = run
            .as_object()
            .ok_or_else(|| anyhow!("{label}.runs[{run_index}]: debe ser un objeto JSON"))?;
        let scale = run_object
            .get("scale")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("{label}.runs[{run_index}]: falta scale entero"))?;
        if scale != 10_000 {
            continue;
        }
        let profile = run_object.get("profile").and_then(Value::as_str);
        if let Some(expected_profile) = expected_profile {
            if profile != Some(expected_profile) {
                continue;
            }
        }
        let profile = profile.ok_or_else(|| {
            anyhow!("{label}.runs[{run_index}]: falta profile para seleccionar la corrida")
        })?;
        let measurements = run_object
            .get("measurements")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("{label}.runs[{run_index}]: falta measurements como array"))?;
        for (measurement_index, measurement) in measurements.iter().enumerate() {
            let measurement_object = measurement.as_object().ok_or_else(|| {
                anyhow!(
                    "{label}.runs[{run_index}].measurements[{measurement_index}]: debe ser un objeto JSON"
                )
            })?;
            if measurement_object.get("variant").and_then(Value::as_str) != Some("disk-reparseo") {
                continue;
            }
            candidates.push((
                profile,
                gate_measurement(
                    measurement,
                    &format!("{label}.runs[{run_index}].measurements[{measurement_index}]"),
                )?,
            ));
        }
    }
    if candidates.is_empty() {
        return Err(match expected_profile {
            Some(profile) => anyhow!(
                "{label}: no hay measurement disk-reparseo a escala 10000 para profile={profile}"
            ),
            None => anyhow!("{label}: no hay measurement disk-reparseo a escala 10000"),
        });
    }
    if candidates.len() > 1 {
        return Err(anyhow!(
            "{label}: hay múltiples measurements disk-reparseo a escala 10000 dentro de profile={}; la corrida no es unívoca",
            candidates[0].0
        ));
    }
    let (profile, metrics) = candidates.pop().expect("candidates no vacío validado");
    Ok(GateSelection { profile, metrics })
}

fn gate_report_metrics<'a>(report: &'a Value, label: &str) -> Result<GateSelection<'a>> {
    gate_report_metrics_for_profile(report, None, label)
}

fn gate_report_schema(value: &Value, label: &str) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("{label}: el informe debe ser un objeto JSON"))?;
    if required_string(object, "schema_version", label)? != "e33-h04-v2-full" {
        return Err(anyhow!("{label}: schema_version debe ser e33-h04-v2-full"));
    }
    Ok(())
}

fn gate_thresholds(value: &Value, label: &str) -> Result<(String, u64, u64)> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("{label}: thresholds debe ser un objeto JSON"))?;
    if required_string(object, "schema_version", label)? != "e33-h05-thresholds-v1" {
        return Err(anyhow!("{label}: schema_version no reconocido"));
    }
    if required_string(object, "ratified_on", label)? != "2026-08-22" {
        return Err(anyhow!("{label}: ratified_on debe ser 2026-08-22"));
    }
    if !required_string(object, "reference", label)?.contains("E33-H05") {
        return Err(anyhow!("{label}: reference debe citar E33-H05"));
    }
    if required_string(object, "variant", label)? != "disk-reparseo"
        || object.get("scale").and_then(Value::as_u64) != Some(10_000)
    {
        return Err(anyhow!(
            "{label}: variant debe ser disk-reparseo y scale debe ser 10000"
        ));
    }
    let machine = required_string(object, "absolute_machine_id", label)?.to_owned();
    Ok((
        machine,
        required_u64(object, "p95_ns", label)?,
        required_u64(object, "cold_open_ns", label)?,
    ))
}

fn gate_baseline_report<'a>(
    value: &'a Value,
    machine_id: &str,
    label: &str,
) -> Result<GateSelection<'a>> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("{label}: baseline machine entry debe ser un objeto JSON"))?;
    if required_string(object, "machine_id", label)? != machine_id {
        return Err(anyhow!("{label}: machine_id no coincide con su clave"));
    }
    gate_report_metrics(
        object
            .get("report")
            .ok_or_else(|| anyhow!("{label}: falta report"))?,
        &format!("{label}.report"),
    )
}

fn gate_baseline<'a>(
    value: &'a Value,
    machine_id: &str,
    label: &str,
) -> Result<Option<GateSelection<'a>>> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("{label}: baseline debe ser un objeto JSON"))?;
    if required_string(object, "schema_version", label)? != "e33-h05-baseline-v1" {
        return Err(anyhow!("{label}: schema_version no reconocido"));
    }
    let absolute = required_string(object, "absolute_machine_id", label)?;
    let machines = object
        .get("machines")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("{label}: machines debe ser un objeto JSON"))?;
    if !machines.contains_key(absolute) {
        return Err(anyhow!(
            "{label}: falta la entrada de absolute_machine_id {absolute}"
        ));
    }
    for (entry_machine_id, entry) in machines {
        gate_baseline_report(
            entry,
            entry_machine_id,
            &format!("{label}.machines.{entry_machine_id}"),
        )?;
    }
    match machines.get(machine_id) {
        Some(entry) => {
            gate_baseline_report(entry, machine_id, &format!("{label}.machines.{machine_id}"))
                .map(Some)
        }
        None => Ok(None),
    }
}

fn run_gate(
    report_path: &Path,
    thresholds_path: &Path,
    baseline_path: &Path,
    machine_id: &str,
) -> Result<()> {
    let report = read_gate_json(report_path, "report")?;
    let thresholds = read_gate_json(thresholds_path, "thresholds")?;
    let baseline = read_gate_json(baseline_path, "baseline")?;
    let (absolute_machine, p95_limit, cold_limit) = gate_thresholds(&thresholds, "thresholds")?;
    let baseline_object = baseline
        .as_object()
        .ok_or_else(|| anyhow!("baseline: debe ser un objeto JSON"))?;
    let baseline_absolute_machine =
        required_string(baseline_object, "absolute_machine_id", "baseline")?;
    if baseline_absolute_machine != absolute_machine {
        return Err(anyhow!(
            "baseline.absolute_machine_id ({baseline_absolute_machine}) no coincide con thresholds.absolute_machine_id ({absolute_machine})"
        ));
    }
    let own_baseline = gate_baseline(&baseline, machine_id, "baseline")?;
    gate_report_schema(&report, "report")?;
    if machine_id != absolute_machine && own_baseline.is_none() {
        println!(
            "PASS gate mode=tendencia machine_id={machine_id}; absolutos no juzgados fuera de {absolute_machine}"
        );
        println!(
            "PASS tendencia sin baseline propia: no hay comparación disponible; no se inventa comparación"
        );
        return Ok(());
    }
    let current_selection = match own_baseline {
        Some(previous) => {
            gate_report_metrics_for_profile(&report, Some(previous.profile), "report")?
        }
        None => gate_report_metrics(&report, "report")?,
    };
    let current_profile = current_selection.profile;
    let current = current_selection.metrics;
    if machine_id == absolute_machine {
        let mut failures = Vec::new();
        for (index, tool) in current.tools.iter().enumerate() {
            let measured = current.p95_ns[index];
            if measured > p95_limit {
                failures.push(format!(
                    "p95 tool={tool} medido={measured} ns limit={p95_limit} ns"
                ));
            } else {
                println!(
                    "PASS mode=absolute tool={tool} p95={measured} ns limit={p95_limit} ns margin={} ns",
                    p95_limit - measured
                );
            }
        }
        if current.cold_open_ns > cold_limit {
            failures.push(format!(
                "cold-open medido={} ns limit={} ns",
                current.cold_open_ns, cold_limit
            ));
        } else {
            println!(
                "PASS mode=absolute cold-open={} ns limit={} ns margin={} ns",
                current.cold_open_ns,
                cold_limit,
                cold_limit - current.cold_open_ns
            );
        }
        if failures.is_empty() {
            println!(
                "PASS gate mode=absolute machine_id={machine_id} profile={current_profile} variant=disk-reparseo scale=10000"
            );
            return Ok(());
        }
        for failure in &failures {
            eprintln!("FAIL umbral violado {failure}");
        }
        return Err(anyhow!("gate absoluto falló ({} métricas)", failures.len()));
    }

    println!(
        "PASS gate mode=tendencia machine_id={machine_id} profile={current_profile}; absolutos no juzgados fuera de {absolute_machine}"
    );
    let Some(previous) = own_baseline else {
        println!("PASS tendencia sin baseline propia profile={current_profile}: no hay comparación disponible; no se inventa comparación");
        return Ok(());
    };
    let mut regressions = Vec::new();
    for (index, tool) in current.tools.iter().enumerate() {
        if current.p95_ns[index] > previous.metrics.p95_ns[index] {
            regressions.push(format!(
                "degradación tendencia tool={tool} medido={} ns baseline={} ns",
                current.p95_ns[index], previous.metrics.p95_ns[index]
            ));
        }
    }
    if current.cold_open_ns > previous.metrics.cold_open_ns {
        regressions.push(format!(
            "degradación tendencia cold-open medido={} ns baseline={} ns",
            current.cold_open_ns, previous.metrics.cold_open_ns
        ));
    }
    if regressions.is_empty() {
        println!("PASS tendencia: no hay degradación frente a la baseline propia");
        return Ok(());
    }
    for regression in &regressions {
        eprintln!("FAIL {regression}");
    }
    Err(anyhow!(
        "gate de tendencia falló ({} métricas)",
        regressions.len()
    ))
}

fn overlay_control(root: &Path) -> Result<()> {
    std::fs::write(
        root.join("control.md"),
        "---\ntags: [h04, control]\nservice: bench\n---\n# Control\nmarker-search-h04\n[child](child.md)\n[missing](missing.md)\n",
    )?;
    std::fs::write(
        root.join("child.md"),
        "---\ntags: [child]\nservice: bench\n---\n# Child\nmarker-get-h04\n[leaf](leaf.md)\n",
    )?;
    std::fs::write(
        root.join("leaf.md"),
        "---\ntags: [leaf]\nservice: bench\n---\n# Leaf\nmarker-impact-h04\n",
    )?;
    std::fs::write(root.join("broken.md"), "---\ntags: [\n---\n# Broken\n")?;
    Ok(())
}

fn copy_smoke_root(source: &Path, target: &Path) -> Result<()> {
    fn copy_tree(source: &Path, target: &Path) -> Result<()> {
        for entry in
            std::fs::read_dir(source).with_context(|| format!("leer {}", source.display()))?
        {
            let entry = entry?;
            let source_path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == ".git" {
                continue;
            }
            if source.file_name().is_some_and(|part| part == ".lodestar")
                && (name == "runtime" || name == "cache" || name == "index.db")
            {
                continue;
            }
            let file_type = std::fs::symlink_metadata(&source_path)?.file_type();
            if file_type.is_symlink() {
                return Err(anyhow!(
                    "el root smoke contiene symlink: {}",
                    source_path.display()
                ));
            }
            let target_path = target.join(entry.file_name());
            if file_type.is_dir() {
                std::fs::create_dir_all(&target_path)?;
                copy_tree(&source_path, &target_path)?;
            } else if file_type.is_file() {
                std::fs::copy(&source_path, &target_path)
                    .with_context(|| format!("copiar {}", source_path.display()))?;
            }
        }
        Ok(())
    }
    copy_tree(source, target)
}

fn wire_calibration_pending() -> Value {
    json!({
        "status": "pending",
        "results": [],
        "tools": ["workspace_status", "knowledge_search"],
        "scale": 10000,
        "command": "python3 docs/qa/testbench/lodestar_harness.py --root <CORPUS_10K> --profile readonly --binary <PATH>/lodestar-mcp --call workspace_status '{}' > <WIRE_JSON_STATUS> ; repetir con --root <CORPUS_10K> --profile readonly --binary <PATH>/lodestar-mcp --call knowledge_search '{\"text\":\"marker-search-h04\",\"where\":\"service = \\\"bench\\\"\"}' > <WIRE_JSON_SEARCH> ; preparar <WIRE_CALIBRATION_JSON> y pasar --wire-calibration-input <WIRE_CALIBRATION_JSON>"
    })
}

fn wire_calibration_from_input(path: &Path) -> Result<Value> {
    let value: Value = serde_json::from_str(
        &std::fs::read_to_string(path)
            .with_context(|| format!("leer wire input {}", path.display()))?,
    )
    .context("parsear wire calibration input")?;
    validate_wire_calibration(&value)?;
    Ok(value)
}

fn validate_wire_calibration(value: &Value) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("wire calibration debe ser un objeto JSON"))?;
    if object.get("status").and_then(Value::as_str) != Some("complete") {
        return Err(anyhow!("wire calibration exige status=complete"));
    }
    if object.get("profile").and_then(Value::as_str) != Some("realista")
        || object.get("scale").and_then(Value::as_u64) != Some(10_000)
        || object.get("runtime_profile").and_then(Value::as_str) != Some("readonly")
    {
        return Err(anyhow!(
            "wire calibration exige profile=realista, scale=10000 y runtime_profile=readonly"
        ));
    }
    for key in [
        "harness",
        "transport",
        "binary",
        "build_profile",
        "process_protocol",
        "clock",
    ] {
        if object
            .get(key)
            .and_then(Value::as_str)
            .map_or(true, str::is_empty)
        {
            return Err(anyhow!("wire calibration carece de metadata {key}"));
        }
    }
    if object.get("transport").and_then(Value::as_str) != Some("JSON-RPC/stdio")
        || object.get("binary").and_then(Value::as_str) != Some("lodestar-mcp")
        || !object
            .get("harness")
            .and_then(Value::as_str)
            .is_some_and(|value| value.ends_with("lodestar_harness.py"))
        || object.get("build_profile").and_then(Value::as_str) != Some("release")
        || object
            .get("corpus_documents")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            < 10_000
    {
        return Err(anyhow!(
            "wire calibration exige JSON-RPC/stdio y lodestar-mcp"
        ));
    }
    let results = object
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("wire calibration.results debe ser array"))?;
    let expected = ["workspace_status", "knowledge_search"];
    if results.len() != expected.len()
        || results
            .iter()
            .enumerate()
            .any(|(index, row)| row.get("tool").and_then(Value::as_str) != Some(expected[index]))
    {
        return Err(anyhow!(
            "wire calibration debe contener exactamente status y search"
        ));
    }
    for row in results {
        let sample_count = row.get("sample_count").and_then(Value::as_u64).unwrap_or(0);
        if sample_count == 0
            || row
                .get("real_seconds")
                .and_then(Value::as_array)
                .map_or(true, |samples| {
                    samples.len() != sample_count as usize
                        || samples
                            .iter()
                            .any(|sample| sample.as_f64().map_or(true, |value| value <= 0.0))
                })
            || row.get("p50_seconds").and_then(Value::as_f64).is_none()
            || row.get("p95_seconds").and_then(Value::as_f64).is_none()
            || row
                .get("p50_seconds")
                .and_then(Value::as_f64)
                .is_some_and(|value| value <= 0.0)
            || row
                .get("p95_seconds")
                .and_then(Value::as_f64)
                .is_some_and(|value| value <= 0.0)
            || row
                .get("payload_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                == 0
            || row
                .get("result_check")
                .and_then(Value::as_object)
                .map_or(true, |check| {
                    check.get("is_error") != Some(&Value::Bool(false))
                })
        {
            return Err(anyhow!("wire calibration tiene una fila incompleta"));
        }
        match row.get("tool").and_then(Value::as_str) {
            Some("workspace_status")
                if row["result_check"]["documents"].as_u64().unwrap_or(0) >= 10_000 => {}
            Some("knowledge_search")
                if row["result_check"]["total_approximate"]
                    .as_u64()
                    .unwrap_or(0)
                    >= 1
                    && row["result_check"]["path"] == Value::String("control.md".into()) => {}
            _ => return Err(anyhow!("wire calibration result_check no ejercita su tool")),
        }
    }
    Ok(())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn resolve_versioned_path(evidence_path: &Path, reference: &str) -> PathBuf {
    let reference = Path::new(reference);
    if reference.is_absolute() {
        return reference.to_owned();
    }
    let alongside = evidence_path
        .parent()
        .map(|parent| parent.join(reference))
        .unwrap_or_else(|| reference.to_owned());
    if alongside.is_file() {
        alongside
    } else {
        repo_root().join(reference)
    }
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("abrir {} para calcular SHA-256", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("leer {} para calcular SHA-256", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn transcript_seconds(path: &Path) -> Result<f64> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("leer transcript {}", path.display()))?;
    let values: Vec<f64> = text
        .lines()
        .filter_map(|line| line.strip_prefix("real "))
        .map(|value| {
            value
                .trim()
                .parse::<f64>()
                .with_context(|| format!("real no parseable en {}", path.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    if values.len() != 1 || values[0] <= 0.0 {
        return Err(anyhow!(
            "transcript {} debe tener exactamente un real positivo",
            path.display()
        ));
    }
    Ok(values[0])
}

fn wire_result_check(tool: &str, raw: &Value) -> Result<Value> {
    if raw.get("is_error") != Some(&Value::Bool(false)) {
        return Err(anyhow!("{tool}: raw wire marcado como error"));
    }
    let structured = raw
        .get("structured")
        .ok_or_else(|| anyhow!("{tool}: raw wire carece de structured"))?;
    let check = match tool {
        "workspace_status" => json!({
            "documents": structured["counts"]["documents"],
            "is_error": false,
        }),
        "knowledge_search" => json!({
            "is_error": false,
            "path": structured["results"][0]["path"],
            "total_approximate": structured["totalApproximate"],
        }),
        _ => return Err(anyhow!("tool wire no soportada: {tool}")),
    };
    if check
        .as_object()
        .is_some_and(|object| object.values().any(Value::is_null))
    {
        return Err(anyhow!(
            "{tool}: structured no permite derivar result_check"
        ));
    }
    Ok(check)
}

fn wire_p95_index(len: usize) -> usize {
    ((len * 95).saturating_sub(1) / 100).min(len - 1)
}

struct WireSample {
    index: u64,
    seconds: f64,
    stdout_bytes: u64,
    structured_bytes: u64,
    result_check: Value,
    arguments: Map<String, Value>,
}

fn validate_wire_calibration_chain(evidence_path: &Path, official_path: &Path) -> Result<()> {
    let evidence: Value = serde_json::from_str(
        &std::fs::read_to_string(evidence_path)
            .with_context(|| format!("leer evidencia wire {}", evidence_path.display()))?,
    )
    .context("parsear evidencia wire")?;
    let official: Value = serde_json::from_str(
        &std::fs::read_to_string(official_path)
            .with_context(|| format!("leer corrida oficial {}", official_path.display()))?,
    )
    .context("parsear corrida oficial")?;
    let observations = evidence
        .get("observations")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("evidencia wire carece de observations"))?;
    let mut by_tool: BTreeMap<String, Vec<WireSample>> = BTreeMap::new();
    for observation in observations {
        let object = observation
            .as_object()
            .ok_or_else(|| anyhow!("observación wire no es objeto"))?;
        let tool = object
            .get("tool")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("observación wire carece de tool"))?;
        if !matches!(tool, "workspace_status" | "knowledge_search") {
            return Err(anyhow!("tool wire inesperada: {tool}"));
        }
        let index = object
            .get("index")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("{tool}: observación carece de index"))?;
        let stdout_ref = object
            .get("raw_stdout")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("{tool} {index}: carece de raw_stdout"))?;
        let transcript_ref = object
            .get("time_transcript")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("{tool} {index}: carece de time_transcript"))?;
        let stdout = resolve_versioned_path(evidence_path, stdout_ref);
        let transcript = resolve_versioned_path(evidence_path, transcript_ref);
        let raw_bytes = std::fs::read(&stdout)
            .with_context(|| format!("leer raw stdout {}", stdout.display()))?;
        let raw: Value = serde_json::from_slice(&raw_bytes)
            .with_context(|| format!("parsear raw stdout {}", stdout.display()))?;
        if raw.get("kind").and_then(Value::as_str) != Some("call")
            || raw.get("tool").and_then(Value::as_str) != Some(tool)
        {
            return Err(anyhow!(
                "{tool} {index}: raw no corresponde a la observación"
            ));
        }
        let stdout_hash = sha256_file(&stdout)?;
        if object.get("sha256_stdout").and_then(Value::as_str) != Some(&stdout_hash) {
            return Err(anyhow!("{tool} {index}: SHA-256 stdout no coincide"));
        }
        let transcript_hash = sha256_file(&transcript)?;
        if object.get("sha256_transcript").and_then(Value::as_str) != Some(&transcript_hash) {
            return Err(anyhow!("{tool} {index}: SHA-256 transcript no coincide"));
        }
        let seconds = transcript_seconds(&transcript)?;
        if object.get("wall_seconds").and_then(Value::as_f64) != Some(seconds) {
            return Err(anyhow!(
                "{tool} {index}: wall_seconds no coincide con transcript"
            ));
        }
        let structured = raw
            .get("structured")
            .ok_or_else(|| anyhow!("{tool} {index}: raw carece de structured"))?;
        let structured_bytes = serde_json::to_vec(structured)?.len() as u64;
        let stdout_bytes = raw_bytes.len() as u64;
        for (field, expected) in [
            ("payload_bytes", stdout_bytes),
            ("payload_bytes_stdout", stdout_bytes),
            ("payload_bytes_structured_content", structured_bytes),
        ] {
            if object.get(field).and_then(Value::as_u64) != Some(expected) {
                return Err(anyhow!(
                    "{tool} {index}: {field} no deriva del raw versionado"
                ));
            }
        }
        let result_check = wire_result_check(tool, &raw)?;
        if object.get("result_check") != Some(&result_check) {
            return Err(anyhow!("{tool} {index}: result_check no coincide con raw"));
        }
        let arguments = object
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("{tool} {index}: args ausentes"))?
            .clone();
        if raw.get("arguments") != Some(&Value::Object(arguments.clone())) {
            return Err(anyhow!("{tool} {index}: args no coinciden con raw"));
        }
        by_tool
            .entry(tool.to_owned())
            .or_default()
            .push(WireSample {
                index,
                seconds,
                stdout_bytes,
                structured_bytes,
                result_check,
                arguments,
            });
    }
    let wire = official
        .get("wire_calibration")
        .ok_or_else(|| anyhow!("corrida oficial carece de wire_calibration"))?;
    validate_wire_calibration(wire)?;
    let official_results = wire
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("corrida oficial carece de resultados wire"))?;
    if official_results.len() != by_tool.len() {
        return Err(anyhow!(
            "resultados oficiales y observaciones no tienen la misma cardinalidad"
        ));
    }
    for (tool, samples) in by_tool {
        let mut samples = samples;
        samples.sort_by_key(|sample| sample.index);
        if samples
            .windows(2)
            .any(|window| window[0].index == window[1].index)
        {
            return Err(anyhow!("{tool}: index de observación duplicado"));
        }
        let official_row = official_results
            .iter()
            .find(|row| row.get("tool").and_then(Value::as_str) == Some(tool.as_str()))
            .ok_or_else(|| anyhow!("corrida oficial carece de {tool}"))?;
        let seconds: Vec<f64> = samples.iter().map(|sample| sample.seconds).collect();
        let mut sorted = seconds.clone();
        sorted.sort_by(f64::total_cmp);
        let expected_check = samples[0].result_check.clone();
        let expected_arguments = Value::Object(samples[0].arguments.clone());
        if samples.iter().any(|sample| {
            sample.stdout_bytes != samples[0].stdout_bytes
                || sample.structured_bytes != samples[0].structured_bytes
                || sample.result_check != expected_check
                || sample.arguments != samples[0].arguments
        }) {
            return Err(anyhow!("{tool}: las observaciones wire no son homogéneas"));
        }
        if official_row.get("sample_count").and_then(Value::as_u64) != Some(samples.len() as u64)
            || official_row.get("real_seconds") != Some(&json!(seconds))
            || official_row.get("p50_seconds") != Some(&json!(sorted[sorted.len() / 2]))
            || official_row.get("p95_seconds") != Some(&json!(sorted[wire_p95_index(sorted.len())]))
            || official_row.get("result_check") != Some(&expected_check)
            || official_row.get("arguments") != Some(&expected_arguments)
            || official_row
                .get("payload_bytes_stdout")
                .and_then(Value::as_u64)
                != Some(samples[0].stdout_bytes)
            || official_row.get("payload_bytes").and_then(Value::as_u64)
                != Some(samples[0].stdout_bytes)
            || official_row
                .get("payload_bytes_structured_content")
                .and_then(Value::as_u64)
                != Some(samples[0].structured_bytes)
        {
            return Err(anyhow!("corrida oficial no deriva completamente de {tool}"));
        }
    }
    Ok(())
}

fn check_a6_dependencies(manifest: &Path) -> Result<()> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .with_context(|| format!("ejecutar cargo metadata para {}", manifest.display()))?;
    if !output.status.success() {
        return Err(anyhow!(
            "cargo metadata falló para {}: {}",
            manifest.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let metadata: Value =
        serde_json::from_slice(&output.stdout).context("parsear cargo metadata")?;
    let manifest_path = manifest
        .canonicalize()
        .unwrap_or_else(|_| manifest.to_owned());
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("cargo metadata carece de packages"))?;
    let package = packages
        .iter()
        .find(|package| {
            package.get("manifest_path").and_then(Value::as_str) == manifest_path.to_str()
        })
        .or_else(|| packages.first())
        .ok_or_else(|| anyhow!("cargo metadata no devolvió el package del manifest"))?;
    let direct_store = package
        .get("dependencies")
        .and_then(Value::as_array)
        .map_or(&[] as &[Value], Vec::as_slice)
        .iter()
        .filter(|dependency| {
            dependency.get("name").and_then(Value::as_str) == Some("lodestar-store")
                || dependency.get("rename").and_then(Value::as_str) == Some("lodestar-store")
        });
    if let Some(dependency) = direct_store.into_iter().next() {
        return Err(anyhow!(
            "A6: dependencia directa a lodestar-store rechazada (kind={:?}, target={:?})",
            dependency.get("kind"),
            dependency.get("target")
        ));
    }
    Ok(())
}

fn a6_repo_root() -> PathBuf {
    std::env::var_os(A6_REPO_ROOT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(repo_root)
}

fn check_a6_contract(root: &Path) -> Result<()> {
    // A6 siempre compara contra develop.  PR checkouts suelen exponer solo
    // origin/develop, mientras que push checkouts suelen conservar la rama local; la
    // variable de GitHub solo cambia la prioridad para el caso PR, nunca la referencia
    // normativa ni permite degradar a HEAD.
    let base_refs: [&str; 2] =
        if std::env::var_os("GITHUB_BASE_REF").as_deref() == Some(OsStr::new("develop")) {
            ["origin/develop", "develop"]
        } else {
            ["develop", "origin/develop"]
        };
    let mut base_contract = None;
    let mut failures = Vec::new();
    for base_ref in base_refs {
        let spec = format!("{base_ref}:contracts/mcp.yml");
        match Command::new("git")
            .args(["show", &spec])
            .current_dir(root)
            .output()
        {
            Ok(output) if output.status.success() => {
                base_contract = Some((base_ref, output.stdout));
                break;
            }
            Ok(output) => failures.push(format!(
                "{base_ref}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) => failures.push(format!("{base_ref}: {error}")),
        }
    }
    let (base_ref, expected) = base_contract.ok_or_else(|| {
        let github_base_ref = std::env::var("GITHUB_BASE_REF").unwrap_or_else(|_| "(vacío)".into());
        anyhow!(
            "A6: no se encontró una referencia base develop para comparar contracts/mcp.yml \
             (GITHUB_BASE_REF={github_base_ref}); se probaron develop y origin/develop: {}",
            failures.join("; ")
        )
    })?;
    let current = root.join("contracts/mcp.yml");
    let current_bytes = std::fs::read(&current)
        .with_context(|| format!("leer contrato MCP actual {}", current.display()))?;
    if current_bytes != expected {
        return Err(anyhow!(
            "A6: contracts/mcp.yml no coincide byte a byte con {base_ref}"
        ));
    }
    Ok(())
}

fn render_markdown(report: &Value) -> String {
    let mut out = String::new();
    let object = report.as_object();
    out.push_str("# Lodestar H04 — banco de rendimiento\n\n");
    if let Some(object) = object {
        for key in [
            "machine",
            "binary",
            "build_profile",
            "commit",
            "seed",
            "profiles",
            "scales",
        ] {
            if let Some(value) = object.get(key) {
                out.push_str(&format!("- **{key}**: `{value}`\n"));
            }
        }
    }
    out.push_str("\n> SQLite-raw evita walk+I/O; `DocumentSet::from_store` reparsea raw. `rebuild` se registra separado.\n\n");
    let runs = object
        .and_then(|value| value.get("runs").and_then(Value::as_array))
        .cloned()
        .unwrap_or_else(|| vec![report.clone()]);
    out.push_str("| Perfil | Escala | Variante | Tool | Muestras | p50 (ns) | p95 (ns) | Payload (bytes) |\n|---|---:|---|---|---:|---:|---:|---:|\n");
    for run in runs {
        let profile = run.get("profile").and_then(Value::as_str).unwrap_or("-");
        let scale = run
            .get("scale")
            .and_then(Value::as_u64)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".into());
        for row in run
            .get("measurements")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let variant = row.get("variant").and_then(Value::as_str).unwrap_or("-");
            if let Some(tools) = row.get("tools").and_then(Value::as_object) {
                for (tool, metric) in tools {
                    out.push_str(&format!(
                        "| {profile} | {scale} | {variant} | {tool} | {} | {} | {} | {} |\n",
                        metric["sample_count"],
                        metric["p50_ns"],
                        metric["p95_ns"],
                        metric["payload_bytes"]
                    ));
                }
            }
            if let Some(rebuild) = row.get("rebuild") {
                out.push_str(&format!(
                    "| {profile} | {scale} | {variant} | rebuild (separate) | {} | {} | {} | - |\n",
                    rebuild["sample_count"], rebuild["p50_ns"], rebuild["p95_ns"]
                ));
            }
        }
    }
    out.push_str("\n## Calibración wire\n\n");
    let wire = object.and_then(|value| value.get("wire_calibration"));
    if wire
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        == Some("complete")
    {
        out.push_str("Estado: **complete**; calibración real JSON-RPC/stdio sobre MCP.\n\n");
        for key in [
            "profile",
            "corpus_profile",
            "runtime_profile",
            "scale",
            "harness",
            "transport",
            "binary",
            "build_profile",
            "corpus_documents",
        ] {
            if let Some(value) = wire.and_then(|value| value.get(key)) {
                out.push_str(&format!("- **wire {key}**: `{value}`\n"));
            }
        }
        out.push_str("\n| Wire tool | Muestras | p50 (s) | p95 (s) | Payload stdout (bytes) | Payload structuredContent (bytes) |\n|---|---:|---:|---:|---:|---:|\n");
        if let Some(results) = wire
            .and_then(|value| value.get("results"))
            .and_then(Value::as_array)
        {
            for result in results {
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} |\n",
                    result["tool"],
                    result["sample_count"],
                    result["p50_seconds"],
                    result["p95_seconds"],
                    result["payload_bytes_stdout"],
                    result["payload_bytes_structured_content"]
                ));
            }
        }
    } else {
        out.push_str("Estado: **pending**; ejecutar el comando `wire_calibration.command` del JSON contra el corpus 10k y pasar el bloque validado con `--wire-calibration-input`. Este informe no inventa latencias de framing.\n");
    }
    out.push_str("\n## Ciclo de cambio App/disco\n\n");
    out.push_str("| Perfil | Escala | Documentos antes | Documentos después | Muestras | p50 (ns) | p95 (ns) | Preparación (ns) | Planes | Applies |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for run in runs_for_cycle(report) {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            run.0,
            run.1,
            run.2["document_count_before"],
            run.2["document_count_after"],
            run.2["metric"]["sample_count"],
            run.2["metric"]["p50_ns"],
            run.2["metric"]["p95_ns"],
            run.2["preparation_ns"],
            run.2["plan_count"],
            run.2["apply_count"],
        ));
    }
    out
}

fn render_extreme_markdown(report: &Value) -> String {
    let mut out = String::new();
    out.push_str("# Lodestar E33-H09 — sonda extrema\n\n");
    for key in [
        "profile",
        "scale",
        "iterations",
        "seed",
        "machine",
        "captured_at",
        "platform",
        "build_profile",
        "commit",
    ] {
        if let Some(value) = report.get(key) {
            out.push_str(&format!("- **{key}**: `{value}`\n"));
        }
    }
    out.push_str("\nLa sonda es opt-in y no alimenta el gate H05/10k ni CI. SQLite rebuild aparece separado de las lecturas.\n\n");
    if let Some(corpus) = report.get("corpus") {
        out.push_str(&format!(
            "## Footprint\n\n- Corpus: `{}` documentos, `{}` bytes Markdown.\n",
            corpus["document_count"], corpus["bytes"]
        ));
    }
    if let Some(sqlite) = report.get("sqlite") {
        out.push_str(&format!(
            "- SQLite principal: `{}` bytes; auxiliares: `{}`; total: `{}`.\n\n",
            sqlite["main_bytes"], sqlite["auxiliary_bytes"], sqlite["total_bytes"]
        ));
    }
    if let Some(preflight) = report.get("preflight") {
        out.push_str(&format!(
            "## Preflight\n\n- Estado: `{}`; confirmado: `{}`; disponible: `{}` bytes; requerido: `{}` bytes.\n",
            preflight["status"],
            preflight["confirmed"],
            preflight["available_bytes"],
            preflight["required_bytes"]
        ));
        if let Some(memory) = preflight
            .get("memory_verification")
            .and_then(Value::as_object)
        {
            out.push_str(&format!(
                "- Memoria/RSS: `{}`; motivo: {}\n\n",
                memory["status"], memory["reason"]
            ));
        } else {
            out.push('\n');
        }
    }
    if let Some(rows) = report.get("measurements").and_then(Value::as_array) {
        out.push_str("RSS por worker aislado (bytes):\n\n");
        for row in rows {
            let variant = row["variant"].as_str().unwrap_or("-");
            let rss = &row["rss"];
            if rss["status"] == "available" {
                out.push_str(&format!(
                    "- `{variant}`: absolute `{}`, baseline `{}`, delta `{}`.\n",
                    rss["absolute_bytes"], rss["baseline_bytes"], rss["delta_bytes"]
                ));
            } else {
                out.push_str(&format!("- `{variant}`: `{}`.\n", rss["status"]));
            }
        }
        out.push('\n');
    }
    out.push_str("| Variante | Tool | Muestras | p50 (ns) | p95 (ns) | Payload (bytes) | RSS |\n|---|---|---:|---:|---:|---:|---|\n");
    if let Some(rows) = report.get("measurements").and_then(Value::as_array) {
        for row in rows {
            let variant = row["variant"].as_str().unwrap_or("-");
            let rss = row
                .get("rss")
                .and_then(|value| value.get("absolute_bytes"))
                .map_or_else(|| row["rss"]["status"].to_string(), Value::to_string);
            if let Some(tools) = row.get("tools").and_then(Value::as_object) {
                for (tool, metric) in tools {
                    out.push_str(&format!(
                        "| {variant} | {tool} | {} | {} | {} | {} | {rss} |\n",
                        metric["sample_count"],
                        metric["p50_ns"],
                        metric["p95_ns"],
                        metric["payload_bytes"]
                    ));
                }
            }
            if let Some(cold) = row.get("cold_open") {
                out.push_str(&format!(
                    "| {variant} | cold-open | {} | {} | {} | - | {rss} |\n",
                    cold["sample_count"], cold["p50_ns"], cold["p95_ns"]
                ));
            }
            if let Some(rebuild) = row.get("rebuild") {
                out.push_str(&format!(
                    "| {variant} | rebuild (separate) | {} | {} | {} | - | {rss} |\n",
                    rebuild["sample_count"], rebuild["p50_ns"], rebuild["p95_ns"]
                ));
            }
        }
    }
    out.push_str("\nEquivalencia funcional por resultado serializado: `");
    out.push_str(
        report["functional_equivalence"]
            .as_bool()
            .map_or("unavailable", |value| if value { "true" } else { "false" }),
    );
    out.push_str("`. RSS declara método, unidades y ámbito en el JSON.\n");
    out
}

fn runs_for_cycle(report: &Value) -> Vec<(String, String, Value)> {
    let runs = report
        .get("runs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![report.clone()]);
    runs.into_iter()
        .map(|run| {
            let profile = run
                .get("profile")
                .and_then(Value::as_str)
                .unwrap_or("-")
                .to_owned();
            let scale = run
                .get("scale")
                .and_then(Value::as_u64)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".into());
            (
                profile,
                scale,
                run.get("change_cycle").cloned().unwrap_or(Value::Null),
            )
        })
        .collect()
}

fn negative_results(app: &App, c: &Calls, doc_set: &lodestar_core::DocumentSet) -> Value {
    let seam = app.read_services(doc_set);
    json!({"knowledge_get": {"app": app_result(app.knowledge_get(&c.missing, &[], None)), "seam": app_result(seam.knowledge_get(&c.missing, &[], None))}, "metadata_inspect": {"app": app_result(app.metadata_inspect("field", None, None, None)), "seam": app_result(seam.metadata_inspect("field", None, None, None))}})
}

fn change_cycle(root: &Path, iterations: usize) -> Result<Value> {
    // El caller de test puede entregar un root exacto que posee (no se toca ningún hermano). En
    // corridas normales se copia el corpus medido a un TempDir RAII independiente; la preparación
    // queda fuera del reloj de plan→apply.
    let preparation_start = Instant::now();
    let owned_temp = if std::env::var_os(CHANGE_PARENT_ENV).is_none() {
        Some(tempfile::TempDir::new().context("crear parent temporal del change cycle")?)
    } else {
        None
    };
    let exact_test_root = std::env::var_os(CHANGE_PARENT_ENV).map(PathBuf::from);
    let cycle_root = if let Some(parent) = exact_test_root.as_ref() {
        PathBuf::from(parent)
    } else {
        let cycle_root = owned_temp.as_ref().unwrap().path().join("change-cycle");
        std::fs::create_dir_all(&cycle_root)?;
        copy_smoke_root(root, &cycle_root)?;
        cycle_root
    };
    std::fs::create_dir_all(&cycle_root)?;
    let control = cycle_root.join("control.md");
    if !control.exists() {
        std::fs::write(&control, "# before-state\n")?;
    }
    let app = App::open(&cycle_root).context("abrir App para change cycle")?;
    let document_count_before = status_count(&app_result(app.workspace_status(Profile::Standard)));
    let preparation_ns = preparation_start.elapsed().as_nanos();
    let mut samples = Vec::with_capacity(iterations);
    let mut receipts = Vec::with_capacity(iterations);
    for index in 1..=iterations {
        let operations = json!([{ "op": "replace_body", "path": "control.md", "body": format!("# before-state\n\nafter-state-{index}\n") }]);
        let start = Instant::now();
        let plan = app
            .change_plan(
                None,
                &operations,
                PlanPolicy {
                    require_valid_result: false,
                    allow_warnings: true,
                },
            )
            .context("change_plan")?;
        let applied = app
            .change_apply(&plan.change_set_id, None)
            .context("change_apply")?;
        let elapsed = start.elapsed();
        let receipt_in_cycle = receipt_relative_path(&applied.receipt_id.0);
        let receipt_on_disk = cycle_root.join(&receipt_in_cycle);
        if !receipt_on_disk.is_file() {
            return Err(anyhow!(
                "change_apply no produjo el recibo esperado: {}",
                receipt_on_disk.display()
            ));
        }
        let receipt = app
            .workspace()
            .load_receipt(&applied.receipt_id)
            .map_err(|error| anyhow!("cargar receipt real {}: {error}", applied.receipt_id.0))?;
        if receipt.id != applied.receipt_id {
            return Err(anyhow!("receipt id no coincide con ApplyResult"));
        }
        samples.push((
            elapsed,
            serde_json::to_value(&applied).context("serializar ApplyResult")?,
        ));
        receipts.push(json!({
            "apply": applied,
            "receipt": receipt,
            "receipt_path": receipt_in_cycle.display().to_string(),
            "changed_paths": ["control.md"]
        }));
    }
    let document_count_after = status_count(&app_result(app.workspace_status(Profile::Standard)));
    Ok(json!({
        "variant": "app/disk",
        "source":"app/disk",
        "provenance": {"source":"app", "acquisition":"disk", "ownership": if exact_test_root.is_some() {"test-owned-exact-root"} else {"private-copy"}},
        "private_copy": exact_test_root.is_none(),
        "document_count_before": document_count_before,
        "document_count_after": document_count_after,
        "preparation_ns": preparation_ns,
        "iterations": iterations,
        "plan_count": iterations,
        "apply_count": iterations,
        "receipts": receipts,
        "changed_paths":["control.md"],
        "metric":metric(&samples)
    }))
}

fn acquisition_probe(root: &Path) -> Result<()> {
    let app = App::open(root).context("abrir App acquisition probe")?;
    let disk_before = app.workspace().document_set().context("disk before")?;
    let store = Store::open_and_build(root).context("store before")?;
    let sqlite_before = store.document_set();
    let ram_before = app.workspace().document_set().context("ram before")?;
    let count = |set: &lodestar_core::DocumentSet| {
        status_count(&app_result(
            app.read_services(set).workspace_status(Profile::Standard),
        ))
    };
    let before = json!({"disk-reparseo":{"document_count":count(&disk_before)},"sqlite-raw":{"document_count":count(&sqlite_before)},"ram-memoizado":{"document_count":count(&ram_before)}});
    println!("{}", json!({"event":"READY"}));
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .context("leer continue")?;
    if line.trim() != "continue" {
        return Err(anyhow!("se esperaba continue"));
    }
    let rebuild_start = Instant::now();
    store.rebuild().context("rebuild después de mutación")?;
    let rebuild_elapsed = rebuild_start.elapsed();
    let disk_after = app.workspace().document_set().context("disk after")?;
    let sqlite_after = store.document_set();
    let after = json!({"disk-reparseo":{"document_count":count(&disk_after)},"sqlite-raw":{"document_count":count(&sqlite_after)},"ram-memoizado":{"document_count":count(&ram_before)}});
    let rebuild = vec![(rebuild_elapsed, json!(true))];
    println!(
        "{}",
        json!({"before":before,"after":after,"rebuild":metric(&rebuild),"samples":{"disk-reparseo":{},"sqlite-raw":{},"ram-memoizado":{}}})
    );
    Ok(())
}

fn change_probe(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let path = root.join("control.md");
    if !path.exists() {
        std::fs::write(&path, "# before-state\n")?;
    }
    let app = App::open(root).context("abrir App change probe")?;
    let operations = json!([{ "op": "replace_body", "path": "control.md", "body": "# before-state\n\nafter-state\n" }]);
    let plan = app
        .change_plan(None, &operations, PlanPolicy::default())
        .context("change_plan")?;
    let apply = app
        .change_apply(&plan.change_set_id, None)
        .context("change_apply")?;
    let receipt_path = receipt_relative_path(&apply.receipt_id.0);
    let receipt_on_disk = root.join(&receipt_path);
    if !receipt_on_disk.is_file() {
        return Err(anyhow!(
            "change_apply no produjo el recibo esperado: {}",
            receipt_on_disk.display()
        ));
    }
    let receipt = app
        .workspace()
        .load_receipt(&apply.receipt_id)
        .map_err(|error| anyhow!("cargar receipt real {}: {error}", apply.receipt_id.0))?;
    if receipt.id != apply.receipt_id {
        return Err(anyhow!("receipt id no coincide con ApplyResult"));
    }
    println!(
        "{}",
        serde_json::to_string(
            &json!({"source":"app/disk","plan":plan,"apply":apply,"receipt":receipt,"changed_paths":["control.md"],"receipt_path":receipt_path.display().to_string()})
        )?
    );
    Ok(())
}

fn git_commit() -> Option<String> {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|commit| commit.trim().to_string())
}

fn git_working_tree_clean() -> bool {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(repo_root)
        .output()
        .map(|output| output.status.success() && output.stdout.is_empty())
        .unwrap_or(false)
}

fn binary_label() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "lodestar-bench".into())
}

fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn machine_label() -> String {
    format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn captured_at() -> String {
    Command::new("date")
        .arg("+%Y-%m-%dT%H:%M:%S%z")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unavailable".into())
}

fn receipt_relative_path(receipt_id: &str) -> PathBuf {
    let stem: String = receipt_id
        .chars()
        .map(|character| match character {
            ':' | '/' | '\\' => '_',
            other => other,
        })
        .collect();
    PathBuf::from(".lodestar")
        .join("runtime")
        .join("receipts")
        .join(format!("{stem}.json"))
}
