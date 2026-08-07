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
/// **Cuando** se abre con `Workspace::document_set()`, **Entonces** el workspace contiene los documentos
/// profundos y **no** contiene `vendor/dep.md`.
///
/// Este es el único test ejecutable de que `discovery` **sustituye** a `io::load_bundle` en las 7
/// llamadas del alcance: sin él, el módulo puede nacer perfecto y no llegar al producto, y los 7
/// tests anteriores pasarían igual.
///
/// Cubre **dos** de los 7 puntos de cableado, no uno, y a propósito:
///
/// - `Workspace::document_set()` (`lib.rs:196`) — la lectura de conocimiento que alimenta `snapshot()`,
///   `analysis()` y por tanto a las dos fachadas.
/// - `Workspace::workspace_revision()` (`lib.rs:100`) — el control optimista del motor
///   transaccional. Es el que **más daño hace si se olvida**: si `workspace_revision` descubriera
///   un conjunto de ficheros distinto del de `document_set()`, el hash de base cubriría documentos que
///   el plan no ve (y al revés), y `reverify_base_revision` empezaría a dar conflictos fantasma —
///   o, peor, a no darlos. La forma de fijarlo sin acoplarse al hash concreto es comprobar **de
///   qué depende**: un fichero excluido por la política no puede mover la revisión; un documento
///   profundo sí.
///
/// Los otros 5 puntos (`transaction.rs:123`, `staging.rs:102`, `recovery.rs:473`,
/// `publish.rs:56,102`) computan el canónico para el diff/journal transaccional y ya están
/// cubiertos por regresión en `tests/transactions.rs`; no los duplico aquí.
#[test]
fn workspace_usa_la_politica_de_descubrimiento() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();
    // Aporta `.gitignore` con `vendor/` + `vendor/dep.md`, y `.lodestarignore` + `borradores/`.
    // El `enorme.md` que también crea se dimensiona contra LIMITE, pero aquí la política es la
    // POR DEFECTO (`document_set()` no recibe una a medida hasta E15-H08), así que este test no asierta
    // nada sobre él: sigue siendo válido cualquiera que sea el `max_document_bytes` por defecto.
    lodestar_fixtures::materialize_disk_only(dir.path(), LIMITE).unwrap();

    let ws = lodestar_workspace::Workspace::open(dir.path()).unwrap();

    // --- `document_set()` ---------------------------------------------------------------
    let doc_set = ws.document_set().unwrap();
    let files = doc_set.files();
    for profundo in [
        "README.md",
        "one/first.md",
        "two/levels/second.md",
        "three/levels/deep/third.md",
    ] {
        assert!(
            contiene(files, profundo),
            "`document_set()` debe descubrir a cualquier profundidad: falta {profundo} \
             (doc_set: {:?})",
            rutas(files)
        );
    }
    assert!(
        !contiene(files, "vendor/dep.md"),
        "`document_set()` debe aplicar la política de descubrimiento (`.gitignore`), no el walker \
         viejo de `io::load_bundle`. Workspace: {:?}",
        rutas(files)
    );
    assert!(
        !contiene(files, "borradores/wip.md"),
        "`document_set()` debe aplicar también el `.lodestarignore`. Workspace: {:?}",
        rutas(files)
    );

    // --- `workspace_revision()` ---------------------------------------------------
    // Misma política ⇒ mismo conjunto de ficheros ⇒ la revisión depende exactamente de lo que
    // el workspace ve. No se asierta el hash (es opaco): se asierta de qué depende.
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
         mueve, `workspace_revision` está descubriendo un conjunto distinto del de `document_set()` y \
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
/// [`workspace_usa_la_politica_de_descubrimiento`] previene entre `document_set()` y
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

// ---------------------------------------------------------------------------
// Criterio 11 (regresión): los patrones de FICHERO de los ignore también aplican
// ---------------------------------------------------------------------------

