//! Paths externos (`referenceRoots`, E11-H04, `ARCHITECTURE.md §19.4/§19.7`, E9-H05).
//!
//! Dos responsabilidades, ambas ancladas en `referenceRoots` del `.lodestar/config.yaml`:
//! 1. [`Workspace::external_refs`]: resuelve contra disco los campos de frontmatter
//!    `implemented_by`/`verified_by` (paths a ficheros de **código**) y devuelve `{path, exists}`
//!    por cada uno — lo consume `knowledge_get(externalReferences)`. Comprobar la existencia en
//!    disco es **I/O**, así que vive aquí y no en `lodestar-core` (invariante #2).
//! 2. [`Workspace::assert_writable`]: la **write policy** del único escritor, que usa
//!    `referenceRoots` como raíces «visibles pero NO escribibles» (inmutables).
//!
//! **DECISIÓN E20-H03**: el diagnóstico `EXTREF-MISSING` (una ref externa rota) se **retira** con el
//! resto de la maquinaria schema (`§20.10`, modelo universal). `referenceRoots` se **conserva**
//! porque sostiene la write policy de `assert_writable`, no solo aquel diagnóstico. `external_refs`
//! sigue resolviendo `{path, exists}` para `knowledge_get`; quien quiera un diagnóstico de ref rota
//! lo deriva de `exists:false`.

use lodestar_core::model;
use lodestar_core::types::RelPath;

use crate::error::WorkspaceError;
use crate::Workspace;

/// Campos de frontmatter que declaran referencias a ficheros externos (E9-H05). Cada uno admite
/// una lista de paths o un único path.
const EXTERNAL_REF_FIELDS: [&str; 2] = ["implemented_by", "verified_by"];

/// Una referencia externa (`implemented_by`/`verified_by`) de un documento, resuelta contra disco.
/// Wire camelCase `{path, exists}` (alimenta `externalReferences` de `knowledge_get`, E10-H10).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct ExternalReference {
    /// El path crudo del frontmatter, p. ej. `"src/x.rs"` (sin normalizar contra `RelPath`: un
    /// path inválido como referencia externa se resuelve igualmente, con `exists:false`, en vez de
    /// descartarse en silencio).
    pub path: String,
    /// `true` si existe un fichero real en disco bajo ese path, relativo al root del workspace.
    pub exists: bool,
}

/// Informe de las referencias externas de UN documento, resueltas contra `referenceRoots`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalRefsReport {
    /// Cada referencia declarada por el documento, con su existencia resuelta.
    pub references: Vec<ExternalReference>,
}

impl Workspace {
    /// Resuelve las referencias externas (`implemented_by`/`verified_by`) del documento contra
    /// disco, devolviendo `{path, exists}` por cada una. Tras E20-H03 **no** produce diagnósticos
    /// (el `EXTREF-MISSING` se retiró); un consumidor deriva la ref rota de `exists:false`.
    ///
    /// Un documento sin frontmatter (o sin ninguno de los dos campos) devuelve un informe vacío,
    /// no un error. `Err` solo si el documento mismo no existe en el workspace.
    ///
    /// **Invariante #6** (`RelPath` es el único chokepoint de path-traversal): el path crudo del
    /// frontmatter NUNCA toca disco directamente. Antes de cualquier `join`/`is_file`:
    /// 1. Se valida con [`RelPath::new`] — una cadena absoluta (`/etc/hosts`), con `..`
    ///    (`../secreto`) o backslash se rechaza aquí, sin tocar el filesystem.
    /// 2. El `RelPath` válido debe caer, además, bajo alguno de los `referenceRoots`
    ///    configurados (contención por segmentos, [`lodestar_core::types::under_root`]) — la spec
    ///    (E11-H04) dice que estos campos apuntan a código BAJO `referenceRoots`; un `RelPath`
    ///    válido pero fuera de todos ellos no se resuelve contra disco tampoco.
    ///
    /// En ambos casos de rechazo, la referencia sale con `exists:false` (nunca `true`): sin esto,
    /// `join` sobre una cadena absoluta o `unchecked` permitiría usar `knowledge_get` como oráculo
    /// de existencia de ficheros arbitrarios del host (`ref_externa_traversal`, hallado por juez).
    pub fn external_refs(&self, document: &RelPath) -> Result<ExternalRefsReport, WorkspaceError> {
        let doc_set = self.document_set()?;
        let raw = doc_set.files().get(document).ok_or_else(|| {
            WorkspaceError::Io(format!("documento no encontrado: {}", document.as_str()))
        })?;
        let parsed = model::parse_file(document.as_str(), raw);
        let Some(fm) = parsed.frontmatter else {
            return Ok(ExternalRefsReport {
                references: Vec::new(),
            });
        };

        let reference_roots = &self.config().workspace.reference_roots;

        let mut references = Vec::new();
        for field in EXTERNAL_REF_FIELDS {
            for raw_path in field_paths(&fm, field) {
                // Único chokepoint de traversal: valida ANTES de tocar disco (paso 1), luego
                // confina a `referenceRoots` (paso 2). Solo si ambos pasan se hace `is_file`.
                let validated = RelPath::new(&raw_path).ok();
                let contained = validated.as_ref().filter(|rp| {
                    reference_roots
                        .iter()
                        .any(|root| lodestar_core::types::under_root(rp, root))
                });
                let exists = contained.is_some_and(|rp| self.root().join(rp.as_str()).is_file());

                references.push(ExternalReference {
                    path: raw_path,
                    exists,
                });
            }
        }
        Ok(ExternalRefsReport { references })
    }

