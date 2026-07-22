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
// ConceptRef / ConceptId — identidad por path (E10-H04). `id` queda diferido/reservado.
// ---------------------------------------------------------------------------

schema_derive! {
/// Id estable de concepto — newtype **diferido**: IDs estables/federación son no-goal de esta
/// historia (`REFACTOR §16`). Existe ya en el wire de `ConceptRef` para no romper compatibilidad
/// cuando se implemente la resolución por id, pero ningún flujo actual lo produce ni lo consume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConceptId(pub String);
}

schema_derive! {
/// Referencia a un concepto, usada por todas las tools de lectura/escritura (`ARCHITECTURE.md
/// §19.3`). v2 resuelve identidad **únicamente por `path`**: `id` es opcional y su resolución
/// queda diferida (`REFACTOR §6.1`, no-goal IDs estables/federación). `{ "path": "a/b.md" }`
/// deserializa con `id: None`; `path` hereda la validación de `RelPath` — rechaza `..`/absolutas
/// (invariante #6, único chokepoint de path-traversal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConceptRef {
    pub path: RelPath,
    #[serde(default)]
    pub id: Option<ConceptId>,
}
}

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
    // --- Familias schema-driven (decisión D-CheckCode, `ARCHITECTURE.md §19.3`) ---
    // Variantes ESTÁTICAS acotadas (no hay espacio de códigos dinámico). El core aún no las
    // produce (eso es E10-H07/E11-H03) — esta historia solo fija el contrato de wire. La clave
    // i18n por código (§12) se satisface con `Check.msg`, que el core emite inline (no hay
    // catálogo i18n en el core; el catálogo de `frontend/src/lib/i18n.ts` está congelado y
    // fuera de alcance de esta historia).
    #[serde(rename = "SCHEMA-REQFIELD")]
    SchemaReqfield,
    #[serde(rename = "SCHEMA-STATUS")]
    SchemaStatus,
    #[serde(rename = "REL-TARGET")]
    RelTarget,
    #[serde(rename = "REL-CARD")]
    RelCard,
    #[serde(rename = "REL-TYPE")]
    RelType,
    /// Referencia externa (`implemented_by`/`verified_by`, E9-H05) a un fichero de código bajo
    /// `referenceRoots` que no existe en disco (E11-H04). Variante propia, no reuso de
    /// `LINK-STUB`/`LINK-REL` (enlaces ENTRE concepts del bundle) ni `REL-TARGET` (relaciones
    /// tipadas a concepts): un `implemented_by`/`verified_by` apunta a código fuera del dominio
    /// OKF, no a un concept — semánticamente distinto de los tres.
    #[serde(rename = "EXTREF-MISSING")]
    ExtrefMissing,
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
            CheckCode::SchemaReqfield => "SCHEMA-REQFIELD",
            CheckCode::SchemaStatus => "SCHEMA-STATUS",
            CheckCode::RelTarget => "REL-TARGET",
            CheckCode::RelCard => "REL-CARD",
            CheckCode::RelType => "REL-TYPE",
            CheckCode::ExtrefMissing => "EXTREF-MISSING",
        }
    }
}

schema_derive! {
/// Rango de líneas (1-based, como el resto de referencias a línea del core) que acota un `Check`
/// dentro de un fichero. Aditivo (E10-H06, `ARCHITECTURE.md §19.3`); los checks OKF clásicos no
/// lo rellenan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub start_line: u32,
    pub end_line: u32,
}
}

schema_derive! {
/// Un arreglo sugerido para un `Check` (E12-H07 los aplica; aquí solo se describen). `safe`
/// indica si se puede aplicar sin revisión humana.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub fix_id: String,
    pub title: String,
    pub safe: bool,
}
}