/// **Dado** un `.gitignore` con patrones **de fichero** (`secreto.md`, `*.local.md`,
/// `docs/api/*.md`), **Cuando** se descubre, **Entonces** ninguno de esos documentos entra en el
/// inventario.
///
/// ## Por qué hace falta un test aparte de `respeta_gitignore`
///
/// `respeta_gitignore` usa `vendor/` — un patrón **de directorio**. Y un patrón de directorio puede
/// funcionar por la razón equivocada: `ignore` no aplica la whitelist de un `Override` a los
/// directorios, así que el directorio se poda antes de descender y sus ficheros no llegan a
/// evaluarse nunca. Los patrones **de fichero** no tienen esa red: se evalúan fichero a fichero,
/// después del `Override`, y en `ignore` *cualquier* match del `Override` —whitelist incluida—
/// cortocircuita y decide (`dir.rs`: «Overrides have the highest precedence»).
///
/// Consecuencia: una implementación que meta el `include` (`**/*.md`) como whitelist del `Override`
/// deja **todo** `.md` whitelisteado antes de que se consulte ningún fichero de ignore, y los
/// patrones de fichero dejan de aplicarse **por completo** — en cualquier proyecto, con `.git/` o
/// sin él. Es una regresión silenciosa (nada falla; simplemente el `.gitignore` deja de valer para
/// ficheros) que `respeta_gitignore` no puede ver.
///
/// Se cubren las tres formas de patrón de fichero que un usuario escribe de verdad:
/// nombre exacto en la raíz, comodín de sufijo, y comodín acotado a un directorio.
#[test]
fn respeta_gitignore_con_patrones_de_fichero() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();

    let ignorados = [
        "secreto.md",           // nombre exacto
        "notas.local.md",       // `*.local.md`
        "docs/api/generado.md", // `docs/api/*.md`
        "one/apuntes.local.md", // el comodín aplica a cualquier profundidad
    ];
    for rel in ignorados {
        let destino = dir.path().join(rel);
        std::fs::create_dir_all(destino.parent().unwrap()).unwrap();
        std::fs::write(&destino, "# No debe entrar en el inventario\n").unwrap();
    }
    // Vecino del patrón acotado que SÍ debe entrar: `docs/api/*.md` no alcanza a `docs/guia.md`.
    std::fs::write(dir.path().join("docs/guia.md"), "# Guía\n").unwrap();
    std::fs::write(
        dir.path().join(".gitignore"),
        "secreto.md\n*.local.md\ndocs/api/*.md\n",
    )
    .unwrap();

    let d = discover(dir.path(), &politica()).unwrap();

    for rel in ignorados {
        assert!(
            !contiene(&d.files, rel),
            "`{rel}` está cubierto por un patrón de FICHERO del `.gitignore`: no puede entrar en \
             el inventario. Si entra, el `include` de la política está actuando como whitelist del \
             `Override` y cortocircuitando los ficheros de ignore. Inventario: {:?}",
            rutas(&d.files)
        );
    }
    assert_eq!(
        rutas(&d.files),
        vec![
            "README.md",
            "docs/guia.md",
            "one/first.md",
            "three/levels/deep/third.md",
            "two/levels/second.md",
        ],
        "…y respetar los patrones de fichero no puede llevarse por delante ningún documento que \
         el `.gitignore` no nombra (`docs/api/*.md` no alcanza a `docs/guia.md`)"
    );
}

/// La misma regresión, por el otro fichero de exclusiones: **Dado** un `.lodestarignore` con
/// patrones de fichero, **Cuando** se descubre, **Entonces** esos documentos quedan fuera.
///
/// Va aparte de `respeta_lodestarignore` (que usa `borradores/`, un patrón de directorio) por la
/// misma razón que su gemelo de `.gitignore`: el patrón de directorio sobrevive por accidente al
/// cortocircuito del `Override`, el de fichero no. Y va aparte del gemelo porque son **dos
/// matchers distintos** de `ignore` (`git_ignore` vs `add_custom_ignore_filename`): arreglar uno no
/// arregla el otro, y `.lodestarignore` es además el único mecanismo de exclusión por fichero que
/// le queda a un proyecto que no usa git.
///
/// Se comprueba además que sigue siendo **independiente** del `.gitignore`: el escenario declara
/// los dos ficheros con patrones distintos y exige que ambos se apliquen.
#[test]
fn respeta_lodestarignore_con_patrones_de_fichero() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();

    for rel in ["privado.md", "one/apuntes.wip.md", "two/levels/ignorado.md"] {
        std::fs::write(dir.path().join(rel), "# No debe entrar en el inventario\n").unwrap();
    }
    std::fs::write(dir.path().join("del-gitignore.md"), "# Tampoco\n").unwrap();
    std::fs::write(
        dir.path().join(".lodestarignore"),
        "privado.md\n*.wip.md\ntwo/levels/ignorado.md\n",
    )
    .unwrap();
    // Los dos mecanismos son independientes y deben aplicarse a la vez.
    std::fs::write(dir.path().join(".gitignore"), "del-gitignore.md\n").unwrap();

    let d = discover(dir.path(), &politica()).unwrap();

    assert_eq!(
        rutas(&d.files),
        vec![
            "README.md",
            "one/first.md",
            "three/levels/deep/third.md",
            "two/levels/second.md",
        ],
        "los patrones de FICHERO del `.lodestarignore` (nombre exacto, comodín y ruta concreta) \
         deben excluir sus documentos, y el `.gitignore` seguir aplicándose en paralelo"
    );
}

