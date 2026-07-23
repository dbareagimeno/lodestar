//! **Descubrimiento recursivo universal** (E15-H07, `ARCHITECTURE.md §20.5`).
//!
//! Sustituye a `io::load_bundle`: todos los `.md` bajo la raíz, **a cualquier profundidad**, son
//! una sola base de conocimiento. Dos diferencias de fondo con el walker de v0.2.x:
//!
//! 1. Devuelve **dos cosas**: el inventario ([`FileMap`]) y los **diagnósticos** de descubrimiento
//!    ([`Check`] con los códigos de `§20.9`). Lo que antes se tiraba por un `eprintln!` que nadie
//!    podía consultar (no-UTF-8, ruta no representable, symlink) ahora es un diagnóstico.
//! 2. La política es **explícita** ([`DiscoveryPolicy`]), con los valores por defecto de `§20.5`.
//!    Desde E15-H08 se construye desde la sección `discovery` de `.lodestar/config.yaml`
//!    ([`crate::config::DiscoverySection::policy`]), con [`CONTROL_PLANE_EXCLUDE`] como suelo duro.
//!
//! Determinismo: el inventario es un `BTreeMap` (orden por ruta) y el recorrido va ordenado por
//! nombre de fichero, de modo que **mismo árbol ⇒ mismo inventario y mismos diagnósticos, en el
//! mismo orden**, con independencia del orden que devuelva el sistema de ficheros.

use std::collections::BTreeMap;
use std::path::Path;

use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use lodestar_core::types::{Check, CheckCode, FileMap, RelPath, Severity};

use crate::error::WorkspaceError;

/// Nombre del fichero de exclusiones propio de Lodestar (mismo formato que un `.gitignore`).
pub const LODESTAR_IGNORE_FILENAME: &str = ".lodestarignore";

/// El **suelo duro** del descubrimiento: el plano de control de Lodestar (`.lodestar/` entero).
///
/// No es un default sobreescribible sino una exclusión que la config puede **añadir pero nunca
/// quitar** ([`crate::config::DiscoverySection::policy`] la inyecta siempre): sostiene la
/// invariante de consistencia de [`DiscoveryPolicy::exclude`].
pub const CONTROL_PLANE_EXCLUDE: &str = ".lodestar/**";

/// Tamaño máximo por documento **por defecto**: 10 MiB.
///
/// Un `.md` de conocimiento no llega ahí ni de lejos (10 MiB son ~10 millones de caracteres
/// ASCII, dos órdenes de magnitud por encima del documento más grande que se ve en la práctica),
/// así que el límite no recorta trabajo legítimo; existe para que un binario renombrado a `.md` o
/// un volcado accidental no se cargue entero en memoria — y ahora, además, se **reporte**
/// (`DOC-TOO-LARGE`) en vez de desaparecer en silencio.
pub const DEFAULT_MAX_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;

/// Política de descubrimiento (`ARCHITECTURE.md §20.5`). Los campos son públicos a propósito: se
/// construye por actualización funcional (`DiscoveryPolicy { .., ..Default::default() }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryPolicy {
    /// Globs (estilo `.gitignore`) de lo que **entra** en el inventario. Por defecto `**/*.md`.
    pub include: Vec<String>,
    /// Globs de lo que queda **fuera**, con prioridad sobre `include`. Por defecto `.git/**` y
    /// **`.lodestar/**` entero** ([`CONTROL_PLANE_EXCLUDE`]) — no solo `runtime/`. `.lodestar/**`
    /// es además el **suelo duro** que la config no puede levantar (E15-H08).
    ///
    /// La razón no es higiene, es una **invariante de consistencia**: todo documento del inventario
    /// tiene que contar para la [`lodestar_core::types::workspace_revision`], o el control optimista
    /// dejaría de protegerlo en silencio (sería nodo del grafo, analizable y escribible, con
    /// cambios que nunca mueven la revisión). Y `workspace_revision` **no puede** dejar de excluir
    /// `.lodestar/` (decisión D5): `StagingDir` materializa ahí un árbol `.md` completo —copias de
    /// los documentos cuya escritura está guardando— así que si contara, `reverify_base_revision`
    /// fallaría *a causa del apply en curso*; el motor transaccional invalidaría su propia base al
    /// preparar la escritura. Igual con las copias de recuperación.
    ///
    /// Por eso el arreglo va por aquí y no por la revisión. `.lodestar/` es el **plano de control**
    /// de Lodestar (config, cache, runtime), nunca conocimiento del usuario.
    pub exclude: Vec<String>,
    /// Aplicar los `.gitignore` del árbol. Por defecto `true`.
    pub respect_gitignore: bool,
    /// Aplicar los [`LODESTAR_IGNORE_FILENAME`] del árbol. Por defecto `true`.
    pub respect_lodestar_ignore: bool,
    /// Seguir symlinks. Por defecto `false`: un symlink se reporta (`SYMLINK-UNSUPPORTED`) y no
    /// entra en el inventario.
    pub follow_symlinks: bool,
    /// Tamaño máximo por documento en bytes; por encima se reporta `DOC-TOO-LARGE` y el documento
    /// no entra en el inventario. Por defecto [`DEFAULT_MAX_DOCUMENT_BYTES`].
    pub max_document_bytes: usize,
}

