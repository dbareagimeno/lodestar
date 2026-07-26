//! Paths externos (`referenceRoots`, `ARCHITECTURE.md §19.4/§19.7`, E9-H05).
//!
//! Una sola responsabilidad, anclada en `referenceRoots` del `.lodestar/config.yaml`:
//! [`Workspace::assert_writable`], la **write policy** del único escritor, que usa `referenceRoots`
//! como raíces «visibles pero NUNCA escribibles» (inmutables). La misma contención excluye esas
//! raíces de [`lodestar_core::types::workspace_revision`].
//!
//! # Lo que ESTUVO aquí y ya no (E23-H12)
//!
//! Este módulo tuvo una segunda responsabilidad: `Workspace::external_refs`, que resolvía contra
//! disco los campos de frontmatter `implemented_by`/`verified_by` (paths a ficheros de **código**)
//! y devolvía `{path, exists}` por cada uno, para `knowledge_get(externalReferences)`. Se **retiró
//! sin sustituto**: eran las últimas claves de frontmatter con **semántica impuesta y no
//! configurable**, contra el invariante 3 de `ARCHITECTURE.md §20.2` (ningún nombre de campo activa
//! reglas especiales, igual que E16-H02 hizo con los nombres de fichero). Hoy `implemented_by` es
//! metadata del usuario como `autor` o `tags`. Con ella cayó la opción `include:["externalReferences"]`
//! de `knowledge_get`, que se quedaba sin fuente.
//!
//! **Ruta de migración**: apuntar a código NO desaparece — un enlace Markdown a un fichero de
//! código se resuelve y se clasifica como [`lodestar_core::types::LinkTarget::WorkspaceFile`]
//! (`§20.6`), con su diagnóstico de destino ausente. Lo que desaparece es que un **nombre de campo**
//! active esa resolución.
//!
//! (Antes, en E20-H03, ya había caído el diagnóstico `EXTREF-MISSING` con el resto de la maquinaria
//! schema, `§20.10`.)

use lodestar_core::types::RelPath;

use crate::error::WorkspaceError;
use crate::Workspace;

impl Workspace {
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