schema_derive! {
/// Un diagnóstico de conformidad. Campos clásicos = los del prototipo `chk(level, code, msg,
/// targets)`. NO `severity`/`message`. `targets` SIEMPRE presente (array, nunca null).
///
/// Campos ADITIVOS (decisión D-CheckCode, `ARCHITECTURE.md §19.3`, E10-H06): `id`/`range`/
/// `related`/`fixes`. Retro-compat: `id`/`range` quedan AUSENTES cuando `None` y `related`/
/// `fixes` serializan `[]` cuando vacíos — un consumidor del wire clásico no ve cambios.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub level: Severity,
    pub code: CheckCode,
    pub msg: String,
    pub targets: Vec<RelPath>,
    /// Identificador estable del diagnóstico dentro de una revisión (p. ej. `diag:blake3:…`,
    /// E10-H12). Ausente para los checks OKF clásicos hasta que un productor lo rellene.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Rango de líneas del fichero afectado, si se conoce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    /// Otros paths relacionados con el diagnóstico (además de `targets`), p. ej. el destino de un
    /// link roto en un check `REL-*`. Siempre presente (array, `[]` si no hay ninguno) — mismo
    /// patrón que `targets`.
    #[serde(default)]
    pub related: Vec<RelPath>,
    /// Arreglos sugeridos (E12-H07 los aplica). Siempre presente; `[]` si no hay ninguno (el
    /// test `check_extension_retrocompat` fija que NO se omite del wire).
    #[serde(default)]
    pub fixes: Vec<Fix>,
}
}