/// El `exclude` de la política **gana** a los ficheros de ignore, también cuando estos
/// *whitelistean* explícitamente el documento.
///
/// Es la mitad que el arreglo de la regresión no puede llevarse por delante: `exclude` es política
/// explícita del usuario y por eso vive en el `Override` del walker, que tiene la precedencia más
/// alta. Un `.gitignore` con `!secreto.md` (un des-ignore) no puede resucitar lo que la política
/// excluyó.
#[test]
fn exclude_gana_a_los_ficheros_de_ignore() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();
    std::fs::write(
        dir.path().join("secreto.md"),
        "# Excluido por la política\n",
    )
    .unwrap();
    // El `.gitignore` intenta lo contrario de lo que dice la política: primero lo ignora, luego lo
    // des-ignora. Gane quien gane dentro del `.gitignore`, la política manda.
    std::fs::write(dir.path().join(".gitignore"), "secreto.md\n!secreto.md\n").unwrap();

    let d = discover(
        dir.path(),
        &DiscoveryPolicy {
            exclude: vec!["secreto.md".to_string()],
            max_document_bytes: LIMITE,
            ..DiscoveryPolicy::default()
        },
    )
    .unwrap();

    assert!(
        !contiene(&d.files, "secreto.md"),
        "el `exclude` de la política es explícito y tiene la precedencia más alta: ni un \
         `!secreto.md` del `.gitignore` puede reabrirlo. Inventario: {:?}",
        rutas(&d.files)
    );
}

// ---------------------------------------------------------------------------
// E23-H09 · Bordes — **Unicode en rutas**
//
// Cero cobertura hasta esta historia, y es el primo hermano de `colision_capitalizacion`: ahí el
// eje era la capitalización (propiedad del VOLUMEN), aquí es la **forma de normalización** Unicode
// (propiedad del volumen Y de quien escribe el enlace). El CI corre en macOS (APFS), Linux (bytes
// crudos) y Windows (UTF-16), así que estos tests **sondean el disco en tiempo de ejecución** en
// vez de decidir por `cfg!(target_os)` — misma técnica que `fs_case_sensitive`.
//
// Son cobertura que faltaba, no fase roja: se espera que pasen.
// ---------------------------------------------------------------------------

/// `café.md` en **NFC** (una sola code unit `é` = U+00E9). Es lo que teclea un humano en macOS o
/// Linux con un teclado normal y lo que produce la mayoría de editores.
const CAFE_NFC: &str = "caf\u{e9}.md";

/// `café.md` en **NFD** (`e` + U+0301, acento combinante). Visualmente idéntico al anterior,
/// **distinto** byte a byte: 9 bytes contra 8.
const CAFE_NFD: &str = "cafe\u{301}.md";

/// Escribe un fichero del workspace creando los directorios intermedios.
fn escribe(dir: &std::path::Path, rel: &str, contenido: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().expect("ruta con padre")).expect("crear directorios");
    std::fs::write(p, contenido).expect("escribir fichero");
}

/// Nombre **tal y como el volumen lo devuelve** por `read_dir` para el fichero que se escribió con
/// el nombre `escrito`. En APFS/ext4/NTFS es el mismo (se preserva la forma dada); en HFS+ vuelve
/// normalizado a NFD. Es el eje que estos tests no pueden asumir y por eso lo miden.
fn nombre_en_disco(raiz: &std::path::Path, escrito: &str) -> String {
    let candidatas: Vec<String> = std::fs::read_dir(raiz)
        .expect("listar la raíz")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md") && n.starts_with("caf"))
        .collect();
    assert_eq!(
        candidatas.len(),
        1,
        "se escribió UN «café.md» (como {:?}); el volumen devuelve {candidatas:?}",
        escrito.as_bytes()
    );
    candidatas.into_iter().next().unwrap()
}

