//! E35-H03 CI36 — los oráculos estructurales deben ser independientes del checkout CRLF.
//!
//! Este test no replica ninguna guarda C5/C6. Solo autentica la frontera textual que alimenta a
//! `section`: los mismos anchors multilínea funcionan con LF, el algoritmo actual falla con CRLF,
//! y CI19 debe normalizar sus fuentes incluidas antes de recortarlas.

const CI19_SOURCE: &str = include_str!("e35_h03_ci19_repair2_red.rs");

const RENAME_START: &str = "fn rename_handle_to(\n    target: &Path,\n    handle: HANDLE,\n    extended_flags: Option<u32>,\n)";
const RENAME_END: &str = "\nfn wide_path(path: &Path)";
const REPLACE_START: &str =
    "pub(crate) fn replace_durable(candidate: PreparedCandidate, active: &Path)";
const REPLACE_END: &str = "\n}\n\n/// Rust 1.80 implements `remove_file`";

fn current_section<'a>(source: &'a str, start: &str, end: &str) -> Result<&'a str, &'static str> {
    let start_at = source.find(start).ok_or("inicio")?;
    let remainder = &source[start_at..];
    let end_at = remainder.find(end).ok_or("final")?;
    Ok(&remainder[..end_at])
}

fn crlf_normalization_contract(ci19: &str) -> Result<(), String> {
    if !ci19.contains("replace(\"\\r\\n\", \"\\n\")") {
        return Err(
            "CI36: falta normalizar CRLF a LF antes de usar anchors multilínea en section".into(),
        );
    }
    for raw_source in [
        "section(\n        STORE_SOURCE,",
        "section(\n        WINDOWS_VFS_SOURCE,",
    ] {
        if ci19.contains(raw_source) {
            return Err(format!(
                "CI36: section todavía recibe include_str sin normalizar: `{raw_source}`"
            ));
        }
    }
    Ok(())
}

#[test]
fn ci36_ci19_normaliza_crlf_antes_de_buscar_anchors_multilinea() {
    let lf = format!("prefijo\n{RENAME_START} {{\n    cuerpo();\n}}{RENAME_END} {{\n}}\n");
    let located = current_section(&lf, RENAME_START, RENAME_END)
        .expect("control LF: los anchors reales deben localizar la sección");
    assert!(
        located.starts_with(RENAME_START) && located.contains("cuerpo();"),
        "guarda anti-vacuidad: el control LF debe recortar el mismo tipo de sección que CI19"
    );

    let crlf = lf.replace('\n', "\r\n");
    assert!(crlf.contains("\r\n"), "la variante debe contener CRLF real");
    assert_eq!(
        crlf.matches("\r\n").count(),
        lf.matches('\n').count(),
        "cada newline LF del control debe convertirse, no añadirse decorativamente"
    );
    assert_eq!(
        current_section(&crlf, RENAME_START, RENAME_END),
        Err("inicio"),
        "el algoritmo actual con anchors LF debe demostrar el fallo de inicio sobre CRLF"
    );

    let end_control = format!(
        "prefijo\n{REPLACE_START} {{\n    rename();\n}}\n\n/// Rust 1.80 implements `remove_file`\n"
    );
    current_section(&end_control, REPLACE_START, REPLACE_END)
        .expect("control LF: el anchor final multilínea debe localizarse");
    let end_crlf = end_control.replace('\n', "\r\n");
    assert!(end_crlf.contains("\r\n"));
    assert_eq!(
        current_section(&end_crlf, REPLACE_START, REPLACE_END),
        Err("final"),
        "con inicio monolínea estable y final multilínea CRLF, el algoritmo actual falla por final"
    );

    crlf_normalization_contract(CI19_SOURCE)
        .unwrap_or_else(|error| panic!("rojo causal CI36: {error}"));
}
