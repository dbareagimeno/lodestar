//! Tests del núcleo (E1): contrato de tipos, modelo, conformidad, analyze, query, generadores, diff.

use std::collections::BTreeMap;

use lodestar_core::diff::{self, ChangeKind, MessageHint};
use lodestar_core::model;
use lodestar_core::types::*;
use lodestar_core::DocumentSet;
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
    // E16-H05: el catálogo pasó al mínimo de `§20.9` — `OKF-FM01` desapareció y `OKF-FM02`/
    // `OKF-FM03`/`OKF-CONFLICT` se renombraron. Lo que este test fija sigue siendo lo mismo: el
    // valor de wire ES la cadena con guion, y `as_str` coincide con la serialización.
    assert_eq!(
        serde_json::to_string(&CheckCode::FmUnclosed).unwrap(),
        "\"FM-UNCLOSED\""
    );
    assert_eq!(CheckCode::DocConflictMarker.as_str(), "DOC-CONFLICT-MARKER");
}

// --- E10-H06: extensión de `Check` (campos aditivos `id`/`range`/`related`/`fixes`) ----------
//
// (E20-H03: el test `schema_code_wire` de las familias `SCHEMA-*`/`REL-*` se retiró con
// `core::schema`; esas variantes de `CheckCode` ya no existen.)

#[test]
fn check_extension_retrocompat() {
    // Un `Check` clásico, construido SIN fixes/range/id/related.
    let c = Check::new(
        Severity::Err,
        CheckCode::FmUnclosed,
        "falta frontmatter",
        vec![RelPath::new("a/b.md").unwrap()],
    );
    let v = serde_json::to_value(&c).unwrap();

    // Retro-compat: los 4 campos clásicos NO cambian de forma ni de valor respecto al wire
    // actual (un consumidor viejo del `Check` no se rompe).
    assert_eq!(v["level"], serde_json::json!("err"));
    assert_eq!(v["code"], serde_json::json!("FM-UNCLOSED"));
    assert_eq!(v["msg"], serde_json::json!("falta frontmatter"));
    assert_eq!(v["targets"], serde_json::json!(["a/b.md"]));

    // Campos nuevos ADITIVOS con su valor por defecto: `fixes` serializa como `[]`
    // y `range` está ausente (o `null`).
    assert_eq!(
        v["fixes"],
        serde_json::json!([]),
        "un Check clásico debe serializar `fixes` como []",
    );
    assert!(
        v.get("range").is_none_or(serde_json::Value::is_null),
        "un Check clásico debe serializar `range` ausente o null",
    );
}