impl Default for DiscoveryPolicy {
    fn default() -> Self {
        DiscoveryPolicy {
            include: vec!["**/*.md".to_string()],
            exclude: vec![".git/**".to_string(), CONTROL_PLANE_EXCLUDE.to_string()],
            respect_gitignore: true,
            respect_lodestar_ignore: true,
            follow_symlinks: false,
            max_document_bytes: DEFAULT_MAX_DOCUMENT_BYTES,
        }
    }
}

/// Resultado del descubrimiento: el inventario y los diagnósticos que lo explican.
#[derive(Debug, Clone, Default)]
pub struct Discovered {
    /// Documentos descubiertos (ruta relativa a la raíz → contenido UTF-8).
    pub files: FileMap,
    /// Diagnósticos de descubrimiento (`§20.9`), en orden determinista.
    pub diagnostics: Vec<Check>,
}

/// Descubre el inventario de documentos bajo `root` según `policy`.
///
/// **Nunca aborta por un fichero**: un `.md` no-UTF-8, sobredimensionado, symlink o con ruta no
/// representable produce un diagnóstico y el recorrido continúa — un solo fichero roto no puede
/// dejar muerta la lectura del workspace entero. Los diagnósticos incluyen los de
/// [`case_collisions`] sobre el inventario resultante.
///
/// # Errores
/// - [`WorkspaceError::Io`] si algún glob de `policy` es inválido (desde E15-H08 la política puede
///   venir del `config.yaml` del usuario, así que es alcanzable con un glob mal escrito).
pub fn discover(root: &Path, policy: &DiscoveryPolicy) -> Result<Discovered, WorkspaceError> {
    let mut builder = WalkBuilder::new(root);
    builder
        .overrides(build_overrides(root, policy)?)
        // Un directorio oculto (`.oculto/`) es conocimiento como cualquier otro: solo lo excluyen
        // los globs y los ficheros de ignore.
        .hidden(false)
        .follow_links(policy.follow_symlinks)
        .git_ignore(policy.respect_gitignore)
        // Sin esto, `WalkBuilder` solo aplica `.gitignore` DENTRO de un repo git — y el caso que
        // persigue la épica es justo el directorio arbitrario sin `.git/`.
        .require_git(false)
        // El inventario depende solo del árbol bajo la raíz: ni ficheros de ignore de directorios
        // ancestros, ni el `.gitignore` global del usuario, ni `.git/info/exclude` (no versionado).
        // Así el mismo árbol da el mismo inventario en cualquier máquina.
        .parents(false)
        .git_global(false)
        .git_exclude(false)
        // Recorrido ordenado: hace deterministas también los DIAGNÓSTICOS (el inventario ya lo es
        // por ser un `BTreeMap`).
        .sort_by_file_name(|a, b| a.cmp(b));
    if policy.respect_lodestar_ignore {
        builder.add_custom_ignore_filename(LODESTAR_IGNORE_FILENAME);
    }

    let mut files = FileMap::new();
    let mut diagnostics: Vec<Check> = Vec::new();

    for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                // Entrada ilegible (p. ej. permisos de un directorio). El catálogo de `§20.9` no
                // tiene código para esto y esta historia no inventa códigos: se avisa por stderr y
                // se sigue, como hacía `io::load_bundle`.
                eprintln!("lodestar: aviso: entrada ilegible en el workspace: {e}");
                continue;
            }
        };
        if entry.depth() == 0 {
            continue; // la propia raíz
        }
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(path);
        let file_type = entry.file_type();

        // Symlink: no se sigue (política) pero TAMPOCO se ignora en silencio — el usuario tiene
        // que enterarse de que hay un documento que Lodestar no está viendo.
        if file_type.is_some_and(|t| t.is_symlink()) {
            diagnostics.push(match rel_path_from(rel) {
                Ok(rp) => Check::new(
                    Severity::Warn,
                    CheckCode::SymlinkUnsupported,
                    format!(
                        "«{}» es un enlace simbólico: Lodestar no sigue symlinks, así que el \
                         documento no entra en el inventario",
                        rp.as_str()
                    ),
                    vec![rp],
                ),
                Err(diag) => diag,
            });
            continue;
        }
        if !file_type.is_some_and(|t| t.is_file()) {
            continue; // directorios, FIFOs, sockets…
        }

        let rp = match rel_path_from(rel) {
            Ok(rp) => rp,
            Err(diag) => {
                diagnostics.push(diag);
                continue;
            }
        };

        // Tamaño ANTES de leer (por eso no se usa `WalkBuilder::max_filesize`, que además
        // descartaría el fichero en silencio): un volcado de 5 GB no se carga en memoria para
        // luego rechazarlo.
        let size = entry.metadata().map(|m| m.len()).ok();
        if size.is_some_and(|n| n > policy.max_document_bytes as u64) {
            diagnostics.push(demasiado_grande(&rp, size, policy.max_document_bytes));
            continue;
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "lodestar: aviso: se salta {} (ilegible): {e}",
                    path.display()
                );
                continue;
            }
        };
        // Red de seguridad para cuando `metadata()` no dio tamaño.
        if bytes.len() > policy.max_document_bytes {
            diagnostics.push(demasiado_grande(
                &rp,
                Some(bytes.len() as u64),
                policy.max_document_bytes,
            ));
            continue;
        }
        match String::from_utf8(bytes) {
            Ok(content) => {
                files.insert(rp, content);
            }
            Err(e) => {
                let pos = e.utf8_error().valid_up_to();
                diagnostics.push(Check::new(
                    Severity::Warn,
                    CheckCode::DocNotUtf8,
                    format!(
                        "«{}» no es UTF-8 válido (primer byte inválido en el offset {pos}): el \
                         documento no entra en el inventario",
                        rp.as_str()
                    ),
                    vec![rp],
                ));
            }
        }
    }

    // Colisiones de capitalización: propiedad del inventario COMPLETO, no de un fichero suelto.
    diagnostics.extend(case_collisions(&files));
    diagnostics.sort_by(|a, b| clave_orden(a).cmp(&clave_orden(b)));

    Ok(Discovered { files, diagnostics })
}

