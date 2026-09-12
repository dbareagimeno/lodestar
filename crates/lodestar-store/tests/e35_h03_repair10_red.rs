//! E35-H03 C1/C4/C7 — clasificación UTF-8 diferida sin enlaces transitorios incorrectos.

use std::fs;
use std::path::Path;

use lodestar_core::types::RelPath;
use lodestar_store::Store;
use rusqlite::Connection;

type LinkRow = (String, String, Option<String>, Option<i64>, Option<String>);

fn write(root: &Path, path: &str, contents: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(target, contents).unwrap();
}

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("RelPath valido")
}

/// C1/C4/C7 — **Dado** un documento temprano que enlaza a dos candidatos posteriores, uno UTF-8
/// valido y otro no UTF-8, **cuando** `Store::rebuild` recorre la policy canonica, **entonces** el
/// valido termina enlazado por FK como `document` y el invalido permanece `workspaceFile`, fuera
/// de `documents` y FTS. El informe cuenta las tres lecturas de candidato pero solo dos
/// proyecciones FTS; un diagnostico local real verifica que la familia diagnostics no es vacua.
#[test]
fn c1_c4_c7_rebuild_clasifica_destinos_posteriores_tras_una_sola_lectura() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "00-source.md",
        "# Source\n\n<<<<<<< ours\n[valid](zz-valid.md) [invalid](yy-invalid.md)\n=======\n>>>>>>> theirs\nneedle-source-repair10\n",
    );
    write(
        root.path(),
        "zz-valid.md",
        "# Valid\n\nneedle-valid-repair10\n",
    );
    write(
        root.path(),
        "yy-invalid.md",
        [
            0xff, 0xfe, b'n', b'e', b'e', b'd', b'l', b'e', b'-', b'i', b'n', b'v', b'a', b'l',
            b'i', b'd', b'-', b'r', b'e', b'p', b'a', b'i', b'r', b'1', b'0', b'\n',
        ],
    );

    let store = Store::open(root.path()).unwrap();
    let report = store.rebuild().expect("rebuild canonico real");

    assert_eq!(
        store.documents().unwrap(),
        vec![rp("00-source.md"), rp("zz-valid.md")],
        "guard anti-vacuidad: solo los dos cuerpos UTF-8 son documentos"
    );
    assert_eq!(
        report["documents_read"].as_u64(),
        Some(3),
        "C4: se lee una vez cada candidato admitido, incluido el que resulta no UTF-8"
    );
    assert_eq!(
        report["fts_inserts"].as_u64(),
        Some(2),
        "C4: solo los dos documentos validos se proyectan en FTS"
    );
    assert_eq!(
        store.fts_candidates("needle-valid-repair10").unwrap(),
        vec![rp("zz-valid.md")],
        "guard anti-vacuidad: el documento posterior valido si entra en FTS"
    );
    assert!(
        store
            .fts_candidates("needle-invalid-repair10")
            .unwrap()
            .is_empty(),
        "C1/C7: los bytes del candidato no UTF-8 nunca entran en FTS"
    );

    let db = Connection::open(root.path().join(".lodestar/index.db")).unwrap();
    let links: Vec<LinkRow> = db
        .prepare(
            "SELECT l.raw_href,l.target_kind,l.target_path,l.target_doc_id,target.path \
             FROM links l \
             LEFT JOIN documents target ON target.doc_id=l.target_doc_id \
             JOIN documents source ON source.doc_id=l.source_doc_id \
             WHERE source.path='00-source.md' ORDER BY l.raw_href",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        links.len(),
        2,
        "guard anti-vacuidad: se proyectan ambos enlaces"
    );
    let invalid = links
        .iter()
        .find(|row| row.0 == "yy-invalid.md")
        .expect("enlace al candidato no UTF-8");
    assert_eq!(invalid.1, "workspaceFile");
    assert_eq!(invalid.2.as_deref(), Some("yy-invalid.md"));
    assert_eq!(
        invalid.3, None,
        "el no-documento no puede recibir target_doc_id"
    );
    assert_eq!(
        invalid.4, None,
        "el LEFT JOIN no debe encontrar documento invalido"
    );
    let valid = links
        .iter()
        .find(|row| row.0 == "zz-valid.md")
        .expect("enlace al documento UTF-8 posterior");
    assert_eq!(valid.1, "document");
    assert_eq!(
        valid.2, None,
        "un FK resuelto no conserva target_path paralelo"
    );
    assert!(
        valid.3.is_some(),
        "el documento posterior debe recibir target_doc_id"
    );
    assert_eq!(valid.4.as_deref(), Some("zz-valid.md"));

    let other_files: Vec<String> = db
        .prepare("SELECT path FROM other_files ORDER BY path")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(other_files, vec!["yy-invalid.md"]);
    let diagnostics: Vec<(String, String)> = db
        .prepare(
            "SELECT d.path,x.code FROM diagnostics x \
             JOIN documents d ON d.doc_id=x.doc_id ORDER BY d.path,x.code",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        diagnostics,
        vec![("00-source.md".into(), "DOC-CONFLICT-MARKER".into())],
        "C7: la proyeccion de diagnostics locales usa el catalogo canonico y no es vacua"
    );
    assert_eq!(store.validation_counts().unwrap(), (1, 0));
}
