//! E35-H03 CI23 — contrato portable del rename por handle en Windows.
//!
//! El runner local no ejecuta Win32. La prueba inspecciona el fuente que compila el crate para
//! fijar la forma documentada de `FILE_RENAME_INFO`: `RootDirectory` referencia el directorio
//! padre abierto sin seguir reparse points y `FileName` contiene solo el nombre relativo.

const WINDOWS_VFS_SOURCE: &str = include_str!("../src/windows_vfs.rs");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start_at = source
        .find(start)
        .unwrap_or_else(|| panic!("guarda anti-vacuidad: falta el inicio `{start}`"));
    let remainder = &source[start_at..];
    let end_at = remainder
        .find(end)
        .unwrap_or_else(|| panic!("guarda anti-vacuidad: falta el final `{end}`"));
    &remainder[..end_at]
}

/// C5/C6 — un path DOS absoluto con `RootDirectory = NULL` fue rechazado por Windows con
/// `ERROR_INVALID_NAME`. La publicación debe anclar el rename al handle reparse-safe del padre y
/// copiar a `FILE_RENAME_INFO.FileName` únicamente el `file_name` relativo. Los shares completos
/// preservan el protocolo de reemplazo mientras existen lectores de la generación anterior.
#[test]
fn c5_c6_windows_rename_usa_root_directory_y_nombre_relativo() {
    let rename = section(
        WINDOWS_VFS_SOURCE,
        "fn rename_handle_to(",
        "\nfn wide_path(path: &Path)",
    );

    let failed_gate_positions: Vec<_> = rename
        .match_indices("if renamed == 0 {")
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        failed_gate_positions.len(),
        2,
        "guarda anti-vacuidad: debe haber un gate para el intento primario y otro para el fallback; positions={failed_gate_positions:?}"
    );
    let failed_gate_at = failed_gate_positions[0];
    let primary = &rename[..failed_gate_at];

    assert!(
        primary.contains("target.parent()") && primary.contains("target.file_name()"),
        "C5 Windows: el intento primario debe separar explícitamente el directorio padre y el file_name relativo"
    );
    assert!(
        primary.contains("CreateFileW("),
        "C5 Windows: el directorio padre debe abrirse como handle para FILE_RENAME_INFO.RootDirectory"
    );
    assert!(
        primary.contains("FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE"),
        "C5/C6 Windows: el handle del padre debe admitir los shares de lectura, escritura y borrado"
    );
    assert!(
        primary.contains("FILE_FLAG_BACKUP_SEMANTICS")
            && primary.contains("FILE_FLAG_OPEN_REPARSE_POINT"),
        "C5/C6 Windows: el padre debe abrirse como directorio y sin seguir reparse points"
    );
    assert!(
        primary.contains("(*info).RootDirectory = parent_handle")
            && !primary.contains("(*info).RootDirectory = ptr::null_mut()"),
        "C5 Windows: RootDirectory del intento primario debe recibir el handle del padre, nunca NULL"
    );
    assert!(
        !primary.contains("wide_path(target)"),
        "C5 Windows: el intento primario no puede serializar el path DOS absoluto completo; debe contener solo file_name"
    );
    assert_eq!(
        primary.matches("SetFileInformationByHandle(").count(),
        1,
        "guarda anti-vacuidad: el intento primario debe efectuar una única operación atómica de rename"
    );
    assert!(
        primary.contains("FileRenameInfoEx"),
        "guarda anti-vacuidad: el intento primario debe conservar FileRenameInfoEx"
    );
}
