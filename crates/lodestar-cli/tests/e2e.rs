//! Tests **end-to-end** de la CLI: viajes completos de usuario cruzando fachadas y procesos
//! reales (binario `lodestar`). Complementan `cli.rs` (que testea contratos puntuales): aquí se
//! encadena el flujo entero. Desde E9-H02 la CLI no expone git (crate `vcs` dormido), así que ya
//! no se necesita invocar el binario `git` real desde estos tests.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lodestar"))
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

// E9-H02 retira `hooks`/`push`/`pull`/`switch`/`merge`/`branch`/`log`/`last-conforming` y
// `check --staged`/`--rev`/`--range` de la superficie de la CLI (el crate `vcs` queda dormido,
// principio rector: retirar exposición, no capacidad). Se retiran los e2e que ejercitaban esa
// superficie porque prueban funcionalidad que ya no existe en la CLI, no el contrato de esta
// historia:
//   - `hooks_bloquean_commit_no_conforme_via_git_real` (usaba `hooks` + `check --staged`)
//   - `push_y_pull_con_remoto_local` (usaba `push`/`pull`/`log`/`last-conforming`)
//   - `flujo_de_ramas_switch_y_merge` (usaba `switch`/`merge`/`branch`)
//   - `check_range_juzga_la_punta` (usaba `check --range`/`--rev`)
// El contrato nuevo (`check` solo juzga el working tree, `--rev` es error de uso) lo cubren
// `help_sin_subcomandos_git`, `check_rev_es_uso` y `check_working_tree_conforme` en `tests/cli.rs`.

/// Errores de uso → exit 2 (contrato congelado): import sin fuente. (Los casos de flags git en
/// conflicto —`--staged`/`--rev`/`--range`— se retiraron con esos flags; ver nota arriba.)
#[test]
fn errores_de_uso_exit_2() {
    let dir = temp_dir("uso");
    write(&dir, "index.md", "---\nokf_version: \"0.1\"\n---\n\n# B\n");
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
