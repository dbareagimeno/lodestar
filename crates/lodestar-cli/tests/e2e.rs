//! Tests **end-to-end** de la CLI: viajes completos de usuario cruzando fachadas y procesos
//! reales (binario `lodestar`, binario `git`, hooks, remoto local). Complementan `cli.rs`
//! (que testea contratos puntuales): aquí se encadena el flujo entero.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lodestar"))
}

fn bin_dir() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_lodestar"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// `git` real con identidad fija y PATH que antepone el binario `lodestar` (para los hooks).
fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    let path = format!(
        "{}:{}",
        bin_dir().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("PATH", path)
        .env("GIT_AUTHOR_NAME", "E2E")
        .env("GIT_AUTHOR_EMAIL", "e2e@test")
        .env("GIT_COMMITTER_NAME", "E2E")
        .env("GIT_COMMITTER_EMAIL", "e2e@test")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git disponible")
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lodestar-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn run(dir: &Path, args: &[&str]) -> i32 {
    bin()
        .arg("--path")
        .arg(dir)
        .args(args)
        .status()
        .unwrap()
        .code()
        .unwrap()
}

const CONCEPT_A: &str =
    "---\ntype: Nota\ntitle: Alfa\ndescription: la primera\ntags: [demo]\n---\n\n# H\n\n[beta](/beta.md)\n";
const CONCEPT_B: &str =
    "---\ntype: Nota\ntitle: Beta\ndescription: la segunda\ntags: [demo]\n---\n\n# H\n\ncuerpo\n";

/// Viaje completo: init → crear → check → romper → arreglar → generadores → export/import.
#[test]
fn viaje_completo_edicion_y_generadores() {
    let dir = temp_dir("viaje");
    let target = dir.join("bundle");

    // init crea el scaffold + git + commit inicial.
    assert_eq!(
        bin().arg("init").arg(&target).status().unwrap().code(),
        Some(0)
    );
    assert!(target.join("index.md").is_file());
    assert!(target.join(".git").is_dir());

    // Añadir concepts conformes → check 0.
    write(&target, "alfa.md", CONCEPT_A);
    write(&target, "beta.md", CONCEPT_B);
    assert_eq!(run(&target, &["check"]), 0);

    // Romper un fichero → check 1 (hard-fail); arreglar → 0.
    write(&target, "rota.md", "# sin frontmatter\n");
    assert_eq!(run(&target, &["check"]), 1);
    std::fs::remove_file(target.join("rota.md")).unwrap();
    assert_eq!(run(&target, &["check"]), 0);

    // Generadores: tags primero (crea `tags/`), luego index (lista el subdir nuevo);
    // después --check sin drift (exit 0).
    assert_eq!(run(&target, &["tags"]), 0);
    assert_eq!(run(&target, &["index"]), 0);
    assert_eq!(run(&target, &["index", "--check"]), 0);
    assert_eq!(run(&target, &["tags", "--check"]), 0);
    assert!(target.join("tags/demo/index.md").is_file());

    // Editar rompe el drift → exit 4; regenerar lo repara.
    write(
        &target,
        "gamma.md",
        CONCEPT_B.replace("Beta", "Gamma").as_str(),
    );
    assert_eq!(run(&target, &["index", "--check"]), 4);
    assert_eq!(run(&target, &["index"]), 0);
    assert_eq!(run(&target, &["index", "--check"]), 0);

    // Export → import en un destino nuevo → mismo veredicto de conformidad.
    let zip = dir.join("out.zip");
    assert_eq!(
        bin()
            .arg("--path")
            .arg(&target)
            .args(["export", "--out"])
            .arg(&zip)
            .status()
            .unwrap()
            .code(),
        Some(0)
    );
    let dest = dir.join("importado");
    std::fs::create_dir_all(&dest).unwrap();
    assert_eq!(
        bin()
            .arg("--path")
            .arg(&dest)
            .arg("import")
            .arg(&zip)
            .status()
            .unwrap()
            .code(),
        Some(0)
    );
    assert!(dest.join("alfa.md").is_file());
    assert!(dest.join("tags/demo/index.md").is_file());
    assert_eq!(run(&dest, &["check"]), 0);
}

/// Hook pre-commit real: `git commit` (binario) bloquea lo no conforme y deja pasar lo conforme.
/// El hook instala `lodestar check --staged` (§13.5): juzga el index, no el working tree.
#[test]
fn hooks_bloquean_commit_no_conforme_via_git_real() {
    let dir = temp_dir("hooks");
    assert_eq!(
        bin().arg("init").arg(&dir).status().unwrap().code(),
        Some(0)
    );
    assert_eq!(run(&dir, &["hooks"]), 0);

    // Stagea un fichero roto → el hook (lodestar check --staged) bloquea el commit.
    write(&dir, "rota.md", "# sin frontmatter\n");
    let add = git(&dir, &["add", "."]);
    assert!(add.status.success(), "{add:?}");
    let commit = git(&dir, &["commit", "-m", "intento roto"]);
    assert!(
        !commit.status.success(),
        "el hook debió bloquear: {}",
        String::from_utf8_lossy(&commit.stdout)
    );

    // Lo arregla, re-stagea → el commit pasa.
    write(&dir, "rota.md", CONCEPT_B);
    git(&dir, &["add", "."]);
    let commit2 = git(&dir, &["commit", "-m", "ahora conforme"]);
    assert!(
        commit2.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&commit2.stderr)
    );

    // Y el hook juzga lo STAGED: working sucio no conforme + staged conforme → commit pasa.
    write(&dir, "otra.md", CONCEPT_A.replace("Alfa", "Otra").as_str());
    git(&dir, &["add", "otra.md"]);
    write(&dir, "sucia-sin-stagear.md", "# rota en el working\n");
    let commit3 = git(&dir, &["commit", "-m", "staged conforme"]);
    assert!(
        commit3.status.success(),
        "el hook debe juzgar el index, no el working: {}",
        String::from_utf8_lossy(&commit3.stderr)
    );
}

