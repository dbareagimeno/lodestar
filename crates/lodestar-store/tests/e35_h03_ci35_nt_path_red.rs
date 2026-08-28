//! E35-H03 CI35 — contrato portable de conversión a namespace NT para FileRenameInfoEx.
//!
//! El helper es deliberadamente puro sobre UTF-16: Windows VFS obtiene esos words desde `Path`,
//! elimina el NUL terminal y solo entonces construye `FILE_RENAME_INFO.FileName`. Así estos casos
//! se ejecutan también fuera de Windows sin duplicar llamadas nativas.

#[path = "../src/windows_nt_path.rs"]
mod windows_nt_path;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

fn converted(value: &str) -> Result<String, String> {
    let converted =
        windows_nt_path::to_nt_rename_path(&wide(value)).map_err(|error| error.to_string())?;
    if converted.last() == Some(&0) {
        return Err("C5: FileNameLength no debe incluir un NUL terminal".into());
    }
    String::from_utf16(&converted).map_err(|error| error.to_string())
}

/// C5 — una ruta drive absoluta se entrega al kernel en namespace NT, no como DOS crudo.
#[test]
fn c5_windows_nt_path_convierte_drive_absoluto() {
    assert_eq!(
        converted(r"C:\cache\index.db").unwrap(),
        r"\??\C:\cache\index.db"
    );
    assert_eq!(
        converted(r"d:\espacio unicode\área\index.db").unwrap(),
        r"\??\d:\espacio unicode\área\index.db",
        "guarda anti-vacuidad: no se normalizan case ni unidades y se preserva UTF-16"
    );
}

/// C5 — UNC usa el prefijo NT `\??\UNC\`; anteponer `\??\` directamente a `\\server`
/// produciría una forma distinta e inválida.
#[test]
fn c5_windows_nt_path_convierte_unc_absoluto() {
    assert_eq!(
        converted(r"\\server\share\cache\index.db").unwrap(),
        r"\??\UNC\server\share\cache\index.db"
    );
}

/// C5 — las formas verbatim ya resueltas se normalizan al mismo namespace NT que sus equivalentes
/// drive/UNC; no se conserva el prefijo Win32 `\\?\` dentro de FILE_RENAME_INFO.
#[test]
fn c5_windows_nt_path_normaliza_verbatim_drive_y_unc() {
    assert_eq!(
        converted(r"\\?\C:\cache\index.db").unwrap(),
        r"\??\C:\cache\index.db"
    );
    assert_eq!(
        converted(r"\\?\UNC\server\share\cache\index.db").unwrap(),
        r"\??\UNC\server\share\cache\index.db"
    );
    assert_eq!(
        converted(r"\??\C:\cache\index.db").unwrap(),
        r"\??\C:\cache\index.db",
        "guarda anti-vacuidad: una ruta NT ya formada debe ser idempotente"
    );
}

/// Negativo C5/C6 — basename, root-relative y drive-relative dependen del cwd o de su unidad y no
/// pueden cruzar la frontera nativa.
#[test]
fn c5_c6_windows_nt_path_rechaza_formas_relativas() {
    for path in [
        r"index.db",
        r"cache\index.db",
        r"\cache\index.db",
        r"C:index.db",
    ] {
        let error = windows_nt_path::to_nt_rename_path(&wide(path))
            .expect_err("la forma relativa debe rechazarse antes de FileRenameInfoEx");
        let error = error.to_string();
        assert!(
            error.contains("absolute") || error.contains("absolut") || error.contains("NT"),
            "C5/C6: diagnóstico no causal para `{path}`: {error}"
        );
    }
}
