//! E35-H03 CI19/review2 — contrato estructural del punto de publicación.
//!
//! Un `fsync` físico no tiene un observable portátil desde una integración y el runner de esta
//! fase es macOS, por lo que el reemplazo Windows tampoco puede ejecutarse. Estos tests leen el
//! mismo fuente que compila el crate y fijan las propiedades binarias del protocolo: el objeto
//! validado llega por handle a una única publicación con destino absoluto NT inequívoco, y después
//! del rename visible la primera operación fallable es la barrera del directorio.

const STORE_SOURCE: &str = include_str!("../src/lib.rs");
const WINDOWS_VFS_SOURCE: &str = include_str!("../src/windows_vfs.rs");

struct NormalizedSources {
    store: String,
    windows_vfs: String,
}

fn normalized_sources() -> NormalizedSources {
    NormalizedSources {
        store: STORE_SOURCE.replace("\r\n", "\n"),
        windows_vfs: WINDOWS_VFS_SOURCE.replace("\r\n", "\n"),
    }
}

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

fn unique_position(haystack: &str, needle: &str) -> Result<usize, String> {
    let positions: Vec<_> = haystack.match_indices(needle).map(|(at, _)| at).collect();
    if positions.len() != 1 {
        return Err(format!(
            "`{needle}` debe identificar exactamente un paso; posiciones={positions:?}"
        ));
    }
    Ok(positions[0])
}

fn require(source: &str, needle: &str, reason: &str) -> Result<(), String> {
    if source.contains(needle) {
        Ok(())
    } else {
        Err(format!("{reason}; falta `{needle}`"))
    }
}

fn reject(source: &str, needle: &str, reason: &str) -> Result<(), String> {
    if source.contains(needle) {
        Err(format!("{reason}; apareció `{needle}`"))
    } else {
        Ok(())
    }
}

fn precedes(source: &str, earlier: &str, later: &str, reason: &str) -> Result<(), String> {
    let earlier_at = unique_position(source, earlier)?;
    let later_at = unique_position(source, later)?;
    if earlier_at < later_at {
        Ok(())
    } else {
        Err(format!(
            "{reason}; orden observado `{later}`@{later_at} antes de `{earlier}`@{earlier_at}"
        ))
    }
}

fn assert_ok(result: Result<(), String>) {
    result.unwrap_or_else(|error| panic!("guarda anti-vacuidad: {error}"));
}

fn assert_rejected(result: Result<(), String>, expected_reason: &str) {
    let error = result.expect_err("la mutación contrafactual debía ser rechazada");
    assert!(
        error.contains(expected_reason),
        "la mutación falló por una razón distinta: esperado `{expected_reason}`, observado `{error}`"
    );
}

fn windows_replace_contract(replace: &str, rename: &str) -> Result<(), String> {
    for forbidden_path_reopen in [
        "CreateFileW(",
        "ReOpenFile(",
        "Connection::open",
        "open_sqlite(",
        "wide_path(candidate",
    ] {
        reject(
            replace,
            forbidden_path_reopen,
            "replace_durable debe consumir el handle ya validado sin reabrir el candidato por path",
        )?;
    }
    require(
        replace,
        "pub(crate) fn replace_durable(candidate: PreparedCandidate, active: &Path)",
        "la publicación Windows debe consumir PreparedCandidate por valor",
    )?;
    require(
        replace,
        "candidate.handle.0",
        "el syscall debe operar sobre el objeto de fichero conservado desde validación",
    )?;
    require(
        replace,
        "Some(FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS)",
        "el replace principal debe conservar simultáneamente replace-if-exists y POSIX semantics",
    )?;
    if replace.matches("rename_handle_to(").count() != 1 {
        return Err("replace_durable debe delegar exactamente un protocolo de rename".into());
    }
    for forbidden_two_step in [
        "remove_file(active)",
        "remove_sidecar(active)",
        "DeleteFileW",
        "MoveFileExW",
        "std::fs::rename",
    ] {
        for protocol in [replace, rename] {
            reject(
                protocol,
                forbidden_two_step,
                "la publicación no puede retirar el nombre activo fuera del único replace",
            )?;
        }
    }

    if rename.matches("SetFileInformationByHandle(").count() != 1 {
        return Err("rename_handle_to debe tener exactamente un syscall atómico".into());
    }
    if rename
        .matches("SetFileInformationByHandle(handle, FileRenameInfoEx")
        .count()
        != 1
    {
        return Err("el único intento debe usar FileRenameInfoEx sobre el mismo handle".into());
    }
    if rename
        .matches("(*info).Anonymous.Flags = extended_flags.unwrap_or(0);")
        .count()
        != 1
    {
        return Err("el único intento debe materializar exactamente una vez los flags".into());
    }
    if rename.matches("if renamed == 0 {").count() != 1 {
        return Err("el único syscall debe tener exactamente un gate de error".into());
    }
    require(
        rename,
        "wide_path(target)",
        "el protocolo debe obtener los UTF-16 del target absoluto",
    )?;
    require(
        rename,
        "windows_nt_path::to_nt_rename_path(&wide)",
        "el path DOS/verbatim debe convertirse a namespace NT antes del syscall",
    )?;
    require(
        rename,
        "(*info).RootDirectory = ptr::null_mut();",
        "el destino absoluto NT debe usar RootDirectory=NULL",
    )?;
    precedes(
        rename,
        "wide_path(target)",
        "windows_nt_path::to_nt_rename_path(&wide)",
        "la conversión NT debe consumir el target UTF-16",
    )?;
    precedes(
        rename,
        "windows_nt_path::to_nt_rename_path(&wide)",
        "let name_bytes = wide.len()",
        "solo la ruta NT convertida puede dimensionar FILE_RENAME_INFO",
    )?;
    for forbidden_refuted_protocol in [
        "(*info).RootDirectory = parent_handle",
        "ERROR_INVALID_PARAMETER",
        "CreateFileW(",
        "target.parent()",
        "target.file_name()",
        "wide_path(Path::new(file_name))",
    ] {
        reject(
            rename,
            forbidden_refuted_protocol,
            "la publicación no puede conservar basename/cwd, parent-handle ni fallback posterior",
        )?;
    }
    Ok(())
}