/// Red por el binario git: push a un remoto local (bare) y pull desde un clon.
#[test]
fn push_y_pull_con_remoto_local() {
    let dir = temp_dir("red");
    let a = dir.join("a");
    assert_eq!(bin().arg("init").arg(&a).status().unwrap().code(), Some(0));

    // Remoto bare + upstream configurado (la gestión de remotos es no-goal: se hace por git).
    let remote = dir.join("remoto.git");
    let init_bare = git(&dir, &["init", "--bare", remote.to_str().unwrap()]);
    assert!(init_bare.status.success());
    git(&a, &["remote", "add", "origin", remote.to_str().unwrap()]);
    let branch = String::from_utf8(git(&a, &["branch", "--show-current"]).stdout)
        .unwrap()
        .trim()
        .to_string();
    let push0 = git(&a, &["push", "-u", "origin", &branch]);
    assert!(push0.status.success(), "{push0:?}");

    // Nuevo contenido → commit (git real) → `lodestar push` exit 0.
    write(&a, "alfa.md", CONCEPT_A);
    write(&a, "beta.md", CONCEPT_B);
    git(&a, &["add", "."]);
    assert!(git(&a, &["commit", "-m", "contenido"]).status.success());
    assert_eq!(run(&a, &["push"]), 0);

    // Clon B: `lodestar pull` trae los cambios de A.
    let b = dir.join("b");
    let clone = git(
        &dir,
        &["clone", remote.to_str().unwrap(), b.to_str().unwrap()],
    );
    assert!(clone.status.success());
    write(&a, "gamma.md", CONCEPT_B.replace("Beta", "Gamma").as_str());
    git(&a, &["add", "."]);
    assert!(git(&a, &["commit", "-m", "gamma"]).status.success());
    assert_eq!(run(&a, &["push"]), 0);
    assert_eq!(run(&b, &["pull"]), 0);
    assert!(b.join("gamma.md").is_file());
    assert_eq!(run(&b, &["check"]), 0);

    // Log y last-conforming funcionan sobre el clon.
    assert_eq!(run(&b, &["log"]), 0);
    assert_eq!(run(&b, &["last-conforming"]), 0);
}

