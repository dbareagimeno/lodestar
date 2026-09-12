//! E35-H03 CI44 — publicación por ruta Win32 absoluta independiente del cwd en Windows.
//!
//! La forma nativa ratificada conserva drive/UNC, rechaza el prefijo Object Manager `\??\`, usa
//! `RootDirectory=NULL` y un único `FileRenameInfoEx`. No depende del cwd ni abre el padre como
//! `RootDirectory` relativo.

use lodestar_store::Store;

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

fn win32_absolute_publication_contract(windows_source: &str) -> Result<(), String> {
    let rename = section(
        windows_source,
        "fn rename_handle_to(",
        "\nfn wide_path(path: &Path)",
    );

    if rename.matches("SetFileInformationByHandle(").count() != 1 {
        return Err("C5: la publicación debe efectuar un único syscall atómico".into());
    }
    if rename
        .matches("SetFileInformationByHandle(handle, FileRenameInfoEx")
        .count()
        != 1
    {
        return Err(
            "C5: el único syscall debe usar FileRenameInfoEx sobre candidate.handle".into(),
        );
    }
    if rename.contains(r#"\??\"#) {
        return Err(
            "C5/C6: el protocolo no puede anteponer el prefijo Object Manager `\\??\\`".into(),
        );
    }
    for required in [
        "wide_path(target)",
        "windows_rename_path::validate_win32_rename_path(&wide)",
        "(*info).RootDirectory = ptr::null_mut();",
        "(*info).Anonymous.Flags = extended_flags.unwrap_or(0);",
    ] {
        if !rename.contains(required) {
            return Err(format!(
                "C5/C6: falta el paso de publicación Win32 absoluta `{required}`"
            ));
        }
    }
    for forbidden in [
        "target.file_name()",
        "wide_path(Path::new(file_name))",
        "target.parent()",
        "(*info).RootDirectory = parent_handle",
        "CreateFileW(",
        "ERROR_INVALID_PARAMETER",
    ] {
        if rename.contains(forbidden) {
            return Err(format!(
                "C5/C6: el protocolo no puede depender de basename/cwd, parent-handle, prefijo Object Manager o fallback; apareció `{forbidden}`"
            ));
        }
    }
    Ok(())
}

/// C5/C6 — el artefacto Windows materializa el swap en una sola operación con destino Win32
/// absoluto preservado, sin dependencia de la unidad del cwd ni de un handle de directorio.
#[test]
fn c5_c6_windows_publicacion_usa_destino_win32_absoluto_y_root_null() {
    win32_absolute_publication_contract(WINDOWS_VFS_SOURCE)
        .unwrap_or_else(|error| panic!("rojo causal CI35: {error}"));
}

/// La API puede seguir aceptando un root relativo: `Store::open` lo fija léxicamente. Esta
/// propiedad ya no se usa como precondición oculta del syscall, que recibe un destino Win32 absoluto.
#[test]
fn store_open_sigue_fijando_root_relativo_sin_acoplarlo_al_rename_win32() {
    let cwd = std::env::current_dir().expect("directorio de trabajo del proceso de test");
    let sandbox = tempfile::Builder::new()
        .prefix("lodestar-ci35-relative-root-")
        .tempdir_in(&cwd)
        .expect("sandbox temporal descendiente del cwd");
    let relative_root = sandbox
        .path()
        .strip_prefix(&cwd)
        .expect("guarda anti-vacuidad: sandbox descendiente");
    assert!(
        !relative_root.is_absolute() && !relative_root.as_os_str().is_empty(),
        "guarda anti-vacuidad: el input debe ser relativo y no vacío"
    );

    let expected = std::path::absolute(relative_root).expect("normalización léxica esperada");
    let store = Store::open(relative_root).expect("Store::open acepta el root relativo existente");
    assert_eq!(store.root(), expected);
}

/// Negativo C5/C6 — `RootDirectory=NULL` exige una ruta Win32 absoluta y no autoriza convertirla
/// al namespace Object Manager `\??\`.
#[test]
fn c5_c6_contrafactual_rechaza_prefijo_object_manager() {
    win32_absolute_publication_contract(WINDOWS_VFS_SOURCE)
        .expect("guarda anti-vacuidad: el fuente real debe cumplir antes del contrafactual");
    let object_manager = WINDOWS_VFS_SOURCE.replacen(
        "windows_rename_path::validate_win32_rename_path(&wide)",
        r#"Ok([r"\??\".encode_utf16().collect::<Vec<_>>(), wide].concat())"#,
        1,
    );
    assert_ne!(object_manager, WINDOWS_VFS_SOURCE);
    let error = win32_absolute_publication_contract(&object_manager)
        .expect_err("anteponer `\\??\\` debe romper el contrato");
    assert!(
        error.contains("Object Manager") || error.contains(r#"\??\"#),
        "el contrafactual falló por otra causa: {error}"
    );
}

/// Negativo C5/C6 — abrir el padre y usarlo como `RootDirectory` reintroduce la topología cuyo
/// éxito nativo no autenticó el FILE_ID publicado.
#[test]
fn c5_c6_contrafactual_rechaza_parent_handle_con_destino_win32() {
    win32_absolute_publication_contract(WINDOWS_VFS_SOURCE)
        .expect("guarda anti-vacuidad: el fuente real debe cumplir antes del contrafactual");
    let parent_handle = WINDOWS_VFS_SOURCE.replacen(
        "(*info).RootDirectory = ptr::null_mut();",
        "(*info).RootDirectory = parent_handle;",
        1,
    );
    assert_ne!(parent_handle, WINDOWS_VFS_SOURCE);
    let error = win32_absolute_publication_contract(&parent_handle)
        .expect_err("RootDirectory=parent_handle debe romper el contrato");
    assert!(
        error.contains("RootDirectory = ptr::null_mut()")
            || error.contains("RootDirectory = parent_handle"),
        "el contrafactual falló por otra causa: {error}"
    );
}
