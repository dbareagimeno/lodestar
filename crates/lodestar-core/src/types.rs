//! El **contrato de tipos**, definido UNA sola vez (`ARCHITECTURE.md §4.1` y `§4.4`).
//!
//! Todas las fachadas hacen `use` de estos tipos; no hay capa DTO paralela (principio #4).
//! El `.d.ts` de TypeScript se genera desde aquí (ts-rs/specta) en E0-H04/E6-H03.

use std::collections::{BTreeMap, BTreeSet};

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
/// Ruta relativa al root del workspace. `RelPath::new` rechaza absolutas y `..`, y normaliza
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
        // → escritura fuera del workspace (zip-slip). También cubre `C:evil.md` (relativa a unidad).
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

    /// El nombre del fichero **sin** la extensión `.md` — el último eslabón de la cadena de
    /// [`crate::model::derived_title`] (`ARCHITECTURE.md §20.4`).
    ///
    /// > `concept_id`/`is_reserved` se retiraron en E16-H02: ningún nombre de fichero activa
    /// > reglas especiales (`REFACTOR_PHASE_2 §Principio 4`).
    pub fn stem(&self) -> &str {
        let base = self.basename();
        base.strip_suffix(".md").unwrap_or(base)
    }

    /// ¿Esta ruta **sería** un documento Markdown? Extensión `.md`, sin distinguir capitalización.
    ///
    /// Es el **único** discriminador de familia del motor (invariante #3), y solo se usa para
    /// juzgar rutas que **no están en el inventario**: qué es un documento se decide siempre por
    /// pertenencia al inventario ([`Inventory::contains_document`]), nunca por el nombre — `§20.6`
    /// prohíbe expresamente clasificar enlaces por extensión. Pero de un destino que no existe no
    /// hay inventario al que preguntar, y hay que decidir igualmente dos cosas:
    ///
    /// - la severidad de `LINK-TARGET-MISSING` (documento ausente = `Err`; fichero del proyecto
    ///   ausente = `Warn`), y
    /// - si el destino es un **fantasma del grafo** ([`LinkTarget::internal_path`]).
    ///
    /// Las dos preguntas comparten esta respuesta a propósito: si divergieran, el grafo tendría
    /// nodos que la conformidad no considera documentos, o al revés.
    pub fn is_markdown(&self) -> bool {
        self.0.to_lowercase().ends_with(".md")
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

/// Mapa de ficheros del workspace: lo que come `DocumentSet::from_files` y lo que devuelve `vcs.tree_files`.
pub type FileMap = BTreeMap<RelPath, String>;

// ---------------------------------------------------------------------------
// DocumentRef / DocumentId — identidad por path (E10-H04). `id` queda diferido/reservado.
// ---------------------------------------------------------------------------

schema_derive! {
/// Id estable de documento — newtype **diferido**: IDs estables/federación son no-goal de esta
/// historia (`REFACTOR §16`). Existe ya en el wire de `DocumentRef` para no romper compatibilidad
/// cuando se implemente la resolución por id, pero ningún flujo actual lo produce ni lo consume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentId(pub String);
}

schema_derive! {
/// Referencia a un documento, usada por todas las tools de lectura/escritura (`ARCHITECTURE.md
/// §19.3`). v2 resuelve identidad **únicamente por `path`**: `id` es opcional y su resolución
/// queda diferida (`REFACTOR §6.1`, no-goal IDs estables/federación). `{ "path": "a/b.md" }`
/// deserializa con `id: None`; `path` hereda la validación de `RelPath` — rechaza `..`/absolutas
/// (invariante #6, único chokepoint de path-traversal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentRef {
    pub path: RelPath,
    #[serde(default)]
    pub id: Option<DocumentId>,
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
/// Los códigos de diagnóstico. UNA sola enum, hoy el **catálogo mínimo** de `ARCHITECTURE.md
/// §20.9` (E16-H05): Lodestar solo informa de lo que le impide *interpretar o modificar con
/// seguridad* un documento, no de si cumple una especificación documental. El valor de wire ES la
/// cadena con guion (rename por variante).
///
/// El catálogo OKF (`OKF-FM01`, `OKF-TYPE`, `REC-TITLE`, `REC-DESC`, `FMT-TAGS`, `FMT-TS`,
/// `BODY-STRUCT`, `ORPHAN`, `OKF-IDX`, `OKF-LOG`) se **retiró**; `OKF-FM02`/`OKF-FM03`/
/// `OKF-CONFLICT` se renombraron a `FM-UNCLOSED`/`FM-YAML-INVALID`/`DOC-CONFLICT-MARKER`.
/// **E17-H03** retiró `LINK-STUB`/`LINK-REL` (el destino inexistente y el enlace relativo del
/// prototipo) en favor de `LINK-TARGET-MISSING`/`LINK-ESCAPES-WORKSPACE`/`LINK-CASE-MISMATCH`,
/// derivados de la clasificación de [`LinkTarget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CheckCode {
    // --- Frontmatter no interpretable (`§20.9`) ---
    /// El bloque de frontmatter abre `---` y nunca cierra: el documento no se puede interpretar.
    #[serde(rename = "FM-UNCLOSED")]
    FmUnclosed,
    /// El bloque está delimitado pero su YAML es sintácticamente inválido. Lleva el `range` de las
    /// líneas de contenido del bloque (delimitadores excluidos), derivado del `span` de
    /// [`ParsedFrontmatter`].
    #[serde(rename = "FM-YAML-INVALID")]
    FmYamlInvalid,
    /// Marcadores de conflicto de merge sin resolver: el documento está a medio mergear y no se
    /// puede modificar con seguridad.
    #[serde(rename = "DOC-CONFLICT-MARKER")]
    DocConflictMarker,
    // Los códigos heredados del prototipo `LINK-STUB`/`LINK-REL` (sin productor desde E17-H03) se
    // retiran del catálogo en E20-H03 junto con la maquinaria de schema: ya nada los nombra (la
    // guarda de `diagnosticos.rs::link_missing_con_rango` que los sostenía se migró a los códigos
    // vivos). Sus reemplazos son `LINK-TARGET-MISSING`/`LINK-ESCAPES-WORKSPACE`/`LINK-CASE-MISMATCH`.
    // --- Enlaces (`§20.9`, E17-H03) ---
    /// El destino de un enlace está contenido en el workspace pero **no existe** (`§20.9`).
    /// Severidad `Err` si el destino sería un documento Markdown (`danglingDocumentLinks: error`) y
    /// `Warn` si sería otro fichero del proyecto (`missingWorkspaceFiles: warning`). E17-H03.
    #[serde(rename = "LINK-TARGET-MISSING")]
    LinkTargetMissing,
    /// El destino de un enlace sale de la raíz del workspace ([`LinkTarget::EscapesWorkspace`]):
    /// Lodestar no puede seguirlo ni reescribirlo. E17-H03.
    #[serde(rename = "LINK-ESCAPES-WORKSPACE")]
    LinkEscapesWorkspace,
    // Las familias schema-driven `SCHEMA-*`/`REL-*` y `EXTREF-MISSING` se RETIRAN en E20-H03: con
    // `core::schema` desaparecen `validate_schema`/`validate_relations` (sus únicos productores) y el
    // diagnóstico de referencias externas de `external_refs` (`§20.10`: el modelo es universal, sin
    // schema, y una relación es un enlace Markdown). El catálogo vivo lo forman los códigos de
    // frontmatter/enlace/descubrimiento de `§20.9`.
    // --- Descubrimiento universal (E15-H07, `ARCHITECTURE.md §20.5`/`§20.9`) ---
    // Los produce `lodestar_workspace::discovery`, no `conform`: describen lo que Lodestar NO
    // pudo incorporar al inventario (o lo que no es portable), no el incumplimiento de una
    // especificación documental.
    /// Un `.md` cuyos bytes no son UTF-8 válido: no se puede interpretar, así que no entra en el
    /// inventario.
    #[serde(rename = "DOC-NOT-UTF8")]
    DocNotUtf8,
    /// Un `.md` por encima del tamaño máximo por documento de la política de descubrimiento.
    #[serde(rename = "DOC-TOO-LARGE")]
    DocTooLarge,
    /// Una ruta no representable (bytes no UTF-8 en Unix, surrogate suelto en Windows) o que no es
    /// una ruta relativa válida del workspace. Su `Check` va SIN `targets`: no hay `RelPath` que
    /// construir — ese es justamente el problema (invariante #6).
    #[serde(rename = "PATH-NOT-UTF8")]
    PathNotUtf8,
    /// Un enlace simbólico encontrado en el árbol: Lodestar no los sigue, y en vez de ignorarlo en
    /// silencio se reporta el documento que queda fuera del inventario.
    #[serde(rename = "SYMLINK-UNSUPPORTED")]
    SymlinkUnsupported,
    /// Rutas que solo difieren en capitalización: en un volumen case-insensitive son el mismo
    /// fichero, así que el workspace no es portable.
    ///
    /// **Dos productores**: el descubrimiento (`§20.5`, colisiones entre ficheros del árbol) y —
    /// desde E17-H03— [`crate::links::diagnose`], cuando un enlace apunta a una ruta que el
    /// inventario tiene *salvo capitalización*.
    #[serde(rename = "LINK-CASE-MISMATCH")]
    LinkCaseMismatch,
}
}

