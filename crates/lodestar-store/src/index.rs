//! Motor de indexación: extracción de filas desde el core y upsert/delete transaccional
//! (`ARCHITECTURE.md §5`, store v2 `§20.12`). El store **no reimplementa checks**: los diagnostics
//! locales salen de `core::analyze`; los de enlace (no locales) se **sintetizan** al leer (ver
//! `synth`). La clasificación de enlaces (`target_kind`) y la metadata (`walk`) son proyecciones de
//! la única verdad del core, nunca un segundo navegador (invariante #3).

use rusqlite::{params, OptionalExtension, Transaction};

use lodestar_core::links;
use lodestar_core::model;
use lodestar_core::types::{CheckCode, FieldPath, Inventory, LinkTarget, RelPath, Severity};

use crate::{error::StoreError, SqlAudit};

/// Prepared projection used by the insert-only cold builder.  The statements are created once
/// per build and reused for every document; unlike `upsert_file`, this path never deletes rows.
pub(crate) struct StreamingProjection<'tx> {
    document_ids: std::collections::BTreeMap<RelPath, i64>,
    document_update: rusqlite::Statement<'tx>,
    reattach_links: rusqlite::Statement<'tx>,
    field_insert: rusqlite::Statement<'tx>,
    field_id: rusqlite::Statement<'tx>,
    metadata_insert: rusqlite::Statement<'tx>,
    link_insert: rusqlite::Statement<'tx>,
    diagnostic_insert: rusqlite::Statement<'tx>,
    fts_insert: rusqlite::Statement<'tx>,
}

/// Documento ya leído y parseado por la pasada de streaming. Mantenerlo tipado evita que la
/// proyección vuelva a parsear el cuerpo y mantiene su firma por debajo del límite de Clippy.
pub(crate) struct ProjectionDocument<'a> {
    pub(crate) path: &'a RelPath,
    pub(crate) raw: &'a str,
    pub(crate) parsed: &'a model::Parsed,
    pub(crate) doc_id: i64,
    pub(crate) mtime: i64,
    pub(crate) size: i64,
}