/// Percent-encoding de los bytes no ASCII de un nombre de fichero: es la forma en que muchos
/// editores (y GitHub) escriben un href con acentos, y la que ejercita el `percent_decode` del
/// core sobre UTF-8 multibyte.
fn percent_encode(nombre: &str) -> String {
    nombre
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'/') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Los diagnósticos de `origen` en el análisis del workspace, como pares `(código, related)`.
fn diagnosticos_de(a: &lodestar_core::types::Analysis, origen: &str) -> Vec<(String, Vec<String>)> {
    a.diagnostics
        .get(&RelPath::new(origen).expect("ruta válida"))
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|c| {
            (
                c.code.as_str().to_string(),
                c.related.iter().map(|r| r.as_str().to_string()).collect(),
            )
        })
        .collect()
}

/// **Dado** documentos con nombres no ASCII (acento, ideogramas CJK, emoji) enlazados desde otro
/// documento, **Cuando** se descubre y se analiza el workspace, **Entonces** los tres entran en el
/// inventario con su ruta exacta y sus tres enlaces **resuelven** (ningún `LINK-TARGET-MISSING`),
/// tanto escritos en literal como percent-encodeados.
///
/// El nombre acentuado se escribe en NFC y se **relee del disco** antes de componer el href: así el
/// criterio («el enlace resuelve») se mide en las tres plataformas sin asumir qué forma preserva el
/// volumen. La divergencia NFC/NFD es el sujeto del test siguiente, no de este.
///
/// Anti-vacuo: el mismo documento enlaza además a un `café-que-no-existe.md`, que **sí** debe salir
/// como `LINK-TARGET-MISSING`. Sin esa mitad, un análisis que no mirara los enlaces pasaría igual.
#[test]
fn unicode_en_rutas() {
    use lodestar_core::types::LinkTarget;

    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        CAFE_NFC,
        "# Café\n\nnota con acento en el nombre.\n",
    );
    escribe(
        dir.path(),
        "日本語.md",
        "# 日本語\n\nnota con ideogramas.\n",
    );
    escribe(
        dir.path(),
        "notas/🚀-lanzamiento.md",
        "# Lanzamiento\n\nnota con emoji en el nombre.\n",
    );

    // La forma que el volumen devuelve de verdad (ver `nombre_en_disco`).
    let cafe = nombre_en_disco(dir.path(), CAFE_NFC);
    eprintln!(
        "[unicode] escrito {:?} → el volumen devuelve {:?} (preserva NFC: {})",
        CAFE_NFC.as_bytes(),
        cafe.as_bytes(),
        cafe == CAFE_NFC
    );
    assert!(
        cafe == CAFE_NFC || cafe == CAFE_NFD,
        "el nombre en disco debe ser una de las dos formas de «café.md», no {:?}",
        cafe.as_bytes()
    );

    let cafe_encoded = percent_encode(&cafe);
    assert_ne!(
        cafe_encoded, cafe,
        "el percent-encoding debe cambiar algo (si no, el enlace codificado no probaría nada)"
    );
    escribe(
        dir.path(),
        "enlazador.md",
        &format!(
            "# Enlazador\n\n\
             * [Café literal]({cafe})\n\
             * [Café codificado]({cafe_encoded})\n\
             * [Japonés](日本語.md)\n\
             * [Lanzamiento](notas/🚀-lanzamiento.md)\n\
             * [Café inexistente](caf\u{e9}-que-no-existe.md)\n"
        ),
    );

    // --- Descubrimiento: los tres nombres no ASCII entran con su ruta EXACTA ------------------
    let d = discover(dir.path(), &politica()).unwrap();
    for esperado in [cafe.as_str(), "日本語.md", "notas/🚀-lanzamiento.md"] {
        assert!(
            contiene(&d.files, esperado),
            "un nombre no ASCII no puede quedarse fuera del inventario (falta {:?}): {:?}",
            esperado.as_bytes(),
            rutas(&d.files)
        );
    }
    assert!(
        d.diagnostics.is_empty(),
        "un nombre no ASCII no es un problema de descubrimiento (ni PATH-NOT-UTF8 ni nada): {}",
        resumen(&d.diagnostics)
    );

    // --- Análisis: los enlaces a esos nombres RESUELVEN --------------------------------------
    let ws = lodestar_workspace::Workspace::open(dir.path()).expect("abrir el workspace");
    let a = ws.analyze().expect("analizar el workspace");
    let salientes = a
        .outgoing
        .get(&RelPath::new("enlazador.md").unwrap())
        .expect("«enlazador.md» debe estar entre los documentos analizados");
    let destino = |href: &str| -> LinkTarget {
        salientes
            .iter()
            .find(|l| l.href == href)
            .unwrap_or_else(|| {
                let vistos: Vec<&str> = salientes.iter().map(|l| l.href.as_str()).collect();
                panic!("«enlazador.md» debe tener un saliente con href «{href}»; tiene {vistos:?}")
            })
            .target
            .clone()
    };
    let doc = |p: &str| LinkTarget::Document(RelPath::new(p).unwrap());

    assert_eq!(
        destino(&cafe),
        doc(&cafe),
        "el enlace literal al documento acentuado debe resolver a ese documento"
    );
    assert_eq!(
        destino(&cafe_encoded),
        doc(&cafe),
        "el enlace percent-encodeado ({cafe_encoded}) debe decodificarse a UTF-8 y resolver al \
         mismo documento"
    );
    assert_eq!(
        destino("日本語.md"),
        doc("日本語.md"),
        "el enlace a un documento con ideogramas debe resolver"
    );
    assert_eq!(
        destino("notas/🚀-lanzamiento.md"),
        doc("notas/🚀-lanzamiento.md"),
        "el enlace a un documento con emoji, en un subdirectorio, debe resolver"
    );

    // --- Anti-vacuo: el único enlace roto del documento es el que se rompió a propósito -------
    let diags = diagnosticos_de(&a, "enlazador.md");
    let perdidos: Vec<&(String, Vec<String>)> = diags
        .iter()
        .filter(|(code, _)| code == "LINK-TARGET-MISSING")
        .collect();
    assert_eq!(
        perdidos.len(),
        1,
        "solo el enlace inventado debe faltar; los cuatro no ASCII resuelven. Diagnósticos: \
         {diags:?}"
    );
    assert_eq!(
        perdidos[0].1,
        vec!["caf\u{e9}-que-no-existe.md".to_string()],
        "y el que falta es exactamente el inventado (con su acento intacto en el `related`)"
    );
}

