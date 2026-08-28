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
use ignore::Match;
use lodestar_core::types::{Check, CheckCode, FileMap, RelPath, Severity};

use crate::error::WorkspaceError;
use crate::Workspace;

pub const GITIGNORE_FILENAME: &str = ".gitignore";

pub use lodestar_discovery::{
    DiscoveredInventory, DiscoveryPolicy, CONTROL_PLANE_EXCLUDE, DEFAULT_MAX_DOCUMENT_BYTES,
    LODESTAR_IGNORE_FILENAME,
};

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

/// Primera pasada del descubrimiento. Conserva exactamente la política del walker canónico, pero
/// solo paths, metadata compacta implícita y diagnósticos; la validación UTF-8 se difiere a la
/// segunda pasada que materializa cada candidato.
pub fn discover_inventory(
    root: &Path,
    policy: &DiscoveryPolicy,
) -> Result<DiscoveredInventory, WorkspaceError> {
    lodestar_discovery::discover_inventory(root, policy)
        .map_err(|error| WorkspaceError::Io(error.to_string()))
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
///
/// Única entrada de descubrimiento: el walker y la admisión pertenecen al crate compartido.
pub fn discover(root: &Path, policy: &DiscoveryPolicy) -> Result<Discovered, WorkspaceError> {
    let inventory = lodestar_discovery::discover_inventory(root, policy)
        .map_err(|error| WorkspaceError::Io(error.to_string()))?;
    let mut files = FileMap::new();
    let mut other_files = inventory.other_files;
    let mut diagnostics = inventory.diagnostics;
    for path in inventory.documents {
        let full = root.join(path.as_str());
        match std::fs::read_to_string(&full) {
            Ok(content) => {
                files.insert(path, content);
            }
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                diagnostics.push(Check::new(
                    Severity::Warn,
                    CheckCode::DocNotUtf8,
                    format!("«{}» no es UTF-8 válido", path.as_str()),
                    vec![path.clone()],
                ));
                other_files.insert(path);
            }
            Err(error) => {
                eprintln!(
                    "lodestar: aviso: se salta {} (ilegible): {error}",
                    full.display()
                );
                other_files.insert(path);
            }
        }
    }
    diagnostics.extend(case_collisions(&files));
    if files.is_empty() {
        diagnostics.push(Check::new(
            Severity::Warn,
            CheckCode::WorkspaceEmpty,
            workspace_empty_message(root, &other_files),
            Vec::new(),
        ));
    }
    diagnostics.sort_by(|left, right| {
        left.code
            .as_str()
            .cmp(right.code.as_str())
            .then_with(|| left.msg.cmp(&right.msg))
    });
    Ok(Discovered {
        files,
        other_files,
        diagnostics,
    })
}

fn workspace_empty_message(root: &Path, other_files: &BTreeSet<RelPath>) -> String {
    let markdown_discarded: Vec<&RelPath> = other_files
        .iter()
        .filter(|path| path.is_markdown())
        .collect();
    let cause = if markdown_discarded.is_empty() {
        "no hay ningún fichero «.md» bajo esa raíz, o la política de descubrimiento \
         (`discovery.include`/`exclude`, `.gitignore`, `.lodestarignore`) los descarta antes de \
         visitarlos: comprueba que es el directorio que creías"
            .to_string()
    } else {
        let listed: Vec<&str> = markdown_discarded
            .iter()
            .take(3)
            .map(|p| p.as_str())
            .collect();
        format!(
            "hay {} fichero(s) «.md» bajo esa raíz ({}{}) pero la política de descubrimiento \
             (`discovery.include`/`exclude`) los deja a TODOS fuera del inventario",
            markdown_discarded.len(),
            listed.join(", "),
            if markdown_discarded.len() > listed.len() {
                ", …"
            } else {
                ""
            }
        )
    };
    format!(
        "el workspace «{}» no tiene NINGÚN documento en el inventario: {cause}",
        root.display()
    )
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

    // (3) `include`, el filtro FINAL sobre lo que sobrevivió a (1) y (2). La compilación y la
    //     equivalencia de casing Markdown pertenecen al inventario compartido: el guard no
    //     mantiene una segunda interpretación.
    if !lodestar_discovery::path_matches_include(root, policy, path)
        .map_err(|error| WorkspaceError::Io(error.to_string()))?
    {
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
    case_collisions_paths(&files.keys().cloned().collect::<Vec<_>>())
}

fn case_collisions_paths(paths: &[RelPath]) -> Vec<Check> {
    let mut grupos: BTreeMap<String, Vec<RelPath>> = BTreeMap::new();
    for path in paths {
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
/// Normaliza el separador de directorios de una ruta relativa **del sistema**.
///
/// En Windows `Path` usa `\\` como separador y hay que traducirlo al `/` canónico de [`RelPath`].
/// En Unix, en cambio, `\\` es un carácter **legítimo dentro de un nombre de fichero**: traducirlo
/// allí convertía un fichero llamado literalmente `a\\b.md` en la ruta `a/b.md`, que puede
/// **enmascarar un documento real** de ese path (E24-H12).
fn normaliza_separadores(s: &str) -> String {
    #[cfg(windows)]
    {
        s.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        s.to_string()
    }
}

#[allow(clippy::result_large_err)]
pub fn rel_path_from(rel: &Path) -> Result<RelPath, Check> {
    let lossy = normaliza_separadores(&rel.to_string_lossy());
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
    RelPath::new(&normaliza_separadores(texto)).map_err(|e| {
        no_representable(format!(
            "no es una ruta relativa válida del workspace ({e})"
        ))
    })
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
