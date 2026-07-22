//! El **contrato de tipos**, definido UNA sola vez (`ARCHITECTURE.md §4.1` y `§4.4`).
//!
//! Todas las fachadas hacen `use` de estos tipos; no hay capa DTO paralela (principio #4).
//! El `.d.ts` de TypeScript se genera desde aquí (ts-rs/specta) en E0-H04/E6-H03.

use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Macro interna: deriva `JsonSchema` solo con la feature `schemars` (para el outputSchema del MCP).
macro_rules! schema_derive {
    ($item:item) => {
        #[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
        $item
    };
}

// ---------------------------------------------------------------------------
// RelPath — newtype VALIDADO (§4.1, §10 fila 9). Único chokepoint de path-traversal.
// ---------------------------------------------------------------------------

schema_derive! {
/// Ruta relativa al root del bundle. `RelPath::new` rechaza absolutas y `..`, y normaliza
/// (separadores a `/`, colapsa `.` y `//`). Prohibido `type RelPath = String`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct RelPath(String);
}

impl RelPath {
    /// Construye un `RelPath` validado. Rechaza rutas absolutas (POSIX **y** de unidad Windows
    /// `C:...`), componentes `..`, backslashes y la cadena vacía.
    pub fn new(s: &str) -> Result<Self, crate::CoreError> {
        // Backslash: en Windows es separador (ambigüedad peligrosa) y en POSIX un char válido
        // pero que el prototipo trata como literal → rechazar cierra el hueco en ambos casos.
        if s.contains('\\') || s.starts_with('/') {
            return Err(crate::CoreError::InvalidRelPath(s.to_string()));
        }
        // Unidad Windows (`C:` / `c:` al inicio): `root.join("C:/x")` DESCARTA el root en Windows
        // → escritura fuera del bundle (zip-slip). También cubre `C:evil.md` (relativa a unidad).
        let b = s.as_bytes();
        if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            return Err(crate::CoreError::InvalidRelPath(s.to_string()));
        }
        let mut parts: Vec<&str> = Vec::new();
        for seg in s.split('/') {
            match seg {
                "" | "." => continue,
                ".." => return Err(crate::CoreError::InvalidRelPath(s.to_string())),
                _ => parts.push(seg),
            }
        }
        if parts.is_empty() {
            return Err(crate::CoreError::InvalidRelPath(s.to_string()));
        }
        Ok(RelPath(parts.join("/")))
    }

    /// La ruta como `&str` (siempre normalizada).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Nombre de fichero (último segmento). Port de `basename`.
    pub fn basename(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// Directorio contenedor con la barra final, o `""` para el root. Port de `dirOf`.
    pub fn dir(&self) -> String {
        match self.0.rfind('/') {
            Some(i) => self.0[..=i].to_string(),
            None => String::new(),
        }
    }

    /// Id de concepto: la ruta sin la extensión `.md`. Port de `conceptId` aplicado al path.
    pub fn concept_id(&self) -> String {
        self.0.strip_suffix(".md").unwrap_or(&self.0).to_string()
    }

    /// `true` si el fichero es reservado (`index.md`/`log.md`).
    pub fn is_reserved(&self) -> bool {
        matches!(self.basename(), "index.md" | "log.md")
    }
}

impl std::fmt::Display for RelPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for RelPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RelPath {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(d)?;
        RelPath::new(&s).map_err(serde::de::Error::custom)
    }
}

/// Mapa de ficheros del bundle: lo que come `Bundle::from_files` y lo que devuelve `vcs.tree_files`.
pub type FileMap = BTreeMap<RelPath, String>;

// ---------------------------------------------------------------------------
// Conformidad: Severity · CheckCode · Check (§4.1, §10 filas 3/4)
// ---------------------------------------------------------------------------

schema_derive! {
/// Orden DELIBERADO: `Err` es el máximo, así `checks.iter().map(|c| c.level).max()` = el peor.
/// Serializa en minúsculas. (`§10` fila 4.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Pass,
    Info,
    Warn,
    Err,
}
}

