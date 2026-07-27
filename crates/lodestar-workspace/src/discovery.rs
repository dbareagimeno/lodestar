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

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use ignore::gitignore::GitignoreBuilder;
use ignore::overrides::{Override, OverrideBuilder};
use ignore::{Match, WalkBuilder};
use lodestar_core::types::{Check, CheckCode, FileMap, RelPath, Severity};

use crate::error::WorkspaceError;
use crate::Workspace;

/// Nombre del fichero de exclusiones propio de Lodestar (mismo formato que un `.gitignore`).
pub const LODESTAR_IGNORE_FILENAME: &str = ".lodestarignore";

/// Nombre del fichero de exclusiones estándar del árbol que el walker respeta cuando
/// [`DiscoveryPolicy::respect_gitignore`] está activo.
pub const GITIGNORE_FILENAME: &str = ".gitignore";

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
    ///
    /// Es un **filtro final sobre los ficheros supervivientes**, no una lista blanca que mande
    /// sobre el resto de la política: lo que `exclude`, `.gitignore` o `.lodestarignore` hayan
    /// dejado fuera sigue fuera aunque case con un `include` (ver [`discover`], «orden de
    /// precedencia»). Un `include` vacío no incluye nada.
    ///
    /// No se aplica a **directorios**: un directorio no entra nunca en el inventario, y podarlo
    /// por `include` cortaría el descenso a los documentos que sí casan (`**/*.md` no casa con
    /// `docs/`, pero sí con `docs/api.md`).
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
    /// Los demás ficheros **que el walker visita**: todo lo que existe bajo la raíz y no acabó en
    /// [`Discovered::files`] — código, imágenes, los `.md` que no pasan `include`, y también los
    /// que quedaron fuera del inventario por symlink, tamaño o codificación.
    ///
    /// Es lo que permite a [`lodestar_core::links::resolve`] clasificar un enlace a un fichero del
    /// proyecto como [`lodestar_core::types::LinkTarget::WorkspaceFile`] («existe, pero no es nodo
    /// del grafo») en vez de como `Missing` (`ARCHITECTURE.md §20.6`, precisión 2). Va en el propio
    /// resultado del descubrimiento —y no en una función aparte— porque el walker **ya visita**
    /// estas entradas: recolectarlas cuesta un `insert` por entrada y **cero I/O extra** (no se lee
    /// su contenido), mientras que una segunda pasada pagaría otro recorrido completo del árbol.
    ///
    /// No contiene **directorios** (no son ficheros y `LinkTarget` no los modela: un enlace a
    /// `guias/` sigue siendo `Missing("guias")`, ver `§20.6` precisión 2b) ni nada **podado** por
    /// `exclude`/`.gitignore`/`.lodestarignore`, que por definición no se visita — el límite
    /// conocido y aceptado de `§20.6`.
    pub other_files: BTreeSet<RelPath>,
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
/// Todo lo que el walker **visita** y no acaba en [`Discovered::files`] —salvo los directorios—
/// acaba en [`Discovered::other_files`]: es el inventario de «existe, pero no es un documento» que
/// necesita la clasificación de enlaces de `§20.6`.
///
/// # Orden de precedencia
///
/// De mayor a menor, y **en este orden**:
///
/// 1. [`DiscoveryPolicy::exclude`] — política explícita del usuario, va en el `Override` del
///    walker, que en `ignore` tiene la precedencia más alta y **cortocircuita** el resto.
/// 2. `.gitignore` / `.lodestarignore` del árbol.
/// 3. [`DiscoveryPolicy::include`] — **filtro final** sobre lo que sobrevivió a 1 y 2.
///
/// Que `include` vaya el último no es un detalle de implementación: es la diferencia entre que un
/// `.gitignore` con `secreto.md` funcione o no. En `ignore`, **cualquier** match del `Override`
/// —whitelist o ignore— corta y decide (`dir.rs`: *«Overrides have the highest precedence»*), así
/// que meter `include` ahí como lista blanca haría que todo `.md` quedase whitelisteado **antes**
/// de que se consultara ningún fichero de ignore, y los patrones de fichero de `.gitignore`
/// dejarían de aplicarse por completo (los de directorio se salvarían de rebote, porque el
/// `Override` no aplica whitelist a directorios). Por eso el `Override` se reserva para `exclude`
/// y el `include` se evalúa aquí, contra el fichero ya superviviente.
///
/// # Errores
/// - [`WorkspaceError::Io`] si algún glob de `policy` es inválido (desde E15-H08 la política puede
///   venir del `config.yaml` del usuario, así que es alcanzable con un glob mal escrito).
pub fn discover(root: &Path, policy: &DiscoveryPolicy) -> Result<Discovered, WorkspaceError> {
    let include = build_include(root, policy)?;
    let mut builder = WalkBuilder::new(root);
    builder
        .overrides(build_excludes(root, policy)?)
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
    let mut other_files: BTreeSet<RelPath> = BTreeSet::new();
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

        // Un directorio nunca entra en el inventario NI en `other_files` (`LinkTarget` no modela
        // directorios, `§20.6` precisión 2b), y tampoco se le aplica el filtro `include`: podarlo
        // aquí cortaría el descenso a los documentos que sí casan.
        if file_type.is_some_and(|t| t.is_dir()) {
            continue;
        }

        // Filtro `include`, el ÚLTIMO de la cadena de precedencia (ver la doc de esta función).
        // Se aplica a todo no-directorio, symlinks incluidos: un `enlace.txt` no es un documento
        // que Lodestar «no esté viendo», así que no merece diagnóstico. Lo que no pasa el filtro no
        // desaparece: es un fichero del proyecto que EXISTE, así que va a `other_files` para que un
        // enlace a él sea `WorkspaceFile` y no `Missing`.
        if !incluido(&include, path) {
            // Una ruta no representable se salta **en silencio** aquí: `PATH-NOT-UTF8` denuncia un
            // documento que Lodestar no puede ver, y esto no es un documento (igual que el filtro
            // `include` no emite diagnóstico).
            if let Ok(rp) = rel_path_from(rel) {
                other_files.insert(rp);
            }
            continue;
        }

        // Symlink: no se sigue (política) pero TAMPOCO se ignora en silencio — el usuario tiene
        // que enterarse de que hay un documento que Lodestar no está viendo.
        if file_type.is_some_and(|t| t.is_symlink()) {
            diagnostics.push(match rel_path_from(rel) {
                Ok(rp) => {
                    let diag = Check::new(
                        Severity::Warn,
                        CheckCode::SymlinkUnsupported,
                        format!(
                            "«{}» es un enlace simbólico: Lodestar no sigue symlinks, así que el \
                             documento no entra en el inventario",
                            rp.as_str()
                        ),
                        vec![rp.clone()],
                    );
                    // No es documento, pero la ruta existe: un enlace a ella no «falta».
                    other_files.insert(rp);
                    diag
                }
                Err(diag) => diag,
            });
            continue;
        }
        if !file_type.is_some_and(|t| t.is_file()) {
            // FIFOs, sockets…: no son documentos, pero existen.
            if let Ok(rp) = rel_path_from(rel) {
                other_files.insert(rp);
            }
            continue;
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
            // Fuera del inventario, pero en disco: `WorkspaceFile`, no `Missing`.
            other_files.insert(rp);
            continue;
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "lodestar: aviso: se salta {} (ilegible): {e}",
                    path.display()
                );
                other_files.insert(rp);
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
            other_files.insert(rp);
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
                    vec![rp.clone()],
                ));
                other_files.insert(rp);
            }
        }
    }

    // Colisiones de capitalización: propiedad del inventario COMPLETO, no de un fichero suelto.
    diagnostics.extend(case_collisions(&files));
    diagnostics.sort_by(|a, b| clave_orden(a).cmp(&clave_orden(b)));

    Ok(Discovered {
        files,
        other_files,
        diagnostics,
    })
}

