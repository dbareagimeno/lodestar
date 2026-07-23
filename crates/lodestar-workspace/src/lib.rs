//! `lodestar-workspace` — el handle unificado (`ARCHITECTURE.md §6`).
//!
//! Compone `lodestar-core` (puro) + `lodestar-store`. Es lo que ven las fachadas. Es el **único
//! escritor**: los comandos nunca escriben la cache; escriben el `.md` (atómico temp+rename).
//!
//! Nota de fase: la cache incremental (`lodestar-store`: SQLite/FTS5 + watcher, E3) es la capa de
//! aceleración. Mientras no esté cableada, la workspace **recarga desde disco** bajo demanda — el
//! core es la autoridad y la cache es derivada/desechable (`§2.3`, `§10` fila 1), así que el resultado
//! es correcto, solo que no incremental.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::Receiver;
use lodestar_core::types::{
    Analysis, Backlinks, Direction, FileMap, FrontmatterPatch, GraphModel, Neighborhood, RelPath,
    WorkspaceRevision, WriteOutcome,
};
use lodestar_core::Bundle;
use lodestar_store::{IndexEvent, Store, Watcher};

use crate::discovery::DiscoveryPolicy;

pub mod config;
pub mod discovery;
mod error;
mod external_refs;
mod gitignore;
mod io;
mod journal;
mod lock;
mod publish;
mod receipts;
mod recovery;
mod runtime;
pub mod schema;
mod snapshot;
mod staging;
mod transaction;

pub use config::WorkspaceConfig;
pub use error::WorkspaceError;
pub use external_refs::{ExternalReference, ExternalRefsReport};
pub use journal::{Journal, JournalState, OpState};
pub use lock::WorkspaceLock;
pub use recovery::RecoveryDir;
pub use schema::WorkspaceSchema;
pub use snapshot::BundleSnapshot;
pub use staging::StagingDir;
pub use transaction::transaction_id;

/// Handle unificado de un bundle abierto.
pub struct Workspace {
    root: PathBuf,
    /// Configuración de la sesión (`.lodestar/config.yaml`), leída **una sola vez** al abrir.
    config: WorkspaceConfig,
    /// Cache incremental (SQLite/FTS5). `None` en modo efímero (CLI one-shot).
    cache: Option<Arc<Store>>,
    /// Watcher vivo (mantiene la observación de disco mientras exista).
    _watcher: Option<Watcher>,
}

impl Workspace {
    /// Abre un workspace sobre un directorio cualquiera. **No** activa la cache incremental
    /// (usa [`Workspace::open_live`] o [`Workspace::enable_cache`]).
    ///
    /// Abrir no exige ceremonia: no hay descubrimiento de repo, ni `init`, ni scaffold obligatorio
    /// (`ARCHITECTURE.md §20.1`). Un directorio con `.md` sueltos —y **sin** `.lodestar/`— es un
    /// workspace válido: la config es opcional y su ausencia da los defaults de `§20.5`.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si `.lodestar/config.yaml` existe pero es inválido. Se comprueba
    ///   **antes** de tocar nada del disco: desde que la config gobierna el descubrimiento
    ///   (E15-H08), degradar a defaults ante un typo haría que Lodestar viera un conjunto de
    ///   documentos distinto del que el usuario declaró sin decir una palabra.
    pub fn open(root: &Path) -> Result<Self, WorkspaceError> {
        // La config se lee UNA vez por sesión: la raíz y su política son fijas mientras el
        // workspace vive (`ARCHITECTURE.md §20.5`).
        let config = WorkspaceConfig::load(root).map_err(WorkspaceError::Io)?;
        // Ajusta el `.gitignore` versionado (cache + runtime desechables, config canónica
        // preservada) y garantiza el scaffold de `.lodestar/runtime/` — ambos best-effort, no
        // abortan la apertura (`ARCHITECTURE.md §19.4`, `DECISIONES.md §0` D5). Ninguno de los dos
        // escribe configuración: un workspace sin `.lodestar/config.yaml` sigue sin tenerlo tras
        // abrirlo.
        gitignore::ensure_gitignore(root);
        runtime::ensure_runtime_scaffold(root);
        Ok(Workspace {
            root: root.to_path_buf(),
            config,
            cache: None,
            _watcher: None,
        })
    }

    /// La configuración de la sesión (`.lodestar/config.yaml` + defaults seguros), leída una sola
    /// vez al abrir. El `lodestar.toml` legado desapareció en E15-H08: hoy es un fichero más del
    /// proyecto.
    pub fn config(&self) -> &WorkspaceConfig {
        &self.config
    }