/// Diagnósticos de **portabilidad** por rutas que solo difieren en capitalización.
///
/// Dos rutas que pliegan a lo mismo en minúsculas son el **mismo fichero** en un volumen
/// case-insensitive (APFS, NTFS): un workspace que las contiene no es portable. Se pliega la
/// **ruta completa**, no el basename — `docs/auth.md` y `packages/api/docs/auth.md` comparten
/// nombre pero son documentos distintos, y reportarlos sería un falso positivo.
///
/// Se emite **un** diagnóstico por grupo de rutas equivalentes (no uno por fichero), nombrando a
/// todas las implicadas en `targets`.
pub fn case_collisions(files: &FileMap) -> Vec<Check> {
    let mut grupos: BTreeMap<String, Vec<RelPath>> = BTreeMap::new();
    for path in files.keys() {
        grupos
            .entry(path.as_str().to_lowercase())
            .or_default()
            .push(path.clone());
    }
    grupos
        .into_iter()
        .filter(|(_, rutas)| rutas.len() > 1)
        .map(|(plegada, rutas)| {
            let listado: Vec<&str> = rutas.iter().map(|p| p.as_str()).collect();
            Check::new(
                Severity::Warn,
                CheckCode::LinkCaseMismatch,
                format!(
                    "{} rutas del inventario difieren solo en capitalización y colisionan en \
                     sistemas de ficheros case-insensitive (pliegan a «{plegada}»): {}",
                    rutas.len(),
                    listado.join(", ")
                ),
                rutas,
            )
        })
        .collect()
}

