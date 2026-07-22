//! Tests del núcleo (E1): contrato de tipos, modelo, conformidad, analyze, query, generadores, diff.

use std::collections::BTreeMap;

use lodestar_core::diff::{self, ChangeKind, MessageHint};
use lodestar_core::generate;
use lodestar_core::model;
use lodestar_core::types::*;
use lodestar_core::Bundle;
// E10-H03: función pura aún NO implementada (fase roja). Se espera reachable en el crate root
// (p. ej. re-exportada vía `pub use types::*`).
use lodestar_core::workspace_revision;

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
    // Absolutas de unidad Windows: `root.join("C:/x")` descartaría el root (zip-slip).
    assert!(RelPath::new("C:\\evil\\x.md").is_err());
    assert!(RelPath::new("C:/evil/x.md").is_err());
    assert!(RelPath::new("c:evil.md").is_err());
    // Backslash: separador en Windows, literal en el proto → rechazo en ambos casos.
    assert!(RelPath::new("a\\b.md").is_err());
}

#[test]
fn relpath_normaliza() {
    assert_eq!(RelPath::new("a//b/./c.md").unwrap().as_str(), "a/b/c.md");
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

// --- E10-H06: extensión de `Check` + familias `SCHEMA-*` / `REL-*` -----------
//
// Fase ROJA: las variantes `SchemaReqfield`/`RelTarget` y los campos nuevos de `Check`
// (`id`/`range`/`related`/`fixes`) todavía NO existen en producción. Estos tests fijan
// el WIRE de los códigos nuevos y la RETRO-COMPAT del `Check` clásico.

#[test]
fn schema_code_wire() {
    // Criterio: `CheckCode::SchemaReqfield` → serializa `"SCHEMA-REQFIELD"`.
    assert_eq!(
        serde_json::to_value(CheckCode::SchemaReqfield).unwrap(),
        serde_json::json!("SCHEMA-REQFIELD"),
    );
    // La familia REL-* comparte el mismo patrón de wire con guion (cubre ambas familias).
    assert_eq!(
        serde_json::to_value(CheckCode::RelTarget).unwrap(),
        serde_json::json!("REL-TARGET"),
    );
}

#[test]
fn check_extension_retrocompat() {
    // Un `Check` de un código OKF clásico, construido SIN fixes/range/id/related.
    let c = Check::new(
        Severity::Err,
        CheckCode::OkfFm01,
        "falta frontmatter",
        vec![RelPath::new("a/b.md").unwrap()],
    );
    let v = serde_json::to_value(&c).unwrap();

    // Retro-compat: los 4 campos clásicos NO cambian de forma ni de valor respecto al wire
    // actual (un consumidor viejo del `Check` no se rompe).
    assert_eq!(v["level"], serde_json::json!("err"));
    assert_eq!(v["code"], serde_json::json!("OKF-FM01"));
    assert_eq!(v["msg"], serde_json::json!("falta frontmatter"));
    assert_eq!(v["targets"], serde_json::json!(["a/b.md"]));

    // Campos nuevos ADITIVOS con su valor por defecto: `fixes` serializa como `[]`
    // y `range` está ausente (o `null`).
    assert_eq!(
        v["fixes"],
        serde_json::json!([]),
        "un Check OKF clásico debe serializar `fixes` como []",
    );
    assert!(
        v.get("range").is_none_or(serde_json::Value::is_null),
        "un Check OKF clásico debe serializar `range` ausente o null",
    );
}

#[test]
fn check_campos_nuevos_por_defecto() {
    // Los 15 checks OKF dejan los campos nuevos en su valor por defecto. Este test fija los
    // NOMBRES Rust de los campos aditivos (id/range/related/fixes) que el diseño D-CheckCode
    // dicta; su presencia hace ROJO por API ausente hasta que se implementen.
    let c = Check::new(Severity::Info, CheckCode::RecTitle, "sin título", vec![]);
    assert!(c.id.is_none());
    assert!(c.range.is_none());
    assert!(c.related.is_empty());
    assert!(c.fixes.is_empty());
}

// --- E10-H02: `ErrorCode` estable en `core::types` ---------------------------
//
// Fase ROJA: el enum `ErrorCode` (16 códigos del contrato, `REFACTOR §13`) todavía NO existe
// en producción. Se espera reachable vía `use lodestar_core::types::*` (patrón de `CheckCode`),
// con wire SCREAMING_SNAKE por `#[serde(rename = "…")]`. Este test fija el WIRE de varios de esos
// códigos; hace ROJO por API ausente hasta que se implemente `ErrorCode`.

#[test]
fn error_code_wire() {
    // Criterio E10-H02 `error_code_wire`: `ErrorCode::RevisionConflict` → `"REVISION_CONFLICT"`.
    assert_eq!(
        serde_json::to_value(ErrorCode::RevisionConflict).unwrap(),
        serde_json::json!("REVISION_CONFLICT"),
    );
    // Blindaje adicional del wire de otros dos códigos del contrato (cubre que TODOS usan
    // SCREAMING_SNAKE y no el `PascalCase` por defecto de serde ni el guion de `CheckCode`).
    assert_eq!(
        serde_json::to_value(ErrorCode::WorkspaceNotFound).unwrap(),
        serde_json::json!("WORKSPACE_NOT_FOUND"),
    );
    assert_eq!(
        serde_json::to_value(ErrorCode::PermissionDenied).unwrap(),
        serde_json::json!("PERMISSION_DENIED"),
    );
}

// --- E10-H04: `ConceptRef` (identidad por path, id opcional/diferido) --------
//
// Fase ROJA: el struct `ConceptRef { path: RelPath, id: Option<ConceptId> }` (`REFACTOR §6.1`)
// todavía NO existe en producción. Se espera reachable vía `use lodestar_core::types::*` (mismo
// patrón que `RelPath`/`ErrorCode`), con una deserialización que acepta `{ "path": … }` y deja el
// `id` ausente como `None`. Estos tests hacen ROJO por API ausente (símbolo `ConceptRef`) hasta que
// se implemente. La resolución contra un bundle (`CONCEPT_NOT_FOUND`) se prueba en `lodestar-app`
// (`tests/concept_ref.rs`), porque exige un `Workspace` abierto y el core es puro.

#[test]
fn ref_por_path() {
    // Criterio `ref_por_path`: `{ "path": "a/b.md" }` deserializa a un `ConceptRef` cuyo `path` es
    // el `RelPath` validado y cuyo `id` queda ausente (`None`) — el id es opcional/diferido.
    let referencia: ConceptRef =
        serde_json::from_str(r#"{"path":"a/b.md"}"#).expect("`{ path: a/b.md }` debe deserializar");
    assert_eq!(
        referencia.path,
        RelPath::new("a/b.md").unwrap(),
        "el `path` deserializado debe ser el RelPath validado `a/b.md`",
    );
    assert!(
        referencia.id.is_none(),
        "sin clave `id` en el JSON, `ConceptRef::id` debe quedar `None`, es {:?}",
        referencia.id,
    );
}

#[test]
fn ref_rechaza_traversal() {
    // Criterio `ref_rechaza_traversal`: `{ "path": "../x" }` NO debe deserializar — `RelPath`
    // rechaza el `..` en su `Deserialize` (invariante #6, único chokepoint de path-traversal), y
    // `ConceptRef` hereda ese rechazo por delegar en el `RelPath` de su campo `path`.
    let resultado = serde_json::from_str::<ConceptRef>(r#"{"path":"../x"}"#);
    assert!(
        resultado.is_err(),
        "un `ConceptRef` con `path` de traversal (`../x`) debe fallar al deserializar, dio {resultado:?}",
    );
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
    let outcome = b.create_concept(&p, "", Some("Nuevo"), "# H\n", None, false);
    assert!(!outcome.written);
    assert!(outcome.rejected.is_some());
    // con type válido → escribible.
    let ok = b.create_concept(&p, "Nota", Some("Nuevo"), "# H\n", None, false);
    assert!(ok.written);
    assert!(ok.rejected.is_none());
}

#[test]
fn create_concept_incluye_timestamp_en_su_posicion_canonica() {
    let b = Bundle::from_files(fm(&[]));
    let p = RelPath::new("nuevo.md").unwrap();
    // Con timestamp (paridad prototipo): aparece antes de `status` (orden KNOWN_FM).
    let ok = b.create_concept(
        &p,
        "Nota",
        Some("Nuevo"),
        "# H\n",
        Some("2026-07-05T10:20:30Z"),
        false,
    );
    assert!(ok.written);
    assert!(
        ok.raw.contains("timestamp: 2026-07-05T10:20:30Z"),
        "falta el timestamp: {}",
        ok.raw
    );
    let ts_pos = ok.raw.find("timestamp:").unwrap();
    let status_pos = ok.raw.find("status:").unwrap();
    assert!(
        ts_pos < status_pos,
        "timestamp debe preceder a status: {}",
        ok.raw
    );
    // Sin timestamp: no se emite la clave.
    let sin = b.create_concept(&p, "Nota", Some("Nuevo"), "# H\n", None, false);
    assert!(
        !sin.raw.contains("timestamp:"),
        "no debía emitir timestamp: {}",
        sin.raw
    );
}

#[test]
fn create_concept_genera_heading_por_defecto_cuando_body_vacio() {
    let b = Bundle::from_files(fm(&[]));
    // body vacío + ty no vacío → `# {ty} - {title}`.
    let p = RelPath::new("mi-cosa.md").unwrap();
    let con_tipo = b.create_concept(&p, "Nota", Some("Mi Cosa"), "", None, false);
    assert!(con_tipo.written);
    assert!(
        con_tipo.raw.contains("# Nota - Mi Cosa\n"),
        "falta el heading con tipo: {}",
        con_tipo.raw
    );
    // ty vacío → `# {title}` (sin separador colgante). type vacío rechaza, pero el raw se computa.
    let sin_tipo = b.create_concept(&p, "", Some("Mi Cosa"), "", None, false);
    assert!(
        sin_tipo.raw.contains("# Mi Cosa\n") && !sin_tipo.raw.contains("# Mi Cosa -"),
        "el heading sin tipo no debe tener separador: {}",
        sin_tipo.raw
    );
    // title None → deriva del path con title_from_path (`mi-cosa` → `Mi Cosa`).
    let sin_titulo = b.create_concept(&p, "Nota", None, "", None, false);
    assert!(
        sin_titulo.raw.contains("# Nota - Mi Cosa\n"),
        "el título debe derivar del path: {}",
        sin_titulo.raw
    );
    // body no vacío → se respeta tal cual, sin generar default.
    let con_body = b.create_concept(&p, "Nota", Some("Mi Cosa"), "# H\n", None, false);
    assert!(
        con_body.raw.contains("# H\n") && !con_body.raw.contains("# Nota - Mi Cosa"),
        "un body explícito no debe reemplazarse: {}",
        con_body.raw
    );
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

// --- Regresiones de paridad con el prototipo (revisión profunda) -------------

#[test]
fn fm_escalares_no_string_se_coercen_como_js() {
    // `type: 123` NO es OKF-FM03 (hard-fail de fichero entero): el proto lo acepta vía String(v).
    let b = Bundle::from_files(fm(&[(
        "n.md",
        "---\ntype: 123\ntitle: 2024\ndescription: true\n---\n\n# H\n\ncuerpo\n",
    )]));
    let a = b.analyze();
    assert_eq!(a.hard_fail, 0, "el veredicto no puede invertirse: {a:?}");
    let checks = &a.per_file[&RelPath::new("n.md").unwrap()];
    assert!(!checks.iter().any(|c| c.code == CheckCode::OkfFm03));
}

#[test]
fn fm_null_explicito_cuenta_como_presente() {
    // `type:` (null) → presente para has:/no: (fmPresent de JS: null !== undefined)…
    let b = Bundle::from_files(fm(&[
        (
            "connull.md",
            "---\ntype:\ntitle: A\ndescription: d\n---\n\n# H\n",
        ),
        ("sintipo.md", "---\ntitle: B\ndescription: d\n---\n\n# H\n"),
    ]));
    let con_type = b.query("has:type");
    assert!(con_type.iter().any(|p| p.as_str() == "connull.md"));
    assert!(!con_type.iter().any(|p| p.as_str() == "sintipo.md"));
    // …y buildRaw lo conserva (`type: null`), no lo borra en silencio.
    let p = RelPath::new("connull.md").unwrap();
    let outcome = b.merge_frontmatter(&p, FrontmatterPatch(BTreeMap::new()));
    assert!(outcome.raw.contains("type: null"), "raw: {:?}", outcome.raw);
}

#[test]
fn fmt_ts_rechaza_iso_con_basura() {
    // El proto valida con Date.parse el string ENTERO: `2024-01-15hello` y `T99:99` son FMT-TS.
    let ok = serde_yaml::Value::String("2024-01-15".into());
    let ok_t = serde_yaml::Value::String("2024-01-15T10:30:00Z".into());
    let bad_tail = serde_yaml::Value::String("2024-01-15hello".into());
    let bad_hour = serde_yaml::Value::String("2024-01-15T99:99".into());
    assert!(model::is_iso(&ok));
    assert!(model::is_iso(&ok_t));
    assert!(!model::is_iso(&bad_tail));
    assert!(!model::is_iso(&bad_hour));
}

#[test]
fn title_from_path_boundaries_como_js() {
    // \b\w del proto: el acento y el punto abren palabra (quirk incluido, es la spec).
    assert_eq!(model::title_from_path("año.md"), "AñO");
    assert_eq!(model::title_from_path("foo.bar.md"), "Foo.Bar");
    assert_eq!(model::title_from_path("mi-nota_2.md"), "Mi Nota 2");
}

#[test]
fn tags_ordenados_con_locale_compare() {
    // localeCompare: "alpha" < "árbol" < "Beta" (no orden de bytes, que pondría Beta primero).
    let b = Bundle::from_files(fm(&[(
        "x.md",
        "---\ntype: N\ntitle: X\ndescription: d\ntags: [Beta, alpha, \u{e1}rbol]\n---\n\n# H\n",
    )]));
    let m = b.gen_tag_indexes();
    let root = m
        .writes
        .get(&RelPath::new("tags/index.md").unwrap())
        .unwrap();
    let pos = |s: &str| root.find(s).unwrap_or(usize::MAX);
    assert!(pos("[alpha]") < pos("[\u{e1}rbol]"), "root: {root}");
    assert!(pos("[\u{e1}rbol]") < pos("[Beta]"), "root: {root}");
}

#[test]
fn gen_index_type_vacio_cae_a_concept() {
    let b = Bundle::from_files(fm(&[(
        "e.md",
        "---\ntype: \"\"\ntitle: E\ndescription: d\n---\n\n# H\n",
    )]));
    let m = b.gen_index("");
    let idx = m.writes.get(&RelPath::new("index.md").unwrap()).unwrap();
    assert!(idx.contains("# Concept\n"), "idx: {idx}");
}

#[test]
fn fm_diff_sin_cambio_fantasma_por_string_vacio() {
    // Añadir `description: ""` no es un cambio (fmFmt(undefined) === fmFmt("")).
    let a = "---\ntype: N\ntitle: X\n---\n\n# H\n";
    let b = "---\ntype: N\ntitle: X\ndescription: \"\"\n---\n\n# H\n";
    assert!(diff::fm_diff(a, b).is_empty());
}

#[test]
fn suggest_msg_status_vacio_cae_a_update() {
    let a = fm(&[(
        "s.md",
        "---\ntype: N\ntitle: S\nstatus: draft\n---\n\n# H\n",
    )]);
    let b = fm(&[("s.md", "---\ntype: N\ntitle: S\nstatus: \"\"\n---\n\n# H\n")]);
    let d = diff::diff_snap(&a, &b);
    assert!(
        matches!(d.suggested, MessageHint::Update { .. }),
        "{:?}",
        d.suggested
    );
}

#[test]
fn diff_snap_ordena_numeric_aware() {
    let a = fm(&[]);
    let b = fm(&[
        ("doc-10.md", "---\ntype: N\ntitle: D10\n---\n\n# H\n"),
        ("doc-2.md", "---\ntype: N\ntitle: D2\n---\n\n# H\n"),
    ]);
    let d = diff::diff_snap(&a, &b);
    let order: Vec<&str> = d.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(order, vec!["doc-2.md", "doc-10.md"]);
}

#[test]
fn backlinks_out_dedup_sin_self_ni_reservados() {
    let b = Bundle::from_files(fm(&[
        (
            "x.md",
            "---\ntype: N\ntitle: X\ndescription: d\n---\n\n[a](/a.md) [a](/a.md) [idx](/index.md) [yo](/x.md)\n",
        ),
        ("a.md", "---\ntype: N\ntitle: A\ndescription: d\n---\n\n# H\n"),
        ("index.md", "---\nokf_version: \"0.1\"\n---\n\n# B\n\n* [x](x.md)\n"),
    ]));
    let bl = b.backlinks(&RelPath::new("x.md").unwrap());
    assert_eq!(
        bl.out.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        vec!["a.md"]
    );
}

#[test]
fn query_campo_vacio_es_texto_suelto() {
    // `":foo"` → field vacío es falsy en JS → texto suelto (busca "foo"), no field-match de "".
    let b = Bundle::from_files(fm(&[(
        "foo-nota.md",
        "---\ntype: N\ntitle: T\ndescription: d\n---\n\n# H\n",
    )]));
    let hits = b.query(":foo");
    assert!(hits.iter().any(|p| p.as_str() == "foo-nota.md"));
}

// --- E10-H03: WorkspaceRevision (identidad de contenido determinista) ---------
//
// La función pura `workspace_revision(files: &FileMap, writable: &[RelPath])` (aún NO
// implementada) calcula una identidad determinista del contenido escribible del workspace:
// filtra a los `writableRoots` (slice vacío = todo el bundle es escribible, coherente con
// E9-H05), EXCLUYE todo `.lodestar/` y cualquier root fuera de `writable` (referenceRoots),
// ordena por `RelPath`, hashea cada contenido con blake3 y combina path+hash en un hash raíz.
// Estos tests aseveran PROPIEDADES (determinismo, exclusión, sensibilidad), nunca el hash
// literal ni el separador exacto del hash raíz — eso lo decide el implementador.

#[test]
fn revision_independiente_del_orden() {
    // Mismo contenido, claves insertadas en órdenes distintos → misma revisión.
    // (Aunque `FileMap` es `BTreeMap` y ya ordena, forzamos el punto insertando en orden
    // inverso: la revisión debe depender solo del contenido, no del orden de inserción.)
    let a = RelPath::new("a.md").unwrap();
    let b = RelPath::new("b/c.md").unwrap();
    let z = RelPath::new("z.md").unwrap();

    let mut ascendente: FileMap = BTreeMap::new();
    ascendente.insert(a.clone(), "contenido A".to_string());
    ascendente.insert(b.clone(), "contenido B".to_string());
    ascendente.insert(z.clone(), "contenido Z".to_string());

    let mut inverso: FileMap = BTreeMap::new();
    inverso.insert(z.clone(), "contenido Z".to_string());
    inverso.insert(b.clone(), "contenido B".to_string());
    inverso.insert(a.clone(), "contenido A".to_string());

    // writable vacío = todo el bundle es escribible.
    assert_eq!(
        workspace_revision(&ascendente, &[]),
        workspace_revision(&inverso, &[]),
    );
}

#[test]
fn revision_excluye_lodestar() {
    // Añadir ficheros bajo `.lodestar/` (cachés/índices/runtime) NO cambia la revisión.
    let mut base: FileMap = BTreeMap::new();
    base.insert(RelPath::new("nota.md").unwrap(), "cuerpo".to_string());
    base.insert(
        RelPath::new("sub/otra.md").unwrap(),
        "más cuerpo".to_string(),
    );

    let mut con_lodestar = base.clone();
    con_lodestar.insert(
        RelPath::new(".lodestar/index.db").unwrap(),
        "binario de la cache".to_string(),
    );
    con_lodestar.insert(
        RelPath::new(".lodestar/runtime/pending.json").unwrap(),
        "estado efímero".to_string(),
    );

    assert_eq!(
        workspace_revision(&base, &[]),
        workspace_revision(&con_lodestar, &[]),
    );
}

#[test]
fn revision_excluye_reference_roots() {
    // Con `writable = ["docs"]`, los ficheros bajo otros roots son referenceRoots (solo lectura)
    // y quedan FUERA de la identidad: cambiar su contenido NO cambia la revisión.
    let docs = RelPath::new("docs/guia.md").unwrap();
    let referencia = RelPath::new("reference/externo.md").unwrap();

    let mut base: FileMap = BTreeMap::new();
    base.insert(docs.clone(), "guia escribible".to_string());
    base.insert(referencia.clone(), "referencia v1".to_string());

    let mut cambio_fuera = base.clone();
    // Cambio FUERA de writable (en el reference root).
    cambio_fuera.insert(referencia.clone(), "referencia v2 muy distinta".to_string());

    let writable = [RelPath::new("docs").unwrap()];
    assert_eq!(
        workspace_revision(&base, &writable),
        workspace_revision(&cambio_fuera, &writable),
    );
}

#[test]
fn revision_sensible_al_contenido() {
    // Cambiar un solo byte en un `.md` DENTRO de writable cambia la revisión.
    let p = RelPath::new("docs/guia.md").unwrap();

    let mut base: FileMap = BTreeMap::new();
    base.insert(p.clone(), "contenido original".to_string());

    let mut un_byte = base.clone();
    un_byte.insert(p.clone(), "contenido originaL".to_string()); // 'l' → 'L'

    let writable = [RelPath::new("docs").unwrap()];
    assert_ne!(
        workspace_revision(&base, &writable),
        workspace_revision(&un_byte, &writable),
    );
}

// ---------------------------------------------------------------------------
// E10-H05 — `core::schema`: tipo `Schema` + wire YAML camelCase.
//
// Fase ROJA (ARCHITECTURE.md §19.2, REFACTOR §4/§9.4): el módulo PURO `core::schema`
// todavía NO existe. Este test fija el contrato de deserialización EN MEMORIA (el core
// nunca abre ficheros: recibe el `Schema` ya deserializado desde un string):
//   Schema { version: String, types: BTreeMap<String, DocType> }
//   DocType { name, description, required_fields, allowed_statuses, fields,
//             relations: BTreeMap<String, RelationDef>, rules, body_template }
// El wire YAML usa claves camelCase (`requiredFields`/`allowedStatuses`/`bodyTemplate`/
// `targetTypes`) mapeadas a los campos snake_case (mismo patrón que `WorkspaceConfig`).
// ---------------------------------------------------------------------------

/// Criterio `carga_doctype`: un `Schema` con un `DocType` `decision`
/// (`requiredFields`/`allowedStatuses`) deserializado desde YAML en memoria →
/// `schema.types["decision"].required_fields == ["title","status","rationale"]`.
#[test]
fn carga_doctype() {
    use lodestar_core::schema::Schema;

    // YAML EN MEMORIA (no hay I/O: el core solo deserializa). Claves camelCase del wire.
    let yaml = "\
version: \"1\"
types:
  decision:
    name: decision
    description: Una decisión de arquitectura
    requiredFields: [title, status, rationale]
    allowedStatuses: [proposed, accepted, rejected, superseded]
";

    let schema: Schema =
        serde_yaml::from_str(yaml).expect("un Schema válido debe deserializar desde YAML");

    let decision = schema
        .types
        .get("decision")
        .expect("el DocType `decision` debe existir en `schema.types`");

    assert_eq!(
        decision.required_fields,
        vec![
            "title".to_string(),
            "status".to_string(),
            "rationale".to_string()
        ],
        "requiredFields del wire camelCase debe mapear a `required_fields` preservando el orden"
    );
    assert!(
        decision.allowed_statuses.iter().any(|s| s == "proposed"),
        "allowedStatuses debe mapear a `allowed_statuses`; eran: {:?}",
        decision.allowed_statuses
    );
}

// --- E10-H07: validación schema-driven (`core::schema::validate_schema`) -----
// Función PURA `validate_schema(&Bundle, &Schema) -> Vec<Check>`: por cada concepto con
// `type` conocido comprueba `required_fields` (falta → SCHEMA-REQFIELD/Err) y `status ∈
// allowed_statuses` (fuera → SCHEMA-STATUS/Err). Aditiva: sin schema, cero checks.

/// Criterio `falta_campo_obligatorio`: `DocType decision` con `requiredFields:[rationale]` y un
/// concepto `decision` SIN `rationale` → un `Check{code:SCHEMA-REQFIELD, level:Err}` sobre ese path,
/// con `msg` no vacío que nombra el campo que falta.
#[test]
fn falta_campo_obligatorio() {
    use lodestar_core::schema::{validate_schema, DocType, Schema};

    // Bundle: un concepto `type: decision` SIN el campo obligatorio `rationale`.
    let b = Bundle::from_files(fm(&[(
        "d.md",
        "---\ntype: decision\ntitle: Migrar a Rust\nstatus: proposed\n---\n\n# H\n\ncuerpo\n",
    )]));

    // Schema: el `DocType decision` exige `rationale`.
    let mut schema = Schema::default();
    schema.types.insert(
        "decision".to_string(),
        DocType {
            name: "decision".to_string(),
            required_fields: vec!["rationale".to_string()],
            ..DocType::default()
        },
    );

    let checks = validate_schema(&b, &schema);

    let path = RelPath::new("d.md").unwrap();
    let reqfield = checks
        .iter()
        .find(|c| c.code == CheckCode::SchemaReqfield)
        .expect("falta `rationale` → debe emitirse un Check SCHEMA-REQFIELD");
    assert_eq!(
        reqfield.level,
        Severity::Err,
        "un campo obligatorio ausente es un error duro"
    );
    assert!(
        reqfield.targets.contains(&path),
        "el check debe apuntar al path del concepto; targets: {:?}",
        reqfield.targets
    );
    assert!(
        !reqfield.msg.is_empty(),
        "el msg del check no debe ser vacío"
    );
    assert!(
        reqfield.msg.contains("rationale"),
        "el msg debe nombrar el campo que falta; msg: {:?}",
        reqfield.msg
    );
}

/// Criterio `status_no_permitido`: un concepto con `status: invented` fuera de `allowedStatuses`
/// → `Check{code:SCHEMA-STATUS, level:Err}` con `msg` no vacío que nombra el status inválido.
/// `required_fields` se deja VACÍO para aislar este criterio del de campos obligatorios.
#[test]
fn status_no_permitido() {
    use lodestar_core::schema::{validate_schema, DocType, Schema};

    // Concepto con `status: invented`, fuera de los estados permitidos.
    let b = Bundle::from_files(fm(&[(
        "d.md",
        "---\ntype: decision\ntitle: X\nstatus: invented\n---\n\n# H\n\ncuerpo\n",
    )]));

    // Schema: `required_fields` VACÍO (aísla el criterio); solo restringe `status`.
    let mut schema = Schema::default();
    schema.types.insert(
        "decision".to_string(),
        DocType {
            name: "decision".to_string(),
            required_fields: Vec::new(),
            allowed_statuses: vec!["proposed".to_string(), "accepted".to_string()],
            ..DocType::default()
        },
    );

    let checks = validate_schema(&b, &schema);

    let status = checks
        .iter()
        .find(|c| c.code == CheckCode::SchemaStatus)
        .expect(
            "`status: invented` fuera de allowedStatuses → debe emitirse un Check SCHEMA-STATUS",
        );
    assert_eq!(
        status.level,
        Severity::Err,
        "un status fuera del lifecycle declarado es un error duro"
    );
    assert!(!status.msg.is_empty(), "el msg del check no debe ser vacío");
    assert!(
        status.msg.contains("invented"),
        "el msg debe nombrar el status no permitido; msg: {:?}",
        status.msg
    );
}

/// Criterio `sin_schema_sin_checks`: el mismo bundle validado contra `Schema::default()` (bundle
/// sin `schema.yaml`) NO produce ningún check schema-driven (compat con bundles OKF actuales).
#[test]
fn sin_schema_sin_checks() {
    use lodestar_core::schema::{validate_schema, Schema};

    let b = Bundle::from_files(fm(&[(
        "d.md",
        "---\ntype: decision\ntitle: X\nstatus: invented\n---\n\n# H\n\ncuerpo\n",
    )]));

    let checks = validate_schema(&b, &Schema::default());

    assert_eq!(
        checks,
        Vec::<Check>::new(),
        "un bundle sin schema no debe producir checks schema-driven"
    );
}

// --- E11-H03: relaciones tipadas (`core::schema::validate_relations`) ---------
//
// Función PURA aún NO implementada (fase roja — compila-falla porque `validate_relations`
// no existe todavía en `crates/lodestar-core/src/schema.rs`). Firma asumida (paralela a
// `validate_schema` de E10-H07):
//
//   pub fn validate_relations(bundle: &Bundle, schema: &Schema) -> Vec<Check>;
//
// Por cada concepto cuyo `type` está declarado en el schema, y por cada relación declarada en
// su `DocType.relations` (BTreeMap<nombre, RelationDef>), lee el campo del frontmatter con ese
// NOMBRE (vive en `Frontmatter.extra`, valor = secuencia YAML de paths target) y comprueba:
//   1. target existe como concepto del bundle → si no, `CheckCode::RelTarget` (Err).
//   2. el `type` del target ∈ `RelationDef.target_types` (vacío = cualquiera) → si no,
//      `CheckCode::RelType` (Err).
//   3. nº de targets respeta `RelationDef.cardinality` ("one" ⇒ máx. 1) → si no,
//      `CheckCode::RelCard` (Err).
// Cada `Check` con `level: Err`, `msg` en español no vacío, `targets` = [path del concepto
// origen] y `range` al campo de la relación. Los paths target del frontmatter se representan
// como el `RelPath` del fichero destino tal cual (p. ej. `capitulo.md`), sin barra inicial.

/// Criterio `relacion_target_roto`: una relación `appears_in` a un target inexistente →
/// `Check{code:REL-TARGET, level:Err}` sobre el concepto origen, con `msg` no vacío y `range`
/// presente (acota el campo de la relación).
#[test]
fn relacion_target_roto() {
    use lodestar_core::schema::{validate_relations, DocType, RelationDef, Schema};

    // Concepto `character` con `appears_in` a un capítulo que no existe en el bundle.
    let b = Bundle::from_files(fm(&[(
        "juan.md",
        "---\ntype: character\ntitle: Juan\nappears_in:\n  - capitulo_fantasma.md\n---\n\n# Juan\n\ncuerpo\n",
    )]));

    // Schema: `character.appears_in` apunta a tipos `chapter`, cardinalidad libre (`many`).
    let mut schema = Schema::default();
    schema.types.insert(
        "character".to_string(),
        DocType {
            name: "character".to_string(),
            relations: BTreeMap::from([(
                "appears_in".to_string(),
                RelationDef {
                    target_types: vec!["chapter".to_string()],
                    cardinality: "many".to_string(),
                },
            )]),
            ..DocType::default()
        },
    );

    let checks = validate_relations(&b, &schema);

    let path = RelPath::new("juan.md").unwrap();
    let target = checks
        .iter()
        .find(|c| c.code == CheckCode::RelTarget)
        .expect("una relación a un target inexistente → debe emitirse un Check REL-TARGET");
    assert_eq!(
        target.level,
        Severity::Err,
        "una relación a un target inexistente es un error duro"
    );
    assert!(
        target.targets.contains(&path),
        "el check debe apuntar al concepto origen; targets: {:?}",
        target.targets
    );
    assert!(!target.msg.is_empty(), "el msg del check no debe ser vacío");
    assert!(
        target.range.is_some(),
        "el check debe acotar el campo de la relación con un `range`"
    );
}

/// Criterio `relacion_tipo_invalido`: una relación a un concepto cuyo `type` NO está en
/// `RelationDef.target_types` → `Check{code:REL-TYPE, level:Err}` sobre el concepto origen, con
/// `msg` no vacío. El target EXISTE y la cardinalidad se respeta (aísla el criterio del tipo).
#[test]
fn relacion_tipo_invalido() {
    use lodestar_core::schema::{validate_relations, DocType, RelationDef, Schema};

    // `juan` (character) → appears_in `espada` (type item), pero `appears_in` solo admite `chapter`.
    let b = Bundle::from_files(fm(&[
        (
            "juan.md",
            "---\ntype: character\ntitle: Juan\nappears_in:\n  - espada.md\n---\n\n# Juan\n\ncuerpo\n",
        ),
        (
            "espada.md",
            "---\ntype: item\ntitle: Espada\n---\n\n# Espada\n\ncuerpo\n",
        ),
    ]));

    let mut schema = Schema::default();
    for t in ["chapter", "item"] {
        schema.types.insert(
            t.to_string(),
            DocType {
                name: t.to_string(),
                ..DocType::default()
            },
        );
    }
    schema.types.insert(
        "character".to_string(),
        DocType {
            name: "character".to_string(),
            relations: BTreeMap::from([(
                "appears_in".to_string(),
                RelationDef {
                    target_types: vec!["chapter".to_string()],
                    cardinality: "many".to_string(),
                },
            )]),
            ..DocType::default()
        },
    );

    let checks = validate_relations(&b, &schema);

    let path = RelPath::new("juan.md").unwrap();
    let tipo = checks
        .iter()
        .find(|c| c.code == CheckCode::RelType)
        .expect("un target de `type` no permitido → debe emitirse un Check REL-TYPE");
    assert_eq!(
        tipo.level,
        Severity::Err,
        "un target de tipo no permitido es un error duro"
    );
    assert!(
        tipo.targets.contains(&path),
        "el check debe apuntar al concepto origen; targets: {:?}",
        tipo.targets
    );
    assert!(!tipo.msg.is_empty(), "el msg del check no debe ser vacío");
}

/// Criterio `relacion_cardinalidad`: una relación de cardinalidad `one` con DOS targets →
/// `Check{code:REL-CARD, level:Err}` sobre el concepto origen, con `msg` no vacío. Ambos targets
/// existen y son de tipo válido (`target_types` vacío = cualquiera) para aislar el criterio.
#[test]
fn relacion_cardinalidad() {
    use lodestar_core::schema::{validate_relations, DocType, RelationDef, Schema};

    // `mentor` es cardinalidad "one" pero `juan` declara DOS mentores (ambos existen, tipo libre).
    let b = Bundle::from_files(fm(&[
        (
            "juan.md",
            "---\ntype: character\ntitle: Juan\nmentor:\n  - pedro.md\n  - ana.md\n---\n\n# Juan\n\ncuerpo\n",
        ),
        (
            "pedro.md",
            "---\ntype: character\ntitle: Pedro\n---\n\n# Pedro\n\ncuerpo\n",
        ),
        (
            "ana.md",
            "---\ntype: character\ntitle: Ana\n---\n\n# Ana\n\ncuerpo\n",
        ),
    ]));

    let mut schema = Schema::default();
    schema.types.insert(
        "character".to_string(),
        DocType {
            name: "character".to_string(),
            relations: BTreeMap::from([(
                "mentor".to_string(),
                RelationDef {
                    target_types: Vec::new(),
                    cardinality: "one".to_string(),
                },
            )]),
            ..DocType::default()
        },
    );

    let checks = validate_relations(&b, &schema);

    let path = RelPath::new("juan.md").unwrap();
    let card = checks
        .iter()
        .find(|c| c.code == CheckCode::RelCard)
        .expect("cardinalidad `one` con dos targets → debe emitirse un Check REL-CARD");
    assert_eq!(
        card.level,
        Severity::Err,
        "exceder la cardinalidad declarada es un error duro"
    );
    assert!(
        card.targets.contains(&path),
        "el check debe apuntar al concepto origen; targets: {:?}",
        card.targets
    );
    assert!(!card.msg.is_empty(), "el msg del check no debe ser vacío");
}

// --- E11-H02: graph_query estructural (path_between / cycles / components) ----
//
// Operaciones puras del core sobre el grafo de enlaces (aristas = `out_links`/`resolve_link`,
// la MISMA representación que `analyze().out`/`inn` y `graph_model`/`neighborhood`). Firmas
// asumidas (fase roja — aún NO existen en `crates/lodestar-core/src/graph.rs`; se exponen como
// métodos de `Bundle`, en línea con `neighborhood`/`graph_model`/`backlinks`):
//
//   impl Bundle {
//       /// Camino más corto DIRIGIDO de `a` a `b` (siguiendo aristas salientes), incluyendo
//       /// ambos extremos. `[a, .., b]`. Vacío (`vec![]`) si no hay camino — NUNCA error.
//       pub fn path_between(&self, a: &RelPath, b: &RelPath) -> Vec<RelPath>;
//       /// Ciclos dirigidos del grafo de enlaces. Cada ciclo es el conjunto de nodos que lo
//       /// forman (un `Vec<RelPath>`). Los nodos acíclicos NO aparecen.
//       pub fn cycles(&self) -> Vec<Vec<RelPath>>;
//       /// Componentes conexas (conectividad no dirigida) del grafo de enlaces. Cada componente
//       /// es el conjunto de sus nodos.
//       pub fn components(&self) -> Vec<Vec<RelPath>>;
//   }
//
// Fixtures: cada concepto lleva frontmatter válido (`type`/`title`/`description`) para ser
// concepto real; las aristas se montan con enlaces markdown `[x](/x.md)` en el cuerpo (mismo
// patrón que `analyze_backlinks_son_inversa_de_out`), sin ghosts ni reservados.

/// Nodo concepto con `body` como cuerpo (donde van los enlaces markdown que forman aristas).
fn nodo(title: &str, body: &str) -> String {
    format!("---\ntype: N\ntitle: {title}\ndescription: d\n---\n\n# H\n\n{body}\n")
}

/// Criterio `path_between_directo`: A→B→C ⇒ `path_between(A,C) == [A,B,C]` (camino más corto
/// dirigido, incluyendo los dos extremos).
#[test]
fn path_between_directo() {
    let b = Bundle::from_files(fm(&[
        ("a.md", &nodo("A", "[b](/b.md)")),
        ("b.md", &nodo("B", "[c](/c.md)")),
        ("c.md", &nodo("C", "cuerpo")),
    ]));
    let a = RelPath::new("a.md").unwrap();
    let c = RelPath::new("c.md").unwrap();

    let camino = b.path_between(&a, &c);

    assert_eq!(
        camino.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        vec!["a.md", "b.md", "c.md"],
        "el camino más corto dirigido A→B→C debe ser exactamente [A,B,C]"
    );
}

/// Criterio `detecta_ciclo`: A→B→A ⇒ `cycles()` reporta el ciclo `{A,B}`. El nodo D→A, acíclico,
/// NO debe aparecer en ningún ciclo reportado.
#[test]
fn detecta_ciclo() {
    let b = Bundle::from_files(fm(&[
        ("a.md", &nodo("A", "[b](/b.md)")),
        ("b.md", &nodo("B", "[a](/a.md)")),
        // D enlaza a A pero nadie enlaza a D: entra al ciclo pero no forma parte de él.
        ("d.md", &nodo("D", "[a](/a.md)")),
    ]));
    let pa = RelPath::new("a.md").unwrap();
    let pb = RelPath::new("b.md").unwrap();
    let pd = RelPath::new("d.md").unwrap();

    let ciclos = b.cycles();

    assert_eq!(ciclos.len(), 1, "debe reportar exactamente un ciclo");
    let miembros: std::collections::BTreeSet<&str> = ciclos[0].iter().map(|p| p.as_str()).collect();
    assert!(
        miembros.contains(pa.as_str()) && miembros.contains(pb.as_str()),
        "el ciclo debe contener A y B, fue {miembros:?}"
    );
    assert!(
        !miembros.contains(pd.as_str()),
        "el nodo acíclico D no debe aparecer en el ciclo"
    );
}

/// Criterio `dos_componentes`: dos subgrafos inconexos (A→B y C→D) ⇒ `components()` devuelve 2
/// componentes, y cada nodo pertenece a exactamente una.
#[test]
fn dos_componentes() {
    let b = Bundle::from_files(fm(&[
        ("a.md", &nodo("A", "[b](/b.md)")),
        ("b.md", &nodo("B", "cuerpo")),
        ("c.md", &nodo("C", "[d](/d.md)")),
        ("d.md", &nodo("D", "cuerpo")),
    ]));

    let comps = b.components();

    assert_eq!(comps.len(), 2, "dos subgrafos inconexos ⇒ 2 componentes");

    // Cada uno de los 4 nodos aparece en exactamente una componente.
    let mut vistos: BTreeMap<&str, usize> = BTreeMap::new();
    for comp in &comps {
        for n in comp {
            *vistos.entry(n.as_str()).or_insert(0) += 1;
        }
    }
    for id in ["a.md", "b.md", "c.md", "d.md"] {
        assert_eq!(
            vistos.get(id).copied().unwrap_or(0),
            1,
            "{id} debe pertenecer a exactamente una componente"
        );
    }
}

/// Criterio `sin_camino`: A y C sin ninguna arista que los conecte ⇒ `path_between(A,C)` es vacío
/// (`vec![]`), NO un error.
#[test]
fn sin_camino() {
    let b = Bundle::from_files(fm(&[
        ("a.md", &nodo("A", "[b](/b.md)")),
        ("b.md", &nodo("B", "cuerpo")),
        // C aislado: ni sale ni entra hacia el grupo de A.
        ("c.md", &nodo("C", "cuerpo")),
    ]));
    let a = RelPath::new("a.md").unwrap();
    let c = RelPath::new("c.md").unwrap();

    let camino = b.path_between(&a, &c);

    assert!(
        camino.is_empty(),
        "sin camino dirigido A→C el resultado debe ser vacío, fue {:?}",
        camino.iter().map(|p| p.as_str()).collect::<Vec<_>>()
    );
}

// --- E12-H01: tipos del plan (`ChangeSet`, `NormalizedOperation`, ids/hashes) -------------------
//
// Fase ROJA: los tipos del plan (`ChangeSet`, `NormalizedOperation`, los newtypes
// `ChangeSetId`/`PlanHash`/`ReceiptId`, y los tipos de análisis `RiskAssessment`/`RiskLevel`/
// `SemanticDiff`/`ValidationReport`) todavía NO existen en producción. Se esperan alcanzables vía
// `use lodestar_core::types::*` (mismo patrón que `WorkspaceRevision`/`ConceptRef`). Estos tests
// hacen ROJO por API ausente (símbolos inexistentes) hasta que E12-H01 los defina en `core::types`.
//
// Forma ASUMIDA del contrato (solo lo que el criterio de aceptación fija; la forma interna de
// `NormalizedOperation` se cierra en E12-H05..H07 y NO se sobre-restringe aquí):
//   ChangeSet {
//       id: ChangeSetId,                       // wire `id`            (newtype string transparente)
//       base_revision: WorkspaceRevision,      // wire `baseWorkspaceRevision` (rename explícito)
//       operations: Vec<NormalizedOperation>,  // wire `operations`
//       plan_hash: PlanHash,                   // wire `planHash`
//       risk: RiskAssessment,                  // wire `risk`
//       semantic_diff: SemanticDiff,           // wire `semanticDiff`
//       validation: ValidationReport,          // wire `validation`
//       expires_at: String,                    // wire `expiresAt`     (timestamp ISO-8601)
//   }
// Supuestos de construcción mínima (documentados para el implementador):
//   - `ChangeSetId`/`PlanHash` son newtypes string transparentes (como `WorkspaceRevision`), con el
//     string construible por literal de tupla `ChangeSetId("…".into())`.
//   - `RiskAssessment { level: RiskLevel, reasons: Vec<String> }` con `enum RiskLevel { Low, .. }`.
//   - `SemanticDiff` y `ValidationReport` derivan `Default` (diff/validación vacíos = mínimos).

/// Construye un `ChangeSet` mínimo (sin operaciones, análisis vacíos) para los tests de forma.
fn changeset_minimo() -> ChangeSet {
    ChangeSet {
        id: ChangeSetId("cs-1".into()),
        base_revision: WorkspaceRevision("blake3:base-abc".into()),
        operations: Vec::<NormalizedOperation>::new(),
        plan_hash: PlanHash("blake3:plan-123".into()),
        risk: RiskAssessment {
            level: RiskLevel::Low,
            reasons: Vec::new(),
        },
        semantic_diff: SemanticDiff::default(),
        validation: ValidationReport::default(),
        expires_at: "2026-07-22T00:00:00Z".to_string(),
    }
}

/// Criterio `changeset_shape`: un `ChangeSet` serializado lleva las claves de wire en camelCase
/// `baseWorkspaceRevision`, `planHash` y `expiresAt` (y NO sus formas snake_case), con sus valores.
#[test]
fn changeset_shape() {
    let v = serde_json::to_value(changeset_minimo()).expect("`ChangeSet` debe serializar a JSON");

    assert!(
        v.is_object(),
        "un `ChangeSet` debe serializar a un objeto JSON, fue {v:?}"
    );

    // Las tres claves de wire que el criterio exige, con su valor (los newtypes son strings
    // transparentes).
    assert_eq!(
        v["baseWorkspaceRevision"],
        serde_json::json!("blake3:base-abc"),
        "la revisión base debe salir como `baseWorkspaceRevision` (camelCase con `Workspace`)",
    );
    assert_eq!(
        v["planHash"],
        serde_json::json!("blake3:plan-123"),
        "el hash del plan debe salir como `planHash`",
    );
    assert_eq!(
        v["expiresAt"],
        serde_json::json!("2026-07-22T00:00:00Z"),
        "la caducidad debe salir como `expiresAt`",
    );

    // El resto de las claves del contrato deben estar presentes (en camelCase).
    for clave in ["id", "operations", "risk", "semanticDiff", "validation"] {
        assert!(
            v.get(clave).is_some(),
            "el `ChangeSet` serializado debe llevar la clave `{clave}`, claves = {:?}",
            v.as_object().map(|o| o.keys().collect::<Vec<_>>()),
        );
    }

    // Blindaje contra un `derive` snake_case o un camelCase ingenuo (`baseRevision`): las formas
    // incorrectas NO deben aparecer.
    for prohibida in [
        "base_revision",
        "baseRevision",
        "plan_hash",
        "expires_at",
        "semantic_diff",
    ] {
        assert!(
            v.get(prohibida).is_none(),
            "el `ChangeSet` NO debe exponer la clave `{prohibida}` (contrato camelCase con rename)",
        );
    }
}

/// Criterio `round-trip serde`: `ChangeSet` sobrevive un ciclo serializar → deserializar sin
/// pérdida (blinda el contrato de wire en ambas direcciones).
#[test]
fn changeset_roundtrip() {
    let original = changeset_minimo();
    let json = serde_json::to_string(&original).expect("`ChangeSet` debe serializar");
    let recuperado: ChangeSet =
        serde_json::from_str(&json).expect("`ChangeSet` debe deserializar desde su propio JSON");
    assert_eq!(
        original, recuperado,
        "el round-trip serde de `ChangeSet` debe ser idéntico",
    );
}

// --- E12-H02: `RiskAssessment` (lógica pura de riesgo) ------------------------------------------
//
// Fase ROJA: la función pura `assess_risk` todavía NO existe en producción. Ubicación ASUMIDA:
// un módulo nuevo `lodestar_core::plan` (E12 = planificación de cambios; el riesgo es análisis de
// plan, no diff ni grafo). Firma ASUMIDA:
//
//     pub fn assess_risk(
//         ops: &[NormalizedOperation],
//         bundle_before: &Bundle,
//         bundle_after: &Bundle,
//     ) -> RiskAssessment
//
// Hasta que E12-H02 la defina, estos dos tests hacen ROJO por SÍMBOLO AUSENTE (compile-fail: el
// módulo `plan`/`assess_risk` no existe), lo que impide compilar el binario de tests de este crate.
// Es el rojo esperado y documentado.
//
// Representación del `deprecate` (el enunciado admite dos): se modela como
// `NormalizedOperation::TransitionStatus { path, to: "deprecated" }` — la variante semántica cuyo
// nombre expresa el ciclo de vida (E12-H07). El `bundle_after` refleja ese estado deprecado para
// que `before`/`after` sean coherentes; los backlinks del concepto no cambian con la transición.
//
// Los tests aseveran PROPIEDADES (nivel de riesgo, razón no vacía que menciona el concepto o los
// backlinks), nunca el texto exacto de la razón ni el umbral interno de la heurística.

/// Bundle con un concepto `core.md` (en el `status` dado) al que apuntan 7 conceptos referentes,
/// más un `index.md` mínimo. Sirve para construir el `before` (activo) y el `after` (deprecado) del
/// criterio `riesgo_deprecate_backlinks`.
fn bundle_con_7_backlinks(status_core: &str) -> Bundle {
    let mut files: FileMap = FileMap::new();
    files.insert(
        RelPath::new("core.md").unwrap(),
        format!(
            "---\ntype: N\ntitle: Core\ndescription: d\nstatus: {status_core}\n---\n\n# Core\n"
        ),
    );
    for i in 1..=7 {
        files.insert(
            RelPath::new(&format!("r{i}.md")).unwrap(),
            format!("---\ntype: N\ntitle: R{i}\ndescription: d\n---\n\n[core](/core.md)\n"),
        );
    }
    files.insert(
        RelPath::new("index.md").unwrap(),
        "---\nokf_version: \"0.1\"\n---\n\n# B\n".to_string(),
    );
    Bundle::from_files(files)
}

/// Criterio `riesgo_deprecate_backlinks`: **Dado** un `deprecate` sobre un concepto con 7 backlinks,
/// **Cuando** se evalúa, **Entonces** `level >= Medium` con una razón que lo menciona.
#[test]
fn riesgo_deprecate_backlinks() {
    let antes = bundle_con_7_backlinks("active");
    let despues = bundle_con_7_backlinks("deprecated");

    // Precondición del fixture: `core.md` recibe exactamente 7 backlinks entrantes (r1..r7).
    let entrantes = antes
        .backlinks(&RelPath::new("core.md").unwrap())
        .inbound
        .len();
    assert_eq!(
        entrantes, 7,
        "el fixture debe dar 7 backlinks a core.md, dio {entrantes}",
    );

    let ops = vec![NormalizedOperation::TransitionStatus {
        path: RelPath::new("core.md").unwrap(),
        to: "deprecated".to_string(),
    }];

    let risk = lodestar_core::plan::assess_risk(&ops, &antes, &despues);

    assert!(
        risk.level >= RiskLevel::Medium,
        "deprecar un concepto con 7 backlinks debe ser al menos Medium, fue {:?}",
        risk.level,
    );
    assert!(
        !risk.reasons.is_empty(),
        "un riesgo >= Medium debe justificarse con al menos una razón",
    );
    // La razón debe mencionar el concepto afectado (`core`) o el alcance del blast-radius (los
    // 7 backlinks) — propiedad, no texto exacto.
    assert!(
        risk.reasons
            .iter()
            .any(|r| r.contains("core") || r.contains('7')),
        "alguna razón debe mencionar el concepto (`core`) o sus backlinks (7); razones = {:?}",
        risk.reasons,
    );
}

/// Criterio `riesgo_bajo_aislado`: **Dado** un `patch_frontmatter` sin backlinks afectados,
/// **Cuando** se evalúa, **Entonces** `level: Low`.
#[test]
fn riesgo_bajo_aislado() {
    // Concepto `sola.md` sin ningún referente: nadie le apunta. `index.md` tampoco lo lista.
    let construir = |titulo: &str| -> Bundle {
        let mut files: FileMap = FileMap::new();
        files.insert(
            RelPath::new("sola.md").unwrap(),
            format!(
                "---\ntype: N\ntitle: {titulo}\ndescription: d\nstatus: draft\n---\n\n# Sola\n"
            ),
        );
        files.insert(
            RelPath::new("index.md").unwrap(),
            "---\nokf_version: \"0.1\"\n---\n\n# B\n".to_string(),
        );
        Bundle::from_files(files)
    };
    let antes = construir("Antes");
    let despues = construir("Despues");

    // Precondición del fixture: `sola.md` no recibe backlinks entrantes ni referencias de index.
    let bl = antes.backlinks(&RelPath::new("sola.md").unwrap());
    assert!(
        bl.inbound.is_empty() && bl.index_refs.is_empty(),
        "el fixture debe dejar sola.md sin backlinks, fue inbound={:?} index_refs={:?}",
        bl.inbound,
        bl.index_refs,
    );

    // `patch_frontmatter` que solo cambia el título (cambio aislado, sin tocar relaciones).
    let mut patch = BTreeMap::new();
    patch.insert(
        "title".to_string(),
        Some(serde_yaml::Value::String("Despues".into())),
    );
    let ops = vec![NormalizedOperation::PatchFrontmatter {
        path: RelPath::new("sola.md").unwrap(),
        patch: FrontmatterPatch(patch),
    }];

    let risk = lodestar_core::plan::assess_risk(&ops, &antes, &despues);

    assert_eq!(
        risk.level,
        RiskLevel::Low,
        "un patch de frontmatter sobre un concepto aislado debe ser riesgo Low, fue {:?} (razones {:?})",
        risk.level,
        risk.reasons,
    );
}

// --- E12-H03: `SemanticDiff` (reusa OkfDiff + diagnósticos introducidos/resueltos) --------------
//
// Fase ROJA: la función pura `semantic_diff` todavía NO existe en producción. Ubicación ASUMIDA:
// el módulo `lodestar_core::plan` (paralela a `assess_risk` de E12-H02: análisis de plan, no diff
// crudo — necesita el `Schema` para computar los checks schema-driven, y `core::diff` es puro sin
// dependencia de schema). Firma ASUMIDA:
//
//     pub fn semantic_diff(
//         before: &Bundle,
//         after: &Bundle,
//         schema: &Schema,
//     ) -> SemanticDiff
//
// Hasta que E12-H03 la defina, estos tres tests hacen ROJO por SÍMBOLO AUSENTE (compile-fail: la
// función `plan::semantic_diff` no existe), lo que impide compilar el binario de tests del crate.
// Es el rojo esperado y documentado.
//
// El tipo `SemanticDiff` (E12-H01) ya existe en `core::types` con la forma:
//   { created, modified, deleted, moved, frontmatter_changes, body_changes, relation_changes,
//     diagnostics_introduced: Vec<Check>, diagnostics_resolved: Vec<Check> }
// `created/modified/deleted/moved/*_changes` reutilizan `core::diff::OkfDiff`/`diff_snap`;
// `diagnostics_introduced` = checks de `after` que NO estaban en `before`; `diagnostics_resolved`
// = checks de `before` que NO están en `after`. Los checks componen `analyze` + `validate_schema`
// + `validate_relations` (para que SCHEMA-REQFIELD y REL-* participen del diff de diagnósticos).
//
// Los tests aseveran PROPIEDADES (el path aparece en `created`/`modified`; un check con el `code`
// esperado aparece en `diagnostics_resolved`/`diagnostics_introduced`), nunca la representación
// interna ni el orden.

/// Criterio `diff_created_modified`: **Dado** un plan que crea A y modifica B, **Cuando** se
/// computa el diff, **Entonces** `created` contiene A y `modified` contiene B.
#[test]
fn diff_created_modified() {
    use lodestar_core::schema::Schema;

    let a = RelPath::new("a.md").unwrap();
    let b = RelPath::new("b.md").unwrap();

    // `before`: solo B. `after`: A nuevo + B con el cuerpo modificado.
    let before = Bundle::from_files(fm(&[(
        "b.md",
        "---\ntype: N\ntitle: B\ndescription: d\nstatus: draft\n---\n\n# B\n\ncuerpo original\n",
    )]));
    let after = Bundle::from_files(fm(&[
        (
            "a.md",
            "---\ntype: N\ntitle: A\ndescription: d\nstatus: draft\n---\n\n# A\n\ncuerpo nuevo\n",
        ),
        (
            "b.md",
            "---\ntype: N\ntitle: B\ndescription: d\nstatus: draft\n---\n\n# B\n\ncuerpo MODIFICADO\n",
        ),
    ]));

    let diff = lodestar_core::plan::semantic_diff(&before, &after, &Schema::default());

    assert!(
        diff.created.contains(&a),
        "A (creado en `after`) debe aparecer en `created`; created = {:?}",
        diff.created,
    );
    assert!(
        diff.modified.contains(&b),
        "B (modificado en `after`) debe aparecer en `modified`; modified = {:?}",
        diff.modified,
    );
}

/// Criterio `diff_resuelve_diagnostico`: **Dado** un plan que corrige un `SCHEMA-REQFIELD` (añade
/// el campo obligatorio ausente), **Cuando** se computa el diff, **Entonces** ese diagnóstico
/// aparece en `diagnosticsResolved`.
#[test]
fn diff_resuelve_diagnostico() {
    use lodestar_core::schema::{DocType, Schema};

    // Schema: el `DocType decision` exige el campo obligatorio `rationale`.
    let mut schema = Schema::default();
    schema.types.insert(
        "decision".to_string(),
        DocType {
            name: "decision".to_string(),
            required_fields: vec!["rationale".to_string()],
            ..DocType::default()
        },
    );

    // `before`: `d.md` (decision) SIN `rationale` → viola SCHEMA-REQFIELD.
    let before = Bundle::from_files(fm(&[(
        "d.md",
        "---\ntype: decision\ntitle: Migrar\nstatus: proposed\n---\n\n# H\n\ncuerpo\n",
    )]));
    // `after`: el mismo concepto CON `rationale` → deja de violar SCHEMA-REQFIELD.
    let after = Bundle::from_files(fm(&[(
        "d.md",
        "---\ntype: decision\ntitle: Migrar\nstatus: proposed\nrationale: porque sí\n---\n\n# H\n\ncuerpo\n",
    )]));

    // Precondición del fixture: `before` viola el requisito y `after` lo cumple (aísla el criterio).
    use lodestar_core::schema::validate_schema;
    assert!(
        validate_schema(&before, &schema)
            .iter()
            .any(|c| c.code == CheckCode::SchemaReqfield),
        "el fixture `before` debe violar SCHEMA-REQFIELD",
    );
    assert!(
        !validate_schema(&after, &schema)
            .iter()
            .any(|c| c.code == CheckCode::SchemaReqfield),
        "el fixture `after` debe corregir SCHEMA-REQFIELD",
    );

    let diff = lodestar_core::plan::semantic_diff(&before, &after, &schema);

    assert!(
        diff.diagnostics_resolved
            .iter()
            .any(|c| c.code == CheckCode::SchemaReqfield),
        "el `SCHEMA-REQFIELD` corregido debe aparecer en `diagnostics_resolved`; resolved = {:?}",
        diff.diagnostics_resolved,
    );
}

/// Criterio `diff_introduce_diagnostico`: **Dado** un plan que rompe una relación (target
/// existente pasa a inexistente), **Cuando** se computa el diff, **Entonces** el diagnóstico
/// `REL-TARGET`/`REL-TYPE` aparece en `diagnosticsIntroduced`.
#[test]
fn diff_introduce_diagnostico() {
    use lodestar_core::schema::{DocType, RelationDef, Schema};

    // Schema: `character.appears_in` apunta a tipos `chapter`, cardinalidad libre (`many`).
    let mut schema = Schema::default();
    schema.types.insert(
        "chapter".to_string(),
        DocType {
            name: "chapter".to_string(),
            ..DocType::default()
        },
    );
    schema.types.insert(
        "character".to_string(),
        DocType {
            name: "character".to_string(),
            relations: BTreeMap::from([(
                "appears_in".to_string(),
                RelationDef {
                    target_types: vec!["chapter".to_string()],
                    cardinality: "many".to_string(),
                },
            )]),
            ..DocType::default()
        },
    );

    // `cap.md` (chapter) existe en ambos estados; solo cambia el target de la relación de `juan`.
    let cap = (
        "cap.md",
        "---\ntype: chapter\ntitle: Capitulo\n---\n\n# Capitulo\n\ncuerpo\n",
    );
    // `before`: `juan.appears_in` → `cap.md` (existe, tipo válido): relación conforme.
    let before = Bundle::from_files(fm(&[
        (
            "juan.md",
            "---\ntype: character\ntitle: Juan\nappears_in:\n  - cap.md\n---\n\n# Juan\n\ncuerpo\n",
        ),
        cap,
    ]));
    // `after`: `juan.appears_in` → `capitulo_fantasma.md` (inexistente): rompe la relación.
    let after = Bundle::from_files(fm(&[
        (
            "juan.md",
            "---\ntype: character\ntitle: Juan\nappears_in:\n  - capitulo_fantasma.md\n---\n\n# Juan\n\ncuerpo\n",
        ),
        cap,
    ]));

    // Precondición: `before` no rompe la relación y `after` sí (aísla el criterio).
    use lodestar_core::schema::validate_relations;
    assert!(
        !validate_relations(&before, &schema)
            .iter()
            .any(|c| c.code == CheckCode::RelTarget || c.code == CheckCode::RelType),
        "el fixture `before` debe tener la relación conforme",
    );
    assert!(
        validate_relations(&after, &schema)
            .iter()
            .any(|c| c.code == CheckCode::RelTarget || c.code == CheckCode::RelType),
        "el fixture `after` debe romper la relación",
    );

    let diff = lodestar_core::plan::semantic_diff(&before, &after, &schema);

    assert!(
        diff.diagnostics_introduced
            .iter()
            .any(|c| c.code == CheckCode::RelTarget || c.code == CheckCode::RelType),
        "la relación rota debe aparecer como `REL-TARGET`/`REL-TYPE` en `diagnostics_introduced`; \
         introduced = {:?}",
        diff.diagnostics_introduced,
    );
}

// --- E12-H04: `ValidationReport` (conformidad del resultado hipotético + policy) ----------------
//
// Fase ROJA: ni la función pura `validate_result` ni la política `can_apply`/`PlanPolicy` existen
// todavía en producción. Ubicación ASUMIDA: el módulo `lodestar_core::plan` (paralela a
// `assess_risk`/`semantic_diff`: análisis del plan, y necesita el `Schema` para contar los checks
// schema-driven —SCHEMA-*/REL-*— del resultado hipotético). Firmas ASUMIDAS:
//
//     pub fn validate_result(bundle: &Bundle, schema: &Schema) -> ValidationReport
//
//     pub struct PlanPolicy {
//         pub require_conformant_result: bool,  // wire `requireConformantResult`
//         pub allow_warnings: bool,             // wire `allowWarnings`
//     }
//     pub fn can_apply(report: &ValidationReport, policy: &PlanPolicy) -> bool
//
// Semántica ASUMIDA (spec E12-H04, `REFACTOR §11.1`):
//   - `validate_result` compone `analyze()` + `validate_schema` + `validate_relations` sobre el
//     bundle hipotético; `summary` cuenta Err/Warn/Info; `conformant = (summary.errors == 0)`;
//     `diagnostics` acumula los `Check`.
//   - `can_apply`: si `require_conformant_result` y NO conforme → false; si `!allow_warnings` y hay
//     warnings → false; en otro caso → true.
//
// El tipo `ValidationReport { conformant, summary{errors,warnings,info}, diagnostics }` (E12-H01)
// ya existe en `core::types`. Hasta que E12-H04 defina `validate_result`/`can_apply`/`PlanPolicy`,
// estos dos tests hacen ROJO por SÍMBOLO AUSENTE (compile-fail: `plan::validate_result`,
// `plan::can_apply` y `plan::PlanPolicy` no existen), lo que impide compilar el binario de tests
// del crate. Es el rojo esperado y documentado.
//
// Los tests aseveran PROPIEDADES (conformidad, conteos, decisión de la política), nunca la
// representación interna ni el orden de los diagnósticos.

/// Criterio `plan_no_conforme_rechaza`: **Dado** un plan cuyo resultado introduce un `Err` (un
/// concepto `decision` sin el campo obligatorio `rationale` → `SCHEMA-REQFIELD`/Err) y
/// `policy.requireConformantResult:true`, **Cuando** se valida, **Entonces** `conformant:false` y
/// el plan NO es aplicable (`can_apply == false`).
/// (Benchmark §17: "Crear un concepto sin campo obligatorio → plan rechazado".)
#[test]
fn plan_no_conforme_rechaza() {
    use lodestar_core::plan::{can_apply, validate_result, PlanPolicy};
    use lodestar_core::schema::{validate_schema, DocType, Schema};

    // Schema: el `DocType decision` exige el campo obligatorio `rationale`.
    let mut schema = Schema::default();
    schema.types.insert(
        "decision".to_string(),
        DocType {
            name: "decision".to_string(),
            required_fields: vec!["rationale".to_string()],
            ..DocType::default()
        },
    );

    // Bundle hipotético resultante del plan: un concepto `decision` SIN `rationale` → Err duro.
    let hipotetico = Bundle::from_files(fm(&[(
        "d.md",
        "---\ntype: decision\ntitle: Migrar a Rust\nstatus: proposed\n---\n\n# H\n\ncuerpo\n",
    )]));

    // Precondición del fixture: el resultado hipotético viola SCHEMA-REQFIELD (aísla el criterio).
    assert!(
        validate_schema(&hipotetico, &schema)
            .iter()
            .any(|c| c.code == CheckCode::SchemaReqfield && c.level == Severity::Err),
        "el bundle hipotético debe introducir un `SCHEMA-REQFIELD`/Err",
    );

    let report = validate_result(&hipotetico, &schema);

    assert!(
        report.summary.errors >= 1,
        "el resultado con un Err debe contar al menos un error; summary = {:?}",
        report.summary,
    );
    assert!(
        !report.conformant,
        "con errores el resultado NO es conforme (`conformant == false`); report = {:?}",
        report,
    );

    let policy = PlanPolicy {
        require_conformant_result: true,
        allow_warnings: true,
    };
    assert!(
        !can_apply(&report, &policy),
        "con `requireConformantResult:true` y resultado no conforme, el plan NO es aplicable",
    );
}

/// Criterio `plan_warnings_permitido`: **Dado** un plan con SOLO warnings (ningún `Err`) y
/// `allowWarnings:true`, **Cuando** se valida, **Entonces** el resultado es conforme
/// (`conformant:true`, 0 errores, `summary.warnings >= 1`) y el plan es aplicable
/// (`can_apply == true`).
#[test]
fn plan_warnings_permitido() {
    use lodestar_core::plan::{can_apply, validate_result, PlanPolicy};
    use lodestar_core::schema::Schema;

    // Bundle hipotético con SOLO warnings: concepto conforme (type/title/description presentes,
    // cuerpo con encabezado) salvo `tags` como escalar → `FMT-TAGS`/Warn. Sin schema activo, no
    // hay checks SCHEMA-*; sin enlaces rotos, no hay más que el warning (y algún Info como ORPHAN,
    // que no afecta a la conformidad).
    let hipotetico = Bundle::from_files(fm(&[(
        "nota.md",
        "---\ntype: Nota\ntitle: T\ndescription: d\ntags: uno\n---\n\n# H\n\ncuerpo\n",
    )]));

    // Precondición del fixture: 0 errores duros (solo warnings) sobre el análisis base.
    assert_eq!(
        hipotetico.analyze().hard_fail,
        0,
        "el bundle hipotético no debe tener ningún Err (solo warnings)",
    );

    let report = validate_result(&hipotetico, &Schema::default());

    assert_eq!(
        report.summary.errors, 0,
        "un resultado con solo warnings tiene 0 errores; summary = {:?}",
        report.summary,
    );
    assert!(
        report.conformant,
        "sin errores el resultado es conforme (`conformant == true`); report = {:?}",
        report,
    );
    assert!(
        report.summary.warnings >= 1,
        "el fixture debe producir al menos un warning (FMT-TAGS); summary = {:?}",
        report.summary,
    );

    let policy = PlanPolicy {
        require_conformant_result: true,
        allow_warnings: true,
    };
    assert!(
        can_apply(&report, &policy),
        "con resultado conforme y `allowWarnings:true`, el plan es aplicable",
    );
}

// --- E12-H05: normalización de operaciones de contenido -----------------------------------------
//
// Fase ROJA: los normalizadores puros de contenido todavía NO existen en producción. Ubicación
// ASUMIDA: el módulo `lodestar_core::plan` (junto a `assess_risk`/`semantic_diff`/`validate_result`
// — es análisis/normalización de plan, y el core es puro). Firmas ASUMIDAS (documentadas por el
// autor de tests; el implementador queda vinculado a ellas):
//
//   pub fn normalize_create(
//       bundle: &Bundle, schema: &Schema, path: &RelPath,
//       doctype: &str, title: Option<&str>, body: Option<String>,
//   ) -> Result<NormalizedOperation, CoreError>;
//   pub fn normalize_replace_text(
//       bundle: &Bundle, path: &RelPath,
//       find: &str, replace: &str, expected_occurrences: Option<usize>,
//   ) -> Result<NormalizedOperation, CoreError>;
//   pub fn normalize_edit_section(
//       bundle: &Bundle, path: &RelPath,
//       heading_path: &[String], mode: EditSectionMode, content: &str,
//   ) -> Result<NormalizedOperation, CoreError>;
//
// Forma RESUELTA de la `NormalizedOperation` de salida (contrato que estos tests fijan): como el
// tipo `NormalizedOperation::EditSection` NO tiene campo para el cuerpo final completo (solo
// `heading_path`/`mode`/`content`), una operación de sección "resuelta a la escritura concreta"
// (E12-H01: "cada una resuelta a las escrituras concretas que producirá") solo puede llevar el
// cuerpo final en `ReplaceBody { path, body }`. Por eso este autor ASUME que `normalize_edit_section`
// devuelve un `NormalizedOperation::ReplaceBody` con el cuerpo entero ya reescrito. `normalize_create`
// devuelve `NormalizedOperation::Create { body: Some(<plantilla resuelta>), .. }` (el propio tipo
// `Create` porta `body: Option<String>`).
//
// Dónde vive la lógica de secciones: hoy `parse_headings`/`locate_section`/`extract_sections` son
// funciones PRIVADAS de `lodestar-app` (E10-H10, `crates/lodestar-app/src/lib.rs`). Como esta
// normalización es del core PURO, este autor ASUME que la lógica de localización de secciones se
// MUEVE a `core` (lo natural: `core::model`, donde ya viven `parse_file`/`build_raw`/`split_front`,
// o `core::plan`) y que `lodestar-app::knowledge_get` pasa a reusarla. El test extra
// `edit_section_ignora_code_fence` cierra la reserva documentada de E10-H10: `parse_headings` NO
// reconoce hoy los bloques de código fenceados (` ``` `) y confundiría un `#` interno con un heading.
//
// Hasta que E12-H05 defina los tres normalizadores, estos cuatro tests hacen ROJO por SÍMBOLO
// AUSENTE (compile-fail: `plan::normalize_create`/`normalize_replace_text`/`normalize_edit_section`
// no existen), lo que impide compilar el binario de tests del crate. Es el rojo esperado.

/// Extrae el cuerpo final de una operación de contenido ya normalizada. Este autor fija que
/// `edit_section` se resuelve a un `ReplaceBody` (ver comentario de sección): cualquier otra
/// variante es un fallo del contrato acordado.
fn cuerpo_resuelto(op: &NormalizedOperation) -> &str {
    match op {
        NormalizedOperation::ReplaceBody { body, .. } => body,
        otro => panic!(
            "una operación de sección normalizada debe resolverse a `ReplaceBody` con el cuerpo \
             final completo; fue {otro:?}",
        ),
    }
}

/// Criterio `create_usa_plantilla`: **Dado** un `create` SIN body para un `DocType` con
/// `bodyTemplate`, **Cuando** se normaliza, **Entonces** el cuerpo sale de la plantilla (con
/// `{title}` sustituido). Se aseveran PROPIEDADES (el cuerpo lleva el marcador distintivo de la
/// plantilla, sustituye el título y no deja el placeholder crudo), no el texto exacto.
#[test]
fn create_usa_plantilla() {
    use lodestar_core::schema::{DocType, Schema};

    // Bundle mínimo (solo el index raíz): el concepto a crear todavía no existe.
    let b = Bundle::from_files(fm(&[(
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# B\n",
    )]));

    // Schema: el `DocType decision` trae una `bodyTemplate` con un marcador inequívoco y `{title}`.
    let mut schema = Schema::default();
    schema.types.insert(
        "decision".to_string(),
        DocType {
            name: "decision".to_string(),
            body_template: Some(
                "## Contexto\n\nDecisión sobre {title}.\n\n## Consecuencias\n".to_string(),
            ),
            ..DocType::default()
        },
    );

    let path = RelPath::new("decisiones/usar-rust.md").unwrap();
    let op = match lodestar_core::plan::normalize_create(
        &b,
        &schema,
        &path,
        "decision",
        Some("Usar Rust"),
        None, // sin body ⇒ debe salir de la plantilla
    ) {
        Ok(op) => op,
        Err(_) => panic!("crear un concepto con plantilla válida no debe fallar la normalización"),
    };

    let NormalizedOperation::Create {
        path: create_path,
        body,
        ..
    } = &op
    else {
        panic!("`create` debe normalizarse a `NormalizedOperation::Create`, fue {op:?}");
    };
    assert_eq!(create_path, &path, "el path resuelto debe ser el pedido");

    let cuerpo = body
        .as_ref()
        .expect("`create` sin body sobre un DocType con plantilla debe rellenar `body: Some(..)`");
    assert!(
        cuerpo.contains("## Contexto"),
        "el cuerpo debe provenir de la plantilla (marcador `## Contexto`); cuerpo = {cuerpo:?}",
    );
    assert!(
        cuerpo.contains("Usar Rust"),
        "la plantilla debe sustituir `{{title}}` por el título; cuerpo = {cuerpo:?}",
    );
    assert!(
        !cuerpo.contains("{title}"),
        "el placeholder `{{title}}` no debe quedar crudo en el cuerpo; cuerpo = {cuerpo:?}",
    );
}

/// Criterio `replace_text_ocurrencias`: **Dado** `replace_text` con `expectedOccurrences:1` y 2
/// coincidencias, **Cuando** se normaliza, **Entonces** error (no aplica). Se añade un control
/// positivo con `expectedOccurrences:2` (el número correcto) para probar que el fallo es
/// EXACTAMENTE el desajuste de conteo y no otro error del fixture.
#[test]
fn replace_text_ocurrencias() {
    // Cuerpo con la palabra `token` EXACTAMENTE dos veces.
    let b = Bundle::from_files(fm(&[(
        "auth.md",
        "---\ntype: guide\ntitle: Auth\ndescription: d\nstatus: draft\n---\n\n# Auth\n\n\
         El token se envía en el header. Renueva el token cada hora.\n",
    )]));
    let path = RelPath::new("auth.md").unwrap();

    // `expectedOccurrences:1` pero hay 2 coincidencias ⇒ error, no aplica.
    let desajuste =
        lodestar_core::plan::normalize_replace_text(&b, &path, "token", "secreto", Some(1));
    assert!(
        desajuste.is_err(),
        "con `expectedOccurrences:1` y 2 coincidencias la normalización debe fallar",
    );

    // Control positivo: con el número correcto (2) sí normaliza.
    let acierto =
        lodestar_core::plan::normalize_replace_text(&b, &path, "token", "secreto", Some(2));
    assert!(
        acierto.is_ok(),
        "con `expectedOccurrences:2` (el número real) la normalización debe tener éxito, \
         demostrando que el error anterior era el desajuste de conteo",
    );
}

/// Criterio `edit_section_acotado`: **Dado** `edit_section(["Security","Token rotation"],
/// mode:replace)`, **Cuando** se normaliza, **Entonces** SOLO esa subsección cambia (su heading se
/// conserva, su contenido se reemplaza; las secciones hermanas y de otro nivel quedan intactas).
#[test]
fn edit_section_acotado() {
    let raw = "---\ntype: guide\ntitle: Seguridad\ndescription: d\nstatus: draft\n---\n\n\
               # Security\n\nIntroducción a la seguridad.\n\n\
               ## Token rotation\n\nRotar cada 90 días.\n\n\
               ## Password policy\n\nMínimo 12 caracteres.\n\n\
               # Deployment\n\nDesplegar con CI.\n";
    let b = Bundle::from_files(fm(&[("seguridad.md", raw)]));
    let path = RelPath::new("seguridad.md").unwrap();

    let heading_path = vec!["Security".to_string(), "Token rotation".to_string()];
    let op = match lodestar_core::plan::normalize_edit_section(
        &b,
        &path,
        &heading_path,
        EditSectionMode::Replace,
        "Rotar cada 24 horas.",
    ) {
        Ok(op) => op,
        Err(_) => panic!("un `headingPath` existente no debe fallar la normalización"),
    };

    let cuerpo = cuerpo_resuelto(&op);
    // La subsección objetivo se reemplaza: contenido nuevo dentro, contenido viejo fuera.
    assert!(
        cuerpo.contains("Rotar cada 24 horas."),
        "el contenido nuevo debe estar en la subsección editada; cuerpo = {cuerpo:?}",
    );
    assert!(
        !cuerpo.contains("Rotar cada 90 días."),
        "el contenido viejo de la subsección editada debe desaparecer; cuerpo = {cuerpo:?}",
    );
    // El heading de la subsección se conserva (mode:replace reemplaza el contenido, no el título).
    assert!(
        cuerpo.contains("## Token rotation"),
        "el heading de la subsección editada debe conservarse; cuerpo = {cuerpo:?}",
    );
    // Las hermanas y las secciones de otro nivel quedan INTACTAS.
    assert!(
        cuerpo.contains("Mínimo 12 caracteres."),
        "la subsección hermana `Password policy` no debe tocarse; cuerpo = {cuerpo:?}",
    );
    assert!(
        cuerpo.contains("Desplegar con CI."),
        "la sección de nivel superior `Deployment` no debe tocarse; cuerpo = {cuerpo:?}",
    );
    assert!(
        cuerpo.contains("Introducción a la seguridad."),
        "el preámbulo de `Security` (fuera de la subsección) no debe tocarse; cuerpo = {cuerpo:?}",
    );
}

/// Criterio EXTRA `edit_section_ignora_code_fence` (cierra la reserva de E10-H10): un cuerpo con un
/// heading FALSO dentro de un bloque de código fenceado (` ``` `). Un `edit_section` sobre una
/// sección real NO debe confundir ese `#` interno con un heading (lo que TRUNCARÍA el rango de la
/// sección al detectar un "hermano" espurio). Con el bug de E10-H10, la sección `Uso` acabaría
/// justo antes del `#` del bloque de código, dejando fuera (sin reemplazar) el propio bloque y el
/// texto posterior; el fix lo evita.
#[test]
fn edit_section_ignora_code_fence() {
    let raw = r#"---
type: guide
title: Uso
description: d
status: draft
---

# Uso

Ejemplo de configuración:

```bash
# Este comentario NO es un heading
export TOKEN=abc
```

Texto después del bloque de código.

# Referencias

Ver el manual.
"#;
    let b = Bundle::from_files(fm(&[("uso.md", raw)]));
    let path = RelPath::new("uso.md").unwrap();

    let heading_path = vec!["Uso".to_string()];
    let op = match lodestar_core::plan::normalize_edit_section(
        &b,
        &path,
        &heading_path,
        EditSectionMode::Replace,
        "NUEVO CUERPO DE USO",
    ) {
        Ok(op) => op,
        Err(_) => panic!("editar la sección `Uso` no debe fallar por el heading falso del fence"),
    };

    let cuerpo = cuerpo_resuelto(&op);
    // El contenido nuevo debe estar.
    assert!(
        cuerpo.contains("NUEVO CUERPO DE USO"),
        "el contenido nuevo debe reemplazar toda la sección `Uso`; cuerpo = {cuerpo:?}",
    );
    // DISCRIMINADORES: todo lo que estaba DENTRO de `Uso` (incl. el bloque de código y el texto
    // posterior) debe haber sido reemplazado. Con el bug del code fence, el rango se truncaría en
    // el `#` interno y estos supervivirían.
    assert!(
        !cuerpo.contains("export TOKEN=abc"),
        "el bloque de código (dentro de `Uso`) debe reemplazarse, no sobrevivir por un rango \
         truncado en el `#` falso; cuerpo = {cuerpo:?}",
    );
    assert!(
        !cuerpo.contains("# Este comentario NO es un heading"),
        "el `#` dentro del fence no es un heading real y su línea debe reemplazarse con el resto \
         de `Uso`; cuerpo = {cuerpo:?}",
    );
    assert!(
        !cuerpo.contains("Texto después del bloque de código."),
        "el texto tras el fence (aún dentro de `Uso`) debe reemplazarse; cuerpo = {cuerpo:?}",
    );
    // La sección real SIGUIENTE queda intacta (guarda de que no arrasamos de más).
    assert!(
        cuerpo.contains("# Referencias") && cuerpo.contains("Ver el manual."),
        "la sección `Referencias` (fuera de `Uso`) debe quedar intacta; cuerpo = {cuerpo:?}",
    );
}

// --- E12-H06: Normalización de operaciones de estructura (`move`, `delete`) ---
//
// Fase ROJA: los normalizadores puros de ESTRUCTURA todavía NO existen en producción. Ubicación
// ASUMIDA: el módulo `lodestar_core::plan` (junto a `normalize_create`/`normalize_replace_text`/
// `normalize_edit_section` de E12-H05 y a `assess_risk`/`semantic_diff` — es análisis/normalización
// de plan, y el core es puro). A diferencia de los normalizadores de contenido, estos producen
// VARIAS `NormalizedOperation` (el rename/borrado + las reescrituras/eliminaciones de los enlaces
// entrantes dentro del MISMO change set), por eso devuelven `Vec<NormalizedOperation>`.
//
// Firmas ASUMIDAS (documentadas por el autor de tests; el implementador queda vinculado a ellas):
//
//   pub fn normalize_move(
//       bundle: &Bundle, from: &RelPath, to: &RelPath, rewrite_inbound_links: bool,
//   ) -> Result<Vec<NormalizedOperation>, CoreError>;
//   pub fn normalize_delete(
//       bundle: &Bundle, path: &RelPath, policy: InboundLinksPolicy,
//   ) -> Result<Vec<NormalizedOperation>, CoreError>;
//
// Forma RESUELTA del `Vec` de salida (contrato que estos tests fijan):
//   * `normalize_move(.., rewrite:true)` → un `NormalizedOperation::Move { from, to, .. }` MÁS,
//     por cada concepto que enlaza a `from`, una operación que reescribe ese enlace a `to`. Como el
//     enlace vive en el CUERPO (`[x](/from.md)`), la reescritura natural es un `ReplaceBody` del
//     concepto entrante con el href actualizado a `/to.md`. Estos tests NO exigen la variante exacta
//     (aceptan cualquier op de contenido cuyo `path` sea el entrante), pero SÍ exigen que el enlace
//     quede realmente reescrito: la op referencia `/destino.md` y ya NO `/target.md`.
//   * `normalize_delete(.., Reject)` sobre un concepto con entrantes → `Err`. El error DEBE ser la
//     variante de `CoreError` que mapea a `ErrorCode::InboundLinksExist` (wire "INBOUND_LINKS_EXIST",
//     definido en `types.rs`). Como hoy `CoreError` NO tiene esa variante, el implementador debe
//     añadirla con ese nombre (`CoreError::InboundLinksExist`, alineado con `ErrorCode`). La aserción
//     es AGNÓSTICA a la forma del payload (tupla/struct/unit): comprueba que el nombre de la variante
//     aparece en el `Debug` del error. Ver `delete_referenciado_rechaza`.
//   * `normalize_delete(.., RemoveLinks)` → un `NormalizedOperation::Delete { path, .. }` MÁS, por
//     cada entrante, una op que quita el enlace (op de contenido cuyo `path` es el entrante y cuyo
//     `Debug` ya NO contiene `/target.md`).
//
// Hasta que E12-H06 defina ambos normalizadores, estos tres tests hacen ROJO por SÍMBOLO AUSENTE
// (compile-fail: `plan::normalize_move`/`plan::normalize_delete` — y la variante de error — no
// existen), lo que impide compilar el binario de tests del crate. Es el rojo esperado.

/// Path del concepto tocado por una op de CONTENIDO (reescritura o eliminación de enlace). Las ops
/// estructurales (`Move`/`Delete`) devuelven `None`: se filtran para aislar las ops de enlace.
fn path_op_enlace(op: &NormalizedOperation) -> Option<RelPath> {
    match op {
        NormalizedOperation::ReplaceBody { path, .. }
        | NormalizedOperation::PatchFrontmatter { path, .. }
        | NormalizedOperation::ReplaceText { path, .. }
        | NormalizedOperation::EditSection { path, .. } => Some(path.clone()),
        _ => None,
    }
}

/// Lista ordenada y deduplicada de paths (para comparar conjuntos de entrantes sin depender del orden).
fn paths_ordenados(mut v: Vec<RelPath>) -> Vec<RelPath> {
    v.sort();
    v.dedup();
    v
}

/// Criterio `move_reescribe_entrantes`: **Dado** un `move` con `rewriteInboundLinks:true` y 30
/// backlinks, **Cuando** se normaliza, **Entonces** el change set incluye el rename MÁS la
/// reescritura de los 30 enlaces entrantes.
#[test]
fn move_reescribe_entrantes() {
    let from = RelPath::new("target.md").unwrap();
    let to = RelPath::new("destino.md").unwrap();

    // Bundle: index raíz + `target.md` + 30 conceptos `r1.md`..`r30.md`, cada uno con un enlace de
    // cuerpo `[target](/target.md)`.
    let mut files: FileMap = FileMap::new();
    files.insert(
        RelPath::new("index.md").unwrap(),
        "---\nokf_version: \"0.1\"\n---\n\n# B\n".to_string(),
    );
    files.insert(
        from.clone(),
        "---\ntype: N\ntitle: Target\ndescription: d\n---\n\n# Target\n".to_string(),
    );
    let mut entrantes_esperados: Vec<RelPath> = Vec::new();
    for i in 1..=30 {
        let p = RelPath::new(&format!("r{i}.md")).unwrap();
        files.insert(
            p.clone(),
            format!("---\ntype: N\ntitle: R{i}\ndescription: d\n---\n\n[target](/target.md)\n"),
        );
        entrantes_esperados.push(p);
    }
    let entrantes_esperados = paths_ordenados(entrantes_esperados);
    let b = Bundle::from_files(files);

    // Precondición del fixture: `target.md` recibe exactamente 30 backlinks entrantes.
    let inbound = b.backlinks(&from).inbound.len();
    assert_eq!(
        inbound, 30,
        "el fixture debe dar 30 backlinks a target.md, dio {inbound}",
    );

    let ops = lodestar_core::plan::normalize_move(&b, &from, &to, true)
        .expect("mover un concepto con backlinks y rewrite:true no debe fallar la normalización");

    // 1) Hay exactamente UN rename `Move { from: target, to: destino }`.
    let moves: Vec<&NormalizedOperation> = ops
        .iter()
        .filter(|op| matches!(op, NormalizedOperation::Move { .. }))
        .collect();
    assert_eq!(
        moves.len(),
        1,
        "el change set debe incluir exactamente un `Move`; ops = {ops:?}",
    );
    let NormalizedOperation::Move {
        from: mv_from,
        to: mv_to,
        ..
    } = moves[0]
    else {
        unreachable!()
    };
    assert_eq!(mv_from, &from, "el `Move` debe renombrar desde `target.md`");
    assert_eq!(mv_to, &to, "el `Move` debe renombrar hacia `destino.md`");

    // 2) El resto de ops son reescrituras de enlace, una por cada uno de los 30 entrantes.
    let reescrituras: Vec<&NormalizedOperation> = ops
        .iter()
        .filter(|op| !matches!(op, NormalizedOperation::Move { .. }))
        .collect();
    let paths_reescritos = paths_ordenados(
        reescrituras
            .iter()
            .map(|op| {
                path_op_enlace(op).unwrap_or_else(|| {
                    panic!(
                        "toda op no-`Move` del change set debe ser una reescritura de contenido de \
                         un concepto entrante; fue {op:?}",
                    )
                })
            })
            .collect(),
    );
    assert_eq!(
        paths_reescritos, entrantes_esperados,
        "debe haber una reescritura para cada uno de los 30 entrantes (sin faltar ni sobrar); \
         reescritos = {paths_reescritos:?}",
    );

    // 3) DISCRIMINADOR: cada reescritura apunta ya al nuevo destino y NO conserva el href viejo (una
    // op vacua que dejara `/target.md` pasaría el conteo pero fallaría aquí).
    for op in &reescrituras {
        let dbg = format!("{op:?}");
        assert!(
            dbg.contains("/destino.md"),
            "la reescritura de un entrante debe apuntar al nuevo href `/destino.md`; op = {op:?}",
        );
        assert!(
            !dbg.contains("/target.md"),
            "la reescritura no debe conservar el href viejo `/target.md`; op = {op:?}",
        );
    }
}

/// Criterio `delete_referenciado_rechaza`: **Dado** un `delete` con `inboundLinksPolicy` por defecto
/// (`reject`) sobre un concepto referenciado, **Cuando** se normaliza, **Entonces** se rechaza con
/// `INBOUND_LINKS_EXIST`.
///
/// Cómo se asevera el rechazo: `normalize_delete(.., Reject)` devuelve `Err`, y el `Debug` del error
/// contiene el nombre de la variante `InboundLinksExist` — es decir, la variante de `CoreError` que
/// el implementador debe añadir alineada con `ErrorCode::InboundLinksExist` (wire "INBOUND_LINKS_EXIST").
/// La comprobación por `Debug` es agnóstica a la forma del payload de la variante (tupla/struct/unit).
#[test]
fn delete_referenciado_rechaza() {
    let target = RelPath::new("target.md").unwrap();

    // Guarda de coherencia con `types.rs`: el `ErrorCode` esperado mapea a este wire.
    assert_eq!(ErrorCode::InboundLinksExist.as_str(), "INBOUND_LINKS_EXIST");

    let b = Bundle::from_files(fm(&[
        ("index.md", "---\nokf_version: \"0.1\"\n---\n\n# B\n"),
        (
            "target.md",
            "---\ntype: N\ntitle: Target\ndescription: d\n---\n\n# Target\n",
        ),
        (
            "r1.md",
            "---\ntype: N\ntitle: R1\ndescription: d\n---\n\n[target](/target.md)\n",
        ),
    ]));

    // Precondición del fixture: `target.md` está referenciado (>= 1 entrante).
    assert!(
        !b.backlinks(&target).inbound.is_empty(),
        "el fixture debe dejar target.md con al menos un entrante",
    );

    let err = lodestar_core::plan::normalize_delete(&b, &target, InboundLinksPolicy::Reject)
        .expect_err(
            "borrar un concepto referenciado con la política por defecto `reject` debe fallar",
        );

    let dbg = format!("{err:?}");
    assert!(
        dbg.contains("InboundLinksExist"),
        "el rechazo debe ser la variante de `CoreError` que mapea a `ErrorCode::InboundLinksExist` \
         (wire \"INBOUND_LINKS_EXIST\"); error = {err:?}",
    );
}

/// Criterio `delete_remove_links`: **Dado** un `delete` con `remove_links` sobre un concepto
/// referenciado, **Cuando** se normaliza, **Entonces** el change set incluye el borrado MÁS quitar
/// esos enlaces en los conceptos entrantes.
#[test]
fn delete_remove_links() {
    let target = RelPath::new("target.md").unwrap();

    let b = Bundle::from_files(fm(&[
        ("index.md", "---\nokf_version: \"0.1\"\n---\n\n# B\n"),
        (
            "target.md",
            "---\ntype: N\ntitle: Target\ndescription: d\n---\n\n# Target\n",
        ),
        (
            "r1.md",
            "---\ntype: N\ntitle: R1\ndescription: d\n---\n\n[target](/target.md)\n",
        ),
        (
            "r2.md",
            "---\ntype: N\ntitle: R2\ndescription: d\n---\n\n[target](/target.md)\n",
        ),
    ]));

    // Precondición del fixture: exactamente 2 entrantes a `target.md`.
    let inbound = b.backlinks(&target).inbound.len();
    assert_eq!(
        inbound, 2,
        "el fixture debe dar 2 backlinks a target.md, dio {inbound}",
    );
    let entrantes_esperados = paths_ordenados(vec![
        RelPath::new("r1.md").unwrap(),
        RelPath::new("r2.md").unwrap(),
    ]);

    let ops = lodestar_core::plan::normalize_delete(&b, &target, InboundLinksPolicy::RemoveLinks)
        .expect("borrar con `remove_links` sobre un concepto referenciado no debe fallar");

    // 1) Hay exactamente un `Delete { path: target }`.
    let deletes: Vec<&NormalizedOperation> = ops
        .iter()
        .filter(|op| matches!(op, NormalizedOperation::Delete { .. }))
        .collect();
    assert_eq!(
        deletes.len(),
        1,
        "el change set debe incluir exactamente un `Delete`; ops = {ops:?}",
    );
    let NormalizedOperation::Delete { path: del_path, .. } = deletes[0] else {
        unreachable!()
    };
    assert_eq!(del_path, &target, "el `Delete` debe borrar `target.md`");

    // 2) El resto de ops quitan el enlace en cada uno de los 2 entrantes.
    let removidas: Vec<&NormalizedOperation> = ops
        .iter()
        .filter(|op| !matches!(op, NormalizedOperation::Delete { .. }))
        .collect();
    let paths_removidos = paths_ordenados(
        removidas
            .iter()
            .map(|op| {
                path_op_enlace(op).unwrap_or_else(|| {
                    panic!(
                        "toda op no-`Delete` del change set debe quitar el enlace de un concepto \
                         entrante; fue {op:?}",
                    )
                })
            })
            .collect(),
    );
    assert_eq!(
        paths_removidos, entrantes_esperados,
        "debe haber una op que quita el enlace por cada uno de los 2 entrantes; \
         removidos = {paths_removidos:?}",
    );

    // 3) DISCRIMINADOR: tras quitar el enlace, la op del entrante ya NO conserva el href al target
    // borrado (una op vacua que dejara `/target.md` pasaría el conteo pero fallaría aquí).
    for op in &removidas {
        let dbg = format!("{op:?}");
        assert!(
            !dbg.contains("/target.md"),
            "la op debe QUITAR el enlace a `/target.md` del entrante, no conservarlo; op = {op:?}",
        );
    }
}

// --- E12-H07: Normalización de operaciones SEMÁNTICAS -------------------------
// (`add_relation` / `remove_relation` / `transition_status` / `apply_fix`)
//
// Fase ROJA: los normalizadores puros SEMÁNTICOS todavía NO existen en producción. Ubicación
// ASUMIDA: el módulo `lodestar_core::plan` (junto a `normalize_create`/`normalize_move`/… — es
// normalización de plan, y el core es puro, invariante #2). A diferencia de las de estructura,
// estas producen la ESCRITURA CONCRETA ya resuelta (un `PatchFrontmatter`), siguiendo el mismo
// criterio que E12-H05 (`normalize_edit_section` resuelve a `ReplaceBody`): las variantes
// `AddRelation`/`RemoveRelation`/`TransitionStatus`/`ApplyFix` del enum son ops de ALTO NIVEL; el
// normalizador las baja a la escritura resuelta que aplicará el único escritor.
//
// Firmas ASUMIDAS (documentadas por el autor de tests; vinculan al implementador):
//
//   pub fn normalize_add_relation(
//       bundle: &Bundle, schema: &Schema,
//       source: &RelPath, relation: &str, target: &RelPath,
//   ) -> Result<NormalizedOperation, CoreError>;
//   pub fn normalize_remove_relation(
//       bundle: &Bundle, schema: &Schema,
//       source: &RelPath, relation: &str, target: &RelPath,
//   ) -> Result<NormalizedOperation, CoreError>;
//   pub fn normalize_transition_status(
//       bundle: &Bundle, schema: &Schema, reference: &RelPath, to: &str,
//   ) -> Result<NormalizedOperation, CoreError>;
//   pub fn normalize_apply_fix(
//       bundle: &Bundle, schema: &Schema, fix_id: &str,
//   ) -> Result<NormalizedOperation, CoreError>;
//
// Contrato que estos tests fijan:
//   * `normalize_add_relation` valida el target contra la `RelationDef` del `DocType` del `source`
//     (el `type` del target ∈ `RelationDef.target_types`, la cardinalidad no se viola). Si viola,
//     `Err` de la variante de `CoreError` que mapea a `ErrorCode::RelationConstraintViolation`
//     (wire "RELATION_CONSTRAINT_VIOLATION", ya definido en `types.rs`). Como hoy `CoreError` NO
//     tiene esa variante, el implementador debe añadirla con ese nombre
//     (`CoreError::RelationConstraintViolation`). La aserción es AGNÓSTICA al payload: comprueba
//     que el nombre de la variante aparece en el `Debug` del error.
//   * `normalize_transition_status` valida `to` contra `allowed_statuses` del `DocType` del `ref`.
//     Si `to` no está permitido → `Err` (rechazo; la spec no fija un wire concreto, así que solo
//     se exige `is_err`). Si está permitido → `Ok(PatchFrontmatter{ status: to })` (discriminador
//     contra un stub que siempre falle).
//   * `normalize_apply_fix` recomputa los diagnósticos del bundle bajo el schema (analyze +
//     validate_schema + validate_relations) y materializa el `Fix` `safe` cuyo `fix_id` casa.
//
// DIAGNÓSTICO FIXABLE ASUMIDO (decisión del autor, documentada para el implementador):
//   El diagnóstico `REL-TARGET` de una relación tipada ROTA (un target que no existe como
//   concepto) debe emitir un `Fix { fix_id, title, safe: true }` cuyo arreglo es «quitar la
//   relación rota». El `fix_id` es estable (derivable del diagnóstico). `normalize_apply_fix`
//   resuelve ese fix a un `PatchFrontmatter` sobre el concepto origen que QUITA el target roto del
//   campo de la relación (deja de referenciarlo). El test obtiene el `fix_id` recomputando
//   `validate_relations` y leyendo `check.fixes[].fix_id` del primer fix `safe`; hoy los checks NO
//   emiten fixes, así que el implementador debe hacer que `validate_relations` adjunte ese `Fix`.
//
// Hasta que E12-H07 defina los normalizadores (y el `Fix` de `REL-TARGET`), estos tres tests hacen
// ROJO por SÍMBOLO AUSENTE (compile-fail: `plan::normalize_add_relation` /
// `plan::normalize_transition_status` / `plan::normalize_apply_fix` — y la variante de error — no
// existen), lo que impide compilar el binario de tests del crate. Es el rojo esperado.

/// Criterio `add_relation_invalida`: **Dado** `add_relation` que viola la `RelationDef` (el `type`
/// del target no está en `target_types`), **Cuando** se normaliza, **Entonces**
/// `RELATION_CONSTRAINT_VIOLATION`.
///
/// Fixture aislado en el TIPO: `mentor` es cardinalidad `many` con `target_types:[character]`, así
/// que añadir un target de tipo `item` viola SOLO la restricción de tipo (no la cardinalidad).
#[test]
fn add_relation_invalida() {
    use lodestar_core::schema::{DocType, RelationDef, Schema};

    // Guarda de coherencia con `types.rs`: el `ErrorCode` esperado mapea a este wire.
    assert_eq!(
        ErrorCode::RelationConstraintViolation.as_str(),
        "RELATION_CONSTRAINT_VIOLATION"
    );

    // `heroe` (character) quiere añadir `mentor -> espada`, pero `espada` es `item`, no `character`.
    let b = Bundle::from_files(fm(&[
        (
            "heroe.md",
            "---\ntype: character\ntitle: Heroe\n---\n\n# Heroe\n\ncuerpo\n",
        ),
        (
            "espada.md",
            "---\ntype: item\ntitle: Espada\n---\n\n# Espada\n\ncuerpo\n",
        ),
    ]));

    let mut schema = Schema::default();
    schema.types.insert(
        "item".to_string(),
        DocType {
            name: "item".to_string(),
            ..DocType::default()
        },
    );
    schema.types.insert(
        "character".to_string(),
        DocType {
            name: "character".to_string(),
            relations: BTreeMap::from([(
                "mentor".to_string(),
                RelationDef {
                    target_types: vec!["character".to_string()],
                    cardinality: "many".to_string(),
                },
            )]),
            ..DocType::default()
        },
    );

    let source = RelPath::new("heroe.md").unwrap();
    let target = RelPath::new("espada.md").unwrap();

    let err = lodestar_core::plan::normalize_add_relation(&b, &schema, &source, "mentor", &target)
        .expect_err(
            "añadir una relación a un target de tipo no permitido debe violar la `RelationDef`",
        );

    let dbg = format!("{err:?}");
    assert!(
        dbg.contains("RelationConstraintViolation"),
        "el rechazo debe ser la variante de `CoreError` que mapea a \
         `ErrorCode::RelationConstraintViolation` (wire \"RELATION_CONSTRAINT_VIOLATION\"); \
         error = {err:?}",
    );
}

/// Criterio `transicion_invalida`: **Dado** `transition_status` a un estado NO permitido, **Cuando**
/// se normaliza, **Entonces** rechazo (`Err`).
///
/// Fixture: `DocType decision` con `allowedStatuses:[proposed, accepted]`; `d1.md` (decision) en
/// `proposed`. Transicionar a `"inventado"` (fuera de la lista) → `Err`. Discriminador contra un
/// stub que siempre falle: transicionar a `"accepted"` (permitido) → `Ok(PatchFrontmatter{status})`.
#[test]
fn transicion_invalida() {
    use lodestar_core::schema::{DocType, Schema};

    let b = Bundle::from_files(fm(&[(
        "d1.md",
        "---\ntype: decision\ntitle: D1\nstatus: proposed\n---\n\n# D1\n\ncuerpo\n",
    )]));

    let mut schema = Schema::default();
    schema.types.insert(
        "decision".to_string(),
        DocType {
            name: "decision".to_string(),
            allowed_statuses: vec!["proposed".to_string(), "accepted".to_string()],
            ..DocType::default()
        },
    );

    let reference = RelPath::new("d1.md").unwrap();

    // 1) Estado NO permitido → rechazo.
    let err =
        lodestar_core::plan::normalize_transition_status(&b, &schema, &reference, "inventado")
            .expect_err("transicionar a un estado fuera de `allowedStatuses` debe rechazarse");
    let _ = err; // el criterio solo exige `Err`; la spec no fija un wire concreto para el rechazo.

    // 2) DISCRIMINADOR: estado permitido → `Ok` con la escritura correctora (`status: accepted`).
    let op = lodestar_core::plan::normalize_transition_status(&b, &schema, &reference, "accepted")
        .expect("transicionar a un estado permitido debe producir la escritura correctora");
    let NormalizedOperation::PatchFrontmatter { path, patch } = &op else {
        panic!("una transición válida debe resolverse a un `PatchFrontmatter`; fue {op:?}");
    };
    assert_eq!(
        path, &reference,
        "el patch debe recaer sobre el concepto transicionado"
    );
    assert!(
        patch.0.contains_key("status"),
        "el patch de una transición válida debe fijar el campo `status`; patch = {patch:?}",
    );
    assert!(
        format!("{patch:?}").contains("accepted"),
        "el patch debe fijar `status: accepted`; patch = {patch:?}",
    );
}

/// Criterio `apply_fix_safe`: **Dado** `apply_fix` con el `fixId` de un fix `safe`, **Cuando** se
/// normaliza, **Entonces** produce la escritura correctora.
///
/// Diagnóstico fixable asumido (ver cabecera de sección): una relación tipada ROTA (`REL-TARGET`)
/// cuyo `Fix` `safe` es «quitar la relación rota». El test obtiene el `fix_id` recomputando
/// `validate_relations` y leyendo el primer `Fix` `safe`; luego exige que `normalize_apply_fix`
/// resuelva a un `PatchFrontmatter` sobre el concepto origen que YA NO referencia el target roto.
#[test]
fn apply_fix_safe() {
    use lodestar_core::schema::{validate_relations, DocType, RelationDef, Schema};

    // `heroe` (character) declara `mentor -> fantasma.md`, pero `fantasma.md` NO existe → REL-TARGET.
    let b = Bundle::from_files(fm(&[(
        "heroe.md",
        "---\ntype: character\ntitle: Heroe\nmentor:\n  - fantasma.md\n---\n\n# Heroe\n\ncuerpo\n",
    )]));

    let mut schema = Schema::default();
    schema.types.insert(
        "character".to_string(),
        DocType {
            name: "character".to_string(),
            relations: BTreeMap::from([(
                "mentor".to_string(),
                RelationDef {
                    target_types: vec!["character".to_string()],
                    cardinality: "many".to_string(),
                },
            )]),
            ..DocType::default()
        },
    );

    // Precondición: el diagnóstico REL-TARGET existe y emite un `Fix` `safe` (lo que el implementador
    // debe añadir a `validate_relations`). De ahí sale el `fix_id` que consume `normalize_apply_fix`.
    let checks = validate_relations(&b, &schema);
    assert!(
        checks.iter().any(|c| c.code == CheckCode::RelTarget),
        "el fixture debe producir un diagnóstico REL-TARGET (relación rota); checks = {checks:?}",
    );
    let fix = checks
        .iter()
        .flat_map(|c| &c.fixes)
        .find(|f| f.safe)
        .expect(
            "el diagnóstico REL-TARGET de una relación rota debe emitir un `Fix{ safe: true }` \
             cuyo arreglo es «quitar la relación rota» (el implementador debe adjuntarlo en \
             `validate_relations`)",
        );
    let fix_id = fix.fix_id.clone();

    let op = lodestar_core::plan::normalize_apply_fix(&b, &schema, &fix_id)
        .expect("aplicar un fix `safe` conocido debe producir la escritura correctora");

    // La escritura correctora es un `PatchFrontmatter` sobre `heroe.md` que quita la relación rota.
    let source = RelPath::new("heroe.md").unwrap();
    let NormalizedOperation::PatchFrontmatter { path, patch } = &op else {
        panic!("aplicar el fix debe resolverse a un `PatchFrontmatter`; fue {op:?}");
    };
    assert_eq!(
        path, &source,
        "el patch debe recaer sobre el concepto de la relación rota"
    );
    assert!(
        patch.0.contains_key("mentor"),
        "el patch debe tocar el campo de la relación rota (`mentor`); patch = {patch:?}",
    );
    assert!(
        !format!("{patch:?}").contains("fantasma"),
        "el patch correctivo debe QUITAR el target roto `fantasma.md` del campo `mentor`, no \
         conservarlo; patch = {patch:?}",
    );
}