/// **Dado** un documento cuyo nombre está en una forma de normalización Unicode y un enlace que lo
/// apunta en **la otra**, **Cuando** se analiza el workspace, **Entonces** el enlace resuelve a la
/// ruta real y el diagnóstico es un **aviso de portabilidad** (`LINK-CASE-MISMATCH`), nunca un
/// `LINK-TARGET-MISSING` bloqueante — E23-H23.
///
/// # Qué defecto cierra
///
/// Hasta E23-H23 Lodestar comparaba rutas **byte a byte**, así que `caf\u{e9}.md` (NFC, 8 bytes) y
/// `cafe\u{301}.md` (NFD, 9 bytes) eran dos rutas distintas. Consecuencias por plataforma, medidas
/// por la sonda que este test imprime:
///
/// - **Linux/ext4** (bytes crudos): el fichero NFD de verdad no existe.
/// - **macOS/APFS** (preserva la forma escrita pero compara *normalization-insensitive*): el
///   fichero **sí se abre** con el nombre en la otra forma.
///
/// O sea que el veredicto era idéntico en las dos plataformas pero **acertado solo en una**: en
/// macOS, un `LINK-TARGET-MISSING` sobre un `.md` es `Err` y **tumbaba la puerta de CI** por un
/// enlace que el sistema operativo y GitHub resuelven. El disparador no es rebuscado: basta con que
/// el fichero lo cree un `git checkout` en macOS y el enlace lo teclee alguien en Linux, o al revés.
///
/// # Por qué se arregló así
///
/// **No** normalizando la ruta canónica: en Linux el fichero está literalmente en NFD, así que un
/// `RelPath` reescrito a NFC dejaría de poder abrirlo — sería peor que el bug. Lo que normaliza a
/// NFC es la **clave de búsqueda tolerante** del inventario (`fold_path`), exactamente igual que ya
/// se hacía con las mayúsculas desde E17-H03. Por eso el diagnóstico resultante es el mismo que el
/// de capitalización: son el mismo problema —dos textos que designan el mismo fichero para el SO
/// pero diferen byte a byte— y merecen el mismo aviso de portabilidad.
#[test]
fn unicode_nfc_y_nfd_resuelven_con_aviso() {
    use lodestar_core::types::{LinkTarget, Severity};

    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        CAFE_NFC,
        "# Café\n\nel documento realmente existe.\n",
    );

    let en_disco = nombre_en_disco(dir.path(), CAFE_NFC);
    // La forma CONTRARIA a la que el volumen guarda: es la que escribirá el enlace.
    let la_otra = if en_disco == CAFE_NFC {
        CAFE_NFD
    } else {
        CAFE_NFC
    };
    let abre_con_la_otra = dir.path().join(la_otra).is_file();
    eprintln!(
        "[unicode-nfc-nfd] en disco={:?} · enlace={:?} · ¿el SO abre el fichero con la forma del \
         enlace? {abre_con_la_otra}",
        en_disco.as_bytes(),
        la_otra.as_bytes()
    );

    escribe(
        dir.path(),
        "enlazador.md",
        &format!("# Enlazador\n\n[Café en la otra forma]({la_otra})\n"),
    );

    let ws = lodestar_workspace::Workspace::open(dir.path()).expect("abrir el workspace");
    let a = ws.analyze().expect("analizar el workspace");

    // El documento SÍ está en el inventario (el test no va de un fichero que falte).
    assert!(
        a.diagnostics.keys().any(|p| p.as_str() == en_disco)
            || a.outgoing.keys().any(|p| p.as_str() == en_disco),
        "«café.md» debe estar entre los documentos analizados (inventario: {:?})",
        a.outgoing.keys().map(RelPath::as_str).collect::<Vec<_>>()
    );

    let saliente = a
        .outgoing
        .get(&RelPath::new("enlazador.md").unwrap())
        .and_then(|ls| ls.first())
        .expect("«enlazador.md» debe tener su único enlace resuelto");
    // El TARGET sigue siendo `Missing`: la ruta tecleada no está en el inventario byte a byte, y
    // el modelo no reescribe lo que el usuario escribió. Es exactamente lo que ya ocurría con la
    // capitalización desde E17-H03 (`Docs/Auth.md` contra `docs/auth.md`), y la coherencia importa:
    // la tolerancia vive en el DIAGNÓSTICO, no en la clasificación.
    assert_eq!(
        saliente.target,
        LinkTarget::Missing(RelPath::new(la_otra).unwrap()),
        "la clasificación es byte a byte y no reescribe el href del usuario \
         (¿el SO lo abre?: {abre_con_la_otra})"
    );

    let checks = a
        .diagnostics
        .get(&RelPath::new("enlazador.md").unwrap())
        .expect("un enlace que difiere en la forma Unicode debe avisar de portabilidad");
    let aviso = checks
        .iter()
        .find(|c| c.code.as_str() == "LINK-CASE-MISMATCH")
        .unwrap_or_else(|| {
            panic!(
                "se esperaba LINK-CASE-MISMATCH; diagnósticos: {}",
                resumen(checks)
            )
        });
    assert_eq!(
        aviso.level,
        Severity::Warn,
        "es un aviso de PORTABILIDAD, no un hard-fail: el enlace funciona aquí, pero puede no \
         funcionar en otro sistema de ficheros"
    );
    assert_eq!(
        aviso.related.first().map(RelPath::as_str),
        Some(en_disco.as_str()),
        "el aviso debe señalar la ruta REAL, que es la pista que el motor no daba antes"
    );

    // LA propiedad que cierra el defecto: ni un solo `Err`. Antes de E23-H23 aquí había un
    // `LINK-TARGET-MISSING` de severidad `Err` que hacía salir `lodestar check` con 1 en macOS.
    assert!(
        !checks.iter().any(|c| c.level == Severity::Err),
        "un enlace que difiere solo en la forma Unicode NO puede tumbar la puerta de CI. \
         Diagnósticos: {}",
        resumen(checks)
    );
    assert!(
        !checks
            .iter()
            .any(|c| c.code.as_str() == "LINK-TARGET-MISSING"),
        "y no se emite además un destino ausente: un solo diagnóstico por enlace. Diagnósticos: {}",
        resumen(checks)
    );
}

