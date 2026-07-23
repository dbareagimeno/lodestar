//! Tests de integración del **descubrimiento recursivo universal** (E15-H07).
//!
//! Fijan el contrato de `lodestar_workspace::discovery`, el módulo que sustituye a
//! `io::load_bundle` (`ARCHITECTURE.md §20.5`, catálogo de diagnósticos en `§20.9`).
//!
//! Dos diferencias de fondo con el `load_bundle` de v0.2.x:
//!
//! 1. El descubrimiento devuelve **dos cosas**: el inventario (`FileMap`) y los **diagnósticos**
//!    de descubrimiento. Hoy los problemas (no-UTF8, entrada ilegible) se tiran por `eprintln!`
//!    y nadie los puede consultar; a partir de aquí son `Check` con código de `§20.9`.
//! 2. La política de descubrimiento es **explícita** (`DiscoveryPolicy`), con los valores por
//!    defecto de `§20.5`. E15-H08 la construirá desde `.lodestar/config.yaml`; aquí se pasa a
//!    mano para no depender de una historia posterior.
//!
//! Los workspaces salen íntegramente de `lodestar-fixtures` (E15-H05): `arbitrary()` y
//! `with_edge_cases()` para lo representable en un `FileMap`, y `materialize_disk_only()` para lo
//! que no lo es (bytes no UTF-8, fichero sobre el límite, symlink, ficheros de ignore).

use lodestar_core::types::{Check, FileMap, RelPath};
use lodestar_workspace::discovery::{case_collisions, discover, rel_path_from, DiscoveryPolicy};

/// Límite de tamaño por documento usado en los tests. Se le pasa **el mismo valor** a
/// `materialize_disk_only`, que escribe `enorme.md` con exactamente `LIMITE + 1` bytes: un solo
/// byte por encima, sin ambigüedad de frontera.
const LIMITE: usize = 4096;

/// Política de los tests: la de `§20.5` por defecto, con el límite de tamaño fijado arriba.
fn politica() -> DiscoveryPolicy {
    DiscoveryPolicy {
        max_document_bytes: LIMITE,
        ..DiscoveryPolicy::default()
    }
}

/// ¿Está `path` en el inventario?
fn contiene(files: &FileMap, path: &str) -> bool {
    files.keys().any(|p| p.as_str() == path)
}

/// ¿Es case-**sensitive** el sistema de ficheros donde se crean los tempdirs?
///
/// Se sondea en tiempo de ejecución (no con `cfg!(target_os)`): la case-sensitivity es una
/// propiedad del **volumen**, no del sistema operativo — un macOS puede tener un volumen
/// case-sensitive y un Linux puede montar exFAT.
fn fs_case_sensitive() -> bool {
    let sonda = tempfile::tempdir().expect("tempdir de sonda");
    std::fs::create_dir(sonda.path().join("sonda-case")).expect("crear sonda");
    !sonda.path().join("SONDA-CASE").exists()
}

/// ¿Hay un diagnóstico con este código de wire apuntando a `target`?
fn hay_diagnostico(diags: &[Check], code: &str, target: &str) -> bool {
    diags
        .iter()
        .any(|c| c.code.as_str() == code && c.targets.iter().any(|t| t.as_str() == target))
}

