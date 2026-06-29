//! Tests del núcleo (E1): contrato de tipos, modelo, conformidad, analyze, query, generadores, diff.

use std::collections::BTreeMap;

use lodestar_core::diff::{self, ChangeKind, MessageHint};
use lodestar_core::generate;
use lodestar_core::model;
use lodestar_core::types::*;
use lodestar_core::Bundle;

fn fm(pairs: &[(&str, &str)]) -> FileMap {
    pairs
        .iter()
        .map(|(p, c)| (RelPath::new(p).unwrap(), (*c).to_string()))
        .collect()
}

// --- E1-H01: RelPath ---------------------------------------------------------

#[test]
fn relpath_rechaza_absolutas_y_dotdot() {
    assert!(RelPath::new("/etc/passwd").is_err());
    assert!(RelPath::new("../x").is_err());
    assert!(RelPath::new("a/../../b").is_err());
    assert!(RelPath::new("").is_err());
}

#[test]
fn relpath_normaliza() {
    assert_eq!(RelPath::new("a//b/./c.md").unwrap().as_str(), "a/b/c.md");
    assert_eq!(RelPath::new("a\\b.md").unwrap().as_str(), "a/b.md");
}

#[test]
fn relpath_deserializa_validando() {
    let bad = serde_json::from_str::<RelPath>("\"../x\"");
    assert!(bad.is_err());
    let ok: RelPath = serde_json::from_str("\"a/b.md\"").unwrap();
    assert_eq!(ok.as_str(), "a/b.md");
}

// --- E1-H03: Severity / CheckCode -------------------------------------------

#[test]
fn severity_max_es_err() {
    let v = [Severity::Err, Severity::Pass];
    assert_eq!(*v.iter().max().unwrap(), Severity::Err);
}

#[test]
fn severity_wire_minusculas() {
    assert_eq!(serde_json::to_string(&Severity::Warn).unwrap(), "\"warn\"");
}

#[test]
fn checkcode_wire_con_guion() {
    assert_eq!(
        serde_json::to_string(&CheckCode::OkfFm01).unwrap(),
        "\"OKF-FM01\""
    );
    assert_eq!(CheckCode::OkfConflict.as_str(), "OKF-CONFLICT");
}

// --- E1-H05: modelo ---------------------------------------------------------

#[test]
fn build_raw_idempotente() {
    let raw = "---\ntype: Concept\ntitle: Alfa\n---\n\n# H\n\ncuerpo\n";
    let parsed = model::parse_file("alfa.md", raw);
    let rebuilt = model::build_raw(parsed.fm.as_ref().unwrap(), &parsed.body);
    let reparsed = model::parse_file("alfa.md", &rebuilt);
    let rebuilt2 = model::build_raw(reparsed.fm.as_ref().unwrap(), &reparsed.body);
    assert_eq!(rebuilt, rebuilt2, "build_raw debe ser idempotente");
}

#[test]
fn resolve_link_casos() {
    assert_eq!(
        model::resolve_link("/a/b.md", "x.md").as_deref(),
        Some("a/b.md")
    );
    assert_eq!(
        model::resolve_link("./b.md", "dir/x.md").as_deref(),
        Some("dir/b.md")
    );
    assert_eq!(model::resolve_link("http://x", "x.md"), None);
    assert_eq!(model::resolve_link("#frag", "x.md"), None);
    assert_eq!(
        model::resolve_link("sub/", "x.md").as_deref(),
        Some("sub/index.md")
    );
}

// --- E1-H06/H07: conformidad y analyze --------------------------------------

fn codes_of(b: &Bundle, path: &str) -> Vec<String> {
    let p = RelPath::new(path).unwrap();
    b.analyze().per_file[&p]
        .iter()
        .map(|c| c.code.as_str().to_string())
        .collect()
}

#[test]
fn conformidad_dispara_cada_codigo() {
    let b = Bundle::from_files(fm(&[
        ("sin-fm.md", "# Solo cuerpo\n"),
        ("sin-cierre.md", "---\ntype: Concept\n"),
        ("malo-yaml.md", "---\ntype: : :\n  - x\n: bad\n---\n\n# H\n"),
        ("sin-tipo.md", "---\ntitle: \n---\n\ncuerpo\n"),
        (
            "malo.md",
            "---\ntype: Nota\ntitle: Malo\ndescription: x\ntags: uno\ntimestamp: ayer\n---\n\n# H\n\n[falta](/no.md) y [r](./o.md)\n",
        ),
        ("conflicto.md", "---\ntype: N\ntitle: C\ndescription: d\n---\n\n# H\n\n<<<<<<< HEAD\na\n=======\nb\n>>>>>>> r\n"),
    ]));
    assert!(codes_of(&b, "sin-fm.md").contains(&"OKF-FM01".to_string()));
    assert!(codes_of(&b, "sin-cierre.md").contains(&"OKF-FM02".to_string()));
    assert!(codes_of(&b, "malo-yaml.md").contains(&"OKF-FM03".to_string()));
    assert!(codes_of(&b, "sin-tipo.md").contains(&"OKF-TYPE".to_string()));
    assert!(codes_of(&b, "sin-tipo.md").contains(&"REC-TITLE".to_string()));
    assert!(codes_of(&b, "sin-tipo.md").contains(&"BODY-STRUCT".to_string()));
    assert!(codes_of(&b, "sin-tipo.md").contains(&"ORPHAN".to_string()));
    let malo = codes_of(&b, "malo.md");
    assert!(malo.contains(&"FMT-TAGS".to_string()));
    assert!(malo.contains(&"FMT-TS".to_string()));
    assert!(malo.contains(&"LINK-STUB".to_string()));
    assert!(malo.contains(&"LINK-REL".to_string()));
    assert!(codes_of(&b, "conflicto.md").contains(&"OKF-CONFLICT".to_string()));
}

