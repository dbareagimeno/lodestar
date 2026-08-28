//! Red CI23: el fallback SMB con `RootDirectory = NULL` solo recibe un destino absoluto si
//! `Store::open` fija esa propiedad antes de derivar `.lodestar/index.db`.

use lodestar_store::Store;

const STORE_SOURCE: &str = include_str!("../src/lib.rs");
const WINDOWS_VFS_SOURCE: &str = include_str!("../src/windows_vfs.rs");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start_at = source
        .find(start)
        .unwrap_or_else(|| panic!("guarda anti-vacuidad: no aparece el inicio {start:?}"));
    let tail = &source[start_at..];
    let end_at = tail
        .find(end)
        .unwrap_or_else(|| panic!("guarda anti-vacuidad: no aparece el final {end:?}"));
    &tail[..end_at]
}

fn absolute_publication_contract(store_source: &str, windows_source: &str) -> Result<(), String> {
    let open = section(
        store_source,
        "    pub fn open(root: &Path) -> Result<Self, StoreError> {",
        "\n    /// Abre la cache y la reconstruye",
    );
    let rename = section(
        windows_source,
        "fn rename_handle_to(",
        "\nfn wide_path(path: &Path)",
    );
    let swap = section(
        store_source,
        "    fn swap_active(",
        "\n    /// Reabre la conexión compartida",
    );
    let store_replace = section(
        store_source,
        "#[cfg(windows)]\nfn replace_durable(",
        "\n\n#[cfg(not(windows))]",
    );
    let windows_replace = section(
        windows_source,
        "pub(crate) fn replace_durable(candidate: PreparedCandidate, active: &Path)",
        "\n}\n\n/// Rust 1.80",
    );

    let absolute_at = open
        .find("std::path::absolute(root)")
        .ok_or("Store::open debe normalizar léxicamente el root con std::path::absolute")?;
    let cache_at = open
        .find("root.join(CACHE_DIR)")
        .ok_or("guarda anti-vacuidad: Store::open ya no deriva CACHE_DIR desde root")?;
    if absolute_at >= cache_at {
        return Err("el root debe ser absoluto antes de construir .lodestar".into());
    }
    if open.contains("canonicalize(") {
        return Err(
            "la normalización requerida es léxica; canonicalize seguiría enlaces y exige existencia"
                .into(),
        );
    }

    let active_binding = "let active = self.root.join(CACHE_DIR).join(DB_FILE);";
    let active_at = swap.find(active_binding).ok_or(
        "swap_active debe derivar active directamente del root absoluto conservado por Store",
    )?;
    let replace_at = swap
        .find("replace_durable(candidate, &active)")
        .ok_or("swap_active debe pasar exactamente &active, no otro path, al replace Windows")?;
    if active_at >= replace_at {
        return Err("active absoluto debe declararse antes del replace Windows".into());
    }
    let active_corridor = &swap[active_at..replace_at];
    if active_corridor.matches("let active").count() != 1 {
        return Err(
            "active no puede sombrearse entre su derivación desde el root absoluto y replace_durable"
                .into(),
        );
    }
    if !swap.contains("replace_durable(candidate, &active)") {
        return Err(
            "swap_active debe pasar exactamente &active, no otro path, al replace Windows".into(),
        );
    }
    if !store_replace.contains("windows_vfs::replace_durable(candidate, active)") {
        return Err(
            "el wrapper Windows debe reenviar sin sustituir el mismo active derivado del root"
                .into(),
        );
    }
    if !windows_replace.contains("rename_handle_to(\n        active,\n        candidate.handle.0,")
    {
        return Err(
            "windows_vfs::replace_durable debe entregar exactamente active como target real de rename_handle_to"
                .into(),
        );
    }

    let fallback_at = rename
        .find("let mut wide = wide_path(target);")
        .ok_or("guarda anti-vacuidad: falta la serialización del destino del fallback")?;
    let fallback = &rename[fallback_at..];
    if !fallback.contains("(*info).RootDirectory = ptr::null_mut()") {
        return Err("guarda anti-vacuidad: el fallback ya no usa RootDirectory = NULL".into());
    }
    Ok(())
}

/// C5/C6 — un root relativo aceptado por la API se fija como absoluto de forma léxica; por tanto
/// todas las rutas internas, incluido el destino que serializa el fallback SMB, son absolutas.
#[test]
fn c5_c6_store_open_absolutiza_root_relativo_antes_de_derivar_cache() {
    let cwd = std::env::current_dir().expect("directorio de trabajo del proceso de test");
    let sandbox = tempfile::Builder::new()
        .prefix("lodestar-ci23-relative-root-")
        .tempdir_in(&cwd)
        .expect("sandbox temporal dentro del cwd para obtener un root relativo portable");
    let relative_root = sandbox
        .path()
        .strip_prefix(&cwd)
        .expect("guarda anti-vacuidad: el sandbox debe ser descendiente del cwd");
    assert!(
        !relative_root.is_absolute() && !relative_root.as_os_str().is_empty(),
        "guarda anti-vacuidad: el input de Store::open debe ser relativo y no vacío: {relative_root:?}"
    );

    let expected_root = std::path::absolute(relative_root)
        .expect("la misma normalización léxica exigida a Store::open");
    let store = Store::open(relative_root).expect("Store::open acepta roots relativos existentes");

    assert!(
        store.root().is_absolute(),
        "C5/C6: Store::open conservó un root relativo; wide_path(target) no lo convierte en absoluto: {:?}",
        store.root()
    );
    assert_eq!(
        store.root(),
        expected_root,
        "C5/C6: la normalización debe ser léxica y preservar la identidad del root"
    );
    assert!(
        store.root().join(".lodestar/index.db").is_absolute(),
        "C5/C6: el target interno del fallback RootDirectory=NULL debe ser absoluto"
    );
}