schema_derive! {
/// Los 15 códigos OKF. UNA sola enum. El valor de wire ES la cadena con guion (rename por variante).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CheckCode {
    #[serde(rename = "OKF-FM01")]
    OkfFm01,
    #[serde(rename = "OKF-FM02")]
    OkfFm02,
    #[serde(rename = "OKF-FM03")]
    OkfFm03,
    #[serde(rename = "OKF-TYPE")]
    OkfType,
    #[serde(rename = "REC-TITLE")]
    RecTitle,
    #[serde(rename = "REC-DESC")]
    RecDesc,
    #[serde(rename = "FMT-TAGS")]
    FmtTags,
    #[serde(rename = "FMT-TS")]
    FmtTs,
    #[serde(rename = "LINK-STUB")]
    LinkStub,
    #[serde(rename = "LINK-REL")]
    LinkRel,
    #[serde(rename = "ORPHAN")]
    Orphan,
    #[serde(rename = "BODY-STRUCT")]
    BodyStruct,
    #[serde(rename = "OKF-IDX")]
    OkfIdx,
    #[serde(rename = "OKF-LOG")]
    OkfLog,
    #[serde(rename = "OKF-CONFLICT")]
    OkfConflict,
}
}

impl CheckCode {
    /// El valor de wire (cadena con guion), p. ej. `"OKF-FM01"`.
    pub fn as_str(self) -> &'static str {
        match self {
            CheckCode::OkfFm01 => "OKF-FM01",
            CheckCode::OkfFm02 => "OKF-FM02",
            CheckCode::OkfFm03 => "OKF-FM03",
            CheckCode::OkfType => "OKF-TYPE",
            CheckCode::RecTitle => "REC-TITLE",
            CheckCode::RecDesc => "REC-DESC",
            CheckCode::FmtTags => "FMT-TAGS",
            CheckCode::FmtTs => "FMT-TS",
            CheckCode::LinkStub => "LINK-STUB",
            CheckCode::LinkRel => "LINK-REL",
            CheckCode::Orphan => "ORPHAN",
            CheckCode::BodyStruct => "BODY-STRUCT",
            CheckCode::OkfIdx => "OKF-IDX",
            CheckCode::OkfLog => "OKF-LOG",
            CheckCode::OkfConflict => "OKF-CONFLICT",
        }
    }
}

schema_derive! {
/// Un diagnóstico de conformidad. Campos = los del prototipo `chk(level, code, msg, targets)`.
/// NO `severity`/`message`. `targets` SIEMPRE presente (array, nunca null).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub level: Severity,
    pub code: CheckCode,
    pub msg: String,
    pub targets: Vec<RelPath>,
}
}

impl Check {
    /// Constructor equivalente al `chk(level, code, msg, targets)` del prototipo.
    pub fn new(
        level: Severity,
        code: CheckCode,
        msg: impl Into<String>,
        targets: Vec<RelPath>,
    ) -> Self {
        Check {
            level,
            code,
            msg: msg.into(),
            targets,
        }
    }
}

// ---------------------------------------------------------------------------
// Modelo de fichero: Frontmatter · ParsedFile · FileKind · FmError (§4.1)
// ---------------------------------------------------------------------------

schema_derive! {
/// Frontmatter: 7 KNOWN_FM tipados + `extra` para claves de productor. `tags`/`timestamp` se
/// guardan RAW (`serde_yaml::Value`) para poder detectar FMT-TAGS (no-lista) y FMT-TS (no-ISO).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Frontmatter {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[cfg_attr(feature = "schemars", schemars(skip))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<serde_yaml::Value>,
    #[cfg_attr(feature = "schemars", schemars(skip))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// `IndexMap` (no `BTreeMap`) para preservar el **orden de aparición** de las claves de
    /// productor, igual que `Object.keys(fm)` del prototipo (`buildRaw`). Con `BTreeMap` se
    /// reordenaban alfabéticamente (divergencia de paridad).
    #[cfg_attr(feature = "schemars", schemars(skip))]
    #[serde(flatten)]
    pub extra: IndexMap<String, serde_yaml::Value>,
    /// Claves KNOWN de tipo string presentes con `null` explícito (`type:\n`). En JS `null !==
    /// undefined`: cuentan como presentes (`fmPresent`) y `buildRaw` las serializa (`k: null`).
    /// Con `Option<String>` el null se perdía como ausencia — esto conserva la distinción.
    #[cfg_attr(feature = "schemars", schemars(skip))]
    #[serde(skip)]
    pub known_null: Vec<String>,
}
}