impl Workspace {
    /// Guard de **descubrimiento** (E15-H09, `REFACTOR_PHASE_2 §Principio 8`): `Err` si escribir en
    /// `path` produciría un documento que el inventario **no vería**.
    ///
    /// Es el complemento de [`Workspace::assert_writable`], y responde a una pregunta distinta: no
    /// «¿tengo permiso para escribir aquí?» (raíces de la config) sino «¿existiría de verdad lo que
    /// escriba aquí?». La diferencia importa porque un `.md` fuera del inventario queda **fuera de
    /// la [`lodestar_core::types::workspace_revision`]**: invisible al grafo y a la búsqueda, sin
    /// protección del control optimista (un segundo `create` en el mismo path no vería colisión y lo
    /// sobrescribiría) y un `change_revert` lo trataría como creado y lo borraría.
    ///
    /// Se consulta la política **efectiva** ([`Workspace::discovery_policy`]) y el estado **actual**
    /// del árbol, sin cachear: el descubrimiento no es config de sesión —un `.gitignore` puede
    /// aparecer entre el plan y el apply sin mover la revisión (no es un `.md`), de modo que ni el
    /// control optimista ni el `planHash` lo detectan—. Por eso el guard tiene que volver a
    /// preguntar en el momento de escribir.
    ///
    /// # Errores
    /// - [`WorkspaceError::PermissionDenied`] con el motivo de la exclusión (glob de
    ///   `discovery.exclude`, patrón de un `.gitignore`/`.lodestarignore` del árbol, o el filtro
    ///   `discovery.include`).
    /// - [`WorkspaceError::Io`] si la política trae un glob inválido (mismo criterio que
    ///   [`discover`]).
    pub fn assert_discoverable(&self, path: &RelPath) -> Result<(), WorkspaceError> {
        match exclusion_reason(self.root(), path, &self.discovery_policy())? {
            None => Ok(()),
            Some(motivo) => Err(WorkspaceError::PermissionDenied(format!(
                "«{}» queda fuera del inventario del workspace ({motivo}): escribir ahí dejaría un \
                 documento invisible al grafo y ciego al control optimista",
                path.as_str()
            ))),
        }
    }
}