    /// Guard del único escritor: `Err(WorkspaceError::PermissionDenied)` si `path` queda **fuera
    /// del inventario** del descubrimiento (E15-H09), si cae bajo un `referenceRoot` (inmutable) o,
    /// cuando `writableRoots` es una lista explícita no vacía, fuera de todos ellos; `Ok(())` en
    /// caso contrario (incluye el caso `writableRoots` vacío = todo el workspace escribible salvo
    /// `referenceRoots`, mismo criterio que [`lodestar_core::types::workspace_revision`]).
    ///
    /// Contención por SEGMENTOS de path (reusa [`lodestar_core::types::under_root`]), nunca por
    /// prefijo de string — así `"src"` no cubre `"srcx/y.rs"`.
    ///
    /// # Descubrimiento primero (E15-H09, `REFACTOR_PHASE_2 §Principio 8`)
    ///
    /// Antes que las raíces se consulta [`Workspace::assert_discoverable`]: escribir donde el
    /// inventario no mira deja un documento invisible al grafo y ciego al control optimista, así
    /// que es un rechazo previo a cualquier consideración de permisos.
    ///
    /// **Cuando los dos criterios se cruzan, manda la exclusión**: un path excluido del
    /// descubrimiento se rechaza aunque caiga bajo un `writableRoot` explícito (p. ej.
    /// `writableRoots: [knowledge]` con un `.gitignore` que ignora `knowledge/borradores/` ⇒
    /// `knowledge/borradores/x.md` NO es escribible). Dos razones:
    ///
    /// 1. `writableRoots` es una lista de **permiso**, no de **habilitación** — la config
    ///    «limita, nunca habilita» (`ARCHITECTURE.md §20.1`), así que declarar una raíz no puede
    ///    resucitar un path que el inventario no ve.
    /// 2. Lo que sostiene la exclusión es una **invariante de correctitud del motor** (todo
    ///    documento del inventario cuenta para [`lodestar_core::types::workspace_revision`],
    ///    `ARCHITECTURE.md §20.5`), no una preferencia del usuario; una preferencia no puede
    ///    levantarla.
    pub fn assert_writable(&self, path: &RelPath) -> Result<(), WorkspaceError> {
        self.assert_discoverable(path)?;

        let ws = &self.config().workspace;

        if ws
            .reference_roots
            .iter()
            .any(|root| lodestar_core::types::under_root(path, root))
        {
            return Err(WorkspaceError::PermissionDenied(format!(
                "«{}» cae bajo un referenceRoot (inmutable)",
                path.as_str()
            )));
        }

        if ws.writable_roots.is_empty()
            || ws
                .writable_roots
                .iter()
                .any(|root| lodestar_core::types::under_root(path, root))
        {
            return Ok(());
        }

        Err(WorkspaceError::PermissionDenied(format!(
            "«{}» no cae bajo ningún writableRoot configurado",
            path.as_str()
        )))
    }
}

/// Lee un campo del frontmatter como lista de paths: una secuencia YAML de strings, o un único
/// `String`.
fn field_paths(fm: &lodestar_core::types::ParsedFrontmatter, field: &str) -> Vec<String> {
    match fm.get_key(field) {
        Some(serde_yaml::Value::Sequence(seq)) => seq
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(serde_yaml::Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}
