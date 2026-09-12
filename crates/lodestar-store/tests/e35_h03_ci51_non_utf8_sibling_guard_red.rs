//! E35-H03 CI51 — la guarda del probe temprano no puede omitir basenames no UTF-8.
//!
//! Este test portable revisa únicamente el consumidor de evidencia CI49. La guarda debe mantener
//! `OsStr`/`OsString` o unidades nativas hasta decidir prefijo y allowlist; convertir a `String` y
//! descartar el error hace que un sibling Windows con UTF-16 no representable desaparezca del
//! oráculo.

const BENCH_PROBE_SOURCE: &str =
    include_str!("../../lodestar-bench/tests/e35_h03_ci47_bench_acquisition_red.rs");

fn sibling_guard(source: &str) -> Result<&str, String> {
    let start_marker =
        "fn assert_committed_snapshot(root: &Path, expected: &BTreeSet<String>, phase: &str)";
    let start = source
        .find(start_marker)
        .ok_or_else(|| "CI51: falta assert_committed_snapshot".to_owned())?;
    let tail = &source[start..];
    let end = tail
        .find("\nfn run_acquisition_case(")
        .ok_or_else(|| "CI51: falta el límite posterior del guard".to_owned())?;
    Ok(&tail[..end])
}

fn non_lossy_sibling_contract(source: &str) -> Result<(), String> {
    let guard = sibling_guard(source)?;
    if !guard.contains("file_name()") {
        return Err("CI51 anti-vacuidad: la guarda no inspecciona basenames reales".into());
    }
    for lossy in [".into_string()", ".to_str()", ".to_string_lossy()"] {
        if guard.contains(lossy) {
            return Err(format!(
                "CI51: `{lossy}` convierte/omite un OsStr no representable (UTF-8 inválido o UTF-16 no Unicode) antes de juzgar el sibling"
            ));
        }
    }

    let native_prefix = (guard.contains(".as_encoded_bytes()") && guard.contains(".starts_with("))
        || (guard.contains(".encode_wide()") && guard.contains("starts_with"));
    if !native_prefix {
        return Err(
            "CI51: el prefijo index.db debe compararse sobre OsStr/bytes codificados/UTF-16 sin pérdida"
                .into(),
        );
    }
    for allowed in ["index.db", "index.db-wal", "index.db-shm"] {
        if !guard.contains(&format!("\"{allowed}\"")) {
            return Err(format!(
                "CI51: falta el único basename permitido `{allowed}`"
            ));
        }
    }
    if !guard.contains("forbidden.is_empty()") {
        return Err(
            "CI51 anti-vacuidad: los siblings distintos del allowlist deben bloquear el probe"
                .into(),
        );
    }
    Ok(())
}

#[test]
fn ci51_guard_de_siblings_no_descarta_nombres_no_utf8() {
    let lossy_control = r#"
fn assert_committed_snapshot(root: &Path, expected: &BTreeSet<String>, phase: &str) {
    let allowed = ["index.db", "index.db-wal", "index.db-shm"];
    let forbidden: Vec<_> = entries
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("index.db") && !allowed.contains(name))
        .collect();
    assert!(forbidden.is_empty());
}
fn run_acquisition_case() {}
"#;
    let lossy_error = non_lossy_sibling_contract(lossy_control)
        .expect_err("CI51 control negativo: descartar un OsStr debe ser rechazado");
    assert!(
        lossy_error.contains(".into_string()"),
        "CI51: el control negativo debe atribuirse al descarte UTF-8; error={lossy_error}"
    );

    let non_lossy_control = r#"
fn assert_committed_snapshot(root: &Path, expected: &BTreeSet<String>, phase: &str) {
    let forbidden: Vec<_> = entries.filter_map(|entry| {
        let name = entry.file_name();
        let encoded = name.as_encoded_bytes();
        let allowed = ["index.db", "index.db-wal", "index.db-shm"];
        (encoded.starts_with(b"index.db")
            && !allowed.iter().any(|item| name == std::ffi::OsStr::new(item)))
            .then_some(name)
    }).collect();
    assert!(forbidden.is_empty());
}
fn run_acquisition_case() {}
"#;
    non_lossy_sibling_contract(non_lossy_control)
        .expect("CI51 control: una comparación OsStr/encoded sin pérdida debe ser admisible");

    non_lossy_sibling_contract(BENCH_PROBE_SOURCE)
        .unwrap_or_else(|error| panic!("rojo causal CI51: {error}"));
}