/// Clase de fichero. `reserved = kind != Concept`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Concept,
    Index,
    Log,
}

/// Error de frontmatter (es un dato, no un `Result`: `parse_file` nunca falla por contenido).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FmError {
    Missing,
    Unclosed,
    Malformed(String),
}

/// Fichero parseado. `parse_file` NUNCA devuelve `Err` por contenido: FM01/02/03 son Checks (datos).
#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub kind: FileKind,
    pub fm: Option<Frontmatter>,
    pub fm_err: Option<FmError>,
    pub body: String,
    pub raw: String,
}

// ---------------------------------------------------------------------------
// Análisis del bundle: Analysis (§4.1, §10 filas 4/5)
// ---------------------------------------------------------------------------

schema_derive! {
/// El resultado de `analyze()`. Nombres = los del prototipo (`inn`, `perFile`, `out`). camelCase en wire.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub concepts: Vec<RelPath>,
    /// Adyacencia de strings (destinos salientes resueltos por concepto).
    pub out: BTreeMap<RelPath, Vec<RelPath>>,
    /// Backlinks (la inversa de `out`).
    pub inn: BTreeMap<RelPath, Vec<RelPath>>,
    pub in_index: BTreeSet<RelPath>,
    pub dangling: Vec<RelPath>,
    pub orphans: Vec<RelPath>,
    pub per_file: BTreeMap<RelPath, Vec<Check>>,
    /// `hard_fail` = nº de ficheros con algún `Err` (conteo, no `.max()`). (`§10` fila 4.)
    pub hard_fail: usize,
    pub warn_count: usize,
    /// Del `index.md` raíz; `None` si falta. Se expone en la conformidad (`§12`).
    pub okf_version: Option<String>,
}
}

// ---------------------------------------------------------------------------
// Grafo: GraphModel · GraphNode · Edge · Direction · Neighborhood (§4.1, §4.2)
// ---------------------------------------------------------------------------

schema_derive! {
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphModel {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
}
}

schema_derive! {
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: RelPath,
    pub ghost: bool,
    pub r#type: Option<String>,
    pub status: Option<String>,
}
}

schema_derive! {
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub source: RelPath,
    pub target: RelPath,
    pub dangling: bool,
}
}

/// Dirección de exploración del grafo. `Out`=dependencias · `In`=blast-radius · `Both`=mapa local.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Out,
    In,
    Both,
}

schema_derive! {
/// Subgrafo dirigido alrededor de un concept (`root` = el centro).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Neighborhood {
    pub root: RelPath,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
}
}

// ---------------------------------------------------------------------------
// DTOs de lectura de Bundle (§4.2)
// ---------------------------------------------------------------------------

schema_derive! {
/// Fila del árbol de concepts. `title` ya resuelto (fm.title o del path); `invalid` = algún Check Err.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConceptSummary {
    pub path: RelPath,
    pub title: String,
    pub r#type: Option<String>,
    pub status: Option<String>,
    pub orphan: bool,
    pub invalid: bool,
}
}

schema_derive! {
/// Un extremo de un enlace + el href crudo tal como aparece en el `.md` (port de `resolveLink`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkRef {
    pub path: RelPath,
    pub href: String,
}
}

schema_derive! {
/// Vecindad de enlaces de un concept (port del panel de backlinks).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backlinks {
    /// Quién enlaza aquí (con el href usado).
    pub inbound: Vec<LinkRef>,
    /// `index.md` que lo listan.
    pub index_refs: Vec<RelPath>,
    /// Destinos salientes resueltos.
    pub out: Vec<RelPath>,
    /// Hrefs salientes que no resuelven a ningún fichero.
    pub dangling: Vec<String>,
}
}

/// Patch de frontmatter (merge-patch RFC 7386). `Some(v)` escribe/reemplaza; `None` BORRA;
/// clave ausente = no se toca. El tercer estado se modela con la pertenencia al mapa.
#[derive(Debug, Clone, Default)]
pub struct FrontmatterPatch(pub BTreeMap<String, Option<serde_yaml::Value>>);