impl CheckCode {
    /// El valor de wire (cadena con guion), p. ej. `"FM-YAML-INVALID"`.
    pub fn as_str(self) -> &'static str {
        match self {
            CheckCode::FmUnclosed => "FM-UNCLOSED",
            CheckCode::FmYamlInvalid => "FM-YAML-INVALID",
            CheckCode::DocConflictMarker => "DOC-CONFLICT-MARKER",
            CheckCode::LinkTargetMissing => "LINK-TARGET-MISSING",
            CheckCode::LinkEscapesWorkspace => "LINK-ESCAPES-WORKSPACE",
            CheckCode::DocNotUtf8 => "DOC-NOT-UTF8",
            CheckCode::DocTooLarge => "DOC-TOO-LARGE",
            CheckCode::PathNotUtf8 => "PATH-NOT-UTF8",
            CheckCode::SymlinkUnsupported => "SYMLINK-UNSUPPORTED",
            CheckCode::LinkCaseMismatch => "LINK-CASE-MISMATCH",
        }
    }
}

schema_derive! {
/// Rango de líneas (1-based, **ambas inclusive**) que acota un `Check` dentro de un fichero.
/// Aditivo (E10-H06, `ARCHITECTURE.md §19.3`); lo rellena quien conoce la posición del problema
/// — p. ej. `FM-YAML-INVALID`, con las líneas de contenido del bloque de frontmatter
/// (delimitadores excluidos, `§20.9`).
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
    /// E10-H12). Ausente hasta que un productor lo rellene.
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
// Modelo documental genérico: FieldPath · ParsedFrontmatter · FmError
// (`ARCHITECTURE.md §20.4`, E16-H01 — supersede los 7 campos tipados de `§4.1`)
// ---------------------------------------------------------------------------

/// Ruta a una propiedad del frontmatter: una secuencia **no vacía** de segmentos ya resueltos.
///
/// Newtype validado (mismo patrón que [`RelPath`]) y no un `String` crudo: la dot-notation es una
/// *sintaxis de entrada*, no la identidad del campo. Por eso hay dos constructores —
/// [`FieldPath::parse`] parte por puntos (lo que teclea un agente) y
/// [`FieldPath::from_segments`] no (la vía para direccionar una clave YAML que *contiene* un
/// punto, p. ej. `sonar.projectKey`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldPath(Vec<String>);

impl FieldPath {
    /// Construye un `FieldPath` desde dot-notation (`"service.tier"`,
    /// `"release.target.date"`). **Siempre** parte por puntos: nunca resuelve a una clave literal
    /// que los contenga.
    ///
    /// # Errores
    /// [`crate::CoreError::InvalidFieldPath`] si el path está vacío o algún segmento lo está
    /// (`""`, `"service."`, `"a..b"`).
    pub fn parse(s: &str) -> Result<Self, crate::CoreError> {
        Self::from_segments(s.split('.'))
    }

    /// Construye un `FieldPath` desde segmentos explícitos, **sin** partir por puntos: es la vía
    /// para direccionar una clave YAML que contiene un punto (la usan el filtro JSON de la query
    /// y el catálogo de metadata, que construyen paths sin pasar por la sintaxis textual).
    ///
    /// # Errores
    /// [`crate::CoreError::InvalidFieldPath`] si la lista está vacía o algún segmento lo está.
    pub fn from_segments<I, S>(segments: I) -> Result<Self, crate::CoreError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let segs: Vec<String> = segments.into_iter().map(Into::into).collect();
        if segs.is_empty() || segs.iter().any(String::is_empty) {
            return Err(crate::CoreError::InvalidFieldPath(segs.join(".")));
        }
        Ok(FieldPath(segs))
    }

    /// Los segmentos del path, en orden de descenso.
    pub fn segments(&self) -> &[String] {
        &self.0
    }
}

impl std::fmt::Display for FieldPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.join("."))
    }
}

impl Serialize for FieldPath {
    /// Serializa como su **string punteado** (`"service.tier"`), la forma de wire de
    /// `metadata_inspect` (E20-H03): un `FieldPath` es la identidad de un campo y en el wire viaja
    /// como su dot-path (vía [`Display`](std::fmt::Display)), nunca como un array de segmentos. Las
    /// claves de wire que lo envuelven (`name` en el catálogo, `field` en la inspección) las fija el
    /// `#[serde(rename)]` del campo que lo contiene, no este `impl`.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

/// Frontmatter parseado de un documento. **Es metadata arbitraria del usuario**: sin campos
/// conocidos, sin lista cerrada, sin conversión automática de tipos y sin borrado de claves
/// desconocidas (`ARCHITECTURE.md §20.4`).
///
/// La ausencia de frontmatter se modela con `Option<ParsedFrontmatter>` (`None`), **no** con
/// `Value::Null`: «sin frontmatter» y «frontmatter vacío» son dos estados distintos del modelo.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFrontmatter {
    /// El YAML del bloque. **Siempre** un `Mapping` (vacío si el bloque está vacío o su YAML no
    /// es un mapa): el accesor y los catálogos de metadata necesitan una forma uniforme.
    pub value: serde_yaml::Value,
    /// Texto YAML **exacto** del bloque, sin los delimitadores `---`.
    pub raw: String,
    /// Rango de **bytes** que ocupa [`Self::raw`] dentro del raw del documento, de modo que
    /// `documento[span] == raw`. Excluye los delimitadores: el patch quirúrgico sustituye
    /// exactamente ese rango y los diagnósticos derivan de él su rango de reporte.
    ///
    /// En un `ParsedFrontmatter` **sintético** (el que construye [`ParsedFrontmatter::from_mapping`]
    /// para los flujos de escritura, que aún no tienen documento) el span es relativo a su propio
    /// `raw`.
    pub span: std::ops::Range<usize>,
}

impl Default for ParsedFrontmatter {
    fn default() -> Self {
        ParsedFrontmatter::from_mapping(serde_yaml::Mapping::new())
    }
}

impl ParsedFrontmatter {
    /// Construye un `ParsedFrontmatter` **sintético** a partir de un mapa YAML ya editado, que no
    /// procede de ningún documento: `raw` es la serialización canónica del mapa y `span` la cubre
    /// entera. Lo usan los flujos de escritura (merge-patch, creación de documentos), que componen
    /// frontmatter en memoria antes de que exista el `.md`.
    pub fn from_mapping(map: serde_yaml::Mapping) -> Self {
        let value = serde_yaml::Value::Mapping(map);
        let raw = serde_yaml::to_string(&value)
            .unwrap_or_default()
            .trim_end()
            .to_string();
        let span = 0..raw.len();
        ParsedFrontmatter { value, raw, span }
    }