// ---------------------------------------------------------------------------
// E24-H12 — Los huecos de descubrimiento dejan de ser silenciosos
//
// Tres agujeros declarados como «no regresiones» desde el juez de E15-H07, pero vivos y observables
// sobre un árbol real. Los tres compartían la misma forma: Lodestar NO veía algo y no lo decía.
// ---------------------------------------------------------------------------

/// **E24-H12** — un `.md` con la extensión en mayúsculas se descubre.
///
/// `**/*.md` es un glob sensible a la capitalización, así que `README.MD` —lo que escribe media
/// cadena de herramientas de Windows— quedaba invisible: ni en el inventario, ni consultable, y un
/// enlace a él resolvía como `Missing`. En un volumen case-insensitive es además literalmente el
/// mismo fichero que `README.md`.
#[test]
fn extension_en_mayusculas_se_descubre() {
    let dir = tempfile::tempdir().unwrap();
    escribe(dir.path(), "README.MD", "# Léeme\n");
    escribe(dir.path(), "normal.md", "# Normal\n");

    let d = discover(dir.path(), &politica()).unwrap();
    let paths: Vec<String> = d.files.keys().map(|p| p.as_str().to_string()).collect();

    assert!(
        paths.iter().any(|p| p == "README.MD"),
        "un `.md` con la extensión en mayúsculas es Markdown y debe entrar en el inventario: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p == "normal.md"),
        "control anti-vacuo: el descubrimiento normal sigue funcionando: {paths:?}"
    );
}