fn publication_order_contract(swap: &str) -> Result<(), String> {
    let replace = "if let Err(error) = replace_durable(candidate, &active)";
    let directory_barrier = "sync_directory(active.parent().expect(\"cache directory\"))?";
    let commit = "#[cfg(windows)]\n        publication.commit();";

    precedes(
        swap,
        replace,
        directory_barrier,
        "la barrera debe ocurrir después del replace visible",
    )?;
    // Las dos ramas cfg terminan inmediatamente en el bloque común de directory_sync. Así, en
    // Windows no se puede insertar otra operación fallable entre el Ok del rename y la barrera.
    require(
        swap,
        "#[cfg(not(windows))]\n        if let Err(error) = replace_durable(next, &active) {\n            return restore_after_publication_failure(&mut guard, standby, error);\n        }\n        let directory_sync = (|| -> Result<(), StoreError> {\n            sync_directory(active.parent().expect(\"cache directory\"))?;",
        "directory_sync debe ser el primer efecto fallable común tras las ramas de rename",
    )?;
    for fallible_after_publish in [
        commit,
        "let published = match open_sqlite(&active)",
        "let activation = activate_published_connection(&published);",
        "activation?;",
        "db_identity(&active).map_err(|error| StoreError::Io(error.to_string()))?",
    ] {
        precedes(
            swap,
            directory_barrier,
            fallible_after_publish,
            "ningún commit, reopen, activación o identidad puede adelantarse a la barrera durable",
        )?;
    }
    Ok(())
}

fn windows_next_parameter_contract(swap: &str) -> Result<(), String> {
    require(
        swap,
        "    fn swap_active(\n        &self,\n        next: &Path,\n        #[cfg(windows)] candidate: windows_vfs::PreparedCandidate,\n        #[cfg(windows)] candidate_standby: Connection,\n    )",
        "swap_active debe preservar la firma y la semántica path de next en todas las plataformas",
    )?;
    for forbidden in [
        "#[allow(unused)]",
        "#[allow(unused_variables)]",
        "#[allow(warnings)]",
        "_next: &Path",
    ] {
        reject(
            swap,
            forbidden,
            "el parámetro next no puede silenciarse con allow ni ocultarse mediante renombrado",
        )?;
    }
    require(
        swap,
        "        #[cfg(windows)]\n        let _ = next;",
        "la compilación Windows debe consumir next explícitamente sin alterar la firma compartida",
    )?;
    Ok(())
}