    /// La [`DiscoveryPolicy`] efectiva del workspace (`ARCHITECTURE.md §20.5`).
    ///
    /// **Punto de inyección único** de la política: se deriva de la sección `discovery` de la
    /// config de la sesión ([`config::DiscoverySection::policy`]), que le añade el suelo duro
    /// `.lodestar/**` — la config puede añadir exclusiones, nunca quitar esa.
    ///
    /// La firma es infalible a propósito: la validación del YAML ocurre **una vez**, al abrir
    /// ([`Workspace::open`]), de modo que un workspace abierto siempre tiene una política válida y
    /// ninguno de sus llamadores tiene que decidir qué hacer con un error de config a mitad de una
    /// lectura.
    pub fn discovery_policy(&self) -> DiscoveryPolicy {
        self.config.discovery.policy()
    }

    /// El inventario `.md` del workspace según [`Workspace::discovery_policy`] (E15-H07).
    ///
    /// Es el **único** camino de lectura del conocimiento canónico desde disco: sustituye al
    /// `io::load_bundle` de v0.2.x en todos sus llamadores, de modo que el bundle, la
    /// [`WorkspaceRevision`] y el motor transaccional vean exactamente el mismo conjunto de
    /// documentos (si divergieran, el control optimista protegería ficheros que el plan ni
    /// siquiera ve).
    ///
    /// Los diagnósticos de descubrimiento se descartan aquí a propósito: el conjunto de llamadores
    /// solo necesita el inventario, y exponerlos a las fachadas es parte de la validación genérica
    /// de E20 (`§20.9`). Quien los necesite hoy llama a [`discovery::discover`] directamente.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si la política trae un glob inválido.
    pub(crate) fn discover_files(&self) -> Result<FileMap, WorkspaceError> {
        Ok(discovery::discover(&self.root, &self.discovery_policy())?.files)
    }

    /// El directorio raíz del bundle abierto (E10-H08: lo expone `App::workspace_status` como
    /// `root` de la proyección de estado).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Computa la [`WorkspaceRevision`] actual del conocimiento **escribible** (E13-H02, E10-H03).
    ///
    /// Carga el `FileMap` canónico desde disco y aplica la única lógica del core
    /// ([`lodestar_core::types::workspace_revision`]) con los `writableRoots` de la config: el
    /// hash blake3 cubre solo los `.md` que Lodestar puede reescribir (ignora `.lodestar/` y, si
    /// hay `writableRoots`, cualquier `.md` fuera de ellos). Es la `baseWorkspaceRevision` que un
    /// plan captura al planificar y que [`Workspace::reverify_base_revision`] re-comprueba al
    /// aplicar.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si falla la lectura del canónico.
    pub fn workspace_revision(&self) -> Result<WorkspaceRevision, WorkspaceError> {
        let files = self.discover_files()?;
        Ok(lodestar_core::types::workspace_revision(
            &files,
            &self.config.workspace.writable_roots,
        ))
    }

    /// Re-verifica el control optimista de escritura (E13-H02): comprueba que la
    /// [`WorkspaceRevision`] actual del conocimiento escribible sigue siendo la `base` que el plan
    /// capturó. Si coincide, `Ok(())`; si el workspace cambió entre plan y apply (otro escritor
    /// tocó los `.md`), devuelve [`WorkspaceError::WriteConflict`] y **no se publica** — publicar
    /// sobre una base obsoleta pisaría ese cambio.
    ///
    /// # Errores
    /// - [`WorkspaceError::WriteConflict`] si la revisión actual difiere de `base`.
    /// - [`WorkspaceError::Io`] si falla el cálculo de la revisión actual.
    pub fn reverify_base_revision(&self, base: &WorkspaceRevision) -> Result<(), WorkspaceError> {
        let actual = self.workspace_revision()?;
        if &actual == base {
            Ok(())
        } else {
            Err(WorkspaceError::WriteConflict(format!(
                "la base del plan ({}) ya no es la revisión actual del workspace ({}): \
                 otro escritor lo modificó entre el plan y el apply",
                base.0, actual.0
            )))
        }
    }

    /// Abre en modo hermético (p. ej. CLI efímera): sin tocar `.gitignore` ni el scaffold de
    /// runtime, y sin cache incremental.
    ///
    /// «Hermético» se refiere a **no escribir** en el workspace, no a saltarse la validación: la
    /// config se carga y se valida igual que en [`Workspace::open`]. Si no lo hiciera, esta vía
    /// serviría un inventario calculado con una política de defaults que el usuario nunca escribió
    /// — justo el fallo silencioso que la validación existe para evitar.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si `.lodestar/config.yaml` existe pero es inválido.
    pub fn open_ephemeral(root: &Path) -> Result<Self, WorkspaceError> {
        let config = WorkspaceConfig::load(root).map_err(WorkspaceError::Io)?;
        Ok(Workspace {
            root: root.to_path_buf(),
            config,
            cache: None,
            _watcher: None,
        })
    }