/// **E24-H12** — un `include` personalizado del usuario se sigue respetando.
///
/// Control anti-vacuo del anterior: la tolerancia solo afecta a la capitalización de la EXTENSIÓN,
/// no convierte el filtro en «todo vale».
#[test]
fn tolerancia_de_extension_no_rompe_el_include() {
    let dir = tempfile::tempdir().unwrap();
    escribe(dir.path(), "docs/dentro.md", "# Dentro\n");
    escribe(dir.path(), "fuera.md", "# Fuera\n");
    let politica_docs = lodestar_workspace::discovery::DiscoveryPolicy {
        include: vec!["docs/**/*.md".to_string()],
        ..politica()
    };

    let d = discover(dir.path(), &politica_docs).unwrap();
    let paths: Vec<String> = d.files.keys().map(|p| p.as_str().to_string()).collect();
    assert!(
        paths.iter().any(|p| p == "docs/dentro.md"),
        "lo que casa el include entra: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p == "fuera.md"),
        "lo que NO casa el include del usuario sigue fuera: {paths:?}"
    );
}

/// **E24-H12** — un symlink de DIRECTORIO se diagnostica.
///
/// No se sigue (política), pero antes tampoco se decía: como no acaba en `.md`, no pasaba el filtro
/// `include` y se iba a `other_files` en silencio, ocultando todos los documentos que hubiera
/// detrás. Un symlink a un fichero suelto ya se diagnosticaba desde E15-H07.
#[cfg(unix)]
#[test]
fn symlink_de_directorio_diagnostica() {
    let dir = tempfile::tempdir().unwrap();
    escribe(dir.path(), "real/hoja.md", "# Hoja\n");
    escribe(dir.path(), "raiz.md", "# Raíz\n");
    std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("atajo")).unwrap();

    let d = discover(dir.path(), &politica()).unwrap();
    let codigos: Vec<&str> = d.diagnostics.iter().map(|c| c.code.as_str()).collect();

    assert!(
        codigos.contains(&"SYMLINK-UNSUPPORTED"),
        "un symlink a un directorio oculta TODOS los documentos que haya detrás: el usuario tiene \
         que enterarse. Diagnósticos: {codigos:?}"
    );
    let paths: Vec<String> = d.files.keys().map(|p| p.as_str().to_string()).collect();
    assert!(
        !paths.iter().any(|p| p.starts_with("atajo/")),
        "y sigue sin seguirse (la política no cambia): {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p == "real/hoja.md"),
        "control anti-vacuo: el documento real, alcanzado por su ruta de verdad, sí está: {paths:?}"
    );
}