/// Resumen legible de los diagnósticos, para los mensajes de fallo.
fn resumen(diags: &[Check]) -> String {
    diags
        .iter()
        .map(|c| {
            let targets: Vec<&str> = c.targets.iter().map(|t| t.as_str()).collect();
            format!("{} {:?}", c.code.as_str(), targets)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Los paths del inventario, ordenados (el `FileMap` es un `BTreeMap`: determinista).
fn rutas(files: &FileMap) -> Vec<&str> {
    files.keys().map(|p| p.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Criterio 1: recursión sin profundidad máxima
// ---------------------------------------------------------------------------

/// **Dado** el fixture `arbitrary()` materializado, **Cuando** se descubre, **Entonces** el
/// inventario tiene los 4 documentos, incluido `three/levels/deep/third.md`.
///
/// Es el caso que la épica persigue: una estructura de carpetas arbitraria, **sin** `index.md` y
/// **sin** frontmatter, es una base de conocimiento completa.
#[test]
fn descubre_a_cualquier_profundidad() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();

    let d = discover(dir.path(), &politica()).unwrap();

    assert_eq!(
        rutas(&d.files),
        vec![
            "README.md",
            "one/first.md",
            "three/levels/deep/third.md",
            "two/levels/second.md",
        ],
        "el inventario debe tener los 4 documentos a cualquier profundidad, en orden determinista"
    );
    assert!(
        contiene(&d.files, "three/levels/deep/third.md"),
        "el documento de tres niveles de profundidad no puede quedarse fuera"
    );
    assert!(
        d.diagnostics.is_empty(),
        "un workspace limpio no debe generar diagnósticos de descubrimiento: {}",
        resumen(&d.diagnostics)
    );
}

// ---------------------------------------------------------------------------
// Criterio 2: `.gitignore`
// ---------------------------------------------------------------------------

/// **Dado** un `.gitignore` con `vendor/`, **Cuando** se descubre, **Entonces** `vendor/dep.md`
/// no está en el inventario.
///
/// Ojo al detalle que fija el alcance de la historia: el tempdir **no** es un repo git (no hay
/// `.git/`), y `ignore::WalkBuilder` solo aplica `.gitignore` dentro de un repo salvo que se le
/// pase `require_git(false)`. Sin eso, este test falla — y falla justo en el escenario
/// "directorio arbitrario" que persigue la épica.
#[test]
fn respeta_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();
    lodestar_fixtures::materialize_disk_only(dir.path(), LIMITE).unwrap();

    let d = discover(dir.path(), &politica()).unwrap();

    assert!(
        !contiene(&d.files, "vendor/dep.md"),
        "`vendor/` está en el .gitignore: no debe entrar en el inventario. Inventario: {:?}",
        rutas(&d.files)
    );
    assert!(
        contiene(&d.files, "README.md"),
        "el resto del inventario sí debe cargarse: {:?}",
        rutas(&d.files)
    );
}

// ---------------------------------------------------------------------------
// Criterio 3: `.lodestarignore`
// ---------------------------------------------------------------------------

/// **Dado** un `.lodestarignore` con `borradores/`, **Cuando** se descubre, **Entonces**
/// `borradores/wip.md` no está en el inventario.
#[test]
fn respeta_lodestarignore() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();
    lodestar_fixtures::materialize_disk_only(dir.path(), LIMITE).unwrap();

    let d = discover(dir.path(), &politica()).unwrap();

    assert!(
        !contiene(&d.files, "borradores/wip.md"),
        "`borradores/` está en el .lodestarignore: no debe entrar en el inventario. \
         Inventario: {:?}",
        rutas(&d.files)
    );
    assert!(
        contiene(&d.files, "one/first.md"),
        "el resto del inventario sí debe cargarse: {:?}",
        rutas(&d.files)
    );
}

// ---------------------------------------------------------------------------
// Criterio 4: symlinks
// ---------------------------------------------------------------------------

/// **Dado** un `.md` que es symlink, **Cuando** se descubre, **Entonces** no entra en el
/// inventario y se emite `SYMLINK-UNSUPPORTED`.
///
/// El punto no es solo `follow_links(false)` (que ya excluiría el symlink en silencio): es que el
/// usuario **se entere** de que hay un documento que Lodestar no está viendo.
///
/// Solo Unix: `materialize_disk_only` únicamente crea el symlink ahí (en Windows exige permisos
/// especiales), así que en otras plataformas el escenario no existe.
#[cfg(unix)]
#[test]
fn symlink_rechazado_con_diagnostico() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();
    lodestar_fixtures::materialize_disk_only(dir.path(), LIMITE).unwrap();
    assert!(
        std::fs::symlink_metadata(dir.path().join("enlace.md"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "precondición: `enlace.md` debe ser un symlink en disco"
    );

    let d = discover(dir.path(), &politica()).unwrap();

    assert!(
        !contiene(&d.files, "enlace.md"),
        "un symlink no es un documento del inventario. Inventario: {:?}",
        rutas(&d.files)
    );
    assert!(
        hay_diagnostico(&d.diagnostics, "SYMLINK-UNSUPPORTED", "enlace.md"),
        "el symlink debe reportarse, no ignorarse en silencio. Diagnósticos: {}",
        resumen(&d.diagnostics)
    );
}

// ---------------------------------------------------------------------------
// Criterio 5: no-UTF8 y tamaño — diagnóstico, no aborto
// ---------------------------------------------------------------------------

/// **Dado** un `.md` no UTF-8 y otro sobre el límite, **Cuando** se descubre, **Entonces** se
/// emiten `DOC-NOT-UTF8` y `DOC-TOO-LARGE` y **el resto del inventario se carga**.
///
/// La segunda mitad es la que importa: un solo fichero roto no puede dejar muerto el
/// descubrimiento entero (hoy `io.rs:46` lo salta con un `eprintln!` que nadie ve; el fallo
/// contrario — abortar — dejaría el workspace inservible por un byte).
#[test]
fn no_utf8_y_grande_no_abortan() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();
    lodestar_fixtures::materialize_disk_only(dir.path(), LIMITE).unwrap();
    assert_eq!(
        std::fs::metadata(dir.path().join("enorme.md"))
            .unwrap()
            .len(),
        LIMITE as u64 + 1,
        "precondición: `enorme.md` debe pesar exactamente LIMITE + 1 bytes"
    );

    let d = discover(dir.path(), &politica()).unwrap();

    assert!(
        hay_diagnostico(&d.diagnostics, "DOC-NOT-UTF8", "binario.md"),
        "un `.md` con bytes no UTF-8 debe reportarse como DOC-NOT-UTF8. Diagnósticos: {}",
        resumen(&d.diagnostics)
    );
    assert!(
        hay_diagnostico(&d.diagnostics, "DOC-TOO-LARGE", "enorme.md"),
        "un `.md` por encima del límite debe reportarse como DOC-TOO-LARGE. Diagnósticos: {}",
        resumen(&d.diagnostics)
    );
    assert!(
        !contiene(&d.files, "binario.md") && !contiene(&d.files, "enorme.md"),
        "ni el no-UTF8 ni el sobredimensionado entran en el inventario: {:?}",
        rutas(&d.files)
    );

    // Y, sobre todo: el resto del inventario está completo.
    for esperado in [
        "README.md",
        "one/first.md",
        "two/levels/second.md",
        "three/levels/deep/third.md",
    ] {
        assert!(
            contiene(&d.files, esperado),
            "un fichero problemático no puede tumbar el descubrimiento: falta {esperado} \
             (inventario: {:?})",
            rutas(&d.files)
        );
    }
}

// ---------------------------------------------------------------------------
// Criterio 6: colisiones de capitalización
// ---------------------------------------------------------------------------

/// **Dado** `docs/auth.md` y un directorio `Docs/`, **Cuando** se descubre, **Entonces** se emite
/// un diagnóstico de portabilidad (`LINK-CASE-MISMATCH` a nivel de inventario).
///
/// ## Por qué este test está partido en dos mitades
///
/// La trampa de este criterio es de física del sistema de ficheros, no de código. En un volumen
/// **case-insensitive** (APFS por defecto en macOS, NTFS en Windows — dos de las tres plataformas
/// del CI) `Docs/Auth.md` **ES** `docs/auth.md`: `std::fs::write` sobre el segundo sobrescribe el
/// primero y en disco queda **un solo fichero**. La consecuencia es fuerte: en esos volúmenes el
/// escenario "dos documentos descubiertos que colisionan al plegar mayúsculas" es
/// **irrealizable por construcción**, y un test que lo montara en disco daría un resultado
/// distinto por plataforma (justo lo que no puede pasar).
///
/// Por eso el criterio se verifica en dos mitades:
///
/// 1. **La detección** — que es una función pura del inventario ([`case_collisions`]) y por tanto
///    se puede alimentar con un `FileMap` en memoria que sí contiene la colisión. Esta mitad corre
///    y asierta de verdad en las tres plataformas. Es también donde vive el diseño: la colisión se
///    reporta como **un** diagnóstico **por grupo** de rutas que pliegan a lo mismo, no uno por
///    fichero, y nombra a todas las implicadas.
/// 2. **El cableado** — que `discover` incorpora esa detección a sus diagnósticos. Solo se puede
///    comprobar donde el volumen es case-sensitive, así que se sondea en tiempo de ejecución; en
///    los volúmenes case-insensitive se comprueba en su lugar la propiedad que sí es observable
///    ahí: los dos escritos colapsaron en un único documento y el descubrimiento **no inventa**
///    una colisión que el disco no tiene.
#[test]
fn colision_capitalizacion() {
    // --- Mitad 1: la detección sobre el inventario (las 3 plataformas) ----------------
    let limpio = lodestar_fixtures::with_edge_cases();
    assert!(
        case_collisions(&limpio).is_empty(),
        "sin colisión real no debe haber diagnóstico: `docs/auth.md` y \
         `packages/api/docs/auth.md` comparten basename pero son rutas distintas — plegar por \
         basename en vez de por ruta completa es un falso positivo"
    );

    let mut con_colision = limpio.clone();
    con_colision.insert(
        RelPath::new("Docs/Auth.md").unwrap(),
        "# Auth (otra capitalización)\n".to_string(),
    );
    let colisiones = case_collisions(&con_colision);
    assert_eq!(
        colisiones.len(),
        1,
        "una colisión = UN diagnóstico por grupo de rutas equivalentes, no uno por fichero: {}",
        resumen(&colisiones)
    );
    let c = &colisiones[0];
    assert_eq!(
        c.code.as_str(),
        "LINK-CASE-MISMATCH",
        "el código de portabilidad del catálogo de `§20.9`"
    );
    let nombradas: Vec<&str> = c.targets.iter().map(|t| t.as_str()).collect();
    for esperado in ["Docs/Auth.md", "docs/auth.md"] {
        assert!(
            nombradas.contains(&esperado),
            "el diagnóstico debe nombrar TODAS las rutas del grupo (falta {esperado}): {nombradas:?}"
        );
    }

    // --- Mitad 2: el cableado dentro de `discover` -----------------------------------
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&limpio, dir.path()).unwrap();
    // El gemelo con otra capitalización no puede vivir en el `FileMap` materializado (en un
    // volumen case-insensitive `materialize` escribiría ambos sobre el mismo fichero): se crea
    // aquí, a mano, para que el escenario dependa solo del disco.
    std::fs::create_dir_all(dir.path().join("Docs")).unwrap();
    std::fs::write(
        dir.path().join("Docs/Auth.md"),
        "# Auth (otra capitalización)\n",
    )
    .unwrap();

    let d = discover(dir.path(), &politica()).unwrap();
    let plegadas: Vec<&str> = rutas(&d.files)
        .into_iter()
        .filter(|p| p.to_lowercase() == "docs/auth.md")
        .collect();
    let reportadas: Vec<&Check> = d
        .diagnostics
        .iter()
        .filter(|c| c.code.as_str() == "LINK-CASE-MISMATCH")
        .collect();

    if fs_case_sensitive() {
        assert_eq!(
            plegadas.len(),
            2,
            "volumen case-sensitive: los dos documentos coexisten (inventario: {:?})",
            rutas(&d.files)
        );
        assert!(
            reportadas.iter().any(|c| c
                .targets
                .iter()
                .any(|t| t.as_str().to_lowercase() == "docs/auth.md")),
            "`discover` debe incorporar la detección de colisiones a sus diagnósticos. \
             Diagnósticos: {}",
            resumen(&d.diagnostics)
        );
    } else {
        assert_eq!(
            plegadas.len(),
            1,
            "volumen case-insensitive: `Docs/Auth.md` y `docs/auth.md` son el MISMO fichero, \
             así que el inventario solo puede tener uno (inventario: {:?})",
            rutas(&d.files)
        );
        assert!(
            reportadas.is_empty(),
            "volumen case-insensitive: no hay dos rutas que colisionen, así que no puede \
             fabricarse un diagnóstico. Diagnósticos: {}",
            resumen(&d.diagnostics)
        );
    }
}

// ---------------------------------------------------------------------------
// Criterio 7: paths con espacios
// ---------------------------------------------------------------------------

/// **Dado** un `.md` con espacios en el path, **Cuando** se descubre, **Entonces** entra en el
/// inventario con su ruta exacta (sin escapar, sin `%20`, sin normalizar el espacio).
#[test]
fn paths_con_espacios() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = lodestar_fixtures::with_edge_cases();
    lodestar_fixtures::materialize(&fixture, dir.path()).unwrap();

    let d = discover(dir.path(), &politica()).unwrap();

    assert!(
        contiene(&d.files, "notas/con espacios.md"),
        "el documento con espacios debe entrar con su ruta EXACTA (ni escapada ni %20-ificada): \
         {:?}",
        rutas(&d.files)
    );
    let esperado = fixture
        .iter()
        .find(|(p, _)| p.as_str() == "notas/con espacios.md")
        .map(|(_, c)| c.clone())
        .expect("el fixture with_edge_cases trae `notas/con espacios.md`");
    let leido = d
        .files
        .iter()
        .find(|(p, _)| p.as_str() == "notas/con espacios.md")
        .map(|(_, c)| c.clone())
        .unwrap();
    assert_eq!(
        leido, esperado,
        "el contenido leído debe ser el del fixture"
    );
}

