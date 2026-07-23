//! Tests **end-to-end** de la CLI: viajes completos de usuario cruzando fachadas y procesos
//! reales (binario `lodestar`). Complementan `cli.rs` (que testea contratos puntuales).
//!
//! E15-H02/H03 dejaron la CLI en `check` + `reindex`: los viajes que encadenaban
//! `init`/generadores/`export`/`import` se retiraron con esos subcomandos, y lo que queda aquí son
//! los e2e de la puerta de CI que siguen vivos.

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

const CONCEPT_B: &str =
    "---\ntype: Nota\ntitle: Beta\ndescription: la segunda\ntags: [demo]\n---\n\n# H\n\ncuerpo\n";

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
