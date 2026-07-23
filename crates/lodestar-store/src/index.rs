//! Motor de indexación: extracción de filas desde el core y upsert/delete transaccional
//! (`ARCHITECTURE.md §5`, store v2 `§20.12`). El store **no reimplementa checks**: los diagnostics
//! locales salen de `core::analyze`; los de enlace (no locales) se **sintetizan** al leer (ver
//! `synth`). La clasificación de enlaces (`target_kind`) y la metadata (`walk`) son proyecciones de
//! la única verdad del core, nunca un segundo navegador (invariante #3).

use rusqlite::{params, Transaction};

use lodestar_core::links;
use lodestar_core::model;
use lodestar_core::types::{CheckCode, Inventory, LinkTarget, RelPath, Severity};

use crate::error::StoreError;

/// ¿Es un diagnóstico de **enlace**? Esos no se materializan: dependen del inventario entero
/// (crear un fichero repara el enlace roto de otro documento), así que materializarlos obligaría a
/// invalidar en cascada. Se sintetizan al leer (`synth::link_diagnostics`, `§10` fila 10).
pub(crate) fn es_de_enlace(code: CheckCode) -> bool {
    matches!(
        code,
        CheckCode::LinkTargetMissing
            | CheckCode::LinkEscapesWorkspace
            | CheckCode::LinkCaseMismatch
    )
}

/// Diagnostics **locales** de un fichero (todos menos los de enlace, que se sintetizan al leer).
///
/// Se computan con el core (autoridad) sobre un workspace de un solo fichero: como los checks
/// locales dependen solo del contenido propio, el resultado —incluido el `range`— es idéntico al
/// del workspace completo.
fn local_diagnostics(path: &RelPath, raw: &str) -> Vec<lodestar_core::types::Check> {
    let mut fm = lodestar_core::types::FileMap::new();
    fm.insert(path.clone(), raw.to_string());
    let doc_set = lodestar_core::DocumentSet::from_files(fm);
    doc_set
        .analyze()
        .diagnostics
        .get(path)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| !es_de_enlace(c.code))
        .collect()
}

/// El valor de wire de un [`Severity`] (`§20.9`), que es la columna `diagnostics.severity`.
fn severity_str(level: Severity) -> &'static str {
    match level {
        Severity::Pass => "pass",
        Severity::Info => "info",
        Severity::Warn => "warn",
        Severity::Err => "err",
    }
}

/// El `value_type` de un valor YAML: el **catálogo cerrado de 6** de `§20.8` (`string`, `number`,
/// `boolean`, `null`, `array`, `object`). Un valor con tag YAML (`!Foo x`) se clasifica por su
/// valor interior — la etiqueta no cambia la forma del dato.
fn value_type(v: &serde_yaml::Value) -> &'static str {
    match v {
        serde_yaml::Value::Null => "null",
        serde_yaml::Value::Bool(_) => "boolean",
        serde_yaml::Value::Number(_) => "number",
        serde_yaml::Value::String(_) => "string",
        serde_yaml::Value::Sequence(_) => "array",
        serde_yaml::Value::Mapping(_) => "object",
        serde_yaml::Value::Tagged(t) => value_type(&t.value),
    }
}

/// El discriminante serde de un [`LinkTarget`] (`document`, `workspaceFile`, `externalUri`,
/// `selfAnchor`, `missing`, `escapesWorkspace`): la etiqueta `kind` que el propio enum define para
/// el wire. La columna `links.target_kind` es la **proyección** de esa clasificación del core, no un
/// vocabulario paralelo de la cache (invariante #3).
fn target_kind(target: &LinkTarget) -> String {
    serde_json::to_value(target)
        .ok()
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_string))
        .unwrap_or_default()
}

/// El path de destino sin fragmento (la columna `links.target_path`), o `None` para los destinos
/// sin path (externo, anchor propio, escape).
fn target_path(target: &LinkTarget) -> Option<String> {
    match target {
        LinkTarget::Document(p) | LinkTarget::WorkspaceFile(p) | LinkTarget::Missing(p) => {
            Some(p.as_str().to_string())
        }
        LinkTarget::ExternalUri(_) | LinkTarget::SelfAnchor(_) | LinkTarget::EscapesWorkspace => {
            None
        }
    }
}

/// ¿El enlace apunta a algo que **existe** en el workspace? Solo un documento o un fichero del
/// proyecto son destinos concretos presentes; un `missing`, un escape, un anchor sin verificar o
/// una URI externa (que el motor no resuelve) cuentan como no resueltos. Los dos casos que la
/// historia fija (`document` → resuelto, `missing` → no) caen aquí sin ambigüedad.
fn is_resolved(target: &LinkTarget) -> bool {
    matches!(
        target,
        LinkTarget::Document(_) | LinkTarget::WorkspaceFile(_)
    )
}

/// Borra todas las filas materializadas de un path (en todas las tablas de documento).
pub(crate) fn delete_file(tx: &Transaction, path: &RelPath) -> Result<(), StoreError> {
    let p = path.as_str();
    tx.execute("DELETE FROM documents WHERE path = ?1", params![p])?;
    tx.execute("DELETE FROM metadata WHERE document_path = ?1", params![p])?;
    tx.execute("DELETE FROM links WHERE source_path = ?1", params![p])?;
    tx.execute(
        "DELETE FROM diagnostics WHERE document_path = ?1",
        params![p],
    )?;
    tx.execute("DELETE FROM files_fts WHERE path = ?1", params![p])?;
    Ok(())
}