/// ¿Por qué quedaría `path` **fuera del inventario**? `None` si el descubrimiento lo vería.
///
/// Es la versión «una ruta, sin recorrer el árbol» de [`discover`]: responde por un path que puede
/// **no existir todavía** (el destino de un `create`/`move`), donde el walker no sirve. Respeta el
/// mismo **orden de precedencia** que [`discover`] —`exclude` → ficheros de ignore del árbol →
/// `include` como filtro final—, así que un `Ok(None)` aquí significa que ese mismo path, una vez
/// escrito, aparecerá en el inventario que devuelve `discover`.
///
/// Lo que **no** puede responder son las exclusiones que dependen del contenido del fichero ya
/// escrito ([`DiscoveryPolicy::max_document_bytes`], UTF-8) ni las de symlink: no son política de
/// ubicación sino propiedades del documento, y Lodestar solo escribe ficheros regulares UTF-8 por su
/// único escritor, así que no puede producirlas.
///
/// # Errores
/// - [`WorkspaceError::Io`] si algún glob de `policy` es inválido (igual que [`discover`]).
pub fn exclusion_reason(
    root: &Path,
    path: &RelPath,
    policy: &DiscoveryPolicy,
) -> Result<Option<String>, WorkspaceError> {
    // (1) `exclude` explícito: la máxima precedencia, como en el walker.
    let excludes = build_excludes(root, policy)?;
    if excluido_por_override(&excludes, path) {
        let motivo = match glob_culpable(root, path, policy) {
            Some(glob) => format!("lo excluye el glob «{glob}» de `discovery.exclude`"),
            None => "lo excluye `discovery.exclude`".to_string(),
        };
        return Ok(Some(motivo));
    }

    // (2) Ficheros de ignore del árbol (`.lodestarignore`/`.gitignore`), del directorio más
    //     profundo hacia la raíz.
    if let Some(motivo) = excluido_por_ficheros_de_ignore(root, path, policy) {
        return Ok(Some(motivo));
    }

    // (3) `include`, el filtro FINAL sobre lo que sobrevivió a (1) y (2).
    let include = build_include(root, policy)?;
    if !incluido(&include, Path::new(path.as_str())) {
        return Ok(Some(format!(
            "no casa con ningún glob de `discovery.include` ({:?})",
            policy.include
        )));
    }

    Ok(None)
}