/// **E24-H12** — en Unix, `\` es un carácter legítimo del nombre y no se traduce a separador.
///
/// Traducirlo convertía un fichero llamado literalmente `a\b.md` en la ruta `a/b.md`, que puede
/// **enmascarar un documento real** de ese path.
#[cfg(unix)]
#[test]
fn barra_invertida_en_unix_no_enmascara() {
    let dir = tempfile::tempdir().unwrap();
    escribe(dir.path(), "a/b.md", "# El documento REAL\n");
    // Un único fichero en la raíz cuyo NOMBRE contiene una barra invertida.
    std::fs::write(dir.path().join("a\\b.md"), "# El impostor\n").unwrap();

    let d = discover(dir.path(), &politica()).unwrap();
    let real = d
        .files
        .iter()
        .find(|(p, _)| p.as_str() == "a/b.md")
        .map(|(_, c)| c.clone());

    assert_eq!(
        real.as_deref(),
        Some("# El documento REAL\n"),
        "`a/b.md` debe seguir siendo el documento real: si la barra invertida se tradujera a \
         separador, el fichero `a\\b.md` de la raíz lo enmascararía. Inventario: {:?}",
        d.files.keys().map(|p| p.as_str()).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// E29-H06 (remate del juez ciego, MENOR-4) — el productor de `WORKSPACE-EMPTY`, a nivel de
// `discover()` directamente. Hasta esta sección la cobertura del código vivía toda aguas abajo
// (`lodestar-app`/`lodestar-cli`/`lodestar-mcp`); este test fija el contrato del PRODUCTOR mismo,
// sin pasar por `full_analysis` ni por su indexado.
// `requirements/epica-29-honestidad-superficie.md §E29-H06` · `decisiones §16(f)`.
// ---------------------------------------------------------------------------

/// **Dado** un directorio vacío (sin ningún fichero), **Cuando** se llama a `discover()`,
/// **Entonces** `Discovered::diagnostics` trae exactamente un `Check` con código `WORKSPACE-EMPTY`,
/// severidad `warn` y **sin `targets`** (no describe un fichero, describe la ausencia de todos —
/// `RelPath::new("")` es inválido por diseño, invariante #6 de `CLAUDE.md`).
#[test]
fn inventario_vacio_produce_workspace_empty() {
    let dir = tempfile::tempdir().unwrap();

    let d = discover(dir.path(), &politica()).unwrap();

    assert!(
        d.files.is_empty(),
        "precondición: el inventario de documentos debe estar vacío de verdad"
    );
    let coincidencias: Vec<&Check> = d
        .diagnostics
        .iter()
        .filter(|c| c.code.as_str() == "WORKSPACE-EMPTY")
        .collect();
    assert_eq!(
        coincidencias.len(),
        1,
        "un directorio vacío debe producir EXACTAMENTE un diagnóstico WORKSPACE-EMPTY (no cero, \
         no varios): {}",
        resumen(&d.diagnostics)
    );
    let check = coincidencias[0];
    assert_eq!(
        check.level,
        lodestar_core::types::Severity::Warn,
        "WORKSPACE-EMPTY debe ser un AVISO: un directorio vacío sigue siendo un workspace válido \
         (§20.1). {check:?}"
    );
    assert!(
        check.targets.is_empty(),
        "WORKSPACE-EMPTY no describe un fichero: debe ir SIN `targets` (como PATH-NOT-UTF8), no \
         con un `RelPath` sintético de la raíz (`RelPath::new(\"\")` es inválido por invariante \
         #6): {check:?}"
    );
}

/// **Dado** un workspace con documentos, **Cuando** se llama a `discover()`, **Entonces** NO
/// aparece `WORKSPACE-EMPTY` — control anti-vacuo del productor: el diagnóstico depende de que el
/// inventario quede REALMENTE vacío, no se dispara siempre.
#[test]
fn inventario_con_documentos_no_produce_workspace_empty() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();

    let d = discover(dir.path(), &politica()).unwrap();

    assert!(
        !d.files.is_empty(),
        "precondición: el fixture `arbitrary()` debe dejar documentos en el inventario"
    );
    assert!(
        !d.diagnostics
            .iter()
            .any(|c| c.code.as_str() == "WORKSPACE-EMPTY"),
        "un workspace con documentos NO debe producir WORKSPACE-EMPTY: {}",
        resumen(&d.diagnostics)
    );
}