    /// El `value` como `Mapping`. Siempre lo es por construcción; devuelve el mapa vacío si
    /// alguien manipuló el campo público hasta dejarlo en otra forma.
    pub fn mapping(&self) -> &serde_yaml::Mapping {
        static VACIO: once_cell::sync::Lazy<serde_yaml::Mapping> =
            once_cell::sync::Lazy::new(serde_yaml::Mapping::new);
        self.value.as_mapping().unwrap_or(&VACIO)
    }

    /// **La** única verdad de acceso a metadata (invariante #3): resuelve un [`FieldPath`]
    /// descendiendo por mapas anidados. Descender por un escalar, o por una clave que no existe,
    /// es ausencia (`None`), nunca un error.
    ///
    /// Una clave presente con valor `null` devuelve `Some(&Value::Null)` — así se distingue de la
    /// clave ausente.
    pub fn get(&self, path: &FieldPath) -> Option<&serde_yaml::Value> {
        let mut actual = &self.value;
        for segmento in &path.0 {
            actual = lookup(actual, segmento)?;
        }
        Some(actual)
    }

    /// Atajo de [`Self::get`] con un `FieldPath` de un **único segmento literal**: la clave de
    /// primer nivel `key`, aunque contenga puntos.
    pub fn get_key(&self, key: &str) -> Option<&serde_yaml::Value> {
        lookup(&self.value, key)
    }

    /// El valor escalar de la clave de primer nivel `key` renderizado a texto, para las columnas
    /// de la cache y los DTO de presentación. `None` si la clave falta o su valor **no** es un
    /// escalar (`null`, lista y mapa no tienen texto: no se aplanan).
    ///
    /// Es *renderizado de salida*, no la coerción de parseo que E16-H01 retiró: el `value` sigue
    /// conservando el tipo YAML real.
    pub fn get_text(&self, key: &str) -> Option<String> {
        self.get_key(key).and_then(scalar_text)
    }

    /// `true` si la clave de primer nivel `key` está presente (aunque su valor sea `null`).
    pub fn contains_key(&self, key: &str) -> bool {
        self.get_key(key).is_some()
    }

    /// Recorrido **recursivo** de toda la metadata direccionable: pares `(FieldPath, &Value)` en
    /// profundidad, **orden de aparición** y padre antes que hijos (`service`, `service.name`,
    /// `service.tier`).
    ///
    /// Es la pieza que el store v2 necesita para materializar la tabla `metadata` (`§20.12`,
    /// E18-H01) **sin escribir un segundo navegador del `Value`** (invariante #3), y la que
    /// heredan el evaluador de consultas (E19) y `metadata_inspect` (E20).
    ///
    /// **Invariante rector**: para todo par `(path, value)` devuelto,
    /// `self.get(&path) == Some(value)`. De él se siguen las cuatro reglas del recorrido:
    ///
    /// - Se **desciende solo por mapas** (igual que [`Self::get`]), y los mapas intermedios se
    ///   emiten **también** como par propio: `service` además de `service.name`/`service.tier`.
    /// - Una **lista es una hoja**: [`FieldPath`] no direcciona posiciones, así que emitir
    ///   `owners.0` inventaría paths que [`Self::get`] no resuelve. El valor entero —con los mapas
    ///   que contenga— viaja en el par de la propia lista.
    /// - Una clave **no escalar** (una lista o un mapa como clave, legales en YAML) no es
    ///   direccionable: ni ella ni su subárbol se emiten.
    /// - Si dos claves del mismo mapa **rinden al mismo texto** (`1:` y `"1":`), se emite solo la
    ///   primera: es la que resuelve [`Self::get`].
    ///
    /// La raíz no se emite: [`FieldPath`] es no vacío por construcción.
    pub fn walk(&self) -> Vec<(FieldPath, &serde_yaml::Value)> {
        let mut out = Vec::new();
        let mut prefijo: Vec<String> = Vec::new();
        walk_mapping(&self.value, &mut prefijo, &mut out);
        out
    }

    /// Pares clave→valor de primer nivel, en **orden de aparición**. Las claves se rinden a texto
    /// (las no escalares se omiten: no son direccionables por [`FieldPath`]).
    pub fn entries(&self) -> Vec<(String, &serde_yaml::Value)> {
        self.mapping()
            .iter()
            .filter_map(|(k, v)| scalar_text(k).map(|k| (k, v)))
            .collect()
    }
}

/// Recorre en profundidad el mapa `valor`, acumulando en `out` un par `(FieldPath, &Value)` por
/// cada propiedad **direccionable por [`ParsedFrontmatter::get`]**, con `prefijo` como camino de
/// segmentos hasta este nivel. Reflejo exacto de [`lookup`] (la única verdad de acceso): de ahí se
/// sigue el invariante rector `get(path) == Some(value)` para todo par emitido.
fn walk_mapping<'a>(
    valor: &'a serde_yaml::Value,
    prefijo: &mut Vec<String>,
    out: &mut Vec<(FieldPath, &'a serde_yaml::Value)>,
) {
    let Some(mapa) = valor.as_mapping() else {
        return;
    };
    // Dedup por el TEXTO de la clave dentro de este mismo mapa: si dos claves rinden al mismo
    // texto (`1:` y `"1":`), `get`/`lookup` resuelve la primera, así que solo esa se emite.
    let mut vistos: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (clave, hijo) in mapa {
        // Una clave no escalar (lista/mapa como clave, legales en YAML) no es direccionable.
        let Some(texto) = scalar_text(clave) else {
            continue;
        };
        if !vistos.insert(texto.clone()) {
            continue;
        }
        prefijo.push(texto);
        // Un segmento vacío (clave `""`) no construye un `FieldPath` válido: ni se emite el par ni
        // se desciende por su subárbol (sería inalcanzable por `get`). El resto sí.
        if let Ok(path) = FieldPath::from_segments(prefijo.iter().cloned()) {
            out.push((path, hijo));
            // Se desciende SOLO por mapas: una lista es una hoja (su valor entero viaja en su par;
            // `FieldPath` no direcciona posiciones).
            if hijo.is_mapping() {
                walk_mapping(hijo, prefijo, out);
            }
        }
        prefijo.pop();
    }
}

/// Busca `segmento` como clave de `valor` si este es un mapa. Compara por el **texto** de la
/// clave, de modo que una clave escalar no-string (`1: x`) sigue siendo direccionable.
fn lookup<'a>(valor: &'a serde_yaml::Value, segmento: &str) -> Option<&'a serde_yaml::Value> {
    valor
        .as_mapping()?
        .iter()
        .find(|(k, _)| scalar_text(k).as_deref() == Some(segmento))
        .map(|(_, v)| v)
}

/// Texto de un escalar YAML (string, número o booleano). `None` para `null`, listas y mapas.
///
/// `pub(crate)`: es **la** única verdad de «texto de un escalar» del core (la usan [`lookup`],
/// [`ParsedFrontmatter::get_text`] y el orden de `values` en `crate::metadata`), de modo que el
/// número `2` y el string `"2"` rinden al mismo texto sin que nadie reimplemente ese render.
pub(crate) fn scalar_text(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Error de frontmatter (es un dato, no un `Result`: `parse_file` nunca falla por contenido).
///
/// **No** hay variante «falta el frontmatter»: desde E16-H01 un documento sin bloque es válido y
/// se modela con `frontmatter: None` (`ARCHITECTURE.md §20.4`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FmError {
    Unclosed,
    Malformed(String),
}