/// ¿Casa `path` (o alguno de sus directorios ancestros) con el `Override` de exclusiones?
///
/// El ascenso por ancestros es necesario porque [`Override::matched`] no lo hace y en semántica
/// `.gitignore` un patrón de directorio (`vendor/`, o el `!vendor` que [`build_excludes`] añade
/// para podar un `vendor/**`) casa con el **directorio**, no con los ficheros de dentro: en el
/// walker eso basta porque el directorio se poda y no se desciende, pero aquí se pregunta por una
/// ruta suelta.
fn excluido_por_override(excludes: &Override, path: &RelPath) -> bool {
    let componentes: Vec<&str> = path.as_str().split('/').collect();
    for i in 1..componentes.len() {
        if excludes
            .matched(componentes[..i].join("/"), true)
            .is_ignore()
        {
            return true;
        }
    }
    excludes.matched(path.as_str(), false).is_ignore()
}

/// El primer glob de [`DiscoveryPolicy::exclude`] que excluye `path`, para poder nombrarlo en el
/// mensaje de error. Camino **frío**: solo se recorre cuando ya se sabe que el path está excluido
/// (el `Override` completo se consulta de una vez, no glob a glob).
fn glob_culpable(root: &Path, path: &RelPath, policy: &DiscoveryPolicy) -> Option<String> {
    for glob in &policy.exclude {
        let solo_este = DiscoveryPolicy {
            exclude: vec![glob.clone()],
            ..policy.clone()
        };
        match build_excludes(root, &solo_este) {
            Ok(ov) if excluido_por_override(&ov, path) => return Some(glob.clone()),
            _ => continue,
        }
    }
    None
}