// ---------------------------------------------------------------------------
// Criterio 8 (añadido tras la fase roja): el módulo está CABLEADO al producto
// ---------------------------------------------------------------------------

/// **Dado** un workspace con documentos a tres niveles y un `.gitignore` que excluye `vendor/`,
/// **Cuando** se abre con `Workspace::bundle()`, **Entonces** el bundle contiene los documentos
/// profundos y **no** contiene `vendor/dep.md`.
///
/// Este es el único test ejecutable de que `discovery` **sustituye** a `io::load_bundle` en las 7
/// llamadas del alcance: sin él, el módulo puede nacer perfecto y no llegar al producto, y los 7
/// tests anteriores pasarían igual.
///
/// Cubre **dos** de los 7 puntos de cableado, no uno, y a propósito:
///
/// - `Workspace::bundle()` (`lib.rs:196`) — la lectura de conocimiento que alimenta `snapshot()`,
///   `analysis()` y por tanto a las dos fachadas.
/// - `Workspace::workspace_revision()` (`lib.rs:100`) — el control optimista del motor
///   transaccional. Es el que **más daño hace si se olvida**: si `workspace_revision` descubriera
///   un conjunto de ficheros distinto del de `bundle()`, el hash de base cubriría documentos que
///   el plan no ve (y al revés), y `reverify_base_revision` empezaría a dar conflictos fantasma —
///   o, peor, a no darlos. La forma de fijarlo sin acoplarse al hash concreto es comprobar **de
///   qué depende**: un fichero excluido por la política no puede mover la revisión; un documento
///   profundo sí.
///
/// Los otros 5 puntos (`transaction.rs:123`, `staging.rs:102`, `recovery.rs:473`,
/// `publish.rs:56,102`) computan el canónico para el diff/journal transaccional y ya están
/// cubiertos por regresión en `tests/transactions.rs`; no los duplico aquí.
#[test]
fn bundle_usa_la_politica_de_descubrimiento() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();
    // Aporta `.gitignore` con `vendor/` + `vendor/dep.md`, y `.lodestarignore` + `borradores/`.
    // El `enorme.md` que también crea se dimensiona contra LIMITE, pero aquí la política es la
    // POR DEFECTO (`bundle()` no recibe una a medida hasta E15-H08), así que este test no asierta
    // nada sobre él: sigue siendo válido cualquiera que sea el `max_document_bytes` por defecto.
    lodestar_fixtures::materialize_disk_only(dir.path(), LIMITE).unwrap();

    let ws = lodestar_workspace::Workspace::open(dir.path()).unwrap();

    // --- `bundle()` ---------------------------------------------------------------
    let bundle = ws.bundle().unwrap();
    let files = bundle.files();
    for profundo in [
        "README.md",
        "one/first.md",
        "two/levels/second.md",
        "three/levels/deep/third.md",
    ] {
        assert!(
            contiene(files, profundo),
            "`bundle()` debe descubrir a cualquier profundidad: falta {profundo} \
             (bundle: {:?})",
            rutas(files)
        );
    }
    assert!(
        !contiene(files, "vendor/dep.md"),
        "`bundle()` debe aplicar la política de descubrimiento (`.gitignore`), no el walker \
         viejo de `io::load_bundle`. Bundle: {:?}",
        rutas(files)
    );
    assert!(
        !contiene(files, "borradores/wip.md"),
        "`bundle()` debe aplicar también el `.lodestarignore`. Bundle: {:?}",
        rutas(files)
    );

    // --- `workspace_revision()` ---------------------------------------------------
    // Misma política ⇒ mismo conjunto de ficheros ⇒ la revisión depende exactamente de lo que
    // el bundle ve. No se asierta el hash (es opaco): se asierta de qué depende.
    let rev_inicial = ws.workspace_revision().unwrap();

    std::fs::write(
        dir.path().join("vendor/dep.md"),
        "# Dependencia (modificada por el test)\n",
    )
    .unwrap();
    assert_eq!(
        ws.workspace_revision().unwrap(),
        rev_inicial,
        "un fichero excluido por la política NO forma parte de la revisión del workspace: si la \
         mueve, `workspace_revision` está descubriendo un conjunto distinto del de `bundle()` y \
         el control optimista pasa a proteger ficheros que el plan ni siquiera ve"
    );

    std::fs::write(
        dir.path().join("three/levels/deep/third.md"),
        "# Tercero (modificado por el test)\n",
    )
    .unwrap();
    assert_ne!(
        ws.workspace_revision().unwrap(),
        rev_inicial,
        "un documento profundo SÍ forma parte de la revisión: si no la mueve, el control \
         optimista es ciego a los cambios anidados"
    );
}