impl Check {
    /// Constructor equivalente al `chk(level, code, msg, targets)` del prototipo. Los campos
    /// aditivos (`id`/`range`/`related`/`fixes`) quedan en su valor por defecto — usa los
    /// builders `.with_*` para rellenarlos.
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
            id: None,
            range: None,
            related: Vec::new(),
            fixes: Vec::new(),
        }
    }

    /// Builder: fija el `id` estable del diagnóstico (E10-H12).
    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Builder: fija el `range` de líneas afectado.
    #[must_use]
    pub fn with_range(mut self, range: Range) -> Self {
        self.range = Some(range);
        self
    }

    /// Builder: fija los paths `related` (p. ej. el destino de una relación tipada).
    #[must_use]
    pub fn with_related(mut self, related: Vec<RelPath>) -> Self {
        self.related = related;
        self
    }

    /// Builder: fija los `fixes` sugeridos (E10-H07/E12-H07).
    #[must_use]
    pub fn with_fixes(mut self, fixes: Vec<Fix>) -> Self {
        self.fixes = fixes;
        self
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
///
/// `Serialize`/`Deserialize`/`PartialEq` (E12-H01): forma parte del wire de `NormalizedOperation`
/// (`create`/`patch_frontmatter`). `#[serde(transparent)]` para que serialice como el objeto YAML
/// plano `{clave: valor|null}`, no envuelto en `{"0": {...}}`. Sin `Eq`: `serde_yaml::Value` solo
/// deriva `PartialEq` (números `f64`). Sin `schema_derive!`/`JsonSchema` (no lo implementa
/// `serde_yaml::Value`) — los campos que la usan en `NormalizedOperation` se marcan
/// `schemars(skip)`, mismo patrón que `Frontmatter.tags`/`.timestamp`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(transparent)]
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
///
/// `pub`: además de [`workspace_revision`], lo reutiliza `lodestar-workspace` (E11-H04,
/// `Workspace::assert_writable`) para la contención de `writableRoots`/`referenceRoots` — un solo
/// algoritmo de contención por segmentos, nunca reimplementado por prefijo de string.
pub fn under_root(path: &RelPath, prefix: &RelPath) -> bool {
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
// Códigos de error estables del protocolo (E10-H02, `ARCHITECTURE.md §19.3`, `REFACTOR §13`).
// ---------------------------------------------------------------------------

schema_derive! {
/// Los 16 códigos de error estables del protocolo (`REFACTOR §13`). UNA sola enum, igual que
/// `CheckCode`: el valor de wire ES la cadena SCREAMING_SNAKE (rename por variante, NO el
/// `PascalCase` por defecto de serde ni el guion de `CheckCode`). Cualquier fachada que traduzca
/// un error a protocolo usa una de estas variantes — está prohibido redefinir estos códigos fuera
/// de `core::types` (grep de CI).
///
/// Esta historia (E10-H02) solo fija el contrato de wire y el punto de mapeo; los flujos reales
/// que producen cada código llegan en E12/E13 (fuera de alcance aquí).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ErrorCode {
    #[serde(rename = "WORKSPACE_NOT_FOUND")]
    WorkspaceNotFound,
    #[serde(rename = "WORKSPACE_RECOVERY_REQUIRED")]
    WorkspaceRecoveryRequired,
    #[serde(rename = "CONCEPT_NOT_FOUND")]
    ConceptNotFound,
    #[serde(rename = "AMBIGUOUS_REFERENCE")]
    AmbiguousReference,
    #[serde(rename = "REVISION_CONFLICT")]
    RevisionConflict,
    #[serde(rename = "PLAN_STALE")]
    PlanStale,
    #[serde(rename = "PLAN_EXPIRED")]
    PlanExpired,
    #[serde(rename = "PERMISSION_DENIED")]
    PermissionDenied,
    #[serde(rename = "INVALID_SCHEMA")]
    InvalidSchema,
    #[serde(rename = "NONCONFORMANT_RESULT")]
    NonconformantResult,
    #[serde(rename = "INBOUND_LINKS_EXIST")]
    InboundLinksExist,
    #[serde(rename = "RELATION_CONSTRAINT_VIOLATION")]
    RelationConstraintViolation,
    #[serde(rename = "WRITE_CONFLICT")]
    WriteConflict,
    #[serde(rename = "RESULT_TOO_LARGE")]
    ResultTooLarge,
    #[serde(rename = "RECOVERY_FAILED")]
    RecoveryFailed,
    #[serde(rename = "INTERNAL_IO_ERROR")]
    InternalIoError,
}
}

impl ErrorCode {
    /// El valor de wire (SCREAMING_SNAKE), p. ej. `"REVISION_CONFLICT"`. Útil para grep/CLI sin
    /// pasar por `serde_json` (mismo patrón que `CheckCode::as_str`).
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::WorkspaceNotFound => "WORKSPACE_NOT_FOUND",
            ErrorCode::WorkspaceRecoveryRequired => "WORKSPACE_RECOVERY_REQUIRED",
            ErrorCode::ConceptNotFound => "CONCEPT_NOT_FOUND",
            ErrorCode::AmbiguousReference => "AMBIGUOUS_REFERENCE",
            ErrorCode::RevisionConflict => "REVISION_CONFLICT",
            ErrorCode::PlanStale => "PLAN_STALE",
            ErrorCode::PlanExpired => "PLAN_EXPIRED",
            ErrorCode::PermissionDenied => "PERMISSION_DENIED",
            ErrorCode::InvalidSchema => "INVALID_SCHEMA",
            ErrorCode::NonconformantResult => "NONCONFORMANT_RESULT",
            ErrorCode::InboundLinksExist => "INBOUND_LINKS_EXIST",
            ErrorCode::RelationConstraintViolation => "RELATION_CONSTRAINT_VIOLATION",
            ErrorCode::WriteConflict => "WRITE_CONFLICT",
            ErrorCode::ResultTooLarge => "RESULT_TOO_LARGE",
            ErrorCode::RecoveryFailed => "RECOVERY_FAILED",
            ErrorCode::InternalIoError => "INTERNAL_IO_ERROR",
        }
    }
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

// ---------------------------------------------------------------------------
// Planificación (`ChangeSet`) — §4.1 vía `ARCHITECTURE.md §19.3`, `REFACTOR §6.4` (E12-H01)
// ---------------------------------------------------------------------------
//
// SOLO las formas: la lógica que produce cada pieza (riesgo, diff semántico, validación,
// normalización de cada operación) es de las historias E12-H02..H07. Aquí se congela el contrato
// de wire de `ChangeSet` (criterio `changeset_shape`) y las 11 variantes de `NormalizedOperation`
// con campos razonables — su forma exacta la cierran esas historias.