// ---------------------------------------------------------------------------
// Enlaces: LinkKind · RawLink · LinkTarget · ResolvedLink · Inventory (§20.6, E17-H01/H02)
// ---------------------------------------------------------------------------
//
// Viven en `types` y no en `links` por el invariante #4 (`LinkTarget` viaja en el wire de
// `knowledge_get.outgoingLinks`). Las derivas de serialización se fijan aquí, **una vez**:
// `LinkTarget` va etiquetado adyacente en camelCase —`{"kind":"document","value":"a/b.md"}`,
// `{"kind":"escapesWorkspace"}`— para que la variante sin payload no cambie la forma de las
// demás. Qué tool lo expone, y bajo qué campo, es E17-H05.

schema_derive! {
/// Sintaxis con la que se escribió un enlace Markdown (`§20.6`). Es el `link_type` del parser:
/// clasifica **la forma**, no el destino (eso es [`LinkTarget`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkKind {
    /// `[texto](destino)`.
    Inline,
    /// `[texto][id]` con su definición `[id]: destino`.
    Reference,
    /// `[id][]`.
    Collapsed,
    /// `[id]`.
    Shortcut,
    /// `<https://example.com>`.
    Autolink,
}
}

schema_derive! {
/// Un enlace tal como aparece en el cuerpo, **sin resolver** (E17-H01).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawLink {
    /// El destino **crudo**, tal como está escrito: sin percent-decoding, con su fragmento y su
    /// query. En un enlace de referencia es el destino de la **definición**.
    pub href: String,
    /// El texto visible del enlace, en plano.
    pub text: String,
    /// Rango de **bytes del destino** dentro del cuerpo (no del enlace entero): `body[span]` es
    /// `href`. Lo consumen el `range` de los diagnósticos (`§20.9`) y la reescritura quirúrgica de
    /// `move_document` (`§20.11`).
    pub span: std::ops::Range<usize>,
    /// La forma sintáctica del enlace.
    pub kind: LinkKind,
}
}

schema_derive! {
/// Clasificación del destino de un enlace (`ARCHITECTURE.md §20.6`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum LinkTarget {
    /// Otro documento Markdown del inventario → **arista del grafo**.
    Document(RelPath),
    /// Fichero del proyecto que existe pero **no** es documento (p. ej. código): no es nodo.
    WorkspaceFile(RelPath),
    /// URI con esquema (`https:`, `mailto:`…).
    ExternalUri(String),
    /// Anchor del propio documento, **sin** la almohadilla.
    SelfAnchor(String),
    /// Destino contenido en el workspace que no existe, con su path ya normalizado.
    Missing(RelPath),
    /// El destino sale de la raíz del workspace. No lleva path: no hay `RelPath` que lo represente.
    EscapesWorkspace,
}
}

impl LinkTarget {
    /// El destino si el enlace conecta con **otro documento** del grafo, o `None` si no.
    ///
    /// Interno = [`LinkTarget::Document`] (arista real) o [`LinkTarget::Missing`] **de un destino
    /// que sería un documento Markdown** ([`RelPath::is_markdown`]): la arista a un fantasma, el
    /// documento que aún no existe pero al que ya se enlaza. Una URI externa, un anchor propio, un
    /// escape o un fichero del proyecto **no** conectan con ningún documento (`§20.7`).
    ///
    /// El filtro por familia solo se aplica a `Missing`, y es lo que impide que un enlace roto a
    /// código (`[x](src/no_existe.rs)`) meta un vértice `.rs` en el grafo de conocimiento, cuyos
    /// nodos son «todos los documentos **Markdown** descubiertos» (`§20.7`). A `Document` no se le
    /// aplica —ni puede aplicársele—: ese destino **está** en el inventario, así que es un documento
    /// aunque la política de descubrimiento admita otras extensiones; decidirlo por el nombre sería
    /// justo la clasificación por extensión que `§20.6` prohíbe. Un enlace roto a código sigue
    /// siendo un colgante diagnosticado (`LINK-TARGET-MISSING`, `Warn`) y sigue apareciendo en
    /// [`Analysis::dangling`]: lo que no es, es un nodo.
    ///
    /// Es la **única** definición de «enlace interno» (invariante #3): la reusan el grafo, los
    /// aislados, la reescritura de `move_document` y la tabla `links` de la cache.
    pub fn internal_path(&self) -> Option<&RelPath> {
        match self {
            LinkTarget::Document(p) => Some(p),
            LinkTarget::Missing(p) if p.is_markdown() => Some(p),
            _ => None,
        }
    }

    /// `true` si el enlace conecta con otro documento del grafo. Ver [`LinkTarget::internal_path`].
    pub fn is_internal(&self) -> bool {
        self.internal_path().is_some()
    }
}

schema_derive! {
/// Un enlace ya resuelto y clasificado (E17-H02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedLink {
    /// El href **original**, byte a byte (paso 10 del algoritmo de `§20.6`).
    pub href: String,
    /// El texto visible del enlace.
    pub text: String,
    /// Rango de bytes del destino dentro del cuerpo del documento origen.
    pub span: std::ops::Range<usize>,
    /// La forma sintáctica del enlace.
    pub kind: LinkKind,
    /// El destino clasificado.
    pub target: LinkTarget,
    /// El fragmento (`#seccion`) **sin** la almohadilla, si lo había. Vive aquí y no dentro de
    /// [`LinkTarget`] porque es ortogonal a la clasificación — y porque es una columna propia de
    /// `links(…, fragment, …)` en el store v2 (`§20.12`).
    pub fragment: Option<String>,
}
}

schema_derive! {
/// Un enlace visto **desde su destino**: quién lo escribe y cómo (`§20.7`, E17-H04).
///
/// Es el elemento de [`Analysis::incoming`], que es literalmente la inversa de
/// [`Analysis::outgoing`]: `link` es **el mismo** [`ResolvedLink`] que su origen tiene entre sus
/// salientes, no una copia recalculada (invariante #3). Anida el enlace en vez de copiar sus
/// campos porque `move_document` necesita su `span` y su `kind` para reescribir el destino.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkReference {
    /// El documento que escribe el enlace.
    pub from: RelPath,
    /// El enlace, ya resuelto y clasificado.
    pub link: ResolvedLink,
}
}

schema_derive! {
/// Un enlace **roto**: quién lo escribe, qué destino pretendía y cómo lo escribió (`§20.7`,
/// E17-H04).
///
/// Invariante: `link.target == LinkTarget::Missing(target)` — `target` es el payload de la
/// variante, no un segundo cálculo. Sustituye al `Vec<RelPath>` de destinos perdidos de la forma
/// anterior, que no permitía decir **quién** enlazaba mal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DanglingLink {
    /// El documento que contiene el enlace roto.
    pub from: RelPath,
    /// El destino pretendido, ya normalizado desde el origen.
    pub target: RelPath,
    /// El enlace tal como se escribió, ya resuelto.
    pub link: ResolvedLink,
}
}

/// Lo que el motor sabe que existe en el workspace, sin tocar el disco (invariante #2).
///
/// Separa **documentos** (los `.md` descubiertos, nodos potenciales del grafo) de los **demás
/// ficheros** del proyecto (código, imágenes, …), que `resolve` necesita para poder clasificar un
/// destino como [`LinkTarget::WorkspaceFile`] en vez de como [`LinkTarget::Missing`]. Quien hace
/// I/O —el descubrimiento de `lodestar-workspace`— es quien lo construye.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    documents: BTreeSet<RelPath>,
    other_files: BTreeSet<RelPath>,
    /// Índice auxiliar `ruta plegada a minúsculas → ruta REAL`, derivado de los dos anteriores.
    /// Solo lo consume [`Inventory::find_ignoring_case`]; no forma parte de la identidad del
    /// inventario más allá de lo que ya determinan `documents`/`other_files`.
    folded: BTreeMap<String, RelPath>,
}

