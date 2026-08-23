//! Rojo E33-H03: los centinelas §22/§24 deben ser parte real del banco y citar sus fichas.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tempfile::tempdir;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate bajo crates/")
        .to_path_buf()
}

fn batch(name: &str) -> Value {
    let path = root().join("docs/qa/testbench/batches").join(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("falta {}: {error}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} inválido: {error}", path.display()))
}

fn cases(lot: &Value) -> &[Value] {
    lot["cases"].as_array().expect("cases array")
}

fn genera_corpus() -> tempfile::TempDir {
    let generated = tempdir().expect("directorio temporal");
    let status = Command::new("python3")
        .arg(root().join("docs/qa/testbench/make_corpus.py"))
        .arg(generated.path())
        .status()
        .expect("ejecutar make_corpus.py");
    assert!(status.success(), "make_corpus.py terminó con {status}");
    generated
}

fn entradas_bytes(dir: &Path) -> Vec<Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("leer {}: {error}", dir.display()))
        .map(|entry| {
            entry
                .expect("entrada del corpus")
                .file_name()
                .to_string_lossy()
                .as_bytes()
                .to_vec()
        })
        .collect()
}

#[test]
fn h03_fija_los_cuatro_casos_sin_aplicar_el_plan_destructivo() {
    let s22 = batch("sentinela_s22.json");
    let s24 = batch("sentinela_s24.json");
    let ids = |value: &Value| {
        value["cases"]
            .as_array()
            .expect("cases array")
            .iter()
            .map(|case| case["id"].as_str().expect("id").to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&s22), ["S22-01", "S22-02"]);
    assert_eq!(ids(&s24), ["S24-01", "S24-02"]);
    for (lot, decision) in [(&s22, "§22"), (&s24, "§24")] {
        for case in lot["cases"].as_array().expect("cases") {
            assert_eq!(case["gate"], true);
            assert!(case["descripcion"]
                .as_str()
                .is_some_and(|description| description.contains(decision)));
        }
    }
    for case in s24["cases"].as_array().expect("cases") {
        let tools = case["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .filter_map(|step| step["tool"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(tools, ["change_plan"]);
        assert!(!tools.contains(&"change_apply"));
    }
}

#[test]
fn h03_s24_nfd_prueba_nfd_frente_a_nfc_y_no_a_nfd_existente() {
    let s24 = batch("sentinela_s24.json");
    let nfc = "revisión.md";
    let nfd = "revisio\u{301}n.md";
    let nfd_path = s24["cases"][1]["steps"][0]["arguments"]["operations"][0]["path"]
        .as_str()
        .expect("S24-02 create path");
    assert!(
        nfd_path.ends_with(nfd),
        "S24-02 debe crear la forma NFD, no un nombre parecido: {nfd_path:?}"
    );
    assert_ne!(
        nfd.as_bytes(),
        nfc.as_bytes(),
        "NFC y NFD deben diferir en bytes"
    );

    // Anti-vacuidad: inspecciona el árbol que realmente genera make_corpus.py. H03 usa una
    // semilla distinta de la pareja histórica `canción` de H01: antes del verde, esta NFC
    // todavía falta y el rojo no se puede satisfacer con un comentario. En ext4 no puede
    // haber ya un gemelo NFD; en APFS/NTFS ambos nombres pueden verse como la misma entrada.
    let generated = genera_corpus();
    let unicode_dir = generated.path().join("centinelas/unicode");
    let matching_names = entradas_bytes(&unicode_dir)
        .into_iter()
        .filter(|name| name == nfc.as_bytes() || name == nfd.as_bytes())
        .collect::<Vec<_>>();
    assert_eq!(
        matching_names.len(),
        1,
        "la semilla revisión debe dejar solo NFC existente antes de planificar NFD; entradas byte a byte: {matching_names:?}"
    );
    std::fs::read_to_string(unicode_dir.join(nfc))
        .expect("la nueva semilla NFC revisión debe existir");
}

#[test]
fn h03_s24_caja_muerde_si_el_gemelo_preexistente_desaparece_o_el_target_es_libre() {
    let generated = genera_corpus();
    let caja_dir = generated.path().join("centinelas/caja");
    let target = b"INFORME.md".to_vec();
    let lower = b"informe.md".to_vec();
    let upper = b"Informe.md".to_vec();
    let entries = entradas_bytes(&caja_dir);

    assert!(
        entries.iter().all(|entry| entry == &lower || entry == &upper),
        "el corpus debe contener solo las variantes declaradas por caja, no un path libre: {entries:?}"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(&target)),
        "S24-01 necesita un gemelo preexistente por caja: {entries:?}"
    );
    assert!(
        !entries.contains(&target),
        "el target exacto INFORME.md no debe existir byte a byte antes del plan: {entries:?}"
    );
    assert!(
        (entries.len() == 1 && entries[0] == lower) || (entries.len() == 2 && entries.contains(&lower) && entries.contains(&upper)),
        "la semántica portable APFS/ext4 exige una entrada normalizada o dos bytes distintos: {entries:?}"
    );
}