schema_derive! {
/// Identificador de un `ChangeSet` (plan). Newtype string transparente, mismo patrón que
/// [`WorkspaceRevision`]/[`ConceptRevision`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChangeSetId(pub String);
}

schema_derive! {
/// Hash determinista de un plan: mismo input (operaciones + `base_revision`) ⇒ mismo hash
/// (`plan_hash_determinista`, E12-H08). Newtype string transparente, `"blake3:<hex>"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlanHash(pub String);
}

schema_derive! {
/// Identificador de un recibo de aplicación de un `ChangeSet` (E13). Newtype string
/// transparente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReceiptId(pub String);
}

schema_derive! {
/// Nivel de riesgo de un plan (E12-H02). Wire en minúsculas — mismos valores `"low"`/`"medium"`/
/// `"high"` que usa (o usará) `impact_analyze` (`contracts/mcp.yml`), un solo vocabulario de
/// riesgo en todo el contrato.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
}
}

schema_derive! {
/// Evaluación de riesgo de un plan (E12-H02 rellena `level`/`reasons`; aquí solo la forma). El
/// valor por defecto es el plan mínimo: riesgo bajo, sin razones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RiskAssessment {
    pub level: RiskLevel,
    pub reasons: Vec<String>,
}
}

schema_derive! {
/// Un `move` dentro de un [`SemanticDiff`]: de dónde a dónde.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovedPath {
    pub from: RelPath,
    pub to: RelPath,
}
}

schema_derive! {
/// Diff semántico entre el bundle actual y el hipotético resultante de aplicar un `ChangeSet`
/// (E12-H03 lo calcula; aquí solo la forma). `frontmatter_changes`/`body_changes`/
/// `relation_changes` son los paths afectados por cada categoría — una forma mínima razonable;
/// E12-H03 puede reusar [`crate::diff::OkfDiff`] como referencia sin que sea obligatorio aquí.
/// `Default` = diff vacío (plan sin efecto observable), usado por los tests de forma.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SemanticDiff {
    pub created: Vec<RelPath>,
    pub modified: Vec<RelPath>,
    pub deleted: Vec<RelPath>,
    pub moved: Vec<MovedPath>,
    pub frontmatter_changes: Vec<RelPath>,
    pub body_changes: Vec<RelPath>,
    pub relation_changes: Vec<RelPath>,
    pub diagnostics_introduced: Vec<Check>,
    pub diagnostics_resolved: Vec<Check>,
}
}

schema_derive! {
/// Conteo de diagnósticos por severidad del resultado hipotético (mismo desglose que
/// `hard_fail`/`warn_count` de [`Analysis`], pero completo con `info`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ValidationSummary {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
}
}

schema_derive! {
/// Veredicto de conformidad del bundle hipotético resultante de un `ChangeSet` (E12-H04 lo
/// calcula sobre `analyze()`; aquí solo la forma). `Default` = conformidad vacía (sin
/// diagnósticos, `conformant: false` — coherente con "nada analizado todavía").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub conformant: bool,
    pub summary: ValidationSummary,
    pub diagnostics: Vec<Check>,
}
}

schema_derive! {
/// Modo de `edit_section` (E12-H05): reemplaza, añade al final o al principio de la subsección
/// acotada por `heading_path`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditSectionMode {
    Replace,
    Append,
    Prepend,
}
}

schema_derive! {
/// Política ante enlaces entrantes al borrar un concepto (E12-H06). `Reject` es el default del
/// prototipo/spec — un `delete` sobre un concepto referenciado se rechaza salvo que se pida
/// explícitamente otra política.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InboundLinksPolicy {
    #[default]
    Reject,
    Retarget,
    RemoveLinks,
    CreateStub,
}
}

