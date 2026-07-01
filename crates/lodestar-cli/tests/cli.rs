//! Tests de integración de la CLI (E2): exit codes congelados y formatos de salida.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lodestar"))
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lodestar-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &std::path::Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

#[test]
fn check_conforme_exit_0() {
    let dir = temp_dir("conforme");
    write(
        &dir,
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    let status = bin().arg("--path").arg(&dir).arg("check").status().unwrap();
    assert_eq!(status.code(), Some(0));
}

#[test]
fn check_hard_fail_exit_1() {
    let dir = temp_dir("hardfail");
    write(&dir, "malo.md", "# sin frontmatter\n");
    let status = bin().arg("--path").arg(&dir).arg("check").status().unwrap();
    assert_eq!(status.code(), Some(1));
}

#[test]
fn check_json_es_valido() {
    let dir = temp_dir("json");
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n",
    );
    let out = bin()
        .arg("--path")
        .arg(&dir)
        .args(["check", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.get("concepts").is_some());
    assert!(v.get("hardFail").is_some(), "wire camelCase");
}

#[test]
fn check_sarif_es_valido() {
    let dir = temp_dir("sarif");
    write(&dir, "malo.md", "# sin frontmatter\n");
    let out = bin()
        .arg("--path")
        .arg(&dir)
        .args(["check", "--sarif"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], "2.1.0");
    assert!(v["runs"][0]["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["ruleId"] == "OKF-FM01"));
}

#[test]
fn index_drift_exit_4_luego_0() {
    let dir = temp_dir("drift");
    write(
        &dir,
        "a.md",
        "---\ntype: Concept\ntitle: A\ndescription: d\n---\n\n# H\n",
    );
    // Sin index.md generado → drift.
    let drift = bin()
        .arg("--path")
        .arg(&dir)
        .args(["index", "--check"])
        .status()
        .unwrap();
    assert_eq!(drift.code(), Some(4));
    // Genera y vuelve a comprobar → 0.
    let gen = bin().arg("--path").arg(&dir).arg("index").status().unwrap();
    assert_eq!(gen.code(), Some(0));
    let ok = bin()
        .arg("--path")
        .arg(&dir)
        .args(["index", "--check"])
        .status()
        .unwrap();
    assert_eq!(ok.code(), Some(0));
}

#[test]
fn export_genera_zip() {
    let dir = temp_dir("export");
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n",
    );
    let out = dir.join("salida.zip");
    let status = bin()
        .arg("--path")
        .arg(&dir)
        .args(["export", "--out"])
        .arg(&out)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0));
    assert!(out.is_file());
}

#[test]
fn init_scaffold() {
    let dir = temp_dir("init");
    let target = dir.join("nuevo");
    let status = bin().arg("init").arg(&target).status().unwrap();
    assert_eq!(status.code(), Some(0));
    assert!(target.join("index.md").is_file());
    assert!(target.join(".gitignore").is_file());
}

#[test]
fn check_staged_sin_git_exit_3() {
    // Sin repo git, `--staged` no tiene árbol staged → error de runtime (exit 3).
    let dir = temp_dir("staged-nogit");
    write(
        &dir,
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    let status = bin()
        .arg("--path")
        .arg(&dir)
        .args(["check", "--staged"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(3));
}

#[test]
fn check_rev_head_tras_init() {
    // `init` crea git + commit inicial; `check --rev HEAD` juzga ese árbol (index.md conforme → 0).
    let dir = temp_dir("checkrev");
    let target = dir.join("b");
    assert_eq!(
        bin().arg("init").arg(&target).status().unwrap().code(),
        Some(0)
    );
    let status = bin()
        .arg("--path")
        .arg(&target)
        .args(["check", "--rev", "HEAD"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0));
}

#[test]
fn import_desde_zip_del_prototipo() {
    // Exporta un bundle a .zip y lo reimporta en un directorio nuevo (roundtrip).
    let dir = temp_dir("import-src");
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n",
    );
    let zip = dir.join("bundle.zip");
    assert_eq!(
        bin()
            .arg("--path")
            .arg(&dir)
            .args(["export", "--out"])
            .arg(&zip)
            .status()
            .unwrap()
            .code(),
        Some(0)
    );
    let dest = temp_dir("import-dest");
    let status = bin()
        .arg("--path")
        .arg(&dest)
        .arg("import")
        .arg(&zip)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0));
    assert!(dest.join("a.md").is_file());
}

#[test]
fn import_rechaza_zip_slip() {
    // Un zip con una ruta con `..` no debe escribir fuera del bundle (chokepoint RelPath).
    let dir = temp_dir("zipslip");
    let zip_path = dir.join("evil.zip");
    {
        let f = std::fs::File::create(&zip_path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        use zip::write::SimpleFileOptions;
        zw.start_file("../evil.md", SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zw, b"---\ntype: X\n---\n\n# H\n").unwrap();
        zw.finish().unwrap();
    }
    let dest = temp_dir("zipslip-dest");
    let status = bin()
        .arg("--path")
        .arg(&dest)
        .arg("import")
        .arg(&zip_path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0)); // no falla, pero...
                                        // ...la ruta insegura se ignora: no se escribe fuera del destino.
    assert!(!dest.parent().unwrap().join("evil.md").exists());
}