/// Negativo C5/C6 — no basta con serializar `target` mediante `wide_path`: el contrato se rompe
/// si se elimina la absolutización previa de `Store::open`, incluso aunque el fallback quede igual.
#[test]
fn c5_c6_contrafactual_rechaza_wide_path_sin_root_absoluto() {
    absolute_publication_contract(STORE_SOURCE, WINDOWS_VFS_SOURCE)
        .expect("C5/C6: Store::open debe establecer la precondición absoluta del fallback SMB");

    let without_absolute_root = STORE_SOURCE.replacen("std::path::absolute(root)", "Ok(root)", 1);
    assert_ne!(
        without_absolute_root, STORE_SOURCE,
        "guarda anti-vacuidad: la mutación debe encontrar la absolutización real"
    );
    assert!(
        absolute_publication_contract(&without_absolute_root, WINDOWS_VFS_SOURCE).is_err(),
        "contrafactual: wide_path(target) con RootDirectory=NULL no puede suplir un target relativo"
    );
}

/// Negativo C5/C6 — absolutizar `Store::root` no sirve si cualquier salto de la publicación
/// sustituye `active`: el argumento efectivo de `rename_handle_to` debe seguir siendo ese mismo
/// destino absoluto, no un literal relativo con el mismo nombre de fichero.
#[test]
fn c5_c6_contrafactual_rechaza_target_relativo_en_el_rename_real() {
    absolute_publication_contract(STORE_SOURCE, WINDOWS_VFS_SOURCE)
        .expect("guarda anti-vacuidad: el fuente real debe conservar la cadena causal de active");

    let relative_at_swap = STORE_SOURCE.replacen(
        "replace_durable(candidate, &active)",
        "replace_durable(candidate, Path::new(\"index.db\"))",
        1,
    );
    assert_ne!(
        relative_at_swap, STORE_SOURCE,
        "guarda anti-vacuidad: la mutación debe encontrar el target del swap Windows"
    );
    let swap_error = absolute_publication_contract(&relative_at_swap, WINDOWS_VFS_SOURCE)
        .expect_err("un root absoluto no legitima sustituir &active por index.db relativo");
    assert!(
        swap_error.contains("pasar exactamente &active"),
        "la mutación del swap fue rechazada por otra causa: {swap_error}"
    );

    for relative_target in ["index.db", "other.db"] {
        let relative_at_syscall = WINDOWS_VFS_SOURCE.replacen(
            "rename_handle_to(\n        active,\n        candidate.handle.0,",
            &format!(
                "rename_handle_to(\n        Path::new(\"{relative_target}\"),\n        candidate.handle.0,"
            ),
            1,
        );
        assert_ne!(
            relative_at_syscall, WINDOWS_VFS_SOURCE,
            "guarda anti-vacuidad: la mutación debe encontrar el target real de rename_handle_to"
        );
        let syscall_error = absolute_publication_contract(STORE_SOURCE, &relative_at_syscall)
            .expect_err("rename_handle_to no puede recibir un target relativo alternativo");
        assert!(
            syscall_error.contains("exactamente active como target real"),
            "la mutación `{relative_target}` fue rechazada por otra causa: {syscall_error}"
        );
    }
}

/// Negativo C5/C6 — el destino absoluto debe conservar su identidad léxica hasta el replace. Una
/// segunda declaración relativa de `active` no puede aprovechar la presencia del binding válido.
#[test]
fn c5_c6_contrafactual_rechaza_sombrear_active_absoluto_antes_de_replace() {
    absolute_publication_contract(STORE_SOURCE, WINDOWS_VFS_SOURCE)
        .expect("guarda anti-vacuidad: el fuente real debe conservar la cadena causal de active");

    let shadowed_active = STORE_SOURCE.replacen(
        "let active = self.root.join(CACHE_DIR).join(DB_FILE);",
        "let active = self.root.join(CACHE_DIR).join(DB_FILE);\n        let active = PathBuf::from(\"index.db\");",
        1,
    );
    assert_ne!(
        shadowed_active, STORE_SOURCE,
        "guarda anti-vacuidad: la mutación debe insertar el sombreado después de derivar active absoluto"
    );
    assert!(
        shadowed_active.contains(
            "let active = PathBuf::from(\"index.db\");\n        // Keep an unopened-WAL handle"
        ),
        "guarda anti-vacuidad: el contrafactual debe sustituir el binding que usa la publicación"
    );
    let shadow_error = absolute_publication_contract(&shadowed_active, WINDOWS_VFS_SOURCE)
        .expect_err("&active no basta si el binding absoluto fue sombreado por index.db relativo");
    assert!(
        shadow_error.contains("active no puede sombrearse"),
        "la mutación por sombreado fue rechazada por otra causa: {shadow_error}"
    );
}