    /// Abre un workspace **en vivo**: cache incremental construida + watcher arrancado.
    /// Es lo que usan las fachadas interactivas (MCP) para recibir `IndexEvent`.
    pub fn open_live(root: &Path) -> Result<Self, WorkspaceError> {
        let mut ws = Workspace::open(root)?;
        ws.enable_cache()?;
        Ok(ws)
    }

    /// Activa (si no lo está) la cache incremental: abre `.lodestar/index.db`, la reconstruye
    /// desde disco y arranca el watcher (único escritor de la cache).
    pub fn enable_cache(&mut self) -> Result<(), WorkspaceError> {
        if self.cache.is_some() {
            return Ok(());
        }
        // El `.gitignore` ya quedó ajustado en `Workspace::open` (cache + runtime ignorados,
        // ANTES de crear la cache) — ver `gitignore::ensure_gitignore`: texto plano, sin git.
        let store = Arc::new(Store::open(&self.root)?);
        // Watcher ANTES del rebuild: un guardado externo durante el rebuild inicial genera
        // evento y se reconcilia; al revés quedaba una ventana ciega hasta el siguiente evento.
        let watcher = store.watch()?;
        store.rebuild()?;
        self.cache = Some(store);
        self._watcher = Some(watcher);
        Ok(())
    }

    /// Suscribe un receptor de [`IndexEvent`] del bus de la cache. Error si la cache no está activa.
    pub fn subscribe(&self) -> Result<Receiver<IndexEvent>, WorkspaceError> {
        self.cache
            .as_ref()
            .map(|s| s.subscribe())
            .ok_or(WorkspaceError::NoCache)
    }

    /// Acceso a la cache incremental (para consultas aceleradas: backlinks/orphans/FTS).
    pub fn cache(&self) -> Option<&Arc<Store>> {
        self.cache.as_ref()
    }

    /// Update **optimista** de la cache tras una escritura por el único escritor (`§10` fila 19):
    /// la UI ve el cambio al instante; el watcher reconcilia después (no-op por el gate de hash).
    fn cache_upsert(&self, path: &RelPath, content: &str) {
        if let Some(store) = &self.cache {
            let _ = store.upsert(path, content, 0, content.len() as i64);
        }
    }

    // NOTA (E15-H02): el `cache_remove` simétrico se retiró con `apply_mutation` — su único
    // llamador. Hoy ninguna escritura de alto nivel borra `.md` del canónico fuera de la
    // transacción, y el borrado transaccional va por `publish_result`, que reconcilia la cache por
    // el watcher. Si vuelve a hacer falta, es `store.remove(path)` con el mismo patrón de arriba.

    // --- lectura ----------------------------------------------------------

    /// Carga el bundle desde disco (el core es la autoridad).
    pub fn bundle(&self) -> Result<Bundle, WorkspaceError> {
        Ok(Bundle::from_files(self.discover_files()?))
    }

    /// Snapshot unificado: files + analysis + graph, todo junto.
    pub fn snapshot(&self) -> Result<BundleSnapshot, WorkspaceError> {
        let bundle = self.bundle()?;
        Ok(BundleSnapshot {
            files: bundle.files().clone(),
            analysis: bundle.analyze().clone(),
            graph: bundle.graph_model(),
        })
    }

    /// Análisis (conformidad/grafo derivados).
    pub fn analyze(&self) -> Result<Analysis, WorkspaceError> {
        Ok(self.bundle()?.analyze().clone())
    }

    /// Vecindad de enlaces de un concept.
    pub fn backlinks(&self, p: &RelPath) -> Result<Backlinks, WorkspaceError> {
        Ok(self.bundle()?.backlinks(p))
    }

    /// Subgrafo dirigido alrededor de un concept.
    pub fn neighborhood(
        &self,
        p: &RelPath,
        depth: u32,
        dir: Direction,
    ) -> Result<Neighborhood, WorkspaceError> {
        Ok(self.bundle()?.neighborhood(p, depth, dir))
    }

    /// Grafo completo.
    pub fn graph_model(&self) -> Result<GraphModel, WorkspaceError> {
        Ok(self.bundle()?.graph_model())
    }

    /// Query estructurada (devuelve paths).
    pub fn query(&self, dsl: &str) -> Result<Vec<RelPath>, WorkspaceError> {
        Ok(self.bundle()?.query(dsl))
    }

    // --- escritura validada (por el ÚNICO escritor) -----------------------