impl Inventory {
    /// Inventario completo: documentos Markdown + resto de ficheros del proyecto.
    ///
    /// Los `.md` que existen en disco pero están **excluidos del descubrimiento** (un `vendor/`
    /// bajo `.gitignore`, p. ej.) van en `other_files`, no en `documents`: existen —así que su
    /// enlace no «falta»— pero no son nodos del grafo (`ARCHITECTURE.md §20.6`, precisión 2).
    pub fn new<D, F>(documents: D, other_files: F) -> Inventory
    where
        D: IntoIterator<Item = RelPath>,
        F: IntoIterator<Item = RelPath>,
    {
        Inventory::build(
            documents.into_iter().collect(),
            other_files.into_iter().collect(),
        )
    }

    /// Atajo: solo los documentos de un [`FileMap`], sin ficheros no-Markdown conocidos.
    pub fn from_documents(files: &FileMap) -> Inventory {
        Inventory::build(files.keys().cloned().collect(), BTreeSet::new())
    }

    /// Construye el inventario y su índice plegado. Los documentos entran **antes** que el resto
    /// de ficheros y en orden de `RelPath`, así que si dos rutas colisionan al plegar
    /// capitalización gana siempre la misma (determinismo: el veredicto no depende del orden de
    /// inserción del llamante).
    fn build(documents: BTreeSet<RelPath>, other_files: BTreeSet<RelPath>) -> Inventory {
        let mut folded: BTreeMap<String, RelPath> = BTreeMap::new();
        for p in documents.iter().chain(other_files.iter()) {
            folded
                .entry(p.as_str().to_lowercase())
                .or_insert_with(|| p.clone());
        }
        Inventory {
            documents,
            other_files,
            folded,
        }
    }

    /// La ruta **real** del inventario que coincide con `path` salvo capitalización, o `None` si
    /// no hay ninguna. Con una coincidencia exacta devuelve la propia `path`.
    ///
    /// Es lo que convierte un destino ausente en `LINK-CASE-MISMATCH` (E17-H03): el veredicto sale
    /// de este inventario **en memoria**, jamás del disco, así que es idéntico en un volumen
    /// case-insensitive (APFS) y en uno que no lo es (ext4) — que es justo el problema de
    /// portabilidad que el diagnóstico denuncia. El plegado es Unicode ([`str::to_lowercase`]),
    /// no ASCII.
    pub fn find_ignoring_case(&self, path: &RelPath) -> Option<&RelPath> {
        self.folded.get(&path.as_str().to_lowercase())
    }

    /// ¿Hay un documento Markdown en esa ruta exacta?
    ///
    /// Comparación **exacta**, sin plegado de mayúsculas: `Docs/Auth.md` no es `docs/auth.md`. De
    /// ahí sale el diagnóstico de portabilidad `LINK-CASE-MISMATCH` (E17-H03).
    pub fn contains_document(&self, path: &RelPath) -> bool {
        self.documents.contains(path)
    }

    /// ¿Existe un fichero del proyecto (no Markdown) en esa ruta exacta?
    pub fn contains_file(&self, path: &RelPath) -> bool {
        self.other_files.contains(path)
    }
}

// ---------------------------------------------------------------------------
// Análisis del workspace: Analysis (§4.1, §10 filas 4/5)
// ---------------------------------------------------------------------------

schema_derive! {
/// El resultado de `analyze()`: el **grafo universal** de `ARCHITECTURE.md §20.7` (E17-H04).
/// Wire en camelCase.
///
/// Nodos = todos los documentos descubiertos; aristas = los enlaces resueltos entre ellos. Los
/// seis campos son exactamente los de `§20.7`: **ninguno es un contador** — `hard_fail`/
/// `warn_count` pasaron a ser [`Analysis::hard_fail`]/[`Analysis::warn_count`], derivados de
/// `diagnostics`, de modo que no puede existir un recuento desincronizado de la lista de la que
/// sale (invariante #3).
///
/// **E16-H02** retiró `in_index`/`okf_version` (y `Backlinks::index_refs`): la pertenencia
/// determinada por índices no existe — `index.md` es un documento como cualquier otro y sus
/// enlaces son aristas normales. **E17-H04** sustituyó la adyacencia de strings (`out`/`inn`) por
/// los enlaces resueltos, `dangling: Vec<RelPath>` por [`DanglingLink`] y `per_file` por
/// `diagnostics`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    /// **Todos** los documentos del workspace, ordenados por `RelPath`: ningún basename se salta
    /// el análisis (`§20.7`).
    pub documents: Vec<RelPath>,
    /// Enlaces salientes ya resueltos, en **orden de aparición** en el cuerpo. Una entrada por
    /// documento (vector vacío si no enlaza a nadie). Lleva **todos** los enlaces —también los
    /// externos, los anchors y los que apuntan a ficheros del proyecto—, no solo las aristas del
    /// grafo: los necesitan `knowledge_get`, `move_document` y la tabla `links` del store v2. El
    /// filtro de qué es arista lo hace el grafo, no esta lista.
    pub outgoing: BTreeMap<RelPath, Vec<ResolvedLink>>,
    /// La inversa de `outgoing`: quién enlaza a cada documento, con **una entrada por enlace**
    /// (un origen que enlaza dos veces aparece dos veces). Una entrada por documento.
    pub incoming: BTreeMap<RelPath, Vec<LinkReference>>,
    /// Documentos **aislados** (`§20.7`): sin enlaces internos entrantes **ni** salientes. Es una
    /// propiedad consultable, **no** un diagnóstico (el código `ORPHAN` murió con E16-H02).
    pub isolated: Vec<RelPath>,
    /// Los enlaces cuyo destino es [`LinkTarget::Missing`], con su origen y su href crudo.
    pub dangling: Vec<DanglingLink>,
    /// Diagnósticos por documento (antes `per_file`). Una entrada por documento.
    pub diagnostics: BTreeMap<RelPath, Vec<Check>>,
}
}

impl Analysis {
    /// Nº de **ficheros** con al menos un diagnóstico [`Severity::Err`] (conteo de ficheros, no
    /// `.max()` ni nº de diagnósticos — `§10` fila 4). Es lo que decide la puerta de CI
    /// (`WorkspaceConfig::gate_blocked`, `lodestar check`).
    ///
    /// Derivado de `diagnostics` en cada llamada: `§20.7` no admite un campo contador, y así el
    /// recuento no puede divergir de la lista de la que sale (invariante #3).
    pub fn hard_fail(&self) -> usize {
        self.diagnostics
            .values()
            .filter(|cs| cs.iter().any(|c| c.level == Severity::Err))
            .count()
    }

    /// Nº total de **diagnósticos** [`Severity::Warn`] del workspace (suma sobre ficheros, no
    /// conteo de ficheros — es la semántica histórica de `warn_count`). Derivado de `diagnostics`,
    /// por la misma razón que [`Analysis::hard_fail`].
    pub fn warn_count(&self) -> usize {
        self.diagnostics
            .values()
            .flatten()
            .filter(|c| c.level == Severity::Warn)
            .count()
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
/// Un nodo del grafo (`§20.7`, E17-H05).
///
/// Perdió `type`/`status` —campos OKF, que dejaron de ser vocabulario del modelo (`§20.3`)— y ganó
/// el **título derivado** de E16-H03 ([`crate::model::derived_title`]). Conserva `ghost`, que
/// distingue el nodo de un destino [`LinkTarget::Missing`] del de un documento real.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: RelPath,
    pub title: String,
    pub ghost: bool,
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
/// Subgrafo dirigido alrededor de un documento (`root` = el centro).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Neighborhood {
    pub root: RelPath,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
}
}

// ---------------------------------------------------------------------------
// DTOs de lectura de DocumentSet (§4.2)
// ---------------------------------------------------------------------------

