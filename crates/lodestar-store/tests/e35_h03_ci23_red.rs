//! E35-H03 CI33 — contrato portable del rename por handle en Windows.
//!
//! Los runners Windows remotos refutaron tres formas: DOS absoluto crudo, basename dependiente del
//! cwd y `RootDirectory` relativo. Esta guarda fija un único `FileRenameInfoEx` sobre el handle
//! validado, con destino absoluto en namespace NT y `RootDirectory = NULL`.

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

/// C5/C6 — la publicación convierte el path absoluto de `active` a `\??\...` antes del syscall.
/// La forma NT es independiente de la unidad del cwd y cubre paths drive/UNC/verbatim sin abrir el
/// padre. Un solo syscall conserva el punto atómico.
#[test]
fn c5_c6_windows_rename_usa_un_solo_destino_nt_absoluto_con_root_null() {
    let rename = section(
        WINDOWS_VFS_SOURCE,
        "fn rename_handle_to(",
        "\nfn wide_path(path: &Path)",
    );

    assert_eq!(
        rename.matches("SetFileInformationByHandle(").count(),
        1,
        "C5 Windows: la publicación debe contener un único syscall de rename atómico"
    );
    assert!(
        rename.contains("wide_path(target)")
            && rename.contains("windows_nt_path::to_nt_rename_path(&wide)")
            && rename.contains("(*info).RootDirectory = ptr::null_mut()"),
        "C5 Windows: el único intento debe convertir el target absoluto al namespace NT y usar RootDirectory=NULL"
    );
    assert!(
        !rename.contains("(*info).RootDirectory = parent_handle")
            && !rename.contains("CreateFileW(")
            && !rename.contains("target.parent()")
            && !rename.contains("target.file_name()")
            && !rename.contains("wide_path(Path::new(file_name))"),
        "C5 Windows: no debe quedar basename/cwd ni parent-handle"
    );
    assert!(
        rename.contains("(*info).Anonymous.Flags = extended_flags.unwrap_or(0);"),
        "C5 Windows: el buffer debe conservar los flags fuertes pedidos por replace_durable"
    );
    assert!(
        rename.contains(
            "SetFileInformationByHandle(handle, FileRenameInfoEx, info.cast(), total_bytes as u32)"
        ),
        "guarda anti-vacuidad: el único syscall debe operar sobre el mismo handle candidato con FileRenameInfoEx"
    );
    assert_eq!(
        rename.matches("if renamed == 0 {").count(),
        1,
        "C5/C6 Windows: el único syscall debe tener un único gate de error, sin fallback post-éxito"
    );
    assert!(
        !rename.contains("ERROR_INVALID_PARAMETER"),
        "C5 Windows: el protocolo ya no puede depender de un intento relativo previo y su fallback"
    );
}