// ---------------------------------------------------------------------------
// Resultados de escritura / generación (§4.2)
// ---------------------------------------------------------------------------

/// Resultado de una escritura validada. Rechazo = `written:false` + `rejected`, NO un `Err`.
#[derive(Debug, Clone)]
pub struct WriteOutcome {
    pub path: RelPath,
    pub raw: String,
    pub hash: [u8; 32],
    pub written: bool,
    pub rejected: Option<String>,
    pub checks: Vec<Check>,
    pub bundle_hard_fail: usize,
}

/// Plan de generación puro: la workspace lo aplica por el único camino de escritura.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mutation {
    pub writes: BTreeMap<RelPath, String>,
    pub deletes: Vec<RelPath>,
}

// ---------------------------------------------------------------------------
// Identidad de contenido determinista: `ConceptRevision` / `WorkspaceRevision`
// (E10-H03, `ARCHITECTURE.md §19.3`, `REFACTOR §6.2/§6.3`). Eleva blake3 (ya usado en
// `WriteOutcome.hash`, `bundle.rs`) a identidad expuesta. Wire = string `"blake3:<hex>"`.
// ---------------------------------------------------------------------------

schema_derive! {
/// Revisión de contenido de un único `.md`: `"blake3:<hex>"` del contenido en disco.
/// Wire = el string tal cual (sin envoltorio de objeto).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConceptRevision(pub String);
}

impl ConceptRevision {
    /// Construye la revisión a partir de un hash blake3 crudo (el mismo patrón que
    /// `WriteOutcome.hash`).
    pub fn from_hash(hash: [u8; 32]) -> Self {
        ConceptRevision(format!("blake3:{}", blake3::Hash::from(hash).to_hex()))
    }
}

schema_derive! {
/// Revisión determinista de (una porción de) el workspace: combina path+contenido de todos los
/// ficheros incluidos, en orden estable. Independiente de mtime, orden de inserción y de
/// cualquier caché/índice. Ver `workspace_revision`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceRevision(pub String);
}

/// `true` si `path` está bajo el root `prefix` (por SEGMENTOS de path, no por prefijo de string):
/// `"docs"` cubre `"docs/guia.md"` pero NO `"docsx/y.md"`. Un `path == prefix` exacto también
/// cuenta como contenido (root con extensión, aunque en la práctica `RelPath` siempre trae `.md`).
fn under_root(path: &RelPath, prefix: &RelPath) -> bool {
    let path = path.as_str();
    let prefix = prefix.as_str();
    path == prefix
        || (path.len() > prefix.len()
            && path.starts_with(prefix)
            && path.as_bytes()[prefix.len()] == b'/')
}

/// `true` si `path` cae bajo `.lodestar/` (cachés, índices, runtime — SIEMPRE excluido de la
/// identidad de contenido).
fn under_lodestar(path: &RelPath) -> bool {
    let s = path.as_str();
    s == ".lodestar" || s.starts_with(".lodestar/")
}

/// Calcula la revisión determinista del workspace escribible.
///
/// Selección de ficheros incluidos:
/// - Excluye SIEMPRE todo lo bajo `.lodestar/` (cachés/índices/runtime, nunca fuente de verdad).
/// - Si `writable` no está vacío, incluye SOLO los ficheros bajo alguno de esos roots (prefijo por
///   segmentos, `under_root`); esto excluye de forma natural los `referenceRoots` (solo lectura).
/// - Si `writable` está vacío, incluye todo lo que no sea `.lodestar/` (todo el bundle es
///   escribible, coherente con E9-H05).
///
/// Determinismo: itera `files` en su orden natural (`FileMap` es `BTreeMap<RelPath, _>`, ya
/// ordenado por `RelPath`), así el resultado depende solo del contenido incluido — nunca del
/// orden de inserción, de mtime ni de ninguna caché.
pub fn workspace_revision(files: &FileMap, writable: &[RelPath]) -> WorkspaceRevision {
    let mut hasher = blake3::Hasher::new();
    for (path, content) in files.iter() {
        if under_lodestar(path) {
            continue;
        }
        if !writable.is_empty() && !writable.iter().any(|root| under_root(path, root)) {
            continue;
        }
        let content_hash = blake3::hash(content.as_bytes());
        hasher.update(path.as_str().as_bytes());
        hasher.update(b"\0"); // separador fijo: evita colisiones tipo "a"+"b" == "ab"+""
        hasher.update(content_hash.as_bytes());
        hasher.update(b"\0");
    }
    WorkspaceRevision(format!("blake3:{}", hasher.finalize().to_hex()))
}