/// C5/C6 + Windows — el candidato se obtiene de la misma conexión de solo lectura que pasa
/// integridad/FK, se mantiene por handle al cerrar SQLite y `replace_durable` lo consume sin volver
/// a resolver el pathname susceptible de sustitución.
#[test]
fn c5_windows_publica_el_mismo_objeto_validado_sin_reopen_por_path() {
    let sources = normalized_sources();
    let validation = section(
        &sources.store,
        "        let validation_conn = schema::open_validation_connection(&next)?;",
        "        let validate_ns = validate_start.elapsed().as_nanos() as u64;",
    );
    assert_ok(precedes(
        validation,
        "let validation_conn = schema::open_validation_connection(&next)?;",
        "let candidate = windows_vfs::prepare_candidate(&validation_conn)",
        "PreparedCandidate debe proceder de la conexión de validación",
    ));
    assert_ok(precedes(
        validation,
        "let candidate = windows_vfs::prepare_candidate(&validation_conn)",
        "let check = schema::validate_database(&validation_conn);",
        "el handle debe fijar el objeto exacto que valida SQLite",
    ));
    assert_ok(precedes(
        validation,
        "let check = schema::validate_database(&validation_conn);",
        "drop(validation_conn);",
        "la conexión debe completar la validación antes de cerrarse",
    ));
    assert_ok(precedes(
        validation,
        "drop(validation_conn);",
        "let candidate_sync = candidate.sync();",
        "el handle candidato debe sobrevivir al cierre de la conexión",
    ));

    let rebuild_to_swap = section(
        &sources.store,
        "        let validation_conn = schema::open_validation_connection(&next)?;",
        "        let swap_ns = swap_started.elapsed().as_nanos() as u64;",
    );
    assert_ok(require(
        rebuild_to_swap,
        "self.swap_active(&next, candidate, candidate_standby)?;",
        "el mismo PreparedCandidate y su fallback ya abierto deben llegar por valor al swap Windows",
    ));

    let store_replace = section(
        &sources.store,
        "#[cfg(windows)]\nfn replace_durable(\n    candidate: windows_vfs::PreparedCandidate,\n    active: &Path,\n)",
        "\n\n#[cfg(not(windows))]",
    );
    assert_ok(require(
        store_replace,
        "windows_vfs::replace_durable(candidate, active)",
        "el wrapper cfg(windows) debe reenviar el PreparedCandidate sin convertirlo en path",
    ));
    for forbidden_reopen in ["CreateFileW(", "ReOpenFile(", "open_sqlite(", "&next"] {
        assert_ok(reject(
            store_replace,
            forbidden_reopen,
            "el wrapper replace_durable no puede reabrir el candidato",
        ));
    }

    let replace = section(
        &sources.windows_vfs,
        "pub(crate) fn replace_durable(candidate: PreparedCandidate, active: &Path)",
        "\n}\n\n/// Rust 1.80 implements `remove_file`",
    );
    let rename = section(
        &sources.windows_vfs,
        "fn rename_handle_to(\n    target: &Path,\n    handle: HANDLE,\n    extended_flags: Option<u32>,\n)",
        "\nfn wide_path(path: &Path)",
    );
    assert_ok(windows_replace_contract(replace, rename));
}

/// C5/C6 — el éxito del rename es el punto visible de la generación nueva. La sincronización del
/// directorio es el primer efecto fallable posterior; solo después se puede confirmar el guard,
/// reabrir SQLite, activar la conexión o publicar su identidad.
#[test]
fn c5_c6_directory_sync_es_la_primera_barrera_fallable_post_rename() {
    let sources = normalized_sources();
    let swap = section(
        &sources.store,
        "    fn swap_active(\n        &self,\n        next: &Path,\n        #[cfg(windows)] candidate: windows_vfs::PreparedCandidate,\n        #[cfg(windows)] candidate_standby: Connection,\n    )",
        "\n    /// Reabre la conexión compartida",
    );
    assert_ok(publication_order_contract(swap));
}

/// CI24 Windows Clippy — `next` sigue siendo parte de la firma portable porque la rama Unix lo
/// publica por pathname. La rama Windows debe reconocerlo explícitamente, sin `allow` ni renombrar
/// el parámetro, mientras publica el `PreparedCandidate` fijado por handle.
#[test]
fn ci24_windows_swap_consumira_next_explicitamente_sin_silenciar_warnings() {
    let sources = normalized_sources();
    let swap = section(
        &sources.store,
        "    fn swap_active(\n        &self,\n        next: &Path,\n        #[cfg(windows)] candidate: windows_vfs::PreparedCandidate,\n        #[cfg(windows)] candidate_standby: Connection,\n    )",
        "\n    /// Reabre la conexión compartida",
    );
    assert_ok(windows_next_parameter_contract(swap));

    let without_explicit_use =
        swap.replacen("        #[cfg(windows)]\n        let _ = next;\n", "", 1);
    assert_ne!(
        without_explicit_use, swap,
        "guarda anti-vacuidad: el contrafactual debe retirar el uso Windows explícito de next"
    );
    assert_rejected(
        windows_next_parameter_contract(&without_explicit_use),
        "debe consumir next explícitamente",
    );

    let hidden_with_allow = swap.replacen(
        "        #[cfg(windows)]\n        let _ = next;",
        "        #[cfg(windows)]\n        #[allow(unused_variables)]\n        let _ = next;",
        1,
    );
    assert_ne!(
        hidden_with_allow, swap,
        "guarda anti-vacuidad: el contrafactual debe insertar el allow prohibido"
    );
    assert_rejected(
        windows_next_parameter_contract(&hidden_with_allow),
        "no puede silenciarse con allow",
    );
}

