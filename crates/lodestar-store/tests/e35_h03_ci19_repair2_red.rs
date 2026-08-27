//! E35-H03 CI19/review2 — contrato estructural del punto de publicación.
//!
//! Un `fsync` físico no tiene un observable portátil desde una integración y el runner de esta
//! fase es macOS, por lo que el reemplazo Windows tampoco puede ejecutarse. Estos tests leen el
//! mismo fuente que compila el crate y fijan las dos propiedades que sí son binarias: después del
//! rename visible no puede existir una salida fallable antes de la barrera del directorio, y la
//! rama Windows debe reemplazar `index.db` mediante una única operación del sistema.

const STORE_SOURCE: &str = include_str!("../src/lib.rs");
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

fn unique_position(haystack: &str, needle: &str) -> usize {
    let positions: Vec<_> = haystack.match_indices(needle).map(|(at, _)| at).collect();
    assert_eq!(
        positions.len(),
        1,
        "guarda anti-vacuidad: `{needle}` debe identificar exactamente un paso; posiciones={positions:?}"
    );
    positions[0]
}

fn assert_precedes(protocol: &str, earlier: &str, later: &str, reason: &str) {
    let earlier_at = unique_position(protocol, earlier);
    let later_at = unique_position(protocol, later);
    assert!(
        earlier_at < later_at,
        "{reason}; orden observado `{later}`@{later_at} antes de `{earlier}`@{earlier_at}"
    );
}

/// C5/C6 + ARCH §20.12.2 — el éxito del rename es el punto en el que `index.db` ya muestra la
/// generación nueva. La sincronización del directorio debe ser el primer paso de éxito posterior:
/// reabrir SQLite, activar WAL, actualizar identidad o confirmar guards antes de esa barrera puede
/// fallar, devolver `Err` y dejar sin ejecutar el único `fsync` que hace durable el nombre nuevo.
#[test]
fn c5_c6_rename_visible_no_tiene_salida_fallable_antes_del_fsync_del_directorio() {
    let swap = section(
        STORE_SOURCE,
        "    fn swap_active(&self, next: &Path) -> Result<(), StoreError> {",
        "\n    /// Reabre la conexión compartida",
    );

    let replace = "if let Err(error) = replace_durable(next, &active)";
    let success_boundary = "#[cfg(windows)]\n        publication.commit();";
    let directory_barrier = "sync_directory(active.parent().expect(\"cache directory\"))?";

    // Anti-vacuidad causal: el test inspecciona la ruta real de publicación y las operaciones
    // fallables reales que han de permanecer después de la barrera, no comentarios ni un helper
    // desconectado.
    assert_precedes(
        swap,
        replace,
        success_boundary,
        "el límite de éxito debe estar después del intento real de reemplazo",
    );
    for fallible_after_publish in [
        "let published = match open_sqlite(&active)",
        "activation?;",
        "db_identity(&active).map_err(|error| StoreError::Io(error.to_string()))?",
    ] {
        assert_precedes(
            swap,
            directory_barrier,
            fallible_after_publish,
            "C5/C6: ninguna operación fallable post-rename puede omitir la barrera durable",
        );
    }
    assert_precedes(
        swap,
        directory_barrier,
        success_boundary,
        "C5/C6: el guard reversible tampoco se confirma antes de hacer durable el rename",
    );
}

/// C5/C6 + Windows — `retirar index.db` seguido de `instalar index.db.next` deja una ventana real
/// sin nombre activo si el proceso cae. La implementación Windows debe usar exactamente una
/// operación `FileRenameInfoEx` con replace-atómico, y el caller no puede soltar su guard antes de
/// que la barrera durable quede confirmada.
#[test]
fn c5_windows_usa_un_solo_replace_atomico_y_protege_su_barrera_durable() {
    let replace = section(
        WINDOWS_VFS_SOURCE,
        "pub(crate) fn replace_durable(next: &Path, active: &Path) -> std::io::Result<()> {",
        "\n}\n\n/// Rust 1.80 implements `remove_file`",
    );
    let rename_primitive = section(
        WINDOWS_VFS_SOURCE,
        "fn rename_handle_to(",
        "\nfn wide_path(path: &Path)",
    );

    assert_eq!(
        replace.matches("rename_handle_to(").count(),
        1,
        "C5 Windows: la publicación debe contener una única operación de rename/reemplazo"
    );
    assert!(
        replace.contains("FILE_RENAME_FLAG_REPLACE_IF_EXISTS")
            && replace.contains("FILE_RENAME_FLAG_POSIX_SEMANTICS"),
        "C5 Windows: FileRenameInfoEx debe reemplazar el nombre existente en una operación"
    );
    assert!(
        replace.contains("FILE_FLAG_WRITE_THROUGH"),
        "C5/C6 Windows: el handle del candidato debe pedir la barrera durable"
    );
    for forbidden_two_step in [
        "remove_file(active)",
        "remove_sidecar(active)",
        "DeleteFileW",
        "MoveFileExW",
        "std::fs::rename",
    ] {
        assert!(
            !replace.contains(forbidden_two_step),
            "C5 Windows: `{forbidden_two_step}` permitiría retirar el nombre activo fuera del único replace"
        );
    }
    assert_eq!(
        rename_primitive
            .matches("SetFileInformationByHandle(")
            .count(),
        1,
        "C5 Windows: el primitive debe terminar en una sola llamada atómica del sistema"
    );
    assert!(
        rename_primitive.contains("FileRenameInfoEx"),
        "guarda anti-vacuidad: los flags de reemplazo deben enviarse como FileRenameInfoEx"
    );

    // La guarda de publicación mantiene reversibles los sidecars. Confirmarla antes del paso que
    // el protocolo común reconoce como barrera permitiría devolver un error posterior sin rollback
    // ni durabilidad demostrada.
    let swap = section(
        STORE_SOURCE,
        "    fn swap_active(&self, next: &Path) -> Result<(), StoreError> {",
        "\n    /// Reabre la conexión compartida",
    );
    assert_precedes(
        swap,
        "sync_directory(active.parent().expect(\"cache directory\"))?",
        "#[cfg(windows)]\n        publication.commit();",
        "C5/C6 Windows: la barrera durable debe quedar protegida por PublicationGuard",
    );
}
