//! Guards for A6 base-ref discovery.
//!
//! These tests use throw-away Git repositories so the contract check cannot accidentally pass
//! because the developer checkout happens to contain a local `develop` branch.  They invoke the
//! canonical guard without `--manifest` and inject the synthetic checkout through its explicit
//! test-only root seam; they are red until that seam is honored by the implementation.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

// Seam interna de la guarda canónica, consumida solo por estos tests herméticos.  La
// implementación debe reconocerla como una sustitución test-only de la raíz A6; los tests no
// pasan `--manifest`, porque ese modo valida deliberadamente un fixture arbitrario.
const A6_REPO_ROOT_ENV: &str = "LODESTAR_BENCH_TEST_A6_REPO_ROOT";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn bench() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"));
    command.env("RUST_BACKTRACE", "1");
    command
}

fn contract_bytes() -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root");
    let bytes = fs::read(root.join("contracts/mcp.yml")).expect("current MCP contract");
    assert!(
        !bytes.is_empty(),
        "anti-vacuidad: el contrato no puede estar vacío"
    );
    bytes
}

fn git(repo: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap_or_else(|error| panic!("ejecutar git {:?}: {error}", args));
    assert!(
        output.status.success(),
        "git {:?}: stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(git(repo, args).stdout)
        .expect("salida git UTF-8")
        .trim()
        .to_owned()
}

fn init_repo(base_contract: &[u8], base_ref: Option<&str>) -> TempDir {
    let repo = TempDir::new().expect("repositorio Git temporal");
    fs::create_dir(repo.path().join("src")).expect("src fixture");
    fs::create_dir(repo.path().join("contracts")).expect("contracts fixture");
    fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"synthetic-a6\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .expect("manifest fixture");
    fs::write(repo.path().join("src/main.rs"), "fn main() {}\n").expect("main fixture");
    fs::write(repo.path().join("contracts/mcp.yml"), base_contract).expect("contract fixture");

    git(repo.path(), &["init"]);
    git(repo.path(), &["config", "user.name", "A6 test"]);
    git(
        repo.path(),
        &["config", "user.email", "a6-test@example.invalid"],
    );
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-m", "base contract"]);
    let base_commit = git_stdout(repo.path(), &["rev-parse", "HEAD"]);
    git(repo.path(), &["switch", "-c", "feature"]);

    match base_ref {
        Some("develop") => {
            // Local checkout shape: the feature branch has a sibling local `develop` ref.
            git(repo.path(), &["branch", "develop"]);
        }
        Some("origin/develop") => {
            // Pull-request checkout shape: no local `develop`, only its remote-tracking ref.
            // Make HEAD deliberately divergent, then restore the working tree to the remote
            // contract.  A guard that falls back to HEAD therefore rejects the valid fixture.
            let mut head_contract = base_contract.to_vec();
            head_contract.extend_from_slice(b"\n# divergent feature HEAD; never use as A6 base\n");
            fs::write(repo.path().join("contracts/mcp.yml"), &head_contract)
                .expect("contrato divergente de HEAD");
            git(repo.path(), &["add", "contracts/mcp.yml"]);
            git(repo.path(), &["commit", "-m", "divergent feature head"]);
            let head_contract_from_git = Command::new("git")
                .args(["show", "HEAD:contracts/mcp.yml"])
                .current_dir(repo.path())
                .output()
                .expect("leer contrato de HEAD");
            assert_eq!(head_contract_from_git.stdout, head_contract);
            assert_ne!(
                head_contract, base_contract,
                "HEAD debe diferir de origin/develop"
            );

            // The checkout's files are the origin/develop bytes even though HEAD is divergent.
            fs::write(repo.path().join("contracts/mcp.yml"), base_contract)
                .expect("restaurar working tree al contrato remoto");
            git(
                repo.path(),
                &["update-ref", "refs/remotes/origin/develop", &base_commit],
            );
            let local = Command::new("git")
                .args(["show-ref", "--verify", "--quiet", "refs/heads/develop"])
                .current_dir(repo.path())
                .status()
                .expect("comprobar ausencia de develop");
            assert!(
                !local.success(),
                "anti-vacuidad: no debe existir refs/heads/develop"
            );
            let remote_contract = Command::new("git")
                .args(["show", "origin/develop:contracts/mcp.yml"])
                .current_dir(repo.path())
                .output()
                .expect("leer contrato remoto");
            assert!(
                remote_contract.status.success(),
                "origin/develop debe existir"
            );
            assert_eq!(remote_contract.stdout, base_contract);
            assert_eq!(
                fs::read(repo.path().join("contracts/mcp.yml")).expect("leer working tree"),
                base_contract,
                "working tree debe coincidir con origin/develop"
            );
            assert_ne!(
                head_contract_from_git.stdout, remote_contract.stdout,
                "anti-vacuidad: HEAD y origin/develop deben ser contratos distintos"
            );
        }
        None => {
            // No base ref at all; the guard must reject this state with actionable context.
            for reference in ["refs/heads/develop", "refs/remotes/origin/develop"] {
                let status = Command::new("git")
                    .args(["show-ref", "--verify", "--quiet", reference])
                    .current_dir(repo.path())
                    .status()
                    .expect("comprobar ausencia de base ref");
                assert!(
                    !status.success(),
                    "anti-vacuidad: existe la referencia {reference}"
                );
            }
        }
        Some(other) => panic!("base ref fixture desconocida: {other}"),
    }
    repo
}

fn run_guard(repo: &Path, github_base_ref: Option<&str>) -> Output {
    let mut command = bench();
    command
        .args(["--check-a6-dependencies"])
        .current_dir(repo)
        .env(A6_REPO_ROOT_ENV, repo)
        .env_remove("GITHUB_BASE_REF");
    if let Some(base_ref) = github_base_ref {
        command.env("GITHUB_BASE_REF", base_ref);
    }
    command
        .output()
        .expect("ejecutar guarda A6 en repo temporal")
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_rejected(output: &Output, context: &str) {
    assert!(
        !output.status.success(),
        "{context}: la guarda debe rechazar; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Criterio: con una referencia local `develop`, A6 usa ese commit y compara todos los bytes.
/// El segundo intento muta una sola línea para impedir una guarda que solo compruebe existencia.
#[test]
fn a6_local_develop_compara_contrato_byte_a_byte_y_rechaza_mutacion() {
    let base = contract_bytes();
    let repo = init_repo(&base, Some("develop"));

    assert_success(&run_guard(repo.path(), None), "A6 local develop válido");

    let mut mutated = base.clone();
    mutated.extend_from_slice(b"\n# mutation that must be rejected\n");
    assert_ne!(
        mutated, base,
        "anti-vacuidad: la mutación debe cambiar bytes"
    );
    fs::write(repo.path().join("contracts/mcp.yml"), mutated).expect("mutar contrato fixture");
    assert_rejected(&run_guard(repo.path(), None), "A6 local develop mutado");
}

/// Criterio: `GITHUB_BASE_REF=develop` puede resolver el fallback local cuando el checkout
/// conserva `develop` y todavía no tiene `origin/develop`; una mutación posterior debe morder.
#[test]
fn a6_github_base_ref_con_solo_develop_local_usa_fallback_y_rechaza_mutacion() {
    let base = contract_bytes();
    let repo = init_repo(&base, Some("develop"));

    let local = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", "refs/heads/develop"])
        .current_dir(repo.path())
        .status()
        .expect("comprobar develop local");
    assert!(local.success(), "anti-vacuidad: debe existir develop local");
    let remote = Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/develop",
        ])
        .current_dir(repo.path())
        .status()
        .expect("comprobar origin/develop");
    assert!(
        !remote.success(),
        "anti-vacuidad: la fixture no debe tener origin/develop"
    );
    assert_eq!(
        fs::read(repo.path().join("contracts/mcp.yml")).expect("contrato actual"),
        base,
        "anti-vacuidad: el checkout coincide con el único develop local"
    );

    assert_success(
        &run_guard(repo.path(), Some("develop")),
        "A6 PR con fallback develop local válido",
    );

    let mut mutated = base.clone();
    mutated.extend_from_slice(b"\n# local fallback mutation that must be rejected\n");
    assert_ne!(
        mutated, base,
        "anti-vacuidad: la mutación debe cambiar bytes"
    );
    fs::write(repo.path().join("contracts/mcp.yml"), mutated).expect("mutar contrato fixture");
    assert_rejected(
        &run_guard(repo.path(), Some("develop")),
        "A6 PR con fallback develop local mutado",
    );
}

/// Criterio: en un pull_request, `GITHUB_BASE_REF=develop` se resuelve en `origin/develop` aun
/// cuando la referencia local `develop` no existe.
#[test]
fn a6_github_base_ref_usa_origin_develop_sin_rama_local_y_rechaza_mutacion() {
    let base = contract_bytes();
    let repo = init_repo(&base, Some("origin/develop"));

    assert_success(
        &run_guard(repo.path(), Some("develop")),
        "A6 origin/develop válido",
    );

    let mut mutated = base.clone();
    mutated.extend_from_slice(b"\n# CI-only mutation that must be rejected\n");
    assert_ne!(
        mutated, base,
        "anti-vacuidad: la mutación debe cambiar bytes"
    );
    fs::write(repo.path().join("contracts/mcp.yml"), mutated).expect("mutar contrato fixture");
    assert_rejected(
        &run_guard(repo.path(), Some("develop")),
        "A6 origin/develop mutado",
    );
}

/// Criterio de portabilidad: en jobs push/detached no existe `GITHUB_BASE_REF`, pero la política
/// del repositorio sigue nombrando `develop`; si solo está disponible `origin/develop`, A6 debe
/// descubrir y comparar esa referencia remota.  La fixture separa HEAD para impedir un fallback.
#[test]
fn a6_sin_github_base_ref_descubre_origin_develop_y_rechaza_mutacion() {
    let base = contract_bytes();
    let repo = init_repo(&base, Some("origin/develop"));

    assert_success(
        &run_guard(repo.path(), None),
        "A6 origin/develop válido sin GITHUB_BASE_REF",
    );

    let mut mutated = base.clone();
    mutated.extend_from_slice(b"\n# push-job mutation that must be rejected\n");
    assert_ne!(
        mutated, base,
        "anti-vacuidad: la mutación debe cambiar bytes"
    );
    fs::write(repo.path().join("contracts/mcp.yml"), mutated).expect("mutar contrato fixture");
    assert_rejected(
        &run_guard(repo.path(), None),
        "A6 origin/develop mutado sin GITHUB_BASE_REF",
    );
}

/// Criterio negativo: sin `develop` ni `origin/develop`, A6 no puede degradarse a una comprobación
/// contra el checkout actual; debe fallar y mencionar cómo falta la referencia base.
#[test]
fn a6_sin_referencia_base_falla_con_diagnostico_util() {
    let base = contract_bytes();
    let repo = init_repo(&base, None);

    let output = run_guard(repo.path(), Some("develop"));
    assert_rejected(&output, "A6 sin referencia base");
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    assert!(
        diagnostic.contains("develop")
            && (diagnostic.contains("base") || diagnostic.contains("ref")),
        "diagnóstico A6 debe identificar la referencia base ausente; salida={diagnostic}"
    );
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CheckoutConfig {
    actions_checkout_v4: bool,
    fetch_depth_zero: bool,
}

fn workflow_checkout_configs(workflow: &str) -> BTreeMap<String, Vec<CheckoutConfig>> {
    let mut jobs = BTreeMap::<String, Vec<CheckoutConfig>>::new();
    let mut in_jobs = false;
    let mut current_job: Option<String> = None;
    let mut current_checkout: Option<(usize, CheckoutConfig)> = None;
    let mut with_indent = None;

    for raw_line in workflow.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();

        if indent == 0 && trimmed == "jobs:" {
            in_jobs = true;
            current_job = None;
            current_checkout = None;
            with_indent = None;
            continue;
        }
        if !in_jobs {
            continue;
        }

        if indent == 2 && !trimmed.starts_with('-') && trimmed.ends_with(':') {
            current_job = Some(trimmed.trim_end_matches(':').to_owned());
            jobs.entry(current_job.clone().expect("job just set"))
                .or_default();
            current_checkout = None;
            with_indent = None;
            continue;
        }

        let Some(job) = current_job.as_ref() else {
            continue;
        };

        if indent == 6 && trimmed.starts_with("- uses:") {
            let action = trimmed["- uses:".len()..]
                .split('#')
                .next()
                .unwrap_or_default()
                .trim();
            let config = CheckoutConfig {
                actions_checkout_v4: action == "actions/checkout@v4",
                fetch_depth_zero: false,
            };
            jobs.get_mut(job).expect("job was registered").push(config);
            let index = jobs.get(job).expect("job was registered").len() - 1;
            current_checkout = Some((index, CheckoutConfig::default()));
            // The entry was inserted above; keep the index while collecting its `with:` map.
            with_indent = None;
            continue;
        }

        let Some((checkout_index, _)) = current_checkout.as_ref() else {
            continue;
        };
        if indent <= 6 {
            current_checkout = None;
            with_indent = None;
            continue;
        }
        if with_indent.is_some_and(|parent| indent <= parent) {
            // A sibling such as `env:` closes the checkout's `with:` map; values nested below
            // that sibling must never be mistaken for checkout inputs.
            with_indent = None;
        }
        if trimmed == "with:" {
            with_indent = Some(indent);
            continue;
        }
        if with_indent.is_some_and(|parent| indent > parent) && trimmed.starts_with("fetch-depth:")
        {
            let value = trimmed["fetch-depth:".len()..]
                .split('#')
                .next()
                .unwrap_or_default()
                .trim();
            if value == "0" {
                jobs.get_mut(job).expect("job was registered")[*checkout_index].fetch_depth_zero =
                    true;
            }
        }
    }

    // The map above records the action itself before parsing its `with:` block.  Keep only the
    // corresponding action identity from that insertion and avoid treating a sibling step as
    // checkout configuration.
    for configs in jobs.values_mut() {
        for config in configs.iter_mut() {
            if !config.actions_checkout_v4 {
                config.fetch_depth_zero = false;
            }
        }
    }
    jobs
}

/// Criterio negativo del parser CI: `fetch-depth: 0` bajo `env:` no pertenece al checkout,
/// aunque `env:` sea hermano de un `with:` previo. Solo el `with:` del propio checkout puede
/// satisfacer la exigencia de historia completa.
#[test]
fn a6_parser_acepta_fetch_depth_solo_en_with_del_checkout_propio() {
    let valid = r#"
jobs:
  rust:
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
"#;
    let valid_jobs = workflow_checkout_configs(valid);
    let valid_checkouts = valid_jobs
        .get("rust")
        .expect("anti-vacuidad: debe existir rust en workflow válido");
    assert_eq!(valid_checkouts.len(), 1, "debe haber un checkout válido");
    assert!(
        valid_checkouts[0].actions_checkout_v4,
        "el paso válido debe ser actions/checkout@v4"
    );
    assert!(
        valid_checkouts[0].fetch_depth_zero,
        "fetch-depth: 0 bajo with: debe aceptarse"
    );

    let misplaced = r#"
jobs:
  rust:
    steps:
      - uses: actions/checkout@v4
        with:
          persist-credentials: false
        env:
          fetch-depth: 0
"#;
    let misplaced_jobs = workflow_checkout_configs(misplaced);
    let misplaced_checkouts = misplaced_jobs
        .get("rust")
        .expect("anti-vacuidad: debe existir rust en workflow inválido");
    assert_eq!(
        misplaced_checkouts.len(),
        1,
        "el paso env debe seguir siendo el mismo checkout, no crear otro"
    );
    assert!(
        misplaced_checkouts[0].actions_checkout_v4,
        "el paso inválido debe seguir identificándose como checkout"
    );
    assert!(
        !misplaced_checkouts[0].fetch_depth_zero,
        "fetch-depth: 0 bajo env: hermano de with: no debe satisfacer el criterio"
    );
}

/// Criterio de portabilidad CI: los checkouts de `rust` y `core-purity` deben traer la historia
/// completa, y el `fetch-depth: 0` debe pertenecer a su propio paso checkout.
/// La comprobación por job impide que un `fetch-depth: 0` en otro job satisfaga accidentalmente
/// una aserción global; además exige que el workflow y ambos jobs existan (anti-vacuidad). Otros
/// jobs pueden configurar historia completa independientemente.
#[test]
fn a6_ci_rust_y_core_purity_configuran_checkout_con_historia_completa() {
    let root = workspace_root();
    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("workflow CI");
    assert!(
        !workflow.trim().is_empty(),
        "anti-vacuidad: ci.yml no puede estar vacío"
    );

    let jobs = workflow_checkout_configs(&workflow);
    let expected_jobs = ["rust", "core-purity"];
    for expected_job in expected_jobs {
        let checkouts = jobs
            .get(expected_job)
            .unwrap_or_else(|| panic!("anti-vacuidad: falta el job CI `{expected_job}`"));
        let valid = checkouts
            .iter()
            .filter(|config| config.actions_checkout_v4 && config.fetch_depth_zero)
            .count();
        assert_eq!(
            valid, 1,
            "el job `{expected_job}` debe tener exactamente un checkout@v4 con fetch-depth: 0; configuración={checkouts:?}"
        );
    }
}