schema_derive! {
/// Fila del árbol de documentos. `title` ya resuelto por [`crate::model::derived_title`];
/// `invalid` = algún Check `Err`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummary {
    pub path: RelPath,
    pub title: String,
    pub r#type: Option<String>,
    pub status: Option<String>,
    /// Sin enlaces internos entrantes ni salientes ([`Analysis::isolated`]). Antes `orphan`, con
    /// otra definición ("sin entrantes y no listado en un índice") — E16-H02.
    pub isolated: bool,
    pub invalid: bool,
}
}

schema_derive! {
/// Vecindad de enlaces de un documento: su porción de [`Analysis::incoming`]/
/// [`Analysis::outgoing`], sin recalcular nada (invariante #3).
///
/// **E16-H02** retiró `index_refs`: un `index.md` que te enlaza es un entrante más de `inbound`,
/// no una relación de pertenencia aparte (`REFACTOR_PHASE_2 §Fase 8 (Eliminar)`). **E17-H05**
/// pasó `inbound`/`out` a los tipos del grafo universal: el `LinkRef` `{path, href}` (que solo
/// llevaba el href) desapareció, y `out` dejó de ser una lista de paths resueltos.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backlinks {
    /// Quién enlaza aquí, con el enlace completo (una entrada por **enlace**).
    pub inbound: Vec<LinkReference>,
    /// Los enlaces salientes del documento, resueltos y en orden de aparición. Incluye los
    /// externos, los anchors y los colgantes: la clasificación va dentro de cada
    /// [`ResolvedLink::target`], no en listas separadas.
    pub out: Vec<ResolvedLink>,
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
    pub workspace_hard_fail: usize,
}

// ---------------------------------------------------------------------------
// Identidad de contenido determinista: `DocumentRevision` / `WorkspaceRevision`
// (E10-H03, `ARCHITECTURE.md §19.3`, `REFACTOR §6.2/§6.3`). Eleva blake3 (ya usado en
// `WriteOutcome.hash`, `document_set.rs`) a identidad expuesta. Wire = string `"blake3:<hex>"`.
// ---------------------------------------------------------------------------

schema_derive! {
/// Revisión de contenido de un único `.md`: `"blake3:<hex>"` del contenido en disco.
/// Wire = el string tal cual (sin envoltorio de objeto).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DocumentRevision(pub String);
}

impl DocumentRevision {
    /// Construye la revisión a partir de un hash blake3 crudo (el mismo patrón que
    /// `WriteOutcome.hash`).
    pub fn from_hash(hash: [u8; 32]) -> Self {
        DocumentRevision(format!("blake3:{}", blake3::Hash::from(hash).to_hex()))
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
/// - Si `writable` está vacío, incluye todo lo que no sea `.lodestar/` (todo el workspace es
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
    #[serde(rename = "DOCUMENT_NOT_FOUND")]
    DocumentNotFound,
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
            ErrorCode::DocumentNotFound => "DOCUMENT_NOT_FOUND",
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
// Planificación (`ChangeSet`) — §4.1 vía `ARCHITECTURE.md §19.3`, `REFACTOR §6.4` (E12-H01)
// ---------------------------------------------------------------------------
//
// SOLO las formas: la lógica que produce cada pieza (riesgo, diff semántico, validación,
// normalización de cada operación) es de las historias E12-H02..H07. Aquí se congela el contrato
// de wire de `ChangeSet` (criterio `changeset_shape`) y las 11 variantes de `NormalizedOperation`
// con campos razonables — su forma exacta la cierran esas historias.

schema_derive! {
/// Identificador de un `ChangeSet` (plan). Newtype string transparente, mismo patrón que
/// [`WorkspaceRevision`]/[`DocumentRevision`].
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
/// Diff semántico entre el workspace actual y el hipotético resultante de aplicar un `ChangeSet`
/// (E12-H03 lo calcula; aquí solo la forma). `frontmatter_changes`/`body_changes`/
/// `relation_changes` son los paths afectados por cada categoría — una forma mínima razonable;
/// E12-H03 puede reusar [`crate::diff::SnapshotDiff`] como referencia sin que sea obligatorio aquí.
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
/// Veredicto de conformidad del workspace hipotético resultante de un `ChangeSet` (E12-H04 lo
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
/// Política ante enlaces entrantes al borrar un documento (E12-H06). `Reject` es el default del
/// prototipo/spec — un `delete` sobre un documento referenciado se rechaza salvo que se pida
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
/// `ChangeSet`. Las **8 operaciones universales** de `§20.11` (contenido: E12-H05 · estructura:
/// E12-H06 · `apply_fix`: E12-H07); aquí solo su forma — los campos son razonables para lo que cada
/// operación resuelve, sin cerrar la lógica que los produce. E21-H01 retiró las 3 operaciones
/// semánticas (`add_relation`/`remove_relation`/`transition_status`): una relación es un enlace
/// Markdown y un estado es una propiedad arbitraria del frontmatter (`§20.11`), así que ambas se
/// expresan con las universales (una transición es un `PatchFrontmatter`).
///
/// El tag de wire (`op`) usa los mismos nombres snake_case que `proposedOperation.kind`
/// (`contracts/mcp.yml`) — un solo vocabulario de tipos de operación en el contrato.
///
/// Sin `Eq`: `Create`/`PatchFrontmatter` llevan `FrontmatterPatch`, que envuelve
/// `serde_yaml::Value` (solo `PartialEq`, por los números `f64`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum NormalizedOperation {
    /// Crea un documento nuevo. `body: None` ⇒ el escritor genera el heading por defecto (tras el
    /// retiro de `core::schema` en E20-H03 ya no hay `bodyTemplate` de `DocType` que expandir).
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
    /// Sustituye el cuerpo completo del documento.
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
    /// Mueve/renombra un documento; `rewrite_inbound_links` decide si sus backlinks se reescriben
    /// dentro del mismo change set (E12-H06).
    Move {
        from: RelPath,
        to: RelPath,
        rewrite_inbound_links: bool,
    },
    /// Borra un documento, sujeto a `inbound_links_policy` si está referenciado (E12-H06).
    Delete {
        path: RelPath,
        inbound_links_policy: InboundLinksPolicy,
    },
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

schema_derive! {
/// Recibo de una aplicación de `ChangeSet` **completada** (E13-H07). Tras sellar la transacción
/// (`done`), se persiste como `.lodestar/runtime/receipts/<receiptId>.json` para poder revertir
/// (E13-H09) y auditar. Runtime desechable (invariante #1), retenido según la config
/// `transactions` (`retainReceiptsFor`/`maximumReceipts`).
///
/// **Fase ROJA (E13-H07)**: aquí solo se congela la forma que el implementador persistirá; la
/// mecánica de escritura/GC vive en `lodestar-workspace`. Los campos son los que fija la spec de
/// la historia (`{ id, changeSetId, previousRevision, resultRevision, changedPaths, semanticDiff }`);
/// si el implementador necesita añadir alguno, que respete estos nombres de wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeReceipt {
    pub id: ReceiptId,
    pub change_set_id: ChangeSetId,
    pub previous_revision: WorkspaceRevision,
    pub result_revision: WorkspaceRevision,
    pub changed_paths: Vec<RelPath>,
    pub semantic_diff: SemanticDiff,
}
}

// ---------------------------------------------------------------------------
// Lenguaje de consulta tipado: QueryValue · ComparisonOperator · FunctionName ·
// Expression · ValueType · TypeError
// (`ARCHITECTURE.md §20.8`, `REFACTOR_PHASE_2 §Fase 5`, E19-H01 — supersede la DSL de subcadena
//  de `§4.3`/`query.rs`, que se retira en E19-H05).
// ---------------------------------------------------------------------------
//
// Aquí se define solo la FORMA del AST y de sus tipos de apoyo (el contrato de wire que toda E19
// hereda). La lógica del evaluador vive en [`crate::eval::evaluate`] (E19-H01/H04) y la del parser
// textual en [`crate::parse::parse`] (E19-H02); el filtro JSON es E19-H03.

/// Un valor literal **tipado** de una consulta: el operando derecho de una [`Expression::Comparison`]
/// y el argumento de una [`Expression::Function`] (`§20.8`, `§Fase 5 (AST unificado)`).
///
/// Refleja los cinco tipos escalares/compuestos que el lenguaje admite como literal —
/// string/número/booleano/`null`/lista— y **conserva el tipo** (no hay coerción): es lo que permite
/// que `priority = "2"` (string) y `priority = 2` (número) sean literales distintos y no el mismo
/// valor renderizado a texto.
///
/// **Forma serde (contrato de wire, `§20.10`)**: `#[serde(untagged)]` para que el campo `value` del
/// filtro JSON de E19-H03 deserialice desde el valor JSON **desnudo** (`"accepted"` → `String`, `2`
/// → `Number`, `true` → `Bool`, `null` → `Null`, `["a","b"]` → `List`) sin envoltura. El número usa
/// [`serde_yaml::Number`] —el mismo dominio numérico que [`ParsedFrontmatter::get`] devuelve— para
/// que la comparación no tenga que cruzar representaciones. El orden de las variantes es el orden en
/// que serde las prueba: `Null` primero (solo casa con `null`), la lista al final. **E19-H03 fija y
/// testea el round-trip JSON exacto**; aquí solo se declara la forma.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QueryValue {
    /// El literal `null`.
    Null,
    /// Un booleano (`true`/`false`).
    Bool(bool),
    /// Un número (entero o real), en el dominio numérico de YAML.
    Number(serde_yaml::Number),
    /// Un string (el literal entrecomillado de la consulta textual).
    String(String),
    /// Una lista de literales — el operando de `contains_any`/`contains_all`.
    List(Vec<QueryValue>),
}

/// Los operadores de una [`Expression::Comparison`] (`§20.8`, `§Fase 5 (Operadores mínimos)`).
///
/// **Un solo `Contains`**: `§Fase 5` lista `contains` bajo «Texto» *y* bajo «Listas»; no son dos
/// operadores, sino uno cuyo significado lo decide el **tipo del campo** (subcadena sobre un string,
/// pertenencia sobre una lista). `contains_any`/`contains_all` son exclusivos de listas.
///
/// **Forma serde (contrato de wire, `§20.10`)**: nombres largos —`equals`, `greater_than_or_equal`,
/// …— porque el filtro JSON de E19-H03 usa `"operator": "equals"`, no el símbolo `=`. La consulta
/// textual de E19-H02 mapea `=`/`>=`/… a estas variantes por su cuenta (el símbolo no es wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparisonOperator {
    /// `=` — igualdad por **valor e igualdad de tipo** (cruce de tipos = `false`, no error).
    #[serde(rename = "equals")]
    Eq,
    /// `!=` — la negación de [`ComparisonOperator::Eq`].
    #[serde(rename = "not_equals")]
    Ne,
    /// `>` — orden estricto; exige ambos operandos numéricos o ambos string (cruce = `TypeError`).
    #[serde(rename = "greater_than")]
    Gt,
    /// `>=` — orden no estricto; mismas reglas de tipo que [`ComparisonOperator::Gt`].
    #[serde(rename = "greater_than_or_equal")]
    Ge,
    /// `<` — orden estricto.
    #[serde(rename = "less_than")]
    Lt,
    /// `<=` — orden no estricto.
    #[serde(rename = "less_than_or_equal")]
    Le,
    /// `contains` — subcadena si el campo es string, pertenencia si es lista (el tipo decide).
    #[serde(rename = "contains")]
    Contains,
    /// `starts_with` — prefijo de texto (solo sobre string).
    #[serde(rename = "starts_with")]
    StartsWith,
    /// `ends_with` — sufijo de texto (solo sobre string).
    #[serde(rename = "ends_with")]
    EndsWith,
    /// `contains_any` — la lista del campo comparte al menos un elemento con el literal (solo lista).
    #[serde(rename = "contains_any")]
    ContainsAny,
    /// `contains_all` — la lista del campo contiene todos los elementos del literal (solo lista).
    #[serde(rename = "contains_all")]
    ContainsAll,
}

