//! E35-H03 CI39 — CI38 debe auditar cada include y la partición CRLF completa.

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

#[test]
fn ci39_ci38_rechaza_include_alias_extra_y_tolerante_convertido_en_sensible() {
    let suite = [
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
    ];
    assert_eq!(
        suite.iter().filter(|(_, _, sensitive)| *sensitive).count(),
        4,
        "anti-vacuidad: la partición base debe contener cuatro consumidores sensibles"
    );
    assert_eq!(
        suite.iter().filter(|(_, _, sensitive)| !*sensitive).count(),
        6,
        "anti-vacuidad: la partición base debe contener seis consumidores tolerantes"
    );
    ci38::audit_suite_fixture(&suite)
        .expect("anti-vacuidad: la suite real 4+6 debe satisfacer el auditor de CI38");

    let ci26_with_uninventoried_include = format!(
        "{CI26}\nconst SECOND_STORE_ALIAS: &str = include_str!(\"../src/schema.rs\");\n\
         fn bypass_inventory() -> bool {{ SECOND_STORE_ALIAS.contains(\"schema_version\") }}\n"
    );
    assert_eq!(
        ci26_with_uninventoried_include
            .matches("include_str!(\"../src/")
            .count(),
        CI26.matches("include_str!(\"../src/").count() + 1,
        "anti-vacuidad: el primer contrafactual debe añadir exactamente un include productivo con binding distinto"
    );
    let mut include_bypass = suite;
    include_bypass[5].1 = &ci26_with_uninventoried_include;
    let include_error = ci38::audit_suite_fixture(&include_bypass).expect_err(
        "CI38 debe rechazar cada include productivo adicional que escape sin normalizar",
    );
    assert!(
        include_error.contains("SECOND_STORE_ALIAS")
            || include_error.contains("include")
            || include_error.contains("inventario"),
        "el contrafactual del include falló por otra causa: {include_error}"
    );

    let ci19_repair3_now_sensitive = format!(
        "{CI19_REPAIR3}\nfn bypass_partition() {{\n\
             let _ = STORE_SOURCE.find(\"fn rebuild_iter<I>(\\n    where\");\n\
         }}\n"
    );
    assert!(
        ci19_repair3_now_sensitive.contains("rebuild_iter<I>(\\n    where"),
        "anti-vacuidad: el segundo contrafactual debe introducir un anchor con salto interno"
    );
    let mut partition_bypass = suite;
    partition_bypass[1].1 = &ci19_repair3_now_sensitive;
    let partition_error = ci38::audit_suite_fixture(&partition_bypass).expect_err(
        "CI38 debe rechazar un consumidor declarado tolerante que adquiere un anchor interno",
    );
    assert!(
        partition_error.contains("e35_h03_ci19_repair3_red.rs")
            && (partition_error.contains("tolerante")
                || partition_error.contains("sensible")
                || partition_error.contains("anchor")),
        "el contrafactual de partición falló por otra causa: {partition_error}"
    );
}