#[test]
fn hard_fail_cuenta_ficheros_no_max() {
    // 1 fichero con Err + 1 conforme → hard_fail == 1 (no se "tapa" con un Pass).
    let b = Bundle::from_files(fm(&[
        ("malo.md", "# sin frontmatter\n"),
        (
            "bueno.md",
            "---\ntype: Nota\ntitle: B\ndescription: d\n---\n\n# H\n\n[x](/malo.md)\n",
        ),
    ]));
    assert_eq!(b.analyze().hard_fail, 1);
}

#[test]
fn analyze_backlinks_son_inversa_de_out() {
    let b = Bundle::from_files(fm(&[
        (
            "a.md",
            "---\ntype: N\ntitle: A\ndescription: d\n---\n\n# H\n\n[b](/b.md)\n",
        ),
        (
            "b.md",
            "---\ntype: N\ntitle: B\ndescription: d\n---\n\n# H\n\ncuerpo\n",
        ),
    ]));
    let a = b.analyze();
    let pa = RelPath::new("a.md").unwrap();
    let pb = RelPath::new("b.md").unwrap();
    assert_eq!(a.out[&pa], vec![pb.clone()]);
    assert_eq!(a.inn[&pb], vec![pa.clone()]);
    assert!(a.orphans.contains(&pa)); // nadie enlaza a 'a'
}

// --- E1-H09: list_concepts / backlinks --------------------------------------

#[test]
fn list_concepts_marca_invalid_y_orphan() {
    let b = Bundle::from_files(fm(&[("malo.md", "# sin fm\n")]));
    let cs = b.list_concepts();
    let c = cs.iter().find(|c| c.path.as_str() == "malo.md").unwrap();
    assert!(c.invalid);
    assert!(c.orphan);
}

// --- E1-H11: query ----------------------------------------------------------

fn query_set(b: &Bundle, dsl: &str) -> Vec<String> {
    b.query(dsl)
        .iter()
        .map(|p| p.as_str().to_string())
        .collect()
}

#[test]
fn query_operadores() {
    let b = Bundle::from_files(fm(&[
        (
            "a.md",
            "---\ntype: Nota\ntitle: Alfa\nstatus: draft\ntags:\n  - x\n---\n\n# H\n\nhola mundo\n",
        ),
        (
            "b.md",
            "---\ntype: Metric\ntitle: Beta\nstatus: accepted\n---\n\n# H\n\notro\n",
        ),
    ]));
    assert_eq!(query_set(&b, "type:nota"), vec!["a.md"]);
    assert_eq!(query_set(&b, "type=metric"), vec!["b.md"]);
    assert_eq!(query_set(&b, "is:draft"), vec!["a.md"]);
    assert_eq!(query_set(&b, "is:accepted"), vec!["b.md"]);
    assert_eq!(query_set(&b, "has:tags"), vec!["a.md"]);
    assert_eq!(query_set(&b, "body:mundo"), vec!["a.md"]);
    // negación y flip
    assert_eq!(query_set(&b, "-type:nota"), vec!["b.md"]);
    assert_eq!(query_set(&b, "type:!nota"), vec!["b.md"]);
    // texto suelto en título
    assert_eq!(query_set(&b, "beta"), vec!["b.md"]);
}

// --- E1-H14: generadores ----------------------------------------------------

#[test]
fn gen_index_determinista() {
    let b = Bundle::from_files(fm(&[(
        "alfa.md",
        "---\ntype: Concept\ntitle: Alfa\ndescription: d\n---\n\n# H\n",
    )]));
    let m1 = b.gen_index("");
    let m2 = b.gen_index("");
    assert_eq!(m1, m2);
    let idx = RelPath::new("index.md").unwrap();
    assert!(m1.writes[&idx].contains("okf_version"));
    assert!(m1.writes[&idx].contains("[Alfa](alfa.md)"));
}

#[test]
fn gen_tag_indexes_purga_obsoletos() {
    let b = Bundle::from_files(fm(&[
        (
            "a.md",
            "---\ntype: N\ntitle: A\ndescription: d\ntags:\n  - rojo\n---\n\n# H\n",
        ),
        ("tags/viejo/index.md", "# viejo\n"),
    ]));
    let m = b.gen_tag_indexes();
    let viejo = RelPath::new("tags/viejo/index.md").unwrap();
    assert!(m.deletes.contains(&viejo), "el tag obsoleto se elimina");
    assert!(m.writes.keys().any(|k| k.as_str() == "tags/index.md"));
}