#[test]
fn check_campos_nuevos_por_defecto() {
    // Un `Check` construido por el constructor clásico deja los campos aditivos en su valor por
    // defecto. Este test fija los NOMBRES Rust de los campos (id/range/related/fixes) que el
    // diseño D-CheckCode dicta.
    let c = Check::new(
        Severity::Info,
        CheckCode::LinkTargetMissing,
        "enlace roto",
        vec![],
    );
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

// --- E10-H04: `DocumentRef` (identidad por path, id opcional/diferido) --------
//
// Fase ROJA: el struct `DocumentRef { path: RelPath, id: Option<DocumentId> }` (`REFACTOR §6.1`)
// todavía NO existe en producción. Se espera reachable vía `use lodestar_core::types::*` (mismo
// patrón que `RelPath`/`ErrorCode`), con una deserialización que acepta `{ "path": … }` y deja el
// `id` ausente como `None`. Estos tests hacen ROJO por API ausente (símbolo `DocumentRef`) hasta que
// se implemente. La resolución contra un workspace (`DOCUMENT_NOT_FOUND`) se prueba en `lodestar-app`
// (`tests/document_ref.rs`), porque exige un `Workspace` abierto y el core es puro.

#[test]
fn ref_por_path() {
    // Criterio `ref_por_path`: `{ "path": "a/b.md" }` deserializa a un `DocumentRef` cuyo `path` es
    // el `RelPath` validado y cuyo `id` queda ausente (`None`) — el id es opcional/diferido.
    let referencia: DocumentRef =
        serde_json::from_str(r#"{"path":"a/b.md"}"#).expect("`{ path: a/b.md }` debe deserializar");
    assert_eq!(
        referencia.path,
        RelPath::new("a/b.md").unwrap(),
        "el `path` deserializado debe ser el RelPath validado `a/b.md`",
    );
    assert!(
        referencia.id.is_none(),
        "sin clave `id` en el JSON, `DocumentRef::id` debe quedar `None`, es {:?}",
        referencia.id,
    );
}

#[test]
fn ref_rechaza_traversal() {
    // Criterio `ref_rechaza_traversal`: `{ "path": "../x" }` NO debe deserializar — `RelPath`
    // rechaza el `..` en su `Deserialize` (invariante #6, único chokepoint de path-traversal), y
    // `DocumentRef` hereda ese rechazo por delegar en el `RelPath` de su campo `path`.
    let resultado = serde_json::from_str::<DocumentRef>(r#"{"path":"../x"}"#);
    assert!(
        resultado.is_err(),
        "un `DocumentRef` con `path` de traversal (`../x`) debe fallar al deserializar, dio {resultado:?}",
    );
}

// --- E1-H05: modelo ---------------------------------------------------------

#[test]
fn build_raw_idempotente() {
    let raw = "---\ntype: Concept\ntitle: Alfa\n---\n\n# H\n\ncuerpo\n";
    let parsed = model::parse_file("alfa.md", raw);
    let rebuilt = model::build_raw(parsed.frontmatter.as_ref(), &parsed.body);
    let reparsed = model::parse_file("alfa.md", &rebuilt);
    let rebuilt2 = model::build_raw(reparsed.frontmatter.as_ref(), &reparsed.body);
    assert_eq!(rebuilt, rebuilt2, "build_raw debe ser idempotente");
}

// E17-H02 retiró `resolve_link_casos`: `model::resolve_link` ya no existe. Su semántica —y la
// que la sustituye, sin `foo/` → `foo/index.md`— la cubren `enlaces.rs::punto_barra_equivale` y
// `enlaces.rs::directorio_no_es_index`.

// --- E1-H06/H07: conformidad y analyze --------------------------------------

fn codes_of(b: &DocumentSet, path: &str) -> Vec<String> {
    let p = RelPath::new(path).unwrap();
    b.analyze().diagnostics[&p]
        .iter()
        .map(|c| c.code.as_str().to_string())
        .collect()
}

/// MIGRADO en E16-H05: era el catálogo OKF entero; ahora es el **catálogo mínimo** de `§20.9`.
/// Sigue siendo el mismo test —«cada código que `conform` puede producir, se produce»—, pero la
/// lista de códigos es otra, y la mitad del fixture pasa a probar el SILENCIO: lo que antes eran
/// seis incumplimientos hoy son, en su mayoría, documentos perfectamente válidos.
#[test]
fn conformidad_dispara_cada_codigo() {
    let b = DocumentSet::from_files(fm(&[
        ("sin-fm.md", "Solo cuerpo, sin encabezados.\n"),
        ("sin-cierre.md", "---\ntype: Concept\n"),
        ("malo-yaml.md", "---\ntype: : :\n  - x\n: bad\n---\n\n# H\n"),
        ("sin-tipo.md", "---\ntitle: \n---\n\ncuerpo\n"),
        (
            "malo.md",
            "---\ntype: Nota\ntitle: Malo\ndescription: x\ntags: uno\ntimestamp: ayer\n---\n\n# H\n\n[falta](/no.md) y [r](./o.md)\n",
        ),
        ("conflicto.md", "---\ntype: N\ntitle: C\ndescription: d\n---\n\n# H\n\n<<<<<<< HEAD\na\n=======\nb\n>>>>>>> r\n"),
    ]));
    // Lo que Lodestar NO puede interpretar o modificar con seguridad: los tres códigos vivos.
    assert!(codes_of(&b, "sin-cierre.md").contains(&"FM-UNCLOSED".to_string()));
    assert!(codes_of(&b, "malo-yaml.md").contains(&"FM-YAML-INVALID".to_string()));
    assert!(codes_of(&b, "conflicto.md").contains(&"DOC-CONFLICT-MARKER".to_string()));
    // Enlaces (E17-H03): el destino inexistente es `LINK-TARGET-MISSING`, y un enlace relativo
    // que RESUELVE no diagnostica nada — `LINK-REL` («usa la ruta completa /…») murió con el
    // modelo que lo justificaba.
    let malo = codes_of(&b, "malo.md");
    assert!(
        malo.contains(&"LINK-TARGET-MISSING".to_string()),
        "{malo:?}"
    );
    assert!(
        !malo.contains(&"LINK-STUB".to_string()) && !malo.contains(&"LINK-REL".to_string()),
        "los códigos de enlace del prototipo se retiraron en E17-H03: {malo:?}"
    );

    // Y el otro lado del catálogo mínimo: un `.md` cualquiera NO incumple nada. Un documento sin
    // frontmatter, uno sin `type` ni encabezados y una metadata «mal formateada» son válidos y
    // silenciosos — se acabaron `OKF-FM01`, `OKF-TYPE`, `REC-*`, `BODY-STRUCT` y `FMT-*`.
    assert_eq!(codes_of(&b, "sin-fm.md"), Vec::<String>::new());
    assert_eq!(codes_of(&b, "sin-tipo.md"), Vec::<String>::new());
    assert!(
        !malo.contains(&"FMT-TAGS".to_string()) && !malo.contains(&"FMT-TS".to_string()),
        "el formato de `tags`/`timestamp` es cosa del usuario: {malo:?}"
    );
    // `ORPHAN` murió con E16-H02: el aislamiento es una propiedad del grafo
    // (`Analysis::isolated`), no un diagnóstico.
    assert!(!codes_of(&b, "sin-tipo.md").contains(&"ORPHAN".to_string()));
}

#[test]
fn hard_fail_cuenta_ficheros_no_max() {
    // 1 fichero con Err + 1 sin problemas → hard_fail == 1 (no se "tapa" ni se suma dos veces).
    // MIGRADO en E16-H05: el fichero con `Err` era «sin frontmatter» (`OKF-FM01`), que ya no es
    // un error; hoy lo es uno cuyo frontmatter Lodestar no sabe leer (`FM-UNCLOSED`).
    let b = DocumentSet::from_files(fm(&[
        ("malo.md", "---\ntype: Nota\n"),
        (
            "bueno.md",
            "---\ntype: Nota\ntitle: B\ndescription: d\n---\n\n# H\n\n[x](/malo.md)\n",
        ),
    ]));
    assert_eq!(b.analyze().hard_fail(), 1);
}

#[test]
fn analyze_backlinks_son_inversa_de_out() {
    let b = DocumentSet::from_files(fm(&[
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
    // MIGRADO en E17-H04: `out`/`inn` son ahora `outgoing`/`incoming`, con el enlace resuelto
    // completo en vez de una adyacencia de paths.
    assert_eq!(
        a.outgoing[&pa]
            .iter()
            .map(|l| l.target.clone())
            .collect::<Vec<_>>(),
        vec![LinkTarget::Document(pb.clone())]
    );
    assert_eq!(
        a.incoming[&pb]
            .iter()
            .map(|r| r.from.clone())
            .collect::<Vec<_>>(),
        vec![pa.clone()]
    );
    assert_eq!(
        a.incoming[&pb][0].link, a.outgoing[&pa][0],
        "`incoming` es la inversa de `outgoing`: el mismo enlace, no una copia recalculada"
    );
    // `a.md` no tiene entrantes, pero SÍ salientes → NO está aislado (`§20.7`, E16-H02).
    assert!(!a.isolated.contains(&pa));
    // `b.md` tiene entrantes → tampoco.
    assert!(!a.isolated.contains(&pb));
}

// --- E1-H09: list_documents / backlinks --------------------------------------

#[test]
fn list_documents_marca_invalid_e_isolated() {
    // MIGRADO en E16-H05: `invalid` = «tiene algún diagnóstico de severidad `Err`». El documento
    // sin frontmatter dejó de tenerlos, así que el fixture pasa a uno con el bloque sin cerrar.
    let b = DocumentSet::from_files(fm(&[("malo.md", "---\ntype: Nota\n")]));
    let cs = b.list_documents();
    let c = cs.iter().find(|c| c.path.as_str() == "malo.md").unwrap();
    assert!(c.invalid);
    // Único documento del workspace: sin entrantes ni salientes → aislado (E16-H02).
    assert!(c.isolated);
}

// --- E1-H11: query ----------------------------------------------------------
//
// RETIRADO en E19-H05: `query_operadores` (y su helper `query_set`) ejercitaba la DSL de tokens con
// semántica de subcadena (`type:nota`, `is:draft`, `has:tags`, `body:mundo`, negación `-`, flip `!`,
// texto suelto), servida por `DocumentSet::query` → `query.rs`. Esa DSL se retiró entera al cablear
// el lenguaje de consulta tipado a `knowledge_search`. Su cobertura la asume el nuevo lenguaje en
// `tests/consulta.rs` (comparaciones tipadas, `has()`/`missing()`, `contains`, booleanos,
// namespaces `graph.*`), cuya semántica es la que hoy filtra la búsqueda (E19-H01…H05).

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
fn create_document_rechaza_no_conforme() {
    // MIGRADO en E16-H05: el rechazo se probaba con `type` vacío (`OKF-TYPE`), que ya no es un
    // error — un documento sin `type` es válido. Lo que sigue en pie es la MECÁNICA: un resultado
    // con severidad `Err` se rechaza sin escribir y sin devolver `Err` del `Result`. Hoy el
    // disparador es un cuerpo con marcadores de merge sin resolver (`DOC-CONFLICT-MARKER`).
    let b = DocumentSet::from_files(fm(&[]));
    let p = RelPath::new("nuevo.md").unwrap();
    let conflictivo = "# H\n\n<<<<<<< HEAD\nuno\n=======\ndos\n>>>>>>> rama\n";
    let outcome = b.create_document(&p, "Nota", Some("Nuevo"), conflictivo, None, false);
    assert!(!outcome.written);
    assert!(outcome.rejected.is_some());
    // Sin el conflicto → escribible. (Y sin `type`, también: ya no hay regla que lo exija.)
    let ok = b.create_document(&p, "", Some("Nuevo"), "# H\n", None, false);
    assert!(ok.written);
    assert!(ok.rejected.is_none());
}

#[test]
fn create_document_incluye_timestamp_en_su_posicion_canonica() {
    let b = DocumentSet::from_files(fm(&[]));
    let p = RelPath::new("nuevo.md").unwrap();
    // Con timestamp: aparece antes de `status`. Desde E16-H01 el orden del `.md` es el orden de
    // inserción de las claves (el `Mapping` de serde_yaml lo preserva), no una lista canónica.
    let ok = b.create_document(
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
    let sin = b.create_document(&p, "Nota", Some("Nuevo"), "# H\n", None, false);
    assert!(
        !sin.raw.contains("timestamp:"),
        "no debía emitir timestamp: {}",
        sin.raw
    );
}

#[test]
fn create_document_genera_heading_por_defecto_cuando_body_vacio() {
    let b = DocumentSet::from_files(fm(&[]));
    // body vacío + ty no vacío → `# {ty} - {title}`.
    let p = RelPath::new("mi-cosa.md").unwrap();
    let con_tipo = b.create_document(&p, "Nota", Some("Mi Cosa"), "", None, false);
    assert!(con_tipo.written);
    assert!(
        con_tipo.raw.contains("# Nota - Mi Cosa\n"),
        "falta el heading con tipo: {}",
        con_tipo.raw
    );
    // ty vacío → `# {title}` (sin separador colgante). type vacío rechaza, pero el raw se computa.
    let sin_tipo = b.create_document(&p, "", Some("Mi Cosa"), "", None, false);
    assert!(
        sin_tipo.raw.contains("# Mi Cosa\n") && !sin_tipo.raw.contains("# Mi Cosa -"),
        "el heading sin tipo no debe tener separador: {}",
        sin_tipo.raw
    );
    // title None → último eslabón de `derived_title`: el nombre del fichero tal cual, sin `.md`
    // y sin Title Case (E16-H03 retiró `title_from_path`).
    let sin_titulo = b.create_document(&p, "Nota", None, "", None, false);
    assert!(
        sin_titulo.raw.contains("# Nota - mi-cosa\n"),
        "el título debe derivar del nombre del fichero: {}",
        sin_titulo.raw
    );
    // body no vacío → se respeta tal cual, sin generar default.
    let con_body = b.create_document(&p, "Nota", Some("Mi Cosa"), "# H\n", None, false);
    assert!(
        con_body.raw.contains("# H\n") && !con_body.raw.contains("# Nota - Mi Cosa"),
        "un body explícito no debe reemplazarse: {}",
        con_body.raw
    );
}

#[test]
fn merge_frontmatter_null_borra() {
    let b = DocumentSet::from_files(fm(&[(
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

// --- Regresiones de paridad con el prototipo (revisión profunda) -------------

#[test]
fn fm_escalares_no_string_no_invierten_el_veredicto() {
    // MIGRADO en E16-H01 (antes `fm_escalares_no_string_se_coercen_como_js`): la coerción `String(v)`
    // del prototipo desapareció, pero la garantía que protegía este test sigue viva y es la que
    // importa — un escalar no-string en una clave cualquiera NO convierte el fichero entero en
    // FM-YAML-INVALID (hard-fail), que invertiría el veredicto de la puerta de CI. Lo que cambia es que
    // ahora el valor conserva su TIPO YAML en vez de convertirse en texto.
    let b = DocumentSet::from_files(fm(&[(
        "n.md",
        "---\ntype: 123\ntitle: 2024\ndescription: true\n---\n\n# H\n\ncuerpo\n",
    )]));
    let a = b.analyze();
    assert_eq!(a.hard_fail(), 0, "el veredicto no puede invertirse: {a:?}");
    let checks = &a.diagnostics[&RelPath::new("n.md").unwrap()];
    assert!(!checks.iter().any(|c| c.code == CheckCode::FmYamlInvalid));

    // El tipo YAML real sobrevive al parseo (ya no hay coerción a string).
    let parsed = model::parse_file("n.md", &b.files()[&RelPath::new("n.md").unwrap()]);
    let pf = parsed.frontmatter.expect("el documento tiene frontmatter");
    assert_eq!(
        pf.get_key("type"),
        Some(&serde_yaml::Value::Number(123.into())),
        "`type: 123` debe seguir siendo el número 123"
    );
    assert_eq!(
        pf.get_key("description"),
        Some(&serde_yaml::Value::Bool(true)),
        "`description: true` debe seguir siendo el booleano true"
    );
    // …y las proyecciones de presentación siguen viéndolo como texto (columnas de cache, DTO).
    let resumen = b.list_documents();
    let fila = resumen
        .iter()
        .find(|c| c.path.as_str() == "n.md")
        .expect("n.md está en el listado");
    assert_eq!(fila.r#type.as_deref(), Some("123"));
    assert_eq!(fila.title, "2024");
}

#[test]
fn fm_null_explicito_sobrevive_a_la_escritura() {
    // Un `type:` con valor `null` explícito debe sobrevivir a la escritura, no borrarse en silencio.
    //
    // MIGRADO en E19-H05: la primera aserción de este test consultaba el `null` explícito con la
    // vieja DSL (`b.query("has:type")`), que se retiró con `query.rs`. La existencia de una clave a
    // `null` («cuenta como presente») la cubre ahora el lenguaje de consulta tipado en
    // `tests/consulta.rs::has_ok` (`has(deprecated_field)` sobre `deprecated_field: null`); aquí se
    // conserva la parte que la DSL nunca probó: que la escritura preserva el `null` explícito.
    let b = DocumentSet::from_files(fm(&[(
        "connull.md",
        "---\ntype:\ntitle: A\ndescription: d\n---\n\n# H\n",
    )]));
    //
    // MIGRADO en E16-H05/H04: este trozo aserraba el resultado de un patch VACÍO. Desde que
    // `merge_frontmatter` delega en el patch quirúrgico (E16-H04), un patch vacío es un no-op
    // REAL —devuelve el documento byte a byte, sin round-trip— así que ya no dice nada sobre la
    // serialización. La intención («un `null` explícito sobrevive a la escritura») se conserva
    // por los dos caminos que sí escriben:
    let p = RelPath::new("connull.md").unwrap();

    //   (a) camino quirúrgico: un patch sobre OTRA clave deja la línea `type:` intacta y `type`
    //       sigue presente con valor nulo.
    let mut patch = BTreeMap::new();
    patch.insert(
        "title".to_string(),
        Some(serde_yaml::Value::String("Otro".into())),
    );
    let outcome = b.merge_frontmatter(&p, FrontmatterPatch(patch));
    assert!(outcome.written, "rejected: {:?}", outcome.rejected);
    let re = model::parse_frontmatter(&outcome.raw).expect("el resultado tiene frontmatter");
    assert_eq!(
        re.get_key("type"),
        Some(&serde_yaml::Value::Null),
        "el `null` explícito sigue presente tras el patch: {:?}",
        outcome.raw
    );

    //   (b) camino de reserialización: `build_raw` lo vuelca como `type: null`, no lo descarta.
    let parsed = model::parse_file("connull.md", &b.files()[&p]);
    let raw = model::build_raw(parsed.frontmatter.as_ref(), &parsed.body);
    assert!(raw.contains("type: null"), "raw: {raw:?}");
}

// BORRADO en E16-H05: `fmt_ts_rechaza_iso_con_basura`. Fijaba la paridad de `model::is_iso` con
// el `Date.parse` del prototipo, que existía SOLO para alimentar `FMT-TS`. Retirado el check (una
// fecha del frontmatter es metadata arbitraria del usuario, `§20.9`), la función se borró y el
// test se queda sin sujeto: no hay nada que migrar, porque el comportamiento que probaba ya no
// debe existir.

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

/// MIGRADO en E17-H04 (antes `backlinks_out_dedup_sin_self`): `Backlinks::out` dejó de ser la
/// lista deduplicada de destinos resueltos y pasó a ser **todos** los enlaces del documento, en
/// orden de aparición y con su clasificación. Enlazar dos veces al mismo destino son dos enlaces
/// (lo que `move_document` necesita reescribir), y el self-enlace es un enlace más: la
/// deduplicación y la exclusión del self viven ahora en el GRAFO, no en la lista.
#[test]
fn backlinks_out_lista_todos_los_enlaces() {
    let b = DocumentSet::from_files(fm(&[
        (
            "x.md",
            "---\ntype: N\ntitle: X\ndescription: d\n---\n\n[a](/a.md) [a](/a.md) [idx](/index.md) [yo](/x.md)\n",
        ),
        ("a.md", "---\ntype: N\ntitle: A\ndescription: d\n---\n\n# H\n"),
        ("index.md", "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# B\n\n* [x](x.md)\n"),
    ]));
    let x = RelPath::new("x.md").unwrap();
    let bl = b.backlinks(&x);
    // Los CUATRO enlaces, en orden de aparición: el repetido no se dedupea y el self-enlace no se
    // excluye. `index.md` SÍ aparece: desde E16-H02 no es un destino reservado, sino un documento
    // como cualquier otro.
    assert_eq!(
        bl.out.iter().map(|l| l.href.as_str()).collect::<Vec<_>>(),
        vec!["/a.md", "/a.md", "/index.md", "/x.md"]
    );
    // …pero el grafo sí: una sola arista por vecino, y el self-enlace es un self-loop.
    let vecindad = b.neighborhood(&x, 1, Direction::Out);
    let vecinos: Vec<&str> = vecindad.edges.iter().map(|e| e.target.as_str()).collect();
    assert_eq!(vecinos, vec!["a.md", "index.md", "x.md"]);
}

// RETIRADO en E19-H05: `query_campo_vacio_es_texto_suelto` fijaba un quirk de la vieja DSL (`":foo"`
// con campo vacío degradaba a texto suelto, port de la falsedad de `""` en JS). La DSL entera se
// retiró con `query.rs`; el `text` de `knowledge_search` sigue siendo subcadena, pero sin sintaxis
// de campos (`campo:valor`), así que el quirk ya no existe ni tiene superficie que probar.

// --- E10-H03: WorkspaceRevision (identidad de contenido determinista) ---------
//
// La función pura `workspace_revision(files: &FileMap, writable: &[RelPath])` (aún NO
// implementada) calcula una identidad determinista del contenido escribible del workspace:
// filtra a los `writableRoots` (slice vacío = todo el workspace es escribible, coherente con
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

    // writable vacío = todo el workspace es escribible.
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
// E20-H03 — RETIRADOS: los tests de `core::schema` (E10-H05 `carga_doctype`, E10-H07
// `falta_campo_obligatorio`/`status_no_permitido`/`sin_schema_sin_checks`, E11-H03
// `relacion_target_roto`/`relacion_tipo_invalido`/`relacion_cardinalidad`) desaparecen con la
// maquinaria de schema (`Schema`/`DocType`/`RelationDef`/`validate_schema`/`validate_relations`).
// El modelo es universal (`§20.10`): no hay tipos, campos obligatorios ni relaciones tipadas que
// validar. La inspección de metadata que los sustituye vive en `tests/` de `core::metadata`
// (E20-H01/H02) y en `crates/lodestar-mcp/tests/mcp.rs` (E20-H03, tool `metadata_inspect`).
// ---------------------------------------------------------------------------

// --- E11-H02: graph_query estructural (path_between / cycles / components) ----
//
// Operaciones puras del core sobre el grafo de enlaces (aristas = `out_links`/`resolve_link`,
// la MISMA representación que `analyze().out`/`inn` y `graph_model`/`neighborhood`). Firmas
// asumidas (fase roja — aún NO existen en `crates/lodestar-core/src/graph.rs`; se exponen como
// métodos de `DocumentSet`, en línea con `neighborhood`/`graph_model`/`backlinks`):
//
//   impl DocumentSet {
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
// Fixtures: cada documento lleva frontmatter válido (`type`/`title`/`description`) para ser
// documento real; las aristas se montan con enlaces markdown `[x](/x.md)` en el cuerpo (mismo
// patrón que `analyze_backlinks_son_inversa_de_out`), sin ghosts ni reservados.

/// Nodo documento con `body` como cuerpo (donde van los enlaces markdown que forman aristas).
fn nodo(title: &str, body: &str) -> String {
    format!("---\ntype: N\ntitle: {title}\ndescription: d\n---\n\n# H\n\n{body}\n")
}

/// Criterio `path_between_directo`: A→B→C ⇒ `path_between(A,C) == [A,B,C]` (camino más corto
/// dirigido, incluyendo los dos extremos).
#[test]
fn path_between_directo() {
    let b = DocumentSet::from_files(fm(&[
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
    let b = DocumentSet::from_files(fm(&[
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
    let b = DocumentSet::from_files(fm(&[
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
    let b = DocumentSet::from_files(fm(&[
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
// `use lodestar_core::types::*` (mismo patrón que `WorkspaceRevision`/`DocumentRef`). Estos tests
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
//         workspace_before: &DocumentSet,
//         workspace_after: &DocumentSet,
//     ) -> RiskAssessment
//
// Hasta que E12-H02 la defina, estos dos tests hacen ROJO por SÍMBOLO AUSENTE (compile-fail: el
// módulo `plan`/`assess_risk` no existe), lo que impide compilar el binario de tests de este crate.
// Es el rojo esperado y documentado.
//
// Representación del `deprecate` (el enunciado admite dos): tras E21-H01 se modela como
// `NormalizedOperation::PatchFrontmatter { path, patch: { status: "deprecated" } }` — una transición
// es un patch de una propiedad de frontmatter arbitraria (`§20.11`), y `assess_risk` la reconoce
// como operación que «encoge» el grafo idéntico a como trataba la retirada `transition_status`
// (`patches_status_to_deprecated`). El `workspace_after` refleja ese estado deprecado para que
// `before`/`after` sean coherentes; los backlinks del documento no cambian con la transición.
//
// Los tests aseveran PROPIEDADES (nivel de riesgo, razón no vacía que menciona el documento o los
// backlinks), nunca el texto exacto de la razón ni el umbral interno de la heurística.

/// Workspace con un documento `core.md` (en el `status` dado) al que apuntan 7 documentos referentes,
/// más un `index.md` mínimo. Sirve para construir el `before` (activo) y el `after` (deprecado) del
/// criterio `riesgo_deprecate_backlinks`.
fn workspace_con_7_backlinks(status_core: &str) -> DocumentSet {
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
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# B\n".to_string(),
    );
    DocumentSet::from_files(files)
}

/// Criterio `riesgo_deprecate_backlinks`: **Dado** un `deprecate` sobre un documento con 7 backlinks,
/// **Cuando** se evalúa, **Entonces** `level >= Medium` con una razón que lo menciona.
#[test]
fn riesgo_deprecate_backlinks() {
    let antes = workspace_con_7_backlinks("active");
    let despues = workspace_con_7_backlinks("deprecated");

    // Precondición del fixture: `core.md` recibe exactamente 7 backlinks entrantes (r1..r7).
    let entrantes = antes
        .backlinks(&RelPath::new("core.md").unwrap())
        .inbound
        .len();
    assert_eq!(
        entrantes, 7,
        "el fixture debe dar 7 backlinks a core.md, dio {entrantes}",
    );

    let mut patch = BTreeMap::new();
    patch.insert(
        "status".to_string(),
        Some(serde_yaml::Value::String("deprecated".into())),
    );
    let ops = vec![NormalizedOperation::PatchFrontmatter {
        path: RelPath::new("core.md").unwrap(),
        patch: FrontmatterPatch(patch),
    }];

    let risk = lodestar_core::plan::assess_risk(&ops, &antes, &despues);

    assert!(
        risk.level >= RiskLevel::Medium,
        "deprecar un documento con 7 backlinks debe ser al menos Medium, fue {:?}",
        risk.level,
    );
    assert!(
        !risk.reasons.is_empty(),
        "un riesgo >= Medium debe justificarse con al menos una razón",
    );
    // La razón debe mencionar el documento afectado (`core`) o el alcance del blast-radius (los
    // 7 backlinks) — propiedad, no texto exacto.
    assert!(
        risk.reasons
            .iter()
            .any(|r| r.contains("core") || r.contains('7')),
        "alguna razón debe mencionar el documento (`core`) o sus backlinks (7); razones = {:?}",
        risk.reasons,
    );
}

/// Criterio `riesgo_bajo_aislado`: **Dado** un `patch_frontmatter` sin backlinks afectados,
/// **Cuando** se evalúa, **Entonces** `level: Low`.
#[test]
fn riesgo_bajo_aislado() {
    // Documento `sola.md` sin ningún referente: nadie le apunta. `index.md` tampoco lo lista.
    let construir = |titulo: &str| -> DocumentSet {
        let mut files: FileMap = FileMap::new();
        files.insert(
            RelPath::new("sola.md").unwrap(),
            format!(
                "---\ntype: N\ntitle: {titulo}\ndescription: d\nstatus: draft\n---\n\n# Sola\n"
            ),
        );
        files.insert(
            RelPath::new("index.md").unwrap(),
            "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# B\n".to_string(),
        );
        DocumentSet::from_files(files)
    };
    let antes = construir("Antes");
    let despues = construir("Despues");

    // Precondición del fixture: `sola.md` no recibe ningún enlace entrante (desde E16-H02 los de
    // un `index.md` serían entrantes normales, no una lista aparte).
    let bl = antes.backlinks(&RelPath::new("sola.md").unwrap());
    assert!(
        bl.inbound.is_empty(),
        "el fixture debe dejar sola.md sin backlinks, fue inbound={:?}",
        bl.inbound,
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
        "un patch de frontmatter sobre un documento aislado debe ser riesgo Low, fue {:?} (razones {:?})",
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
//         before: &DocumentSet,
//         after: &DocumentSet,
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
    let a = RelPath::new("a.md").unwrap();
    let b = RelPath::new("b.md").unwrap();

    // `before`: solo B. `after`: A nuevo + B con el cuerpo modificado.
    let before = DocumentSet::from_files(fm(&[(
        "b.md",
        "---\ntype: N\ntitle: B\ndescription: d\nstatus: draft\n---\n\n# B\n\ncuerpo original\n",
    )]));
    let after = DocumentSet::from_files(fm(&[
        (
            "a.md",
            "---\ntype: N\ntitle: A\ndescription: d\nstatus: draft\n---\n\n# A\n\ncuerpo nuevo\n",
        ),
        (
            "b.md",
            "---\ntype: N\ntitle: B\ndescription: d\nstatus: draft\n---\n\n# B\n\ncuerpo MODIFICADO\n",
        ),
    ]));

    let diff = lodestar_core::plan::semantic_diff(&before, &after);

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

/// Criterio `diff_resuelve_diagnostico` (RECOMPUESTO E20-H03): **Dado** un plan que corrige un
/// enlace roto (`LINK-TARGET-MISSING`, código vivo de `§20.9` que sustituye al retirado
/// `SCHEMA-REQFIELD`), **Cuando** se computa el diff, **Entonces** ese diagnóstico aparece en
/// `diagnosticsResolved`.
#[test]
fn diff_resuelve_diagnostico() {
    // `before`: `d.md` enlaza a un `.md` inexistente → `LINK-TARGET-MISSING` (Err).
    let before = DocumentSet::from_files(fm(&[(
        "d.md",
        "---\ntype: decision\ntitle: Migrar\nstatus: proposed\n---\n\n# H\n\n[roto](no-existe.md)\n",
    )]));
    // `after`: el mismo documento SIN el enlace roto → deja de violar `LINK-TARGET-MISSING`.
    let after = DocumentSet::from_files(fm(&[(
        "d.md",
        "---\ntype: decision\ntitle: Migrar\nstatus: proposed\n---\n\n# H\n\ncuerpo sin enlaces\n",
    )]));

    // Precondición del fixture: `before` viola el requisito y `after` lo cumple (aísla el criterio).
    assert!(
        before
            .analyze()
            .diagnostics
            .values()
            .flatten()
            .any(|c| c.code == CheckCode::LinkTargetMissing),
        "el fixture `before` debe tener un LINK-TARGET-MISSING",
    );
    assert!(
        after
            .analyze()
            .diagnostics
            .values()
            .flatten()
            .all(|c| c.code != CheckCode::LinkTargetMissing),
        "el fixture `after` debe corregir el LINK-TARGET-MISSING",
    );

    let diff = lodestar_core::plan::semantic_diff(&before, &after);

    assert!(
        diff.diagnostics_resolved
            .iter()
            .any(|c| c.code == CheckCode::LinkTargetMissing),
        "el `LINK-TARGET-MISSING` corregido debe aparecer en `diagnostics_resolved`; resolved = {:?}",
        diff.diagnostics_resolved,
    );
}

/// Criterio `diff_introduce_diagnostico` (RECOMPUESTO E20-H03): **Dado** un plan que rompe un enlace
/// (target existente pasa a inexistente), **Cuando** se computa el diff, **Entonces** el diagnóstico
/// `LINK-TARGET-MISSING` (código vivo, sustituye a `REL-TARGET`/`REL-TYPE`) aparece en
/// `diagnosticsIntroduced`.
#[test]
fn diff_introduce_diagnostico() {
    // `cap.md` existe en ambos estados; solo cambia el enlace del cuerpo de `juan`.
    let cap = (
        "cap.md",
        "---\ntype: chapter\ntitle: Capitulo\n---\n\n# Capitulo\n\ncuerpo\n",
    );
    // `before`: `juan` enlaza a `cap.md` (existe): enlace conforme.
    let before = DocumentSet::from_files(fm(&[
        (
            "juan.md",
            "---\ntype: character\ntitle: Juan\n---\n\n# Juan\n\n[cap](cap.md)\n",
        ),
        cap,
    ]));
    // `after`: `juan` enlaza a `capitulo_fantasma.md` (inexistente): rompe el enlace.
    let after = DocumentSet::from_files(fm(&[
        (
            "juan.md",
            "---\ntype: character\ntitle: Juan\n---\n\n# Juan\n\n[cap](capitulo_fantasma.md)\n",
        ),
        cap,
    ]));

    // Precondición: `before` no rompe el enlace y `after` sí (aísla el criterio).
    assert!(
        before
            .analyze()
            .diagnostics
            .values()
            .flatten()
            .all(|c| c.code != CheckCode::LinkTargetMissing),
        "el fixture `before` debe tener el enlace conforme",
    );
    assert!(
        after
            .analyze()
            .diagnostics
            .values()
            .flatten()
            .any(|c| c.code == CheckCode::LinkTargetMissing),
        "el fixture `after` debe romper el enlace",
    );

    let diff = lodestar_core::plan::semantic_diff(&before, &after);

    assert!(
        diff.diagnostics_introduced
            .iter()
            .any(|c| c.code == CheckCode::LinkTargetMissing),
        "el enlace roto debe aparecer como `LINK-TARGET-MISSING` en `diagnostics_introduced`; \
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
//     pub fn validate_result(doc_set: &DocumentSet, schema: &Schema) -> ValidationReport
//
//     pub struct PlanPolicy {
//         pub require_conformant_result: bool,  // wire `requireConformantResult`
//         pub allow_warnings: bool,             // wire `allowWarnings`
//     }
//     pub fn can_apply(report: &ValidationReport, policy: &PlanPolicy) -> bool
//
// Semántica ASUMIDA (spec E12-H04, `REFACTOR §11.1`):
//   - `validate_result` compone `analyze()` + `validate_schema` + `validate_relations` sobre el
//     `DocumentSet` hipotético; `summary` cuenta Err/Warn/Info; `conformant = (summary.errors == 0)`;
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

/// Criterio `plan_no_conforme_rechaza` (RECOMPUESTO E20-H03): **Dado** un plan cuyo resultado
/// introduce un `Err` (un documento con un enlace a un `.md` inexistente → `LINK-TARGET-MISSING`/Err,
/// código vivo que sustituye a `SCHEMA-REQFIELD`) y `policy.requireConformantResult:true`, **Cuando**
/// se valida, **Entonces** `conformant:false` y el plan NO es aplicable (`can_apply == false`).
/// (Benchmark §17: "un resultado no conforme → plan rechazado".)
#[test]
fn plan_no_conforme_rechaza() {
    use lodestar_core::plan::{can_apply, validate_result, PlanPolicy};

    // DocumentSet hipotético resultante del plan: un documento con un enlace roto → Err duro.
    let hipotetico = DocumentSet::from_files(fm(&[(
        "d.md",
        "---\ntype: decision\ntitle: Migrar a Rust\nstatus: proposed\n---\n\n# H\n\n[roto](no-existe.md)\n",
    )]));

    // Precondición del fixture: el resultado hipotético introduce un LINK-TARGET-MISSING/Err.
    assert!(
        hipotetico
            .analyze()
            .diagnostics
            .values()
            .flatten()
            .any(|c| c.code == CheckCode::LinkTargetMissing && c.level == Severity::Err),
        "el workspace hipotético debe introducir un `LINK-TARGET-MISSING`/Err",
    );

    let report = validate_result(&hipotetico);

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

    // MIGRADO en E16-H05: el fixture disparaba `FMT-TAGS`/Warn con `tags` como escalar, y ese
    // código se retiró — el catálogo mínimo de `§20.9` no tiene HOY ningún productor de `Warn`
    // dentro de `all_checks` (los códigos vivos de documento son `Err`). Así que el criterio se
    // prueba en dos mitades:
    //   (a) sobre un workspace real: sin errores → conforme y aplicable;
    //   (b) sobre un `ValidationReport` construido a mano con warnings: es la ÚNICA forma de
    //       ejercitar hoy la rama `allowWarnings` de `can_apply`, y sigue siendo el contrato que
    //       el criterio fija (E17/E20 devolverán códigos `Warn` al catálogo).
    let hipotetico = DocumentSet::from_files(fm(&[(
        "nota.md",
        "---\ntype: Nota\ntitle: T\ndescription: d\ntags: uno\n---\n\n# H\n\ncuerpo\n",
    )]));

    // Precondición del fixture: 0 errores duros.
    assert_eq!(
        hipotetico.analyze().hard_fail(),
        0,
        "el workspace hipotético no debe tener ningún Err",
    );

    let report = validate_result(&hipotetico);

    assert_eq!(
        report.summary.errors, 0,
        "un resultado sin errores tiene 0 errores; summary = {:?}",
        report.summary,
    );
    assert!(
        report.conformant,
        "sin errores el resultado es conforme (`conformant == true`); report = {:?}",
        report,
    );

    let policy = PlanPolicy {
        require_conformant_result: true,
        allow_warnings: true,
    };
    assert!(
        can_apply(&report, &policy),
        "con resultado conforme y `allowWarnings:true`, el plan es aplicable",
    );

    // (b) La rama `allowWarnings` propiamente dicha: un resultado conforme CON warnings es
    //     aplicable si la política los permite, y solo entonces.
    let mut con_warnings = report.clone();
    con_warnings.summary.warnings = 2;
    assert!(
        can_apply(&con_warnings, &policy),
        "con `allowWarnings:true`, los warnings no bloquean; report = {con_warnings:?}",
    );
    assert!(
        !can_apply(
            &con_warnings,
            &PlanPolicy {
                require_conformant_result: true,
                allow_warnings: false,
            }
        ),
        "con `allowWarnings:false`, un solo warning bloquea el plan; report = {con_warnings:?}",
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
//       doc_set: &DocumentSet, schema: &Schema, path: &RelPath,
//       doctype: &str, title: Option<&str>, body: Option<String>,
//   ) -> Result<NormalizedOperation, CoreError>;
//   pub fn normalize_replace_text(
//       doc_set: &DocumentSet, path: &RelPath,
//       find: &str, replace: &str, expected_occurrences: Option<usize>,
//   ) -> Result<NormalizedOperation, CoreError>;
//   pub fn normalize_edit_section(
//       doc_set: &DocumentSet, path: &RelPath,
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

// E20-H03 — RETIRADO `create_usa_plantilla`: `bodyTemplate` era un campo de `DocType`
// (`core::schema`), que desaparece con el modelo universal (`§20.10`). `normalize_create` ya no
// expande plantillas; `body: None` deja el cuerpo en `None` (lo cubre `escenario_02_crear_valido`
// del benchmark, que crea un documento sin plantilla).

/// Criterio `replace_text_ocurrencias`: **Dado** `replace_text` con `expectedOccurrences:1` y 2
/// coincidencias, **Cuando** se normaliza, **Entonces** error (no aplica). Se añade un control
/// positivo con `expectedOccurrences:2` (el número correcto) para probar que el fallo es
/// EXACTAMENTE el desajuste de conteo y no otro error del fixture.
#[test]
fn replace_text_ocurrencias() {
    // Cuerpo con la palabra `token` EXACTAMENTE dos veces.
    let b = DocumentSet::from_files(fm(&[(
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
    let b = DocumentSet::from_files(fm(&[("seguridad.md", raw)]));
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
    let b = DocumentSet::from_files(fm(&[("uso.md", raw)]));
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
//       doc_set: &DocumentSet, from: &RelPath, to: &RelPath, rewrite_inbound_links: bool,
//   ) -> Result<Vec<NormalizedOperation>, CoreError>;
//   pub fn normalize_delete(
//       doc_set: &DocumentSet, path: &RelPath, policy: InboundLinksPolicy,
//   ) -> Result<Vec<NormalizedOperation>, CoreError>;
//
// Forma RESUELTA del `Vec` de salida (contrato que estos tests fijan):
//   * `normalize_move(.., rewrite:true)` → un `NormalizedOperation::Move { from, to, .. }` MÁS,
//     por cada documento que enlaza a `from`, una operación que reescribe ese enlace a `to`. Como el
//     enlace vive en el CUERPO (`[x](/from.md)`), la reescritura natural es un `ReplaceBody` del
//     documento entrante con el href actualizado a `/to.md`. Estos tests NO exigen la variante exacta
//     (aceptan cualquier op de contenido cuyo `path` sea el entrante), pero SÍ exigen que el enlace
//     quede realmente reescrito: la op referencia `/destino.md` y ya NO `/target.md`.
//   * `normalize_delete(.., Reject)` sobre un documento con entrantes → `Err`. El error DEBE ser la
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

/// Path del documento tocado por una op de CONTENIDO (reescritura o eliminación de enlace). Las ops
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

    // Workspace: index raíz + `target.md` + 30 documentos `r1.md`..`r30.md`, cada uno con un enlace de
    // cuerpo `[target](/target.md)`.
    let mut files: FileMap = FileMap::new();
    files.insert(
        RelPath::new("index.md").unwrap(),
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# B\n".to_string(),
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
    let b = DocumentSet::from_files(files);

    // Precondición del fixture: `target.md` recibe exactamente 30 backlinks entrantes.
    let inbound = b.backlinks(&from).inbound.len();
    assert_eq!(
        inbound, 30,
        "el fixture debe dar 30 backlinks a target.md, dio {inbound}",
    );

    let ops = lodestar_core::plan::normalize_move(&b, &from, &to, true)
        .expect("mover un documento con backlinks y rewrite:true no debe fallar la normalización");

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
                         un documento entrante; fue {op:?}",
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
/// (`reject`) sobre un documento referenciado, **Cuando** se normaliza, **Entonces** se rechaza con
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

    let b = DocumentSet::from_files(fm(&[
        ("index.md", "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# B\n"),
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
            "borrar un documento referenciado con la política por defecto `reject` debe fallar",
        );

    let dbg = format!("{err:?}");
    assert!(
        dbg.contains("InboundLinksExist"),
        "el rechazo debe ser la variante de `CoreError` que mapea a `ErrorCode::InboundLinksExist` \
         (wire \"INBOUND_LINKS_EXIST\"); error = {err:?}",
    );
}

/// Criterio `delete_remove_links`: **Dado** un `delete` con `remove_links` sobre un documento
/// referenciado, **Cuando** se normaliza, **Entonces** el change set incluye el borrado MÁS quitar
/// esos enlaces en los documentos entrantes.
#[test]
fn delete_remove_links() {
    let target = RelPath::new("target.md").unwrap();

    let b = DocumentSet::from_files(fm(&[
        ("index.md", "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# B\n"),
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
        .expect("borrar con `remove_links` sobre un documento referenciado no debe fallar");

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
                        "toda op no-`Delete` del change set debe quitar el enlace de un documento \
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

// ---------------------------------------------------------------------------
// E20-H03 — RETIRADOS los tests E12-H07 de operaciones SEMÁNTICAS con validación de schema:
//   · `add_relation_invalida` (validaba el target contra `RelationDef` → RELATION_CONSTRAINT_VIOLATION)
//   · `transicion_invalida`   (validaba `to` contra `allowedStatuses` → InvalidStatusTransition)
//   · `apply_fix_safe`        (materializaba el `Fix{safe}` del `REL-TARGET` de una relación rota)
// Con `core::schema` desaparecen tipos, `RelationDef`, `allowedStatuses` y el productor de `Fix`
// (`validate_relations`). Las operaciones `add_relation`/`transition_status` sobreviven SIN validar
// (escriben el campo del frontmatter tal cual, `§20.10`) y `apply_fix` responde siempre
// `FixNotFound` (ningún diagnóstico adjunta fixes); Fase 12 retira las tres. Su normalización
// trivial ya no tiene criterio propio que fijar aquí.
// ---------------------------------------------------------------------------

// ===========================================================================
// E21-H03 — `move_document` con reescritura de backlinks (`ARCHITECTURE.md §20.11`,
// `REFACTOR_PHASE_2 §Fase 12 (Movimiento de documentos)`). Fase ROJA.
//
// A diferencia del test E12-H06 `move_reescribe_entrantes` (que usa enlaces raíz-absolutos
// `/target.md`, insensibles a la profundidad del origen), estos dos fijan el CORAZÓN de la
// historia:
//   · `move_reescribe_backlinks` — 3 backlinks INLINE relativos desde profundidades DISTINTAS: el
//     href recalculado es distinto desde cada origen (`docs/security/auth.md` vs `security/auth.md`
//     vs `../../docs/security/auth.md`), y label + fragmento se conservan. Con la reescritura por
//     regex vigente (`LINK_REWRITE_RE`) ya funciona para enlaces inline — es el GUARD que impedirá
//     que la migración a reescritura por `span` (que exige `move_reescribe_referencia`) regrese el
//     recálculo relativo.
//   · `move_reescribe_referencia` — un backlink de REFERENCIA (`[t][id]` + `[id]: destino`): la
//     definición debe reescribirse. La regex `LINK_REWRITE_RE` solo ve `[texto](href)` inline, NO
//     las definiciones `[id]: href`, así que HOY este test es ROJO — fuerza la reescritura por el
//     `span` de bytes del destino (E17-H01 lo dejó anotado como propio de `move_document`,
//     `§20.11`), que sí alcanza la definición.
// ===========================================================================

/// Cuerpo (post-frontmatter) del `ReplaceBody` que reescribe el documento `path`, o `panic` si el
/// change set no incluye ninguno para ese origen. Es la vista por la que se juzga qué quedó escrito
/// en cada backlink reescrito.
fn body_de_replace(ops: &[NormalizedOperation], path: &RelPath) -> String {
    ops.iter()
        .find_map(|op| match op {
            NormalizedOperation::ReplaceBody { path: p, body } if p == path => Some(body.clone()),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!("el change set debe incluir un ReplaceBody para {path:?}: {ops:?}")
        })
}

/// Los paths (ordenados y deduplicados) de los orígenes entrantes de un documento.
fn origenes_entrantes(b: &DocumentSet, target: &RelPath) -> Vec<RelPath> {
    paths_ordenados(
        b.backlinks(target)
            .inbound
            .into_iter()
            .map(|l| l.from)
            .collect(),
    )
}

/// Criterio `move_reescribe_backlinks`: **Dado** `docs/auth.md` con 3 backlinks desde profundidades
/// DISTINTAS, **Cuando** se mueve a `docs/security/auth.md` con `rewriteInboundLinks`, **Entonces**
/// los 3 orígenes tienen el enlace recalculado (relativo correcto desde cada uno) conservando su
/// label (y su fragmento).
///
/// El recálculo relativo es el núcleo: desde la raíz el nuevo href es `docs/security/auth.md`; desde
/// un hermano en `docs/` es `security/auth.md`; desde `pkg/a/b.md` (3 niveles) es
/// `../../docs/security/auth.md`. Tres orígenes a la misma profundidad NO probarían nada — la gracia
/// es que el destino relativo es DISTINTO desde cada uno.
#[test]
fn move_reescribe_backlinks() {
    let from = RelPath::new("docs/auth.md").unwrap();
    let to = RelPath::new("docs/security/auth.md").unwrap();

    // Tres orígenes a profundidades 0/1/3, cada uno con un enlace INLINE relativo a `docs/auth.md`.
    // El de `pkg/a/b.md` lleva label multi-palabra y fragmento, para fijar que ambos sobreviven.
    let b = DocumentSet::from_files(fm(&[
        (
            "docs/auth.md",
            "---\ntitle: Auth\n---\n\n# Auth\n\n## rotacion\n\ncuerpo\n",
        ),
        (
            "README.md",
            "---\ntitle: Readme\n---\n\nRaíz: [Autenticación](docs/auth.md).\n",
        ),
        (
            "docs/otro.md",
            "---\ntitle: Otro\n---\n\nHermano: [Auth](auth.md).\n",
        ),
        (
            "pkg/a/b.md",
            "---\ntitle: B\n---\n\nProfundo: [Ver rotación](../../docs/auth.md#rotacion).\n",
        ),
    ]));

    // Precondición del fixture: los 3 documentos son backlinks de `docs/auth.md`.
    let esperados = paths_ordenados(vec![
        RelPath::new("README.md").unwrap(),
        RelPath::new("docs/otro.md").unwrap(),
        RelPath::new("pkg/a/b.md").unwrap(),
    ]);
    assert_eq!(
        origenes_entrantes(&b, &from),
        esperados,
        "el fixture debe dejar `docs/auth.md` con backlinks desde README.md, docs/otro.md y pkg/a/b.md",
    );

    let ops = lodestar_core::plan::normalize_move(&b, &from, &to, true)
        .expect("mover con backlinks relativos y rewrite:true no debe fallar la normalización");

    // 1) Un único `Move` + una reescritura por cada uno de los 3 orígenes.
    let moves = ops
        .iter()
        .filter(|op| matches!(op, NormalizedOperation::Move { .. }))
        .count();
    assert_eq!(moves, 1, "debe haber exactamente un `Move`; ops = {ops:?}");
    let reescritos = paths_ordenados(ops.iter().filter_map(path_op_enlace).collect::<Vec<_>>());
    assert_eq!(
        reescritos, esperados,
        "debe reescribirse exactamente cada uno de los 3 orígenes; reescritos = {reescritos:?}",
    );

    // 2) Cada origen recalcula su href relativo (distinto según su profundidad), conservando label y
    //    fragmento. La aserción por substring exacto casa label + href + fragmento a la vez.
    let readme = body_de_replace(&ops, &RelPath::new("README.md").unwrap());
    assert!(
        readme.contains("[Autenticación](docs/security/auth.md)"),
        "desde la raíz el enlace debe recalcularse a `docs/security/auth.md` conservando el label; \
         cuerpo = {readme:?}",
    );
    assert!(
        !readme.contains("](docs/auth.md)"),
        "el enlace de la raíz no debe conservar el destino viejo; cuerpo = {readme:?}",
    );

    let otro = body_de_replace(&ops, &RelPath::new("docs/otro.md").unwrap());
    assert!(
        otro.contains("[Auth](security/auth.md)"),
        "desde un hermano en `docs/` el enlace debe recalcularse a `security/auth.md`; \
         cuerpo = {otro:?}",
    );
    assert!(
        !otro.contains("](auth.md)"),
        "el enlace del hermano no debe conservar el destino viejo `auth.md`; cuerpo = {otro:?}",
    );

    let profundo = body_de_replace(&ops, &RelPath::new("pkg/a/b.md").unwrap());
    assert!(
        profundo.contains("[Ver rotación](../../docs/security/auth.md#rotacion)"),
        "desde `pkg/a/b.md` (3 niveles) el enlace debe recalcularse a `../../docs/security/auth.md` \
         conservando label y fragmento `#rotacion`; cuerpo = {profundo:?}",
    );
    assert!(
        !profundo.contains("](../../docs/auth.md#"),
        "el enlace profundo no debe conservar el destino viejo; cuerpo = {profundo:?}",
    );
}

/// Criterio `move_reescribe_referencia`: **Dado** un backlink que es un enlace de REFERENCIA
/// (`[t][id]` con su definición `[id]: destino`), **Cuando** el documento se mueve, **Entonces** la
/// DEFINICIÓN se reescribe al nuevo destino relativo.
///
/// ROJO esperado HOY: `rewrite_body_links` reescribe con `LINK_REWRITE_RE`, una regex que solo casa
/// enlaces inline `[texto](href)`; NO ve las definiciones `[id]: href` ni el uso `[t][id]`, así que
/// el cuerpo del entrante vuelve intacto y la definición sigue apuntando al destino viejo. El
/// arreglo es reescribir por el `span` de bytes del destino (que en un enlace de referencia cae
/// dentro de su definición, E17-H01), no por regex — `§20.11`.
#[test]
fn move_reescribe_referencia() {
    let from = RelPath::new("docs/auth.md").unwrap();
    let to = RelPath::new("docs/security/auth.md").unwrap();

    let b = DocumentSet::from_files(fm(&[
        ("docs/auth.md", "---\ntitle: Auth\n---\n\n# Auth\n"),
        (
            "docs/otro.md",
            "---\ntitle: Otro\n---\n\nVer [autenticación][auth].\n\n[auth]: auth.md\n",
        ),
    ]));

    // Precondición: el enlace de referencia cuenta como backlink de `docs/auth.md`.
    assert_eq!(
        origenes_entrantes(&b, &from),
        vec![RelPath::new("docs/otro.md").unwrap()],
        "el enlace de referencia `[autenticación][auth]` + `[auth]: auth.md` debe ser un backlink \
         de `docs/auth.md`",
    );

    let ops = lodestar_core::plan::normalize_move(&b, &from, &to, true).expect(
        "mover con un backlink de referencia y rewrite:true no debe fallar la normalización",
    );

    let otro = body_de_replace(&ops, &RelPath::new("docs/otro.md").unwrap());

    // El uso `[autenticación][auth]` se conserva intacto (solo cambia el destino, en la definición).
    assert!(
        otro.contains("[autenticación][auth]"),
        "el uso del enlace de referencia debe conservarse; cuerpo = {otro:?}",
    );
    // Y la DEFINICIÓN se reescribe al nuevo destino relativo desde `docs/otro.md`: `security/auth.md`.
    assert!(
        otro.contains("[auth]: security/auth.md"),
        "la DEFINICIÓN del enlace de referencia debe reescribirse a `[auth]: security/auth.md` \
         (reescritura por el `span` del destino, no por la regex de enlaces inline); cuerpo = {otro:?}",
    );
    assert!(
        !otro.contains("[auth]: auth.md"),
        "la definición no debe conservar el destino viejo `auth.md`; cuerpo = {otro:?}",
    );
}
