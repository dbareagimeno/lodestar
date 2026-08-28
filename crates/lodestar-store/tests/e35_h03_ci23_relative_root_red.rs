//! E35-H03 CI33 — publicación same-directory independiente del cwd en Windows.
//!
//! `.lodestar/index.db.next` y `.lodestar/index.db` comparten directorio por spec. La forma nativa
//! ratificada usa solo `target.file_name()`, `RootDirectory=NULL` y un único `FileRenameInfoEx`:
//! no serializa el path DOS completo y tampoco abre el padre como `RootDirectory` relativo.

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

fn same_directory_publication_contract(windows_source: &str) -> Result<(), String> {
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
    for required in [
        "target.file_name()",
        "wide_path(Path::new(file_name))",
        "(*info).RootDirectory = ptr::null_mut();",
        "(*info).Anonymous.Flags = extended_flags.unwrap_or(0);",
    ] {
        if !rename.contains(required) {
            return Err(format!(
                "C5/C6: falta el paso same-directory ratificado `{required}`"
            ));
        }
    }
    for forbidden in [
        "wide_path(target)",
        "target.parent()",
        "(*info).RootDirectory = parent_handle",
        "CreateFileW(",
        "ERROR_INVALID_PARAMETER",
    ] {
        if rename.contains(forbidden) {
            return Err(format!(
                "C5/C6: el protocolo no puede depender de path completo, parent-handle o fallback; apareció `{forbidden}`"
            ));
        }
    }
    Ok(())
}

/// C5/C6 — el artefacto Windows materializa el swap same-directory en una sola operación, con
/// nombre simple y sin dependencia del cwd ni de un handle de directorio.
#[test]
fn c5_c6_windows_publicacion_same_directory_usa_solo_file_name_y_root_null() {
    same_directory_publication_contract(WINDOWS_VFS_SOURCE)
        .unwrap_or_else(|error| panic!("rojo causal CI33: {error}"));
}

/// La API puede seguir aceptando un root relativo: `Store::open` lo fija léxicamente. Esta
/// propiedad ya no se usa como precondición oculta del syscall, que recibe solo `file_name`.
#[test]
fn store_open_sigue_fijando_root_relativo_sin_acoplarlo_al_rename_win32() {
    let cwd = std::env::current_dir().expect("directorio de trabajo del proceso de test");
    let sandbox = tempfile::Builder::new()
        .prefix("lodestar-ci33-relative-root-")
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

/// Negativo C5/C6 — `RootDirectory=NULL` no autoriza recuperar el path DOS completo: la evidencia
/// nativa que motivó este arbitraje rechazó precisamente esa forma.
#[test]
fn c5_c6_contrafactual_rechaza_wide_path_del_target_completo() {
    same_directory_publication_contract(WINDOWS_VFS_SOURCE)
        .expect("guarda anti-vacuidad: el fuente real debe cumplir antes del contrafactual");
    let full_target =
        WINDOWS_VFS_SOURCE.replacen("wide_path(Path::new(file_name))", "wide_path(target)", 1);
    assert_ne!(full_target, WINDOWS_VFS_SOURCE);
    let error = same_directory_publication_contract(&full_target)
        .expect_err("wide_path(target) debe romper el contrato same-directory");
    assert!(
        error.contains("wide_path(Path::new(file_name))") || error.contains("wide_path(target)"),
        "el contrafactual falló por otra causa: {error}"
    );
}

/// Negativo C5/C6 — abrir el padre y usarlo como `RootDirectory` reintroduce la topología cuyo
/// éxito nativo no autenticó el FILE_ID publicado.
#[test]
fn c5_c6_contrafactual_rechaza_parent_handle_con_nombre_simple() {
    same_directory_publication_contract(WINDOWS_VFS_SOURCE)
        .expect("guarda anti-vacuidad: el fuente real debe cumplir antes del contrafactual");
    let parent_handle = WINDOWS_VFS_SOURCE.replacen(
        "(*info).RootDirectory = ptr::null_mut();",
        "(*info).RootDirectory = parent_handle;",
        1,
    );
    assert_ne!(parent_handle, WINDOWS_VFS_SOURCE);
    let error = same_directory_publication_contract(&parent_handle)
        .expect_err("RootDirectory=parent_handle debe romper el contrato");
    assert!(
        error.contains("RootDirectory = ptr::null_mut()")
            || error.contains("RootDirectory = parent_handle"),
        "el contrafactual falló por otra causa: {error}"
    );
}