impl<'tx> StreamingProjection<'tx> {
    pub(crate) fn prepare(
        tx: &'tx Transaction<'tx>,
        mut begin_prepare: impl FnMut() -> u64,
        mut finish_prepare: impl FnMut(u64, &str, &str) -> Result<(), StoreError>,
        mut on_prepare: impl FnMut(&str, &str),
    ) -> Result<Self, StoreError> {
        let mark = begin_prepare();
        let document_update = tx.prepare("UPDATE documents SET title=?1,body=?2,frontmatter_json=?3,frontmatter_text=?4,content_hash=?5,mtime=?6,size=?7 WHERE doc_id=?8")?;
        finish_prepare(mark, "streaming.document_update", "UPDATE documents SET title=?1,body=?2,frontmatter_json=?3,frontmatter_text=?4,content_hash=?5,mtime=?6,size=?7 WHERE doc_id=?8")?;
        on_prepare("streaming.document_update", "UPDATE documents SET title=?1,body=?2,frontmatter_json=?3,frontmatter_text=?4,content_hash=?5,mtime=?6,size=?7 WHERE doc_id=?8");
        let mark = begin_prepare();
        let reattach_links = tx.prepare("UPDATE links SET target_doc_id=?1,target_path=NULL,target_kind=?2,resolved=?3,is_edge=?4 WHERE target_path=?5 AND target_kind=?6 AND target_doc_id IS NULL")?;
        finish_prepare(mark, "streaming.reattach_links", "UPDATE links SET target_doc_id=?1,target_path=NULL,target_kind=?2,resolved=?3,is_edge=?4 WHERE target_path=?5 AND target_kind=?6 AND target_doc_id IS NULL")?;
        on_prepare("streaming.reattach_links", "UPDATE links SET target_doc_id=?1,target_path=NULL,target_kind=?2,resolved=?3,is_edge=?4 WHERE target_path=?5 AND target_kind=?6 AND target_doc_id IS NULL");
        let mark = begin_prepare();
        let field_insert = tx.prepare("INSERT OR IGNORE INTO fields(field_path) VALUES(?1)")?;
        finish_prepare(
            mark,
            "streaming.field_insert",
            "INSERT OR IGNORE INTO fields(field_path) VALUES(?1)",
        )?;
        on_prepare(
            "streaming.field_insert",
            "INSERT OR IGNORE INTO fields(field_path) VALUES(?1)",
        );
        let mark = begin_prepare();
        let field_id = tx.prepare("SELECT field_id FROM fields WHERE field_path=?1")?;
        finish_prepare(
            mark,
            "streaming.field_id",
            "SELECT field_id FROM fields WHERE field_path=?1",
        )?;
        on_prepare(
            "streaming.field_id",
            "SELECT field_id FROM fields WHERE field_path=?1",
        );
        let mark = begin_prepare();
        let metadata_insert = tx.prepare(
            "INSERT INTO metadata(doc_id,field_id,value_json,value_type) VALUES(?1,?2,?3,?4)",
        )?;
        finish_prepare(
            mark,
            "streaming.metadata_insert",
            "INSERT INTO metadata(doc_id,field_id,value_json,value_type) VALUES(?1,?2,?3,?4)",
        )?;
        on_prepare(
            "streaming.metadata_insert",
            "INSERT INTO metadata(doc_id,field_id,value_json,value_type) VALUES(?1,?2,?3,?4)",
        );
        let mark = begin_prepare();
        let link_insert = tx.prepare("INSERT INTO links(source_doc_id,target_doc_id,raw_href,target_kind,target_path,fragment,resolved,is_edge) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
        finish_prepare(mark, "streaming.link_insert", "INSERT INTO links(source_doc_id,target_doc_id,raw_href,target_kind,target_path,fragment,resolved,is_edge) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
        on_prepare("streaming.link_insert", "INSERT INTO links(source_doc_id,target_doc_id,raw_href,target_kind,target_path,fragment,resolved,is_edge) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)");
        let mark = begin_prepare();
        let diagnostic_insert = tx.prepare("INSERT INTO diagnostics(doc_id,code,severity,message,range_json) VALUES(?1,?2,?3,?4,?5)")?;
        finish_prepare(mark, "streaming.diagnostic_insert", "INSERT INTO diagnostics(doc_id,code,severity,message,range_json) VALUES(?1,?2,?3,?4,?5)")?;
        on_prepare("streaming.diagnostic_insert", "INSERT INTO diagnostics(doc_id,code,severity,message,range_json) VALUES(?1,?2,?3,?4,?5)");
        let mark = begin_prepare();
        let fts_insert = tx.prepare("INSERT INTO documents_fts(rowid,path,title,body,frontmatter_text) VALUES(?1,?2,?3,?4,?5)")?;
        finish_prepare(mark, "streaming.fts_insert", "INSERT INTO documents_fts(rowid,path,title,body,frontmatter_text) VALUES(?1,?2,?3,?4,?5)")?;
        on_prepare("streaming.fts_insert", "INSERT INTO documents_fts(rowid,path,title,body,frontmatter_text) VALUES(?1,?2,?3,?4,?5)");
        Ok(Self {
            document_ids: std::collections::BTreeMap::new(),
            document_update,
            reattach_links,
            field_insert,
            field_id,
            metadata_insert,
            link_insert,
            diagnostic_insert,
            fts_insert,
        })
    }

    pub(crate) fn insert(
        &mut self,
        document: ProjectionDocument<'_>,
        inventory: &Inventory,
        sql_audit: &SqlAudit,
        mut on_insert: impl FnMut(&str),
    ) -> Result<(), StoreError> {
        let ProjectionDocument {
            path,
            raw,
            parsed,
            doc_id,
            mtime,
            size,
        } = document;
        // Updating the external-content row can make FTS5 maintain its shadow tables before the
        // explicit FTS insert below. Keep the audited allowance alive for the whole projection.
        let _fts_execution = sql_audit.fts_execution();
        let fm = parsed.frontmatter.clone();
        let hash = blake3::hash(raw.as_bytes());
        let title = model::derived_title(fm.as_ref(), &parsed.body, path);
        let fm_json = fm
            .as_ref()
            .map(|f| serde_json::to_string(&f.value).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|| "{}".to_string());
        // Metadata and FTS share one canonical text value from the one permitted `walk`. Keep
        // this String alive across both statements so contentless FTS can later be deleted with
        // exactly the same tuple persisted in `documents`.
        let (frontmatter_text, metadata_by_path) = if let Some(f) = fm.as_ref() {
            let mut fts_frontmatter = Vec::new();
            let mut metadata_by_path: std::collections::BTreeMap<
                String,
                (FieldPath, String, &'static str, bool),
            > = std::collections::BTreeMap::new();
            for (field_path, value) in f.walk() {
                let anchored = field_path.es_namespace_reservado();
                let field_path = if anchored {
                    field_path.anclado()
                } else {
                    field_path
                };
                let vtype = value_type(value);
                let value_json = serde_json::to_string(
                    &serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
                )
                .unwrap_or_else(|_| "null".to_string());
                if vtype == "string" || vtype == "array" {
                    fts_frontmatter.push(value_json.clone());
                }
                let key = field_path.to_string();
                let replace = metadata_by_path
                    .get(&key)
                    .map_or(true, |(_, _, _, previous_anchored)| {
                        anchored && !previous_anchored
                    });
                if replace {
                    metadata_by_path.insert(key, (field_path, value_json, vtype, anchored));
                }
            }
            (fts_frontmatter.join(" "), Some(metadata_by_path))
        } else {
            (String::new(), None)
        };
        self.document_update.execute(rusqlite::params![
            &title,
            raw,
            fm_json,
            &frontmatter_text,
            hash.as_bytes().as_slice(),
            mtime,
            size,
            doc_id,
        ])?;
        self.document_ids.insert(path.clone(), doc_id);
        let document_target = LinkTarget::Document(path.clone());
        let workspace_file_target = LinkTarget::WorkspaceFile(path.clone());
        self.reattach_links.execute(rusqlite::params![
            doc_id,
            target_kind(&document_target),
            is_resolved(&document_target) as i64,
            document_target.internal_path().is_some() as i64,
            path.as_str(),
            target_kind(&workspace_file_target),
        ])?;
        if let Some(metadata_by_path) = metadata_by_path {
            for (field_path, value_json, vtype, _) in metadata_by_path.into_values() {
                self.field_insert.execute([field_path.to_string()])?;
                let field_id: i64 = self
                    .field_id
                    .query_row([field_path.to_string()], |r| r.get(0))?;
                self.metadata_insert
                    .execute(rusqlite::params![doc_id, field_id, value_json, vtype,])?;
                on_insert("metadata");
            }
        }
        self.fts_insert.execute(rusqlite::params![
            doc_id,
            path.as_str(),
            &title,
            raw,
            &frontmatter_text,
        ])?;
        on_insert("documents_fts");

        for raw_link in links::extract_links(&parsed.body) {
            let resolved = links::resolve(&raw_link, path, inventory);
            let target_doc_id: Option<i64> = match &resolved.target {
                LinkTarget::Document(target) => self.document_ids.get(target).copied(),
                _ => None,
            };
            let persisted_target_path = if target_doc_id.is_some() {
                None
            } else {
                target_path(&resolved.target)
            };
            self.link_insert.execute(rusqlite::params![
                doc_id,
                target_doc_id,
                resolved.href,
                target_kind(&resolved.target),
                persisted_target_path,
                resolved.fragment,
                is_resolved(&resolved.target) as i64,
                resolved.target.internal_path().is_some() as i64,
            ])?;
            on_insert("links");
        }
        for check in lodestar_core::local_diagnostics(path, parsed, raw) {
            let range_json = check
                .range
                .map(|r| serde_json::to_string(&r).unwrap_or_else(|_| "null".to_string()));
            self.diagnostic_insert.execute(rusqlite::params![
                doc_id,
                check.code.as_str(),
                severity_str(check.level),
                check.msg,
                range_json,
            ])?;
            on_insert("diagnostics");
        }
        Ok(())
    }
}

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
/// sin path persistible (externo, anchor propio o escape). Un `WorkspaceDirectory(Some(path))`
/// conserva su ruta; la variante `None` representa la raíz y sigue sin path.
fn target_path(target: &LinkTarget) -> Option<String> {
    match target {
        LinkTarget::Document(p) | LinkTarget::WorkspaceFile(p) | LinkTarget::Missing(p) => {
            Some(p.as_str().to_string())
        }
        LinkTarget::WorkspaceDirectory(path) => path.as_ref().map(|path| path.as_str().to_string()),
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
    let Some(doc_id) = tx
        .query_row(
            "SELECT doc_id FROM documents WHERE path = ?1",
            params![path.as_str()],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    else {
        return Ok(());
    };
    tx.execute(
        "UPDATE links SET target_path=(SELECT path FROM documents WHERE doc_id=?1),target_doc_id=NULL,target_kind='missing',resolved=0 WHERE target_doc_id=?1",
        params![doc_id],
    )?;
    delete_fts_row(tx, doc_id)?;
    tx.execute("DELETE FROM documents WHERE doc_id = ?1", params![doc_id])?;
    Ok(())
}

fn delete_fts_row(tx: &Transaction, doc_id: i64) -> Result<(), StoreError> {
    let old = tx
        .query_row(
            "SELECT path, title, body, frontmatter_text FROM documents WHERE doc_id=?1 AND length(content_hash)>0",
            params![doc_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    // Contentless FTS5 no admite un escaneo por `rowid` para comprobar presencia. Un hash no vacío
    // identifica las filas que el único escritor ya indexó; el comando `delete` es idempotente para
    // el caso de una cache parcialmente escrita y necesita los valores antiguos explícitos.
    if let Some((path, title, body, frontmatter_text)) = old {
        tx.execute(
            "INSERT INTO documents_fts(documents_fts,rowid,path,title,body,frontmatter_text) VALUES('delete',?1,?2,?3,?4,?5)",
            params![doc_id, path, title, body, frontmatter_text],
        )?;
    }
    Ok(())
}

/// Reserva todos los `doc_id` de un rebuild antes de proyectar metadata, enlaces y diagnósticos.
/// Así un enlace hacia un documento que aparece después en el walker sigue teniendo su FK.
pub(crate) fn seed_document_ids(tx: &Transaction, paths: &[RelPath]) -> Result<(), StoreError> {
    for path in paths {
        tx.execute(
            "INSERT OR IGNORE INTO documents(path,title,body,frontmatter_json,frontmatter_text,content_hash,mtime,size) VALUES(?1,'','','{}','',zeroblob(0),0,0)",
            params![path.as_str()],
        )?;
    }
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

    let existing_doc_id: Option<i64> = tx
        .query_row(
            "SELECT doc_id FROM documents WHERE path=?1",
            params![p],
            |r| r.get(0),
        )
        .optional()?;
    let doc_id = match existing_doc_id {
        Some(doc_id) => doc_id,
        None => {
            tx.execute(
                "INSERT INTO documents(path,title,body,frontmatter_json,frontmatter_text,content_hash,mtime,size) VALUES(?1,'','','{}','',zeroblob(0),0,0)",
                params![p],
            )?;
            tx.last_insert_rowid()
        }
    };
    // El único escritor actualiza el documento y sus proyecciones en la misma transacción.
    delete_fts_row(tx, doc_id)?;
    tx.execute("DELETE FROM metadata WHERE doc_id=?1", params![doc_id])?;
    tx.execute("DELETE FROM links WHERE source_doc_id=?1", params![doc_id])?;
    tx.execute("DELETE FROM diagnostics WHERE doc_id=?1", params![doc_id])?;
    tx.execute(
        "UPDATE documents SET path=?1,title=?2,body=?3,frontmatter_json=?4,frontmatter_text=?5,content_hash=?6,mtime=?7,size=?8 WHERE doc_id=?9",
        params![p, title, raw, fm_json, "", hash.as_bytes().as_slice(), mtime, size, doc_id],
    )?;
    // La materialización incremental puede haber escrito antes un enlace dangling hacia este
    // path. Al reaparecer el documento, se reata el FK y su clasificación.
    let document_target = LinkTarget::Document(path.clone());
    let missing_target = LinkTarget::Missing(path.clone());
    tx.execute(
        "UPDATE links SET target_doc_id=?1,target_path=NULL,target_kind=?2,resolved=?3,is_edge=?4 WHERE target_path=?5 AND target_kind=?6 AND target_doc_id IS NULL",
        params![
            doc_id,
            target_kind(&document_target),
            is_resolved(&document_target) as i64,
            document_target.internal_path().is_some() as i64,
            p,
            target_kind(&missing_target),
        ],
    )?;

    // Metadata genérica: una fila por propiedad direccionable del frontmatter (`walk`, E18-H01),
    // mapas intermedios incluidos y listas como hoja. `walk` ES la única verdad de acceso: la cache
    // nunca navega el `Value` por su cuenta (invariante #3).
    //
    // El mismo recorrido alimenta el FTS (E18-H03): los valores TEXTUALES de la metadata van a
    // `frontmatter_text`. Son los escalares `string` —también las hojas string de un objeto, que
    // `walk` aplana en filas `string` con su field path— y las `array`, cuya representación JSON
    // conserva las cadenas que contienen (una lista no se desciende: `owners: [platform, security]`
    // es una sola fila `array`, y sin su `value_json` la palabra «security» no sería indexable).
    // Números y booleanos no aportan al texto libre (`§20.12`), así que se excluyen. Se recogen del
    // mismo `walk` que puebla `metadata` para no volver a navegar el `Value`.
    let mut fts_frontmatter: Vec<String> = Vec::new();
    if let Some(f) = fm.as_ref() {
        let mut metadata_by_path: std::collections::BTreeMap<
            String,
            (FieldPath, String, &'static str, bool),
        > = std::collections::BTreeMap::new();
        for (field_path, valor) in f.walk() {
            let anchored = field_path.es_namespace_reservado();
            let field_path: FieldPath = if anchored {
                field_path.anclado()
            } else {
                field_path
            };
            let vtype = value_type(valor);
            let value_json = serde_json::to_string(
                &serde_json::to_value(valor).unwrap_or(serde_json::Value::Null),
            )
            .unwrap_or_else(|_| "null".to_string());
            if vtype == "string" || vtype == "array" {
                fts_frontmatter.push(value_json.clone());
            }
            let key = field_path.to_string();
            let replace = metadata_by_path
                .get(&key)
                .map_or(true, |(_, _, _, previous_anchored)| {
                    anchored && !previous_anchored
                });
            if replace {
                metadata_by_path.insert(key, (field_path, value_json, vtype, anchored));
            }
        }
        for (field_path, value_json, vtype, _) in metadata_by_path.into_values() {
            tx.execute(
                "INSERT OR IGNORE INTO fields(field_path) VALUES(?1)",
                params![field_path.to_string()],
            )?;
            let field_id: i64 = tx.query_row(
                "SELECT field_id FROM fields WHERE field_path=?1",
                params![field_path.to_string()],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO metadata(doc_id,field_id,value_json,value_type) VALUES(?1,?2,?3,?4)",
                params![doc_id, field_id, value_json, vtype],
            )?;
        }
    }

    // FTS5 sin campos privilegiados (E18-H03, `§20.12`): `path` + título derivado + `body` +
    // `frontmatter_text` (los valores textuales de la metadata, recogidos arriba). `description`
    // deja de tener trato especial: es metadata como cualquier otra. Sigue siendo solo un acelerador.
    let frontmatter_text = fts_frontmatter.join(" ");
    tx.execute(
        "UPDATE documents SET frontmatter_text=?1 WHERE doc_id=?2",
        params![frontmatter_text, doc_id],
    )?;
    tx.execute(
        "INSERT INTO documents_fts(rowid,path,title,body,frontmatter_text) VALUES(?1,?2,?3,?4,?5)",
        params![doc_id, p, title, raw, frontmatter_text],
    )?;

    // Enlaces: TODOS los del cuerpo, en orden de aparición, con su clasificación (`§20.6`). A
    // diferencia del store v1 —que solo guardaba las aristas internas resueltas— aquí se materializa
    // cada enlace con su `target_kind`, de modo que la cache puede responder por externos, anchors y
    // ficheros del proyecto. `is_edge` (= `internal_path().is_some()`) marca las aristas del grafo:
    // lo computa el core para no reimplementar `is_markdown` en SQL, y es lo que filtran las
    // consultas de grafo de `synth` (backlinks/aislados/colgantes/blast-radius).
    for raw_link in links::extract_links(&parsed.body) {
        let resuelto = links::resolve(&raw_link, path, inventory);
        let target_doc_id: Option<i64> = match &resuelto.target {
            LinkTarget::Document(target) => tx
                .query_row(
                    "SELECT doc_id FROM documents WHERE path=?1",
                    params![target.as_str()],
                    |r| r.get(0),
                )
                .optional()?,
            _ => None,
        };
        let persisted_target_path = if target_doc_id.is_some() {
            None
        } else {
            target_path(&resuelto.target)
        };
        tx.execute(
            r#"INSERT INTO links
               (source_doc_id, target_doc_id, raw_href, target_kind, target_path, fragment, resolved, is_edge)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8)"#,
            params![
                doc_id,
                target_doc_id,
                resuelto.href,
                target_kind(&resuelto.target),
                persisted_target_path,
                resuelto.fragment,
                is_resolved(&resuelto.target) as i64,
                resuelto.target.internal_path().is_some() as i64,
            ],
        )?;
    }

    // Diagnostics locales (el core es la autoridad; los de enlace se sintetizan al leer). `range` va
    // serializado a `range_json` (E18-H02), o `NULL` si el diagnóstico no conoce su posición.
    for check in lodestar_core::local_diagnostics(path, &parsed, raw) {
        let range_json = check
            .range
            .map(|r| serde_json::to_string(&r).unwrap_or_else(|_| "null".to_string()));
        tx.execute(
            "INSERT INTO diagnostics (doc_id, code, severity, message, range_json) VALUES (?1,?2,?3,?4,?5)",
            params![
                doc_id,
                check.code.as_str(),
                severity_str(check.level),
                check.msg,
                range_json,
            ],
        )?;
    }

    Ok(())
}