/// Upsert de un documento: borra sus filas previas e inserta las nuevas
/// (documents/metadata/links/diagnostics/fts).
///
/// `inventory` es el inventario del workspace (documentos + `other_files`) con el que se resuelven y
/// clasifican los enlaces: es lo que distingue un `workspaceFile` de un `missing` (E18-H02). Lo
/// aporta el llamante —fresco del disco en un rebuild, reconstruido de la cache en un upsert
/// incremental— porque la clasificación de un enlace depende del workspace entero, no solo del
/// documento que se indexa.
pub(crate) fn upsert_file(
    tx: &Transaction,
    path: &RelPath,
    raw: &str,
    mtime: i64,
    size: i64,
    inventory: &Inventory,
) -> Result<(), StoreError> {
    delete_file(tx, path)?;

    let parsed = model::parse_file(path.as_str(), raw);
    let fm = parsed.frontmatter.clone();
    let hash = blake3::hash(raw.as_bytes());
    // El título DERIVADO (`§20.4`): frontmatter.title → primer H1 → nombre del fichero. No es el
    // campo `title` del usuario (que sigue siendo metadata como cualquiera).
    let title = model::derived_title(fm.as_ref(), &parsed.body, path);
    // La cache materializa el frontmatter ARBITRARIO tal cual (E16-H01): el `value` YAML entero.
    let fm_json = fm
        .as_ref()
        .map(|f| serde_json::to_string(&f.value).unwrap_or_else(|_| "{}".to_string()))
        .unwrap_or_else(|| "{}".to_string());
    let p = path.as_str();

    tx.execute(
        r#"INSERT INTO documents
           (path, title, body, raw, frontmatter_json, content_hash, mtime, size)
           VALUES (?1,?2,?3,?4,?5,?6,?7,?8)"#,
        params![
            p,
            title,
            parsed.body,
            raw,
            fm_json,
            hash.as_bytes().as_slice(),
            mtime,
            size,
        ],
    )?;

    // FTS5: `title` derivado + `description` (valor textual de esa clave, si la hay) + `body`. El
    // rediseño sin campos privilegiados es E18-H03; aquí solo es un acelerador.
    tx.execute(
        "INSERT INTO files_fts (path, title, description, body) VALUES (?1,?2,?3,?4)",
        params![
            p,
            title,
            fm.as_ref()
                .and_then(|f| f.get_text("description"))
                .unwrap_or_default(),
            parsed.body,
        ],
    )?;

    // Metadata genérica: una fila por propiedad direccionable del frontmatter (`walk`, E18-H01),
    // mapas intermedios incluidos y listas como hoja. `walk` ES la única verdad de acceso: la cache
    // nunca navega el `Value` por su cuenta (invariante #3).
    if let Some(f) = fm.as_ref() {
        for (field_path, valor) in f.walk() {
            let value_json = serde_json::to_string(
                &serde_json::to_value(valor).unwrap_or(serde_json::Value::Null),
            )
            .unwrap_or_else(|_| "null".to_string());
            tx.execute(
                "INSERT INTO metadata (document_path, field_path, value_json, value_type) \
                 VALUES (?1,?2,?3,?4)",
                params![p, field_path.to_string(), value_json, value_type(valor)],
            )?;
        }
    }

    // Enlaces: TODOS los del cuerpo, en orden de aparición, con su clasificación (`§20.6`). A
    // diferencia del store v1 —que solo guardaba las aristas internas resueltas— aquí se materializa
    // cada enlace con su `target_kind`, de modo que la cache puede responder por externos, anchors y
    // ficheros del proyecto. `is_edge` (= `internal_path().is_some()`) marca las aristas del grafo:
    // lo computa el core para no reimplementar `is_markdown` en SQL, y es lo que filtran las
    // consultas de grafo de `synth` (backlinks/aislados/colgantes/blast-radius).
    for raw_link in links::extract_links(&parsed.body) {
        let resuelto = links::resolve(&raw_link, path, inventory);
        tx.execute(
            r#"INSERT INTO links
               (source_path, raw_href, target_kind, target_path, fragment, resolved, is_edge)
               VALUES (?1,?2,?3,?4,?5,?6,?7)"#,
            params![
                p,
                resuelto.href,
                target_kind(&resuelto.target),
                target_path(&resuelto.target),
                resuelto.fragment,
                is_resolved(&resuelto.target) as i64,
                resuelto.target.internal_path().is_some() as i64,
            ],
        )?;
    }

    // Diagnostics locales (el core es la autoridad; los de enlace se sintetizan al leer). `range` va
    // serializado a `range_json` (E18-H02), o `NULL` si el diagnóstico no conoce su posición.
    for check in local_diagnostics(path, raw) {
        let range_json = check
            .range
            .map(|r| serde_json::to_string(&r).unwrap_or_else(|_| "null".to_string()));
        tx.execute(
            "INSERT INTO diagnostics (document_path, code, severity, message, range_json) \
             VALUES (?1,?2,?3,?4,?5)",
            params![
                p,
                check.code.as_str(),
                severity_str(check.level),
                check.msg,
                range_json,
            ],
        )?;
    }

    Ok(())
}