/// Las funciones de **existencia** de una [`Expression::Function`] (`§20.8`, `§Fase 5 (Existencia)`).
///
/// `has(x)` es «la propiedad `x` está presente» y `missing(x)` su negación. Existencia se juzga con
/// [`ParsedFrontmatter::get`] (presente aunque su valor sea `null`/`""`/`[]`), **no** con la vieja
/// heurística `fmPresent` de `query.rs` (que trataba `""` y la lista vacía como ausencia).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionName {
    /// `has(x)` — la propiedad existe.
    Has,
    /// `missing(x)` — la propiedad no existe.
    Missing,
}

/// El **AST unificado** del lenguaje de consulta (`§20.8`, `§Fase 5 (AST unificado)`): tanto la
/// consulta textual `where` (E19-H02) como el filtro estructurado `filter` (E19-H03) se traducen a
/// este único árbol, y **producen exactamente el mismo resultado**.
///
/// La evalúa [`crate::eval::evaluate`], que respeta los tipos YAML sin coerción (E19-H01).
///
/// **serde diferido a E19-H03**: `Comparison` lleva un [`FieldPath`], que hoy no es
/// `Serialize`/`Deserialize`; el filtro JSON de E19-H03 —el único consumidor de wire de este
/// árbol— añadirá esa capacidad y la forma etiquetada (`{and:[…]}`, `{field,operator,value}`) junto
/// con su test de round-trip. En E19-H01 el AST se construye **en memoria** (por el evaluador y sus
/// tests), así que aquí basta con `PartialEq` para poder compararlo.
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    /// Una comparación `campo operador valor` (`priority >= 2`, `owners contains "security"`).
    Comparison {
        field: FieldPath,
        operator: ComparisonOperator,
        value: QueryValue,
    },
    /// Una llamada de existencia (`has(status)`, `missing(reviewed_at)`). El argumento nombra la
    /// propiedad como [`QueryValue::String`] (la forma que impone el AST de `§20.8`,
    /// `arguments: Vec<QueryValue>`); el evaluador lo reinterpreta como [`FieldPath`].
    Function {
        name: FunctionName,
        arguments: Vec<QueryValue>,
    },
    /// Conjunción: verdadera si **todas** sus ramas lo son.
    And(Vec<Expression>),
    /// Disyunción: verdadera si **alguna** de sus ramas lo es.
    Or(Vec<Expression>),
    /// Negación.
    Not(Box<Expression>),
}

/// El tipo YAML **observado** de un valor, para poblar los operandos de un [`TypeError`] («qué
/// encontró»). Es la clasificación mínima que distingue las cinco familias que el lenguaje trata de
/// forma distinta (escalar ordenable vs no ordenable vs lista vs mapa).
///
/// `PartialOrd`/`Ord` (E20-H01): [`ValueType`] es la clave del mapa `inferred_types` de
/// [`FieldStats`]/[`FieldInspection`]; el orden de declaración da un [`BTreeMap`] determinista.
///
/// `Serialize` con `rename_all = "lowercase"` (E20-H03): en el wire de `metadata_inspect` cada tipo
/// es la **clave en minúscula** de `inferredTypes` (`{"string": 5, "number": 1}`) — el `BTreeMap`
/// con clave [`ValueType`] serializa a ese objeto. Es la forma que fija `§Fase 6`.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueType {
    /// `null`.
    Null,
    /// Un booleano — **no** ordenable.
    Bool,
    /// Un número — ordenable entre números.
    Number,
    /// Un string — ordenable entre strings (lexicográfico) y contenedor de subcadenas.
    String,
    /// Una lista.
    List,
    /// Un mapa/objeto.
    Mapping,
}

