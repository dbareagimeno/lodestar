//! E35-H03 CI40 — el auditor CI38 debe reconocer sintaxis Rust equivalente, no solo grafías.

#[path = "e35_h03_ci37_ci23_crlf_red.rs"]
mod ci38;

const CI19_REPAIR2: &str = include_str!("e35_h03_ci19_repair2_red.rs");
const CI19_REPAIR3: &str = include_str!("e35_h03_ci19_repair3_red.rs");
const CI23: &str = include_str!("e35_h03_ci23_red.rs");
const CI23_RELATIVE_ROOT: &str = include_str!("e35_h03_ci23_relative_root_red.rs");
const CI23_REVIEW: &str = include_str!("e35_h03_ci23_review_repair_red.rs");
const CI26: &str = include_str!("e35_h03_ci26_placeholder_red.rs");
const CI27: &str = include_str!("e35_h03_ci27_identity_red.rs");
const CI33: &str = include_str!("e35_h03_ci33_windows_rename_red.rs");
const FINAL_RSS: &str = include_str!("e35_h03_final_review_rss_red.rs");
const REPAIR9: &str = include_str!("e35_h03_repair9_red.rs");

fn real_suite() -> [(&'static str, &'static str, bool); 10] {
    [
        ("e35_h03_ci19_repair2_red.rs", CI19_REPAIR2, true),
        ("e35_h03_ci19_repair3_red.rs", CI19_REPAIR3, false),
        ("e35_h03_ci23_red.rs", CI23, false),
        (
            "e35_h03_ci23_relative_root_red.rs",
            CI23_RELATIVE_ROOT,
            false,
        ),
        ("e35_h03_ci23_review_repair_red.rs", CI23_REVIEW, true),
        ("e35_h03_ci26_placeholder_red.rs", CI26, true),
        ("e35_h03_ci27_identity_red.rs", CI27, true),
        ("e35_h03_ci33_windows_rename_red.rs", CI33, false),
        ("e35_h03_final_review_rss_red.rs", FINAL_RSS, false),
        ("e35_h03_repair9_red.rs", REPAIR9, false),
    ]
}

fn assert_real_partition(suite: &[(&str, &str, bool)]) {
    assert_eq!(
        suite.len(),
        10,
        "anti-vacuidad: la suite real tiene diez consumidores"
    );
    assert_eq!(
        suite.iter().filter(|(_, _, sensitive)| *sensitive).count(),
        4,
        "anti-vacuidad: la suite real conserva cuatro consumidores sensibles"
    );
    assert_eq!(
        suite.iter().filter(|(_, _, sensitive)| !*sensitive).count(),
        6,
        "anti-vacuidad: la suite real conserva seis consumidores tolerantes"
    );
    ci38::audit_suite_fixture(suite)
        .expect("anti-vacuidad: la suite real 4+6 debe superar el auditor CI38");
}

#[test]
fn ci40_rechaza_segundo_include_productivo_escrito_como_raw_string() {
    let suite = real_suite();
    assert_real_partition(&suite);

    const RAW_INCLUDE: &str =
        "const SECOND_STORE_ALIAS: &str = include_str!(r\"../src/schema.rs\");";
    const USE: &str =
        "fn ci40_uses_second_store_alias() -> bool { SECOND_STORE_ALIAS.contains(\"schema_version\") }";
    let ci26_with_raw_include = format!("{CI26}\n{RAW_INCLUDE}\n{USE}\n");
    assert_eq!(
        ci26_with_raw_include.matches(RAW_INCLUDE).count(),
        1,
        "anti-vacuidad: el contrafactual añade exactamente el include raw requerido"
    );
    assert_eq!(
        ci26_with_raw_include
            .match_indices("SECOND_STORE_ALIAS")
            .count(),
        2,
        "anti-vacuidad: el binding nuevo se declara y además se consume"
    );
    assert_eq!(
        ci26_with_raw_include.matches("include_str!(").count(),
        CI26.matches("include_str!(").count() + 1,
        "anti-vacuidad: CI26 adquiere exactamente un segundo include productivo"
    );

    let mut bypass = suite;
    bypass[5].1 = &ci26_with_raw_include;
    let error = ci38::audit_suite_fixture(&bypass)
        .expect_err("CI38 debe rechazar el include raw por inventario de include y binding");
    assert!(
        error.contains("e35_h03_ci26_placeholder_red.rs")
            && (error.contains("SECOND_STORE_ALIAS")
                || error.contains("include")
                || error.contains("inventario")),
        "el rechazo del include raw debe ser causal y específico: {error}"
    );
}

#[test]
fn ci40_rechaza_anchor_interno_tolerante_extraido_a_constante() {
    let suite = real_suite();
    assert_real_partition(&suite);

    const INLINE_START: &str = "\"pub(crate) fn validate_database(\",";
    const NAMED_START: &str = "VALIDATION_START,";
    const DECLARATION: &str =
        "const VALIDATION_START: &str = \"pub(crate) fn validate_database(\\n\";";
    let store_references_before = CI19_REPAIR3.match_indices("STORE_SOURCE").count();
    let replaced = CI19_REPAIR3.replacen(INLINE_START, NAMED_START, 1);
    assert_ne!(
        replaced, CI19_REPAIR3,
        "anti-vacuidad: debe sustituirse exactamente el anchor inline de validate_database"
    );
    let ci19_with_named_anchor = format!("{DECLARATION}\n{replaced}");
    assert_eq!(
        ci19_with_named_anchor.matches(DECLARATION).count(),
        1,
        "anti-vacuidad: el anchor con LF interno se declara una sola vez"
    );
    assert_eq!(
        ci19_with_named_anchor.matches(NAMED_START).count(),
        1,
        "anti-vacuidad: section consume el identificador del anchor una sola vez"
    );
    assert_eq!(
        ci19_with_named_anchor
            .match_indices("STORE_SOURCE")
            .count(),
        store_references_before,
        "anti-vacuidad: el conteo raw de STORE_SOURCE no cambia; cualquier rechazo debe deberse al anchor extraído"
    );
    assert_eq!(
        store_references_before, 3,
        "anti-vacuidad: CI19-repair3 conserva su inventario real de STORE_SOURCE"
    );

    let mut bypass = suite;
    bypass[1].1 = &ci19_with_named_anchor;
    let error = ci38::audit_suite_fixture(&bypass).expect_err(
        "CI38 debe rechazar un consumidor tolerante que adquiere un anchor interno mediante identificador",
    );
    assert!(
        error.contains("e35_h03_ci19_repair3_red.rs")
            && error.contains("tolerante")
            && error.contains("anchor"),
        "el rechazo del anchor nombrado debe ser causal y específico: {error}"
    );
}
