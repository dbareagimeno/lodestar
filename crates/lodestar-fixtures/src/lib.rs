//! Bundles de ejemplo reusables por los tests (E0-H03).
//!
//! Expone `FileMap`s deterministas que disparan cada `CheckCode`, además de un generador
//! sintético parametrizable (para los benches del `§11`).

use lodestar_core::types::{FileMap, RelPath};

/// Construye un `FileMap` a partir de pares `(path, contenido)`.
pub fn file_map(pairs: &[(&str, &str)]) -> FileMap {
    pairs
        .iter()
        .map(|(p, c)| {
            (
                RelPath::new(p).expect("fixture con path válido"),
                (*c).to_string(),
            )
        })
        .collect()
}

/// Bundle conforme mínimo: un index raíz + un concept válido que se enlazan mutuamente.
pub fn conformant() -> FileMap {
    file_map(&[
        ("index.md", "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n# Concept\n\n* [Alfa](alfa.md)\n"),
        (
            "alfa.md",
            "---\ntype: Concept\ntitle: Alfa\ndescription: Primer concept\n---\n\n# Resumen\n\nEnlaza a [Beta](/beta.md).\n",
        ),
        (
            "beta.md",
            "---\ntype: Concept\ntitle: Beta\ndescription: Segundo concept\n---\n\n# Resumen\n\nVuelve a [Alfa](/alfa.md).\n",
        ),
    ])
}

/// Bundle con un concept que dispara cada familia de checks (FM, type, recomendaciones, links, etc.).
pub fn with_issues() -> FileMap {
    file_map(&[
        // Sin frontmatter → OKF-FM01.
        ("sin-fm.md", "# Solo cuerpo\n"),
        // Frontmatter sin cerrar → OKF-FM02.
        ("sin-cierre.md", "---\ntype: Concept\n"),
        // type ausente → OKF-TYPE; sin title/desc → REC-*; sin encabezado → BODY-STRUCT; huérfano → ORPHAN.
        ("sin-tipo.md", "---\ntitle: \n---\n\ncuerpo sin encabezado\n"),
        // tags no-lista → FMT-TAGS; timestamp no-ISO → FMT-TS; enlace a inexistente → LINK-STUB; relativo → LINK-REL.
        (
            "malo.md",
            "---\ntype: Nota\ntitle: Malo\ndescription: x\ntags: uno\ntimestamp: ayer\n---\n\n# H\n\n[falta](/no-existe.md) y [rel](./otro.md)\n",
        ),
        // Marcadores de conflicto → OKF-CONFLICT.
        (
            "conflicto.md",
            "---\ntype: Nota\ntitle: C\ndescription: d\n---\n\n# H\n\n<<<<<<< HEAD\nuno\n=======\ndos\n>>>>>>> rama\n",
        ),
    ])
}

/// Genera un bundle sintético de `n` concepts deterministas (semilla fija → bytes idénticos).
pub fn synthetic(n: usize) -> FileMap {
    let mut pairs: Vec<(RelPath, String)> = Vec::with_capacity(n + 1);
    pairs.push((
        RelPath::new("index.md").unwrap(),
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n".to_string(),
    ));
    for i in 0..n {
        let next = (i + 1) % n;
        let raw = format!(
            "---\ntype: Concept\ntitle: Concept {i}\ndescription: sintetico {i}\n---\n\n# Resumen\n\nEnlaza a [siguiente](/c{next:06}.md).\n"
        );
        pairs.push((RelPath::new(&format!("c{i:06}.md")).unwrap(), raw));
    }
    pairs.into_iter().collect()
}