// ---------------------------------------------------------------------------
// Tipos de versionado (git) — también en core::types (§4.4). Sin git2, sin I/O.
// ---------------------------------------------------------------------------

schema_derive! {
/// SHA de commit. Newtype validado (hex). `git2::Oid` NUNCA cruza la frontera de vcs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Sha(String);
}

impl Sha {
    /// Construye un `Sha` validando que sea hex de 4..=64 caracteres.
    pub fn new(s: &str) -> Result<Self, crate::CoreError> {
        let ok = (4..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit());
        if ok {
            Ok(Sha(s.to_ascii_lowercase()))
        } else {
            Err(crate::CoreError::InvalidSha(s.to_string()))
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Forma corta (7 chars) para la UI.
    pub fn short(&self) -> String {
        self.0.chars().take(7).collect()
    }
}

impl std::fmt::Display for Sha {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Sha {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(d)?;
        Sha::new(&s).map_err(serde::de::Error::custom)
    }
}

schema_derive! {
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub email: String,
}
}

schema_derive! {
/// Una fila del historial. `time_unix` en SEGUNDOS unix (como git).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitRow {
    pub id: Sha,
    pub short: String,
    pub message: String,
    pub author: Author,
    pub time_unix: i64,
    pub parents: Vec<Sha>,
    pub conformance: Option<CommitConformance>,
}
}

schema_derive! {
/// Conformidad de un commit = proyección de `Analysis` sobre su árbol. Cacheada CRUDA (sin strictness).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitConformance {
    pub hard_fail: usize,
    pub warn_count: usize,
    pub conform: bool,
}
}

/// Estado del repo — detecta merge/rebase en curso (bloquea el commit hasta resolver).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoState {
    Clean,
    Merging,
    Rebasing,
    CherryPicking,
    Reverting,
}

schema_derive! {
/// Una rama. `upstream` = rama remota de seguimiento (p.ej. "origin/main"), si la hay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    pub is_head: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
}
}

/// Tipo de operación de red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    Push,
    Pull,
}

/// Resultado de una operación de red (push/pull) — vía binario `git`.
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    pub kind: SyncKind,
    pub ok: bool,
    pub summary: String,
}

impl Frontmatter {
    /// Vista del frontmatter como pares clave→valor (known fields presentes + extras), al estilo
    /// del objeto JS del prototipo. Útil para query (`fmGet`/`Object.values`).
    pub fn as_pairs(&self) -> Vec<(String, serde_yaml::Value)> {
        let mut v: Vec<(String, serde_yaml::Value)> = Vec::new();
        let s = |x: &String| serde_yaml::Value::String(x.clone());
        // Un known con null explícito se emite como par `(k, Null)` — presente, como en JS.
        let push_known =
            |v: &mut Vec<(String, serde_yaml::Value)>, k: &str, val: Option<serde_yaml::Value>| {
                if let Some(val) = val {
                    v.push((k.to_string(), val));
                } else if self.known_null.iter().any(|n| n == k) {
                    v.push((k.to_string(), serde_yaml::Value::Null));
                }
            };
        push_known(&mut v, "type", self.r#type.as_ref().map(s));
        push_known(&mut v, "title", self.title.as_ref().map(s));
        push_known(&mut v, "description", self.description.as_ref().map(s));
        push_known(&mut v, "resource", self.resource.as_ref().map(s));
        push_known(&mut v, "tags", self.tags.clone());
        push_known(&mut v, "timestamp", self.timestamp.clone());
        push_known(&mut v, "status", self.status.as_ref().map(s));
        for (k, val) in &self.extra {
            v.push((k.clone(), val.clone()));
        }
        v
    }
}

// Constantes canónicas del modelo OKF (port del prototipo).
pub(crate) const KNOWN_FM: [&str; 7] = [
    "type",
    "title",
    "description",
    "resource",
    "tags",
    "timestamp",
    "status",
];