/// Convierte una ruta **relativa a la raíz** del sistema de ficheros en un [`RelPath`].
///
/// Normaliza el separador nativo a `/`: en Windows el walker entrega
/// `three\levels\deep\third.md` y [`RelPath::new`] rechaza los backslashes (invariante #6), así
/// que sin esta normalización el descubrimiento entero se caería ahí.
///
/// # Errores
/// Devuelve un [`Check`] `PATH-NOT-UTF8` cuando la ruta no es representable (bytes no UTF-8 en
/// Unix, surrogate suelto en Windows) o cuando no es una ruta relativa válida del workspace. En
/// ese `Check` **`targets` queda vacío**: no hay `RelPath` que construir —ese *es* el problema— y
/// colar el path crudo violaría el invariante #6. El `msg` lleva la representación lossy, que es
/// lo único que permite al usuario localizar el fichero.
// `Check` es grande (136 B) y clippy sugiere boxearlo, pero el error de esta función ES un
// diagnóstico del catálogo `§20.9` que el llamador empuja tal cual a `Discovered::diagnostics`:
// boxearlo solo añadiría una indirección y un `*` en cada uso, en un camino que además es frío
// (una ruta no representable por workspace, no por documento).
#[allow(clippy::result_large_err)]
pub fn rel_path_from(rel: &Path) -> Result<RelPath, Check> {
    let lossy = rel.to_string_lossy().replace('\\', "/");
    let no_representable = |motivo: String| {
        Check::new(
            Severity::Warn,
            CheckCode::PathNotUtf8,
            format!("«{lossy}» {motivo}: el documento no entra en el inventario"),
            Vec::new(),
        )
    };
    let Some(texto) = rel.to_str() else {
        return Err(no_representable(
            "tiene una ruta no representable como UTF-8".to_string(),
        ));
    };
    RelPath::new(&texto.replace('\\', "/")).map_err(|e| {
        no_representable(format!(
            "no es una ruta relativa válida del workspace ({e})"
        ))
    })
}

/// El `Check` de `DOC-TOO-LARGE` para `rp`, con el tamaño observado si se conoce.
fn demasiado_grande(rp: &RelPath, size: Option<u64>, limite: usize) -> Check {
    let observado = match size {
        Some(n) => format!("{n} bytes"),
        None => "tamaño desconocido".to_string(),
    };
    Check::new(
        Severity::Warn,
        CheckCode::DocTooLarge,
        format!(
            "«{}» supera el tamaño máximo por documento ({observado} > {limite}): el documento no \
             entra en el inventario",
            rp.as_str()
        ),
        vec![rp.clone()],
    )
}

/// Clave de orden total de un diagnóstico (código, rutas, mensaje) — hace determinista la salida
/// aunque el recorrido del sistema de ficheros no lo sea.
fn clave_orden(c: &Check) -> (&'static str, Vec<&str>, &str) {
    (
        c.code.as_str(),
        c.targets.iter().map(|t| t.as_str()).collect(),
        c.msg.as_str(),
    )
}

/// Traduce `include`/`exclude` al `Override` de `ignore`, que aplica semántica `.gitignore` a los
/// globs **durante** el recorrido (y por tanto poda directorios en vez de filtrar a posteriori).
///
/// - Los `include` entran tal cual (whitelist): lo que no case con ninguno queda fuera.
/// - Los `exclude` entran negados y **después**, para que ganen (última regla que casa manda).
/// - Cada `exclude` con forma `pre/**` añade además `pre` para **podar el directorio** entero: en
///   semántica `.gitignore`, `.git/**` casa con lo que hay dentro de `.git` pero no con `.git`, y
///   sin la poda se recorrería el repo git completo para tirar cada entrada una a una.
fn build_overrides(
    root: &Path,
    policy: &DiscoveryPolicy,
) -> Result<ignore::overrides::Override, WorkspaceError> {
    let mut builder = OverrideBuilder::new(root);
    let glob_err = |g: &str, e: ignore::Error| {
        WorkspaceError::Io(format!(
            "glob inválido en la política de descubrimiento «{g}»: {e}"
        ))
    };
    for glob in &policy.include {
        builder.add(glob).map_err(|e| glob_err(glob, e))?;
    }
    for glob in &policy.exclude {
        if let Some(dir) = glob.strip_suffix("/**") {
            let podado = format!("!{dir}");
            builder.add(&podado).map_err(|e| glob_err(glob, e))?;
        }
        let negado = format!("!{glob}");
        builder.add(&negado).map_err(|e| glob_err(glob, e))?;
    }
    builder
        .build()
        .map_err(|e| WorkspaceError::Io(format!("política de descubrimiento inválida: {e}")))
}