/// Guardas de mutación — prueban que los oráculos anteriores no son vacuos ni aceptan versiones
/// plausibles pero incorrectas del protocolo Windows.
#[test]
fn guardas_contrafactuales_rechazan_reopen_relative_flags_y_commit_prematuro() {
    let sources = normalized_sources();
    let replace = section(
        &sources.windows_vfs,
        "pub(crate) fn replace_durable(candidate: PreparedCandidate, active: &Path)",
        "\n}\n\n/// Rust 1.80 implements `remove_file`",
    );
    let rename = section(
        &sources.windows_vfs,
        "fn rename_handle_to(\n    target: &Path,\n    handle: HANDLE,\n    extended_flags: Option<u32>,\n)",
        "\nfn wide_path(path: &Path)",
    );
    assert_ok(windows_replace_contract(replace, rename));

    let reopened = replace.replacen(
        "    rename_handle_to(",
        "    let _reopened = CreateFileW(candidate_path);\n    rename_handle_to(",
        1,
    );
    assert_rejected(
        windows_replace_contract(&reopened, rename),
        "sin reabrir el candidato por path",
    );

    let unconditional = rename.replacen("if renamed == 0 {", "if true {", 1);
    assert_rejected(
        windows_replace_contract(replace, &unconditional),
        "exactamente un gate de error",
    );

    let relative_root = rename.replacen(
        "(*info).RootDirectory = ptr::null_mut();",
        "(*info).RootDirectory = parent_handle;",
        1,
    );
    assert_rejected(
        windows_replace_contract(replace, &relative_root),
        "RootDirectory=NULL",
    );

    let raw_dos_target =
        rename.replacen("windows_nt_path::to_nt_rename_path(&wide)", "Ok(wide)", 1);
    assert_rejected(
        windows_replace_contract(replace, &raw_dos_target),
        "convertirse a namespace NT",
    );

    let weakened_flags = replace.replacen(
        "Some(FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS)",
        "Some(FILE_RENAME_FLAG_REPLACE_IF_EXISTS)",
        1,
    );
    assert_rejected(
        windows_replace_contract(&weakened_flags, rename),
        "replace-if-exists y POSIX semantics",
    );

    let swap = section(
        &sources.store,
        "    fn swap_active(\n        &self,\n        next: &Path,\n        #[cfg(windows)] candidate: windows_vfs::PreparedCandidate,\n        #[cfg(windows)] candidate_standby: Connection,\n    )",
        "\n    /// Reabre la conexión compartida",
    );
    let without_late_commit =
        swap.replacen("#[cfg(windows)]\n        publication.commit();", "", 1);
    let early_commit = without_late_commit.replacen(
        "        let directory_sync = (|| -> Result<(), StoreError> {",
        "        #[cfg(windows)]\n        publication.commit();\n        let directory_sync = (|| -> Result<(), StoreError> {",
        1,
    );
    assert_rejected(
        publication_order_contract(&early_commit),
        "primer efecto fallable común",
    );
}

/// Negativo C5/C6 — conservar la ruta NT en otra rama no legitima introducir un primer intento
/// alternativo: el protocolo completo debe contener un solo syscall y un solo target.
#[test]
fn c5_c6_contrafactual_rechaza_anteponer_un_rename_relativo() {
    let sources = normalized_sources();
    let replace = section(
        &sources.windows_vfs,
        "pub(crate) fn replace_durable(candidate: PreparedCandidate, active: &Path)",
        "\n}\n\n/// Rust 1.80 implements `remove_file`",
    );
    let rename = section(
        &sources.windows_vfs,
        "fn rename_handle_to(\n    target: &Path,\n    handle: HANDLE,\n    extended_flags: Option<u32>,\n)",
        "\nfn wide_path(path: &Path)",
    );
    assert_ok(windows_replace_contract(replace, rename));

    let extra_relative = rename.replacen(
        "let renamed = unsafe {",
        "let _relative_renamed = unsafe {\n        SetFileInformationByHandle(handle, FileRenameInfoEx, info.cast(), total_bytes as u32)\n    };\n    let renamed = unsafe {",
        1,
    );
    assert_ne!(
        extra_relative, rename,
        "guarda anti-vacuidad: la mutación debe anteponer un segundo syscall"
    );
    assert!(
        extra_relative
            .matches("SetFileInformationByHandle(")
            .count()
            == rename.matches("SetFileInformationByHandle(").count() + 1,
        "guarda anti-vacuidad: el contrafactual debe tener exactamente un rename adicional"
    );
    assert_rejected(
        windows_replace_contract(replace, &extra_relative),
        "exactamente un syscall atómico",
    );
}