// ---------------------------------------------------------------------------
// Criterio 9 (añadido tras la fase roja): `PATH-NOT-UTF8`
// ---------------------------------------------------------------------------

/// Una ruta no representable como UTF-8 se convierte en el diagnóstico `PATH-NOT-UTF8`, no en un
/// `continue` silencioso.
///
/// **Por qué es un test unitario de la función pura y no un fixture de disco**: no existe un
/// escenario de disco portable. En Windows los nombres de fichero son UTF-16 validado y en APFS
/// son UTF-8 validado — el sistema **rechaza** crear el fichero. Solo en Linux/ext4 se puede
/// materializar, así que un test de disco sería vacuo en 2 de las 3 plataformas del CI (el mismo
/// problema que `colision_capitalizacion`). En memoria, en cambio, la ruta inválida se construye
/// en las tres: bytes sueltos vía `OsString::from_vec` en Unix, surrogate suelto vía
/// `OsString::from_wide` en Windows.
#[test]
fn path_no_utf8_diagnostica() {
    // Camino feliz, y de paso el contrato de separador: la conversión devuelve SIEMPRE la forma
    // canónica con barras, venga del separador nativo que venga. En Windows el walker entrega
    // `three\levels\deep\third.md` y `RelPath::new` RECHAZA los backslashes (invariante #6), así
    // que sin esta normalización el descubrimiento entero se cae en Windows.
    let nativo: std::path::PathBuf = ["three", "levels", "deep", "third.md"].iter().collect();
    assert_eq!(
        rel_path_from(&nativo)
            .expect("una ruta relativa válida se convierte sin diagnóstico")
            .as_str(),
        "three/levels/deep/third.md",
        "la conversión normaliza el separador nativo a `/`"
    );

    // Ruta no representable.
    let invalida = path_no_representable();
    let diag = rel_path_from(std::path::Path::new(&invalida))
        .expect_err("una ruta no representable como UTF-8 debe producir un diagnóstico");
    assert_eq!(
        diag.code.as_str(),
        "PATH-NOT-UTF8",
        "el código del catálogo de `§20.9` para rutas no representables"
    );
    assert!(
        diag.targets.is_empty(),
        "no se puede construir un `RelPath` para esta ruta — ese ES el problema. `targets` queda \
         vacío antes que colar un path crudo (invariante #6): {:?}",
        diag.targets
    );
    let msg = &diag.msg;
    assert!(
        msg.contains("notas/") && msg.contains(".md"),
        "si el path no cabe en `targets`, el mensaje es lo ÚNICO que permite al usuario localizar \
         el fichero: debe llevar su representación lossy. Mensaje: {msg:?}"
    );
}

