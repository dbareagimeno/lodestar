//! E35-H03 C4/C7 — seam puro para reutilizar el parseo ya calculado.

use lodestar_core::model;
use lodestar_core::types::{CheckCode, FileMap, RelPath};
use lodestar_core::DocumentSet;

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("RelPath valido")
}

/// C4/C7 — **Dado** un `Parsed` ya calculado por la proyeccion streaming, **cuando** se derivan
/// los diagnosticos que dependen solo de ese documento, **entonces** el core acepta ese parseo sin
/// obligar al store a construir otro `DocumentSet`; el resultado coincide con la autoridad y no
/// incluye diagnosticos de enlace.
#[test]
fn c4_c7_local_diagnostics_reutiliza_parsed_y_conserva_paridad_core() {
    let path = rp("docs/local.md");
    let raw = "\u{feff}# Local\n\n<<<<<<< ours\n[missing](missing.md)\n=======\n>>>>>>> theirs\n";
    let parsed = model::parse_file(path.as_str(), raw);

    let actual = lodestar_core::local_diagnostics(&path, &parsed, raw);

    let mut files = FileMap::new();
    files.insert(path.clone(), raw.to_string());
    let authoritative = DocumentSet::from_files(files);
    let all = &authoritative.analyze().diagnostics[&path];
    assert!(
        all.iter().any(|check| check.code == CheckCode::LinkTargetMissing),
        "guard anti-vacuidad: DocumentSet tambien contiene un diagnostico de enlace que debe filtrarse"
    );
    let expected: Vec<_> = all
        .iter()
        .filter(|check| {
            !matches!(
                check.code,
                CheckCode::LinkTargetMissing
                    | CheckCode::LinkEscapesWorkspace
                    | CheckCode::LinkCaseMismatch
            )
        })
        .cloned()
        .collect();
    assert!(
        expected.len() >= 2,
        "guard anti-vacuidad: BOM y marcador de conflicto ejercitan diagnosticos locales reales"
    );
    assert_eq!(actual, expected, "C7: el seam puro conserva paridad exacta");
}
