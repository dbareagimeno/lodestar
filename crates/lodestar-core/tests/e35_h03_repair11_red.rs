//! E35-H03 C4/C7 — promoción incremental determinista del inventario compacto.

use lodestar_core::types::{Inventory, RelPath};

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("RelPath válida")
}

/// C7 — un candidato validado como documento prevalece en el índice plegado sobre un
/// `other_file` equivalente por capitalización y normalización Unicode, aunque ese fichero fuese
/// el representante determinista antes de la promoción.
#[test]
fn c7_promoted_document_precede_case_nfc_equivalent_other_file() {
    let previous_other = rp("docs/CAFÉ.md");
    let promoted = rp("docs/cafe\u{301}.md");
    let lookup = rp("DOCS/CAFÉ.MD");
    let mut inventory = Inventory::new(
        std::iter::empty(),
        [previous_other.clone(), promoted.clone()],
    );

    assert_eq!(
        inventory.find_ignoring_case(&lookup),
        Some(&previous_other),
        "guarda anti-vacuidad: el other_file debe ser el representante plegado previo"
    );

    inventory.promote_document(promoted.clone());

    assert_eq!(
        inventory.find_ignoring_case(&lookup),
        Some(&promoted),
        "C7: el documento promovido debe prevalecer sobre un other_file plegado equivalente"
    );
}

/// C4/C7 — si dos documentos colisionan en la clave case/NFC-folded, el representante final es
/// siempre el menor `RelPath`, igual que en `Inventory::new`, y no depende del orden streaming.
#[test]
fn c4_c7_folded_document_winner_is_lexicographic_for_both_promotion_orders() {
    let lexicographically_first = rp("docs/CAFÉ.md");
    let lexicographically_last = rp("docs/cafe\u{301}.md");
    let lookup = rp("docs/Café.md");
    assert!(
        lexicographically_first < lexicographically_last,
        "guarda anti-vacuidad: la fixture debe distinguir las dos ramas de orden"
    );

    for order in [
        [
            lexicographically_last.clone(),
            lexicographically_first.clone(),
        ],
        [
            lexicographically_first.clone(),
            lexicographically_last.clone(),
        ],
    ] {
        let mut streamed = Inventory::new(std::iter::empty(), order.clone());
        for candidate in order {
            streamed.promote_document(candidate);
        }

        let canonical = Inventory::new(
            [
                lexicographically_first.clone(),
                lexicographically_last.clone(),
            ],
            std::iter::empty(),
        );
        assert_eq!(
            streamed.find_ignoring_case(&lookup),
            canonical.find_ignoring_case(&lookup),
            "C4/C7: la promoción incremental debe conservar la construcción canónica"
        );
        assert_eq!(
            streamed.find_ignoring_case(&lookup),
            Some(&lexicographically_first),
            "C7: entre documentos plegados equivalentes gana el menor RelPath"
        );
    }
}

/// C7 — la promoción mueve la identidad exacta entre las dos clases sin contaminar el fichero
/// plegado-equivalente que permanece como asset.
#[test]
fn c7_promotion_updates_exact_document_and_file_membership() {
    let remaining_file = rp("docs/CAFÉ.md");
    let promoted = rp("docs/cafe\u{301}.md");
    let mut inventory = Inventory::new(
        std::iter::empty(),
        [remaining_file.clone(), promoted.clone()],
    );

    assert!(inventory.contains_file(&promoted));
    assert!(!inventory.contains_document(&promoted));

    inventory.promote_document(promoted.clone());

    assert!(
        inventory.contains_document(&promoted),
        "C7: la ruta exacta validada debe pertenecer a documentos"
    );
    assert!(
        !inventory.contains_file(&promoted),
        "C7: una ruta promovida no puede conservar clasificación duplicada"
    );
    assert!(
        inventory.contains_file(&remaining_file),
        "guarda anti-vacuidad: la promoción no debe reclasificar la ruta equivalente"
    );
    assert!(!inventory.contains_document(&remaining_file));
}