/// ¿Excluye a `path` algún `.lodestarignore`/`.gitignore` del árbol? Devuelve el motivo legible.
///
/// Reproduce la precedencia de `ignore`: gana el fichero de ignore del directorio **más profundo**
/// (por eso el recorrido va del padre del documento hacia la raíz) y, dentro de un directorio, el
/// `.lodestarignore` (custom ignore) antes que el `.gitignore`. Una regla de re-inclusión (`!x`)
/// que case corta la búsqueda igual que un match de exclusión: el fichero más cercano decide.
fn excluido_por_ficheros_de_ignore(
    root: &Path,
    path: &RelPath,
    policy: &DiscoveryPolicy,
) -> Option<String> {
    let ficheros: Vec<&str> = [
        (policy.respect_lodestar_ignore, LODESTAR_IGNORE_FILENAME),
        (policy.respect_gitignore, GITIGNORE_FILENAME),
    ]
    .into_iter()
    .filter(|(activo, _)| *activo)
    .map(|(_, nombre)| nombre)
    .collect();
    if ficheros.is_empty() {
        return None;
    }

    let componentes: Vec<&str> = path.as_str().split('/').collect();
    for i in (0..componentes.len()).rev() {
        let dir = if i == 0 {
            root.to_path_buf()
        } else {
            root.join(componentes[..i].join("/"))
        };
        // Ruta del documento RELATIVA al directorio que hospeda el fichero de ignore: es como
        // `ignore` evalúa cada matcher (y evita depender del heurístico de `strip`).
        let relativa = componentes[i..].join("/");
        for nombre in &ficheros {
            let fichero = dir.join(nombre);
            if !fichero.is_file() {
                continue;
            }
            let mut builder = GitignoreBuilder::new(&dir);
            if builder.add(&fichero).is_some() {
                continue; // ilegible/malformado: el walker tampoco lo aplicaría
            }
            let Ok(matcher) = builder.build() else {
                continue;
            };
            match matcher.matched_path_or_any_parents(&relativa, false) {
                Match::Ignore(glob) => {
                    let ubicacion = fichero.strip_prefix(root).unwrap_or(&fichero);
                    return Some(format!(
                        "lo ignora el patrón «{}» de «{}»",
                        glob.original(),
                        ubicacion.display()
                    ));
                }
                // Re-inclusión explícita: decide el fichero más cercano, no se sigue subiendo.
                Match::Whitelist(_) => return None,
                Match::None => {}
            }
        }
    }
    None
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

/// El error de un glob inválido de la política, con el glob culpable en el mensaje.
fn glob_invalido(glob: &str, e: ignore::Error) -> WorkspaceError {
    WorkspaceError::Io(format!(
        "glob inválido en la política de descubrimiento «{glob}»: {e}"
    ))
}

/// Traduce **solo** [`DiscoveryPolicy::exclude`] al `Override` del walker, que es lo que le da la
/// precedencia máxima que la política explícita del usuario debe tener (por encima de los
/// `.gitignore`/`.lodestarignore` del árbol) y lo que permite **podar directorios durante** el
/// recorrido en vez de filtrar a posteriori.
///
/// - Los globs entran **negados**: en la semántica invertida de `OverrideBuilder`, un `!` al
///   principio significa «ignora esto».
/// - Cada `exclude` con forma `pre/**` añade además `pre` para podar el directorio entero: en
///   semántica `.gitignore`, `.git/**` casa con lo que hay dentro de `.git` pero no con `.git`, y
///   sin la poda se recorrería el repo git completo para tirar cada entrada una a una.
///
/// El `include` **no** entra aquí a propósito; ver la doc de [`discover`].
fn build_excludes(root: &Path, policy: &DiscoveryPolicy) -> Result<Override, WorkspaceError> {
    let mut builder = OverrideBuilder::new(root);
    for glob in &policy.exclude {
        if let Some(dir) = glob.strip_suffix("/**") {
            let podado = format!("!{dir}");
            builder.add(&podado).map_err(|e| glob_invalido(glob, e))?;
        }
        let negado = format!("!{glob}");
        builder.add(&negado).map_err(|e| glob_invalido(glob, e))?;
    }
    builder
        .build()
        .map_err(|e| WorkspaceError::Io(format!("política de descubrimiento inválida: {e}")))
}

/// Compila [`DiscoveryPolicy::include`] como matcher **independiente** del walker.
///
/// Se usa un `Override` (y no un `GlobSet` a pelo) para que los globs de `include` conserven
/// exactamente la misma semántica `.gitignore` que tenían cuando iban dentro del `Override` del
/// walker —`**/*.md` casa igual en la raíz que a diez niveles— y para que un glob mal escrito dé
/// el mismo error que los de `exclude`. Lo que cambia no es cómo casa `include`, sino **cuándo** se
/// consulta: ver [`discover`].
fn build_include(root: &Path, policy: &DiscoveryPolicy) -> Result<Override, WorkspaceError> {
    let mut builder = OverrideBuilder::new(root);
    for glob in &policy.include {
        builder.add(glob).map_err(|e| glob_invalido(glob, e))?;
    }
    builder
        .build()
        .map_err(|e| WorkspaceError::Io(format!("política de descubrimiento inválida: {e}")))
}

/// ¿Pasa `path` (un no-directorio) el filtro `include`?
///
/// Se consulta con la ruta completa que entrega el walker, igual que hace `ignore` con su propio
/// `Override`: el matcher se construyó con la misma raíz y le quita el prefijo él solo. Un
/// `include` vacío da un matcher vacío, que no casa con nada — coherente con «la config limita,
/// nunca habilita»: una lista blanca sin entradas no incluye nada.
fn incluido(include: &Override, path: &Path) -> bool {
    include.matched(path, false).is_whitelist()
}
