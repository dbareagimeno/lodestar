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