    /// Rechaza una escritura del canónico si hay una recuperación PENDIENTE (E13-H06): un
    /// write-ahead journal no-`done` bajo `.lodestar/runtime/journal/` significa que una
    /// transacción anterior se interrumpió a mitad y [`Workspace::recover`] aún no la
    /// completó/restauró. El gate se comprueba ANTES de tocar el canónico —para no publicar sobre
    /// un estado a medio recuperar (principio «nunca un estado parcial silencioso»)— en toda
    /// escritura de alto nivel del canónico ([`Workspace::create_concept`],
    /// [`Workspace::write_concept`], [`Workspace::merge_frontmatter`]) y en [`Workspace::publish`]
    /// (que excluye su propio journal en curso). La restauración de `recover` NO pasa por aquí: escribe por `io::write_atomic`/
    /// `io::delete` directamente, de modo que puede reparar el canónico con el gate levantado.
    fn guard_recovery(&self) -> Result<(), WorkspaceError> {
        if self.recovery_pending() {
            return Err(WorkspaceError::WorkspaceRecoveryRequired(
                "hay un journal de publicación sin completar bajo .lodestar/runtime/journal/: \
                 ejecuta la recuperación (Workspace::recover) antes de volver a escribir"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Crea un concept validado y lo escribe por el único escritor (si es conforme).
    pub fn create_concept(
        &self,
        p: &RelPath,
        ty: &str,
        title: Option<&str>,
        body: &str,
        allow_nonconformant: bool,
    ) -> Result<WriteOutcome, WorkspaceError> {
        self.guard_recovery()?;
        let bundle = self.bundle()?;
        let now = now_iso8601();
        let outcome = bundle.create_concept(p, ty, title, body, Some(&now), allow_nonconformant);
        if outcome.written {
            io::write_atomic(&self.root, &outcome.path, &outcome.raw)?;
            self.cache_upsert(&outcome.path, &outcome.raw);
        }
        Ok(outcome)
    }

    /// Escribe contenido **crudo** en un concept (editor multi-escritor), validado por el core.
    /// Rechazo = `written:false` (no un `Err`). Escribe por el único escritor si es conforme.
    pub fn write_concept(
        &self,
        p: &RelPath,
        raw: &str,
        allow_nonconformant: bool,
    ) -> Result<WriteOutcome, WorkspaceError> {
        self.guard_recovery()?;
        let bundle = self.bundle()?;
        let outcome = bundle.write_concept_raw(p, raw, allow_nonconformant);
        if outcome.written {
            io::write_atomic(&self.root, &outcome.path, &outcome.raw)?;
            self.cache_upsert(&outcome.path, &outcome.raw);
        }
        Ok(outcome)
    }

    /// Lee el contenido crudo de un concept desde disco.
    pub fn read_concept(&self, p: &RelPath) -> Result<String, WorkspaceError> {
        std::fs::read_to_string(self.root.join(p.as_str()))
            .map_err(|e| WorkspaceError::Io(e.to_string()))
    }

    /// Lista las filas del árbol de concepts (título/orphan/invalid resueltos por el core).
    pub fn list_concepts(
        &self,
    ) -> Result<Vec<lodestar_core::types::ConceptSummary>, WorkspaceError> {
        Ok(self.bundle()?.list_concepts())
    }

    /// Aplica un patch de frontmatter (null-borra) y lo escribe si es conforme.
    pub fn merge_frontmatter(
        &self,
        p: &RelPath,
        patch: FrontmatterPatch,
    ) -> Result<WriteOutcome, WorkspaceError> {
        self.guard_recovery()?;
        let bundle = self.bundle()?;
        let outcome = bundle.merge_frontmatter(p, patch);
        if outcome.written {
            io::write_atomic(&self.root, &outcome.path, &outcome.raw)?;
            self.cache_upsert(&outcome.path, &outcome.raw);
        }
        Ok(outcome)
    }
}

/// Instante actual UTC en ISO-8601 con precisión de segundos: `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Paridad con el prototipo, que escribe `new Date().toISOString().replace(/\.\d+Z$/,"Z")`
/// (truncando los milisegundos). El core es puro y no toca el reloj; la workspace —único escritor
/// con I/O— computa el instante y lo inyecta en `create_concept`. Se formatea a mano (algoritmo
/// civil-desde-días de Howard Hinnant) para no arrastrar una dependencia de fecha/hora.
fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);
    // civil_from_days (Hinnant): días desde 1970-01-01 → (año, mes, día) proleptic gregoriano.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

#[cfg(test)]
mod time_tests {
    use super::now_iso8601;

    #[test]
    fn now_iso8601_tiene_formato_y_es_iso_para_el_core() {
        let s = now_iso8601();
        // Forma exacta: `YYYY-MM-DDTHH:MM:SSZ` (20 caracteres, sin milisegundos).
        assert_eq!(s.len(), 20, "formato inesperado: {s}");
        assert!(
            s.ends_with('Z') && s.as_bytes()[10] == b'T',
            "formato inesperado: {s}"
        );
        // El core debe aceptarlo como ISO (si no, FMT-TS marcaría warn en toda página creada).
        let v = serde_yaml::Value::String(s.clone());
        assert!(
            lodestar_core::model::is_iso(&v),
            "el core no reconoce como ISO: {s}"
        );
    }
}