schema_derive! {
/// Una operación YA normalizada (resuelta a path(s) y contenido/patch concretos) dentro de un
/// `ChangeSet`. Las 11 variantes del alcance de E12 (contenido: E12-H05 · estructura: E12-H06 ·
/// semántica: E12-H07); aquí solo su forma — los campos son razonables para lo que cada operación
/// resuelve, sin cerrar la lógica que los produce.
///
/// El tag de wire (`op`) usa los mismos nombres snake_case que `proposedOperation.kind`
/// (`contracts/mcp.yml`) — un solo vocabulario de tipos de operación en el contrato.
///
/// Sin `Eq`: `Create`/`PatchFrontmatter` llevan `FrontmatterPatch`, que envuelve
/// `serde_yaml::Value` (solo `PartialEq`, por los números `f64`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum NormalizedOperation {
    /// Crea un concepto nuevo. `body: None` ⇒ se rellena con la `bodyTemplate` del `DocType`
    /// (E12-H05); aquí la resolución de la plantilla NO ocurre todavía.
    Create {
        path: RelPath,
        #[cfg_attr(feature = "schemars", schemars(skip))]
        frontmatter: FrontmatterPatch,
        body: Option<String>,
    },
    /// Parchea el frontmatter existente (null en el patch = borra la clave, `FrontmatterPatch`).
    PatchFrontmatter {
        path: RelPath,
        #[cfg_attr(feature = "schemars", schemars(skip))]
        patch: FrontmatterPatch,
    },
    /// Sustituye el cuerpo completo del concepto.
    ReplaceBody { path: RelPath, body: String },
    /// Edita SOLO la subsección acotada por `heading_path` (p. ej. `["Security", "Token
    /// rotation"]`), con el modo indicado.
    EditSection {
        path: RelPath,
        heading_path: Vec<String>,
        mode: EditSectionMode,
        content: String,
    },
    /// Reemplaza texto literal; `expected_occurrences` (si se da) hace fallar la normalización
    /// cuando el número de coincidencias no casa (E12-H05).
    ReplaceText {
        path: RelPath,
        find: String,
        replace: String,
        expected_occurrences: Option<usize>,
    },
    /// Mueve/renombra un concepto; `rewrite_inbound_links` decide si sus backlinks se reescriben
    /// dentro del mismo change set (E12-H06).
    Move {
        from: RelPath,
        to: RelPath,
        rewrite_inbound_links: bool,
    },
    /// Borra un concepto, sujeto a `inbound_links_policy` si está referenciado (E12-H06).
    Delete {
        path: RelPath,
        inbound_links_policy: InboundLinksPolicy,
    },
    /// Añade una relación tipada (validada contra `RelationDef`, E12-H07).
    AddRelation {
        source: RelPath,
        relation: String,
        target: RelPath,
    },
    /// Quita una relación tipada existente.
    RemoveRelation {
        source: RelPath,
        relation: String,
        target: RelPath,
    },
    /// Transiciona el `status` de un concepto (validado contra `allowedStatuses`/lifecycle,
    /// E12-H07).
    TransitionStatus { path: RelPath, to: String },
    /// Materializa un `Fix` `safe` sugerido por un diagnóstico previo (`Fix.fix_id`).
    ApplyFix { fix_id: String },
}
}

schema_derive! {
/// El plan de cambios completo: operaciones normalizadas + análisis (riesgo/diff/validación) +
/// caducidad, sin tocar disco (E12-H08 lo ensambla; aquí solo la forma, criterio
/// `changeset_shape`). Wire con renames EXPLÍCITOS donde `rename_all = "camelCase"` no basta
/// (`base_revision` → `baseWorkspaceRevision`, no `baseRevision`).
///
/// Sin `Eq` (transitivo desde `NormalizedOperation`/`FrontmatterPatch`, ver ahí); el criterio
/// `changeset_roundtrip` solo necesita `PartialEq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSet {
    pub id: ChangeSetId,
    #[serde(rename = "baseWorkspaceRevision")]
    pub base_revision: WorkspaceRevision,
    pub operations: Vec<NormalizedOperation>,
    #[serde(rename = "planHash")]
    pub plan_hash: PlanHash,
    pub risk: RiskAssessment,
    #[serde(rename = "semanticDiff")]
    pub semantic_diff: SemanticDiff,
    pub validation: ValidationReport,
    #[serde(rename = "expiresAt")]
    pub expires_at: String,
}
}