impl ValueType {
    /// Clasifica un [`serde_yaml::Value`] en su [`ValueType`]. La usa el evaluador para poblar los
    /// operandos de un [`TypeError`]; se declara aquí (no en `eval`) por vivir junto al enum.
    pub fn of(value: &serde_yaml::Value) -> ValueType {
        match value {
            serde_yaml::Value::Null => ValueType::Null,
            serde_yaml::Value::Bool(_) => ValueType::Bool,
            serde_yaml::Value::Number(_) => ValueType::Number,
            serde_yaml::Value::String(_) => ValueType::String,
            serde_yaml::Value::Sequence(_) => ValueType::List,
            serde_yaml::Value::Mapping(_) => ValueType::Mapping,
            // `Tagged` (un `!Tag valor` de YAML) se clasifica por su valor interno.
            serde_yaml::Value::Tagged(t) => ValueType::of(&t.value),
        }
    }
}

/// El error de tipo del evaluador (`§20.8`, `§Fase 5 (Semántica de tipos)`): la consecuencia de
/// prohibir la coerción implícita. Es lo que separa este lenguaje de un grep — `priority >= "high"`
/// **no** es `false`, es un error.
///
/// Lleva estructurado *qué esperaba* (la variante) y *qué encontró* (los [`ValueType`] de los
/// operandos), de modo que E20/E21 puedan mapearlo a `ErrorCode::InvalidSchema` con un mensaje
/// legible sin volver a inspeccionar el `Value`. Tipo **propio** —y no una variante de
/// [`crate::CoreError`]— porque no es un fallo del núcleo sino un dato del `Result` del evaluador
/// (mismo espíritu que [`FmError`]): un `where` mal tipado es entrada del agente, y quien lo
/// traduce a protocolo (la fachada) decide su envoltorio.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeError {
    /// Una comparación de **orden** (`> >= < <=`) cuyos operandos no admiten orden entre sí: un
    /// número frente a un string (**orden cruzado**), o un tipo no ordenable en cualquiera de los
    /// lados (booleano, `null`, lista, mapa). El orden solo está definido entre dos números o entre
    /// dos strings (lexicográfico). Contrasta con `=`/`!=`, que **nunca** es error: el cruce de
    /// tipos en igualdad es `false`.
    OrderNotDefined {
        field: FieldPath,
        operator: ComparisonOperator,
        /// El tipo del **campo** (operando izquierdo).
        field_type: ValueType,
        /// El tipo del **literal** (operando derecho).
        value_type: ValueType,
    },
    /// Un operador de **lista** (`contains`/`contains_any`/`contains_all`) sobre un campo que no es
    /// lista. `contains` admite además un string (subcadena), así que solo es error sobre un
    /// escalar **no string**; `contains_any`/`contains_all` son exclusivos de listas y son error
    /// también sobre un string. Un campo **inexistente** no llega aquí: la ausencia es `false`.
    NotAList {
        field: FieldPath,
        operator: ComparisonOperator,
        /// El tipo que tenía el campo (nunca `List`).
        found: ValueType,
    },
}

// ---------------------------------------------------------------------------
// Inspección de metadata: MetadataCatalog · FieldStats · FieldInspection · ValueCount
// (`ARCHITECTURE.md §20.10`, `REFACTOR_PHASE_2 §Fase 6`, E20-H01/H02 — supersede `schema_inspect`)
// ---------------------------------------------------------------------------
//
// La FORMA de los tipos de retorno de `crate::metadata::catalog`/`inspect_field`: el contrato de
// wire que hereda la tool `metadata_inspect` (E20-H03). Aquí se fija el mapeo de wire (`§Fase 6`):
//   · el `FieldPath` viaja como su string PUNTEADO (`Serialize` de `FieldPath`, arriba), bajo la
//     clave `"name"` en el catálogo (`#[serde(rename)]`) y `"field"` en la inspección;
//   · `inferred_types` (`BTreeMap<ValueType, usize>`) se aplana al objeto `{tipo-en-minúscula:
//     conteo}` gracias al `Serialize` de `ValueType` (`rename_all = "lowercase"`);
//   · `value` (`serde_yaml::Value`) conserva su tipo JSON natural (número/string/…), sin coerción.
// Se conservan [`FieldPath`] y [`ValueType`] como identidad —«una sola verdad de qué es un campo y
// de qué tipo» (invariante #3)—, no una representación paralela en `String`. El wire se DERIVA
// (no hay capa DTO paralela, invariante #4). `Deserialize` no se deriva: el core PRODUCE estos
// tipos (los computa `crate::metadata`), no los consume del wire; la tool solo serializa.

schema_derive! {
/// El **catálogo de propiedades** del workspace (`§Fase 6`, «Catálogo de propiedades»): una fila por
/// `field_path` que aparece en algún documento. Wire: `{ "fields": [ … ] }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetadataCatalog {
    /// Los campos del workspace, en orden **determinista** por [`FieldPath`].
    pub fields: Vec<FieldStats>,
}
}

schema_derive! {
/// Estadísticas de un `field_path` en el catálogo (`§Fase 6`). Wire:
/// `{ "name": "status", "presentIn": N, "inferredTypes": { "<tipo>": N } }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldStats {
    /// El path de la propiedad, **tal como lo emite** [`ParsedFrontmatter::walk`]: incluye los mapas
    /// intermedios (`service`) además de las hojas direccionables (`service.name`, `service.tier`),
    /// para que el catálogo enumere el mismo conjunto de campos que indexa el store v2 (E18) — una
    /// sola verdad de qué es un campo (invariante #3). En el wire es la clave **`name`** (`§Fase 6`),
    /// con el `FieldPath` rendido a su string punteado.
    #[serde(rename = "name")]
    #[cfg_attr(feature = "schemars", schemars(with = "String"))]
    pub field: FieldPath,
    /// Nº de documentos en los que la propiedad aparece.
    pub present_in: usize,
    /// Tipos observados y su conteo, clasificando cada valor con [`ValueType::of`]. Invariante:
    /// `inferred_types.values().sum() == present_in` (una observación de tipo por documento presente).
    /// Wire: objeto `{tipo-en-minúscula: conteo}`.
    pub inferred_types: BTreeMap<ValueType, usize>,
}
}

schema_derive! {
/// La **inspección de una propiedad** (`§Fase 6`, «Inspección de una propiedad»). Wire:
/// `{ "field": "status", "presentIn": N, "missingIn": N, "inferredTypes": {…}, "values": [ … ] }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldInspection {
    /// El path inspeccionado. En el wire es la clave **`field`** (`§Fase 6`), con el `FieldPath`
    /// rendido a su string punteado.
    #[cfg_attr(feature = "schemars", schemars(with = "String"))]
    pub field: FieldPath,
    /// Nº de documentos en los que aparece.
    pub present_in: usize,
    /// Nº de documentos en los que NO aparece. Invariante:
    /// `present_in + missing_in == nº total de documentos del workspace`.
    pub missing_in: usize,
    /// Tipos observados y su conteo ([`ValueType::of`] de cada valor). Wire: objeto
    /// `{tipo-en-minúscula: conteo}`.
    pub inferred_types: BTreeMap<ValueType, usize>,
    /// Los valores **escalares** más frecuentes con su conteo, en orden **determinista**: por conteo
    /// descendente y, a igual conteo, por el texto del valor ascendente. Un valor lista u objeto
    /// cuenta en `present_in`/`inferred_types` pero **no** aparece aquí (`§Fase 6`: solo escalares).
    pub values: Vec<ValueCount>,
}
}

schema_derive! {
/// Un valor escalar observado y en cuántos documentos aparece (`§Fase 6`, `{value, count}`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueCount {
    /// El valor escalar, **con su tipo YAML real** (sin coerción: el número `2` y el string `"2"`
    /// son valores distintos, con conteos distintos). En el wire conserva su tipo JSON natural.
    #[cfg_attr(feature = "schemars", schemars(with = "serde_json::Value"))]
    pub value: serde_yaml::Value,
    /// Cuántos documentos tienen exactamente ese valor en la propiedad.
    pub count: usize,
}
}