// --- E1-H16: export ---------------------------------------------------------

#[test]
fn export_zip_round_trip() {
    let files = fm(&[("a.md", "contenido a"), ("dir/b.md", "contenido b")]);
    let b = Bundle::from_files(files.clone());
    let mut buf = std::io::Cursor::new(Vec::new());
    b.export_zip(&mut buf).unwrap();
    let mut archive = zip::ZipArchive::new(buf).unwrap();
    assert_eq!(archive.len(), 2);
    let mut names: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect();
    names.sort();
    assert_eq!(names, vec!["a.md", "dir/b.md"]);
}

// --- E1-H17: diff -----------------------------------------------------------

#[test]
fn diff_segrega_generados_y_sugiere_mensaje() {
    let a: FileMap = BTreeMap::new();
    let b = fm(&[
        (
            "alfa.md",
            "---\ntype: Concept\ntitle: Alfa\ndescription: d\n---\n\n# H\n",
        ),
        ("index.md", "# Bundle\n"),
    ]);
    let d = diff::diff_snap(&a, &b);
    // index.md va a 'generated', no a 'files'.
    assert!(d.generated.iter().any(|g| g.path.as_str() == "index.md"));
    assert!(d.files.iter().all(|f| f.path.as_str() != "index.md"));
    assert_eq!(d.stats.added, 1);
    assert_eq!(d.files[0].kind, ChangeKind::Add);
    match d.suggested {
        MessageHint::AddSingle { title } => assert_eq!(title, "Alfa"),
        other => panic!("se esperaba AddSingle, fue {other:?}"),
    }
}

#[test]
fn diff_status_change() {
    let a = fm(&[(
        "x.md",
        "---\ntype: N\ntitle: X\ndescription: d\nstatus: draft\n---\n\n# H\n",
    )]);
    let b = fm(&[(
        "x.md",
        "---\ntype: N\ntitle: X\ndescription: d\nstatus: accepted\n---\n\n# H\n",
    )]);
    let d = diff::diff_snap(&a, &b);
    assert_eq!(d.status_changes.len(), 1);
    assert_eq!(d.status_changes[0].from.as_deref(), Some("draft"));
    assert_eq!(d.status_changes[0].to.as_deref(), Some("accepted"));
    match d.suggested {
        MessageHint::StatusSingle { to, .. } => assert_eq!(to, "accepted"),
        other => panic!("se esperaba StatusSingle, fue {other:?}"),
    }
}

#[test]
fn diff_guarda_no_revienta_con_fichero_grande() {
    // 20k líneas distintas a cada lado: con la guarda de tamaño no debe colgarse ni reventar memoria.
    let big_a: String = (0..20_000).map(|i| format!("a{i}\n")).collect();
    let big_b: String = (0..20_000).map(|i| format!("b{i}\n")).collect();
    let a = fm(&[(
        "g.md",
        &format!("---\ntype: N\ntitle: G\ndescription: d\n---\n\n{big_a}"),
    )]);
    let b = fm(&[(
        "g.md",
        &format!("---\ntype: N\ntitle: G\ndescription: d\n---\n\n{big_b}"),
    )]);
    let d = diff::diff_snap(&a, &b);
    assert_eq!(d.stats.modified, 1);
}

// --- E1-H13: escritura validada ---------------------------------------------

#[test]
fn create_concept_rechaza_no_conforme() {
    let b = Bundle::from_files(fm(&[]));
    let p = RelPath::new("nuevo.md").unwrap();
    // type vacío → rechazado (no Err de Result).
    let outcome = b.create_concept(&p, "", Some("Nuevo"), "# H\n", false);
    assert!(!outcome.written);
    assert!(outcome.rejected.is_some());
    // con type válido → escribible.
    let ok = b.create_concept(&p, "Nota", Some("Nuevo"), "# H\n", false);
    assert!(ok.written);
    assert!(ok.rejected.is_none());
}

#[test]
fn merge_frontmatter_null_borra() {
    let b = Bundle::from_files(fm(&[(
        "x.md",
        "---\ntype: N\ntitle: X\ndescription: d\nstatus: draft\n---\n\n# H\n",
    )]));
    let p = RelPath::new("x.md").unwrap();
    let mut patch = BTreeMap::new();
    patch.insert("status".to_string(), None); // borra status
    patch.insert(
        "title".to_string(),
        Some(serde_yaml::Value::String("Nuevo".into())),
    );
    let outcome = b.merge_frontmatter(&p, FrontmatterPatch(patch));
    assert!(!outcome.raw.contains("status:"));
    assert!(outcome.raw.contains("title: Nuevo"));
}

#[test]
fn generadores_consistentes_via_mutation() {
    let _ = generate::slugify_tag("Hólà Múndo/x");
    assert_eq!(generate::slugify_tag("Hólà Múndo"), "hólà-múndo");
}