/// Flujo de ramas por la CLI: switch --create, editar, merge de vuelta.
#[test]
fn flujo_de_ramas_switch_y_merge() {
    let dir = temp_dir("ramas");
    assert_eq!(
        bin().arg("init").arg(&dir).status().unwrap().code(),
        Some(0)
    );
    write(&dir, "alfa.md", CONCEPT_A);
    write(&dir, "beta.md", CONCEPT_B);
    git(&dir, &["add", "."]);
    assert!(git(&dir, &["commit", "-m", "base"]).status.success());
    let base = String::from_utf8(git(&dir, &["branch", "--show-current"]).stdout)
        .unwrap()
        .trim()
        .to_string();

    // Rama nueva con un concept extra.
    assert_eq!(run(&dir, &["switch", "feature", "--create"]), 0);
    write(
        &dir,
        "gamma.md",
        CONCEPT_B.replace("Beta", "Gamma").as_str(),
    );
    git(&dir, &["add", "."]);
    assert!(git(&dir, &["commit", "-m", "feature: gamma"])
        .status
        .success());

    // Vuelta a la base (gamma desaparece del working tree) y merge (gamma vuelve).
    assert_eq!(run(&dir, &["switch", &base]), 0);
    assert!(!dir.join("gamma.md").exists());
    assert_eq!(run(&dir, &["merge", "feature"]), 0);
    assert!(dir.join("gamma.md").is_file());
    assert_eq!(run(&dir, &["check"]), 0);
    assert_eq!(run(&dir, &["branch"]), 0);
}

/// `--range a..b` juzga la punta `b` (semántica congelada).
#[test]
fn check_range_juzga_la_punta() {
    let dir = temp_dir("range");
    assert_eq!(
        bin().arg("init").arg(&dir).status().unwrap().code(),
        Some(0)
    );
    // Commit 2: roto. Commit 3: arreglado.
    write(&dir, "x.md", "# sin frontmatter\n");
    git(&dir, &["add", "."]);
    assert!(git(&dir, &["commit", "-m", "roto"]).status.success());
    write(&dir, "x.md", CONCEPT_B);
    git(&dir, &["add", "."]);
    assert!(git(&dir, &["commit", "-m", "arreglado"]).status.success());

    // La punta (HEAD, arreglado) es lo que se juzga → 0 aunque el rango cruce el commit roto.
    assert_eq!(run(&dir, &["check", "--range", "HEAD~2..HEAD"]), 0);
    // Y juzgar directamente el commit roto → 1.
    assert_eq!(run(&dir, &["check", "--rev", "HEAD~1"]), 1);
}

/// Errores de uso → exit 2 (contrato congelado): flags en conflicto e import sin fuente.
#[test]
fn errores_de_uso_exit_2() {
    let dir = temp_dir("uso");
    write(&dir, "index.md", "---\nokf_version: \"0.1\"\n---\n\n# B\n");
    assert_eq!(run(&dir, &["check", "--staged", "--rev", "HEAD"]), 2);
    assert_eq!(run(&dir, &["check", "--rev", "HEAD", "--range", "a..b"]), 2);
    assert_eq!(run(&dir, &["import"]), 2);
}

/// `init` sin argumento inicializa el CWD, no un bundle ancestro.
#[test]
fn init_sin_arg_usa_cwd_no_el_ancestro() {
    let parent = temp_dir("init-anidado");
    write(
        &parent,
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# B\n",
    );
    let sub = parent.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let status = bin().arg("init").current_dir(&sub).status().unwrap();
    assert_eq!(status.code(), Some(0));
    assert!(
        sub.join("index.md").is_file(),
        "init debe crear el bundle en el cwd"
    );
    assert!(
        !parent.join(".git").exists(),
        "init no debe tocar el bundle ancestro"
    );
}

/// Un `lodestar.toml` inválido NO relaja la puerta en silencio: exit 3.
#[test]
fn config_invalida_es_error_de_runtime() {
    let dir = temp_dir("toml-roto");
    write(&dir, "index.md", "---\nokf_version: \"0.1\"\n---\n\n# B\n");
    write(&dir, "lodestar.toml", "[gate\nblock_warnings = true\n");
    assert_eq!(run(&dir, &["check"]), 3);
}

/// Un `.md` no-UTF8 no aborta el check: se salta con aviso y el resto se juzga.
#[test]
fn md_no_utf8_no_aborta_el_check() {
    let dir = temp_dir("no-utf8");
    write(&dir, "index.md", "---\nokf_version: \"0.1\"\n---\n\n# B\n");
    write(&dir, "buena.md", CONCEPT_B);
    std::fs::write(dir.join("latin1.md"), b"---\ntype: Nota\n---\n\n# a\xf1o\n").unwrap();
    assert_eq!(run(&dir, &["check"]), 0);
}
