//! E35-H03 CI44 — contrato portable del nombre Win32 para `FILE_RENAME_INFO`.
//!
//! El helper es deliberadamente puro sobre UTF-16: Windows VFS obtiene esos words desde `Path`,
//! elimina el NUL terminal, valida que sea una ruta Win32 absoluta y construye
//! `FILE_RENAME_INFO.FileName` sin traducirla al namespace Object Manager. Así estos casos se
//! ejecutan también fuera de Windows sin duplicar llamadas nativas.

#[path = "../src/windows_rename_path.rs"]
mod windows_rename_path;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

fn validated(value: &str) -> Result<String, String> {
    let validated = wide(value);
    windows_rename_path::validate_win32_rename_path(&validated)
        .map_err(|error| error.to_string())?;
    if validated.last() == Some(&0) {
        return Err("C5: FileNameLength no debe incluir un NUL terminal".into());
    }
    String::from_utf16(&validated).map_err(|error| error.to_string())
}

/// C5 — `FILE_RENAME_INFO.FileName` recibe la ruta drive Win32 absoluta sin transformarla.
#[test]
fn c5_windows_filename_preserva_drive_absoluto() {
    assert_eq!(
        validated(r"C:\cache\index.db").unwrap(),
        r"C:\cache\index.db",
        "rojo causal CI44: FileName no debe anteponer el prefijo Object Manager `\\??\\`"
    );
    assert_eq!(
        validated(r"d:\espacio unicode\área\index.db").unwrap(),
        r"d:\espacio unicode\área\index.db",
        "guarda anti-vacuidad: no se normalizan case ni unidades y se preserva UTF-16"
    );
}

/// C5 — una ruta UNC absoluta conserva exactamente sus dos barras iniciales, servidor y share.
#[test]
fn c5_windows_filename_preserva_unc_absoluto() {
    assert_eq!(
        validated(r"\\server\share\cache\index.db").unwrap(),
        r"\\server\share\cache\index.db",
        "rojo causal CI44: FileName debe preservar el path UNC Win32, no convertirlo a `\\??\\UNC\\`"
    );
}

/// Negativo C5 — `\??\` pertenece al namespace Object Manager y no es una ruta Win32 drive/UNC
/// válida para este contrato de `FILE_RENAME_INFO`.
#[test]
fn c5_windows_filename_rechaza_prefijo_object_manager() {
    let error = windows_rename_path::validate_win32_rename_path(&wide(r"\??\C:\cache\index.db"))
        .expect_err("rojo causal CI44: `\\??\\` debe rechazarse antes de FileRenameInfoEx");
    assert!(
        error.to_string().contains("Win32") || error.to_string().contains("Object Manager"),
        "C5: el diagnóstico debe identificar la frontera Win32/Object Manager: {error}"
    );
}

/// Negativo C5/C6 — basename, root-relative y drive-relative dependen del cwd o de su unidad y no
/// pueden cruzar la frontera nativa.
#[test]
fn c5_c6_windows_rename_path_rechaza_formas_relativas() {
    for path in [
        r"index.db",
        r"cache\index.db",
        r"\cache\index.db",
        r"C:index.db",
    ] {
        let error = windows_rename_path::validate_win32_rename_path(&wide(path))
            .expect_err("la forma relativa debe rechazarse antes de FileRenameInfoEx");
        let error = error.to_string();
        assert!(
            error.contains("absolute") || error.contains("absolut") || error.contains("NT"),
            "C5/C6: diagnóstico no causal para `{path}`: {error}"
        );
    }
}