#[test]
fn h01_arbol_generado_preserva_cancion_nfc_nfd_sin_sufijo_artificial() {
    let generated = genera_corpus();
    let unicode_dir = generated.path().join("centinelas/unicode");
    let nfc = "canción.md";
    let nfd = "cancio\u{301}n.md";
    let nfc_bytes = nfc.as_bytes().to_vec();
    let nfd_bytes = nfd.as_bytes().to_vec();
    let entries = entradas_bytes(&unicode_dir);
    let pair = entries
        .iter()
        .filter(|entry| **entry == nfc_bytes || **entry == nfd_bytes)
        .cloned()
        .collect::<Vec<_>>();

    assert_ne!(nfc_bytes, nfd_bytes, "NFC y NFD deben diferir en bytes");
    assert_eq!(
        pair.len(),
        if pair.contains(&nfc_bytes) && pair.contains(&nfd_bytes) {
            2
        } else {
            1
        },
        "solo las formas NFC/NFD exactas pueden contar como el par: {pair:?}"
    );
    assert!(
        pair.len() == 2 || (pair.len() == 1 && std::fs::metadata(unicode_dir.join(nfc)).is_ok() && std::fs::metadata(unicode_dir.join(nfd)).is_ok()),
        "ext4 debe exponer ambas entradas byte-distintas; APFS normalizante puede exponer una, pero ambas formas deben resolver: {pair:?}"
    );
    assert!(
        !entries
            .iter()
            .any(|entry| entry.as_slice() == "canción-nfd.md".as_bytes()),
        "el gemelo NFD no puede disfrazarse con un sufijo ASCII: {entries:?}"
    );
    if pair.len() == 2 {
        assert_ne!(
            std::fs::read(unicode_dir.join(nfc)).expect("contenido NFC"),
            std::fs::read(unicode_dir.join(nfd)).expect("contenido NFD"),
            "en ext4 las dos entradas deben conservar el contenido de cada escritura"
        );
    }
}

#[test]
fn h03_s24_describe_la_dependencia_apfs_ntfs_frente_a_ext4() {
    let s24 = batch("sentinela_s24.json");
    for case in cases(&s24) {
        let description = case["descripcion"].as_str().expect("descripción de S24");
        for platform in ["APFS", "NTFS", "ext4"] {
            assert!(
                description.contains(platform),
                "{} debe citar explícitamente {platform} (APFS/NTFS vs ext4): {description}",
                case["id"]
            );
        }
    }
}

#[test]
fn h03_esta_cableada_en_run_all_y_anotada_sin_cerrar_decisiones() {
    let harness = std::fs::read_to_string(root().join("docs/qa/testbench/lodestar_harness.py"))
        .expect("harness");
    for lot in ["batches/sentinela_s22.json", "batches/sentinela_s24.json"] {
        assert!(
            harness.contains(&format!("\"{lot}\"")),
            "{lot} no está en LOTES_DEL_GATE"
        );
    }
    for (file, case_ids) in [
        (
            "decisiones/22-integridad-referencial-frontmatter.md",
            ["S22-01", "S22-02"],
        ),
        (
            "decisiones/24-equivalencia-caja-unicode.md",
            ["S24-01", "S24-02"],
        ),
    ] {
        let text = std::fs::read_to_string(root().join(file)).expect("ficha");
        assert!(
            text.contains("estado: \"abierta\""),
            "H03 no puede cerrar {file}"
        );
        let reviewed = text
            .lines()
            .find_map(|line| line.strip_prefix("revisada_en: "))
            .expect("ficha debe declarar revisada_en");
        assert!(
            reviewed.starts_with("\"2026-08-") && reviewed.ends_with("\""),
            "{file} debe estar revisada en 2026-08-XX: {reviewed}"
        );
        for case_id in case_ids {
            assert!(
                text.contains(case_id),
                "{file} no anota su centinela {case_id}"
            );
        }
    }
    let corpus = std::fs::read_to_string(root().join("docs/qa/testbench/make_corpus.py"))
        .expect("generador");
    assert!(
        corpus.contains("affects:"),
        "falta la segunda referencia rota de §22"
    );
    assert!(
        corpus.contains("  - 99"),
        "relacionadas debe contener el valor huérfano 99"
    );
}