/// Una ruta relativa que **no** es representable como UTF-8, construida en memoria.
///
/// `notas/<inválido>.md` en ambas plataformas, para que el mensaje lossy sea comparable.
#[cfg(unix)]
fn path_no_representable() -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt as _;
    // 0xFF nunca aparece en una secuencia UTF-8 válida.
    std::ffi::OsString::from_vec(b"notas/\xFF.md".to_vec())
}

/// Ver la versión Unix. Aquí el inválido es un **surrogate suelto** (`0xD800`): legal en el
/// UTF-16 de Windows, no convertible a UTF-8.
#[cfg(windows)]
fn path_no_representable() -> std::ffi::OsString {
    use std::os::windows::ffi::OsStringExt as _;
    let unidades: Vec<u16> = "notas/"
        .encode_utf16()
        .chain(std::iter::once(0xD800))
        .chain(".md".encode_utf16())
        .collect();
    std::ffi::OsString::from_wide(&unidades)
}

// ---------------------------------------------------------------------------
// Criterio 10: `.lodestar/` es el plano de control, no conocimiento
// ---------------------------------------------------------------------------

/// **Dado** un `.md` bajo `.lodestar/` (una plantilla, o un fichero suelto), **Cuando** se
/// descubre con la política por defecto, **Entonces** no entra en el inventario.
///
/// ## El agujero que cierra
///
/// El walker viejo (`io::load_bundle`) podaba **cualquier** directorio `.lodestar` a cualquier
/// profundidad; la política por defecto de `§20.5` excluía solo `.lodestar/runtime/**`. En esa
/// rendija cabía `.lodestar/templates/plantilla.md`, que pasaba a ser un documento del inventario
/// con todas las consecuencias: nodo del grafo, resultado de `knowledge_search`, sujeto de
/// `change_apply`, `move_document` y `delete_document`.
///
/// Y ahí estaba la incoherencia: [`lodestar_core::types::workspace_revision`] excluye **todo**
/// `.lodestar/` (decisión D5 — `.lodestar/` nunca es fuente de verdad). Un documento así sería
/// escribible por el motor transaccional y **sus cambios jamás moverían la revisión del
/// workspace**: el control optimista dejaría de protegerlo en silencio. Es el mismo fallo que
/// [`bundle_usa_la_politica_de_descubrimiento`] previene entre `bundle()` y
/// `workspace_revision()`, entrando por el otro lado.
///
/// ## Por qué se cierra por el lado del descubrimiento
///
/// D5 no es una convención arbitraria: es lo que impide que la revisión observe su propia
/// maquinaria. `StagingDir` materializa bajo `.lodestar/runtime/staging/` un **árbol `.md`
/// completo** — copias de los mismos documentos cuya escritura está guardando. Si `.lodestar/`
/// contara para la revisión, `reverify_base_revision` fallaría *a causa del apply en curso*.
/// Ampliar `workspace_revision` no es una alternativa; excluir `.lodestar/` del inventario sí.
///
/// ## Las dos mitades del test
///
/// 1. **La exclusión**, en los tres sitios donde puede aparecer un `.md` de control: bajo
///    `templates/`, suelto en la raíz de `.lodestar/`, y bajo `runtime/` (este último ya salía
///    excluido — va como guarda de regresión).
/// 2. **El invariante que hay detrás**, que es lo que de verdad importa y sobrevive a cualquier
///    cambio futuro de la lista de globs: *todo documento del inventario cuenta para la revisión
///    del workspace*. Se comprueba de la única forma observable desde fuera — tocando cada
///    documento descubierto y exigiendo que la revisión se mueva. Un documento invisible para la
///    revisión hace fallar el bucle, esté donde esté y lo excluya quien lo excluya.
#[test]
fn lodestar_interno_no_es_conocimiento() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();

    let control = [
        // El caso real de hoy: las plantillas de `.lodestar/templates/` son ENTRADA de la
        // generación, no documentos de la base.
        ".lodestar/templates/plantilla.md",
        // Un `.md` suelto en la raíz del directorio de control.
        ".lodestar/nota.md",
        // Runtime: ya excluido antes de esta enmienda; guarda de regresión.
        ".lodestar/runtime/staging/copia.md",
    ];
    for rel in control {
        let destino = dir.path().join(rel);
        std::fs::create_dir_all(destino.parent().unwrap()).unwrap();
        std::fs::write(&destino, "# Fichero de control\n").unwrap();
    }

    // Política POR DEFECTO explícita: el criterio es sobre los valores de `§20.5`, no sobre una
    // política a medida (la configurable llega en E15-H08).
    let d = discover(dir.path(), &DiscoveryPolicy::default()).unwrap();

    // --- Mitad 1: la exclusión ------------------------------------------------------
    for rel in control {
        assert!(
            !contiene(&d.files, rel),
            "`.lodestar/` es el plano de control de Lodestar, no conocimiento del usuario: \
             {rel} no puede entrar en el inventario. Inventario: {:?}",
            rutas(&d.files)
        );
    }
    assert_eq!(
        rutas(&d.files),
        vec![
            "README.md",
            "one/first.md",
            "three/levels/deep/third.md",
            "two/levels/second.md",
        ],
        "excluir `.lodestar/` no puede llevarse por delante ningún documento del usuario"
    );

    // --- Mitad 2: el invariante -----------------------------------------------------
    // Todo documento del inventario cuenta para la revisión del workspace. Si alguno no la
    // moviera, sería escribible por el motor transaccional y ciego al control optimista.
    let revision =
        |files: &FileMap| lodestar_core::types::workspace_revision(files, &[] as &[RelPath]);
    let descubiertos: Vec<String> = rutas(&d.files).into_iter().map(String::from).collect();
    let mut anterior = revision(&d.files);
    for rel in descubiertos {
        let destino = dir.path().join(&rel);
        let mut contenido = std::fs::read_to_string(&destino).unwrap();
        contenido.push_str("\n<!-- tocado por el test -->\n");
        std::fs::write(&destino, contenido).unwrap();

        let actual = revision(
            &discover(dir.path(), &DiscoveryPolicy::default())
                .unwrap()
                .files,
        );
        assert_ne!(
            actual, anterior,
            "`{rel}` está en el inventario pero cambiarlo NO mueve la revisión del workspace: \
             sería un documento escribible al que el control optimista no protege"
        );
        anterior = actual;
    }
}
