//! E35-H03 CI23 review repair — portable guards for the Windows publication protocol.
//!
//! These tests run on every host and inspect the Windows implementation that the crate ships.
//! They pin relationships that cannot be exercised on a Unix runner: the validated SQLite file
//! object must remain the publication source, and the target must be published by one unambiguous
//! Win32-absolute `FileRenameInfoEx` operation.

const STORE_SOURCE: &str = include_str!("../src/lib.rs");
const SCHEMA_SOURCE: &str = include_str!("../src/schema.rs");
const WINDOWS_VFS_SOURCE: &str = include_str!("../src/windows_vfs.rs");

struct NormalizedSources {
    store: String,
    schema: String,
    windows_vfs: String,
}

fn normalized_sources() -> NormalizedSources {
    NormalizedSources {
        store: STORE_SOURCE.replace("\r\n", "\n"),
        schema: SCHEMA_SOURCE.replace("\r\n", "\n"),
        windows_vfs: WINDOWS_VFS_SOURCE.replace("\r\n", "\n"),
    }
}

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start_at = source
        .find(start)
        .unwrap_or_else(|| panic!("anti-vacuity guard: missing section start `{start}`"));
    let remainder = &source[start_at..];
    let end_at = remainder
        .find(end)
        .unwrap_or_else(|| panic!("anti-vacuity guard: missing section end `{end}`"));
    &remainder[..end_at]
}

fn unique_position(haystack: &str, needle: &str) -> usize {
    let positions: Vec<_> = haystack.match_indices(needle).map(|(at, _)| at).collect();
    assert_eq!(
        positions.len(),
        1,
        "anti-vacuity guard: `{needle}` must identify exactly one protocol step; positions={positions:?}"
    );
    positions[0]
}

fn assert_precedes(protocol: &str, earlier: &str, later: &str, criterion: &str) {
    let earlier_at = unique_position(protocol, earlier);
    let later_at = unique_position(protocol, later);
    assert!(
        earlier_at < later_at,
        "{criterion}; observed `{later}`@{later_at} before `{earlier}`@{earlier_at}"
    );
}

fn contract_unique_position(source: &str, needle: &str, step: &str) -> Result<usize, String> {
    let positions: Vec<_> = source.match_indices(needle).map(|(at, _)| at).collect();
    if positions.len() == 1 {
        Ok(positions[0])
    } else {
        Err(format!(
            "{step}: `{needle}` debe identificar exactamente un paso; posiciones={positions:?}"
        ))
    }
}

fn parenthesized_call<'a>(source: &'a str, marker: &str) -> Result<&'a str, String> {
    let marker_at = contract_unique_position(
        source,
        marker,
        "sqlite3_file_control debe identificar exactamente la consulta del file pointer",
    )?;
    let open_at = marker_at
        + marker
            .rfind('(')
            .ok_or("guarda anti-vacuidad: el marcador de la llamada no contiene `(`")?;
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[open_at..].iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or("guarda anti-vacuidad: llamada sqlite3_file_control desbalanceada")?;
                if depth == 0 {
                    return Ok(&source[marker_at..=open_at + offset]);
                }
            }
            _ => {}
        }
    }
    Err("guarda anti-vacuidad: llamada sqlite3_file_control sin cierre".into())
}

fn prepared_candidate_contract(preparation: &str) -> Result<(), String> {
    for (forbidden, reason) in [
        ("CreateFileW(", "no puede abrir el candidato por path"),
        (
            "wide_path(",
            "no puede resolver el candidato a un path Windows",
        ),
        ("canonicalize(", "no puede resolver el path del candidato"),
        (
            "connection.path",
            "no puede recuperar un path desde Connection",
        ),
        (
            "Connection::open",
            "no puede abrir otra Connection por path",
        ),
        (
            "open_with_flags(",
            "no puede usar helpers de apertura por path",
        ),
        ("open_sqlite(", "no puede usar helpers de apertura por path"),
        (
            "Path::new(",
            "no puede reconstruir el candidato desde un path",
        ),
        (
            ".join(",
            "no puede derivar el candidato mediante resolución de path",
        ),
    ] {
        if preparation.contains(forbidden) {
            return Err(format!(
                "prepare_candidate {reason}; apareció `{forbidden}`"
            ));
        }
    }

    let file_pointer_query = parenthesized_call(preparation, "ffi::sqlite3_file_control(")?;
    contract_unique_position(
        file_pointer_query,
        "\n            connection.handle(),",
        "SQLITE_FCNTL_FILE_POINTER debe consultar exactamente connection.handle()",
    )?;
    contract_unique_position(
        file_pointer_query,
        "c\"main\".as_ptr(),",
        "SQLITE_FCNTL_FILE_POINTER debe consultar exactamente la base main con un literal C idiomático",
    )?;
    contract_unique_position(
        file_pointer_query,
        "ffi::SQLITE_FCNTL_FILE_POINTER,",
        "sqlite3_file_control debe usar SQLITE_FCNTL_FILE_POINTER",
    )?;
    contract_unique_position(
        file_pointer_query,
        "ptr::addr_of_mut!(file).cast(),",
        "SQLITE_FCNTL_FILE_POINTER debe escribir exactamente en la variable file luego validada",
    )?;

    let handle_at = file_pointer_query
        .find("\n            connection.handle(),")
        .unwrap();
    let main_at = file_pointer_query.find("c\"main\".as_ptr(),").unwrap();
    let opcode_at = file_pointer_query
        .find("ffi::SQLITE_FCNTL_FILE_POINTER,")
        .unwrap();
    let output_at = file_pointer_query
        .find("ptr::addr_of_mut!(file).cast(),")
        .unwrap();
    if !(handle_at < main_at && main_at < opcode_at && opcode_at < output_at) {
        return Err(
            "sqlite3_file_control debe recibir, en orden, connection.handle(), main, FILE_POINTER y addr_of_mut!(file)"
                .into(),
        );
    }

    let reopen = parenthesized_call(preparation, "ReOpenFile(")?;
    contract_unique_position(
        reopen,
        "\n            GENERIC_READ | GENERIC_WRITE | DELETE,",
        "ReOpenFile debe conservar acceso exacto GENERIC_READ | GENERIC_WRITE | DELETE",
    )?;
    contract_unique_position(
        reopen,
        "\n            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,",
        "ReOpenFile debe compartir exactamente lectura, escritura y borrado",
    )?;

    let ordered_steps = [
        (
            "ffi::sqlite3_file_control(",
            "debe consultar exactamente una vez SQLITE_FCNTL_FILE_POINTER",
        ),
        (
            "if result != ffi::SQLITE_OK || file.is_null() {",
            "debe comprobar conjuntamente el éxito y el file pointer",
        ),
        (
            "let original = unsafe { (*file.cast::<WinFilePrefix>()).handle };",
            "debe extraer original exactamente del sqlite3_file validado",
        ),
        (
            "ReOpenFile(\n            original,",
            "ReOpenFile debe recibir exactamente original",
        ),
        (
            "if handle == INVALID_HANDLE_VALUE {",
            "debe comprobar el resultado de ReOpenFile antes de poseerlo",
        ),
        (
            "handle: OwnedHandle(handle),",
            "PreparedCandidate debe poseer exactamente el handle devuelto por ReOpenFile",
        ),
    ];

    let mut previous = None;
    for (needle, step) in ordered_steps {
        let at = contract_unique_position(preparation, needle, step)?;
        if let Some((previous_at, previous_step)) = previous {
            if previous_at >= at {
                return Err(format!(
                    "orden causal inválido: `{previous_step}`@{previous_at} debe preceder `{step}`@{at}"
                ));
            }
        }
        previous = Some((at, step));
    }

    let original_binding = "let original = unsafe { (*file.cast::<WinFilePrefix>()).handle };";
    let original_at = contract_unique_position(
        preparation,
        original_binding,
        "original debe declararse exactamente una vez desde el sqlite3_file validado",
    )?;
    let reopen_at = contract_unique_position(
        preparation,
        "ReOpenFile(\n            original,",
        "ReOpenFile debe consumir exactamente el original extraido",
    )?;
    let causal_corridor = &preparation[original_at..reopen_at];
    if causal_corridor.matches("let original").count() != 1 {
        return Err(
            "original no puede sombrearse entre su extracción del sqlite3_file validado y ReOpenFile"
                .into(),
        );
    }

    Ok(())
}

fn assert_contract_rejected(result: Result<(), String>, expected_reason: &str) {
    let error = result.expect_err("la mutación contrafactual debía romper el contrato");
    assert!(
        error.contains(expected_reason),
        "la mutación fue rechazada por una razón distinta: esperada `{expected_reason}`, observada `{error}`"
    );
}

/// C5/C6 — validation, the pause seam, and the atomic rename must all refer to one pinned file
/// object. Extracting SQLite's `sqlite3_file` and duplicating its native handle prevents a path
/// reopen from selecting a different `.next`; sharing read/write/delete preserves SQLite's own
/// concurrent-open protocol while the pinned identity prevents publishing a different object.
#[test]
fn c5_c6_windows_publica_el_mismo_handle_validado_sin_reabrir_next_por_path() {
    let sources = normalized_sources();
    let rebuild = section(
        &sources.store,
        "fn rebuild_from_inventory_with_duration(",
        "\n    fn swap_active(",
    );
    let validation = section(
        &sources.schema,
        "pub(crate) fn validate_database(",
        "\npub(crate) fn read_user_version(",
    );

    assert!(
        rebuild.contains("pause_before_swap") && rebuild.contains("swap_active"),
        "anti-vacuity guard: the inspected rebuild must contain pause and publication boundaries"
    );
    assert!(
        validation.contains("PRAGMA integrity_check")
            && validation.contains("PRAGMA foreign_key_check"),
        "anti-vacuity guard: validation must still cover both SQLite integrity and foreign keys"
    );

    assert!(
        sources.schema.contains("open_validation_connection")
            && sources.schema.contains("OpenFlags::SQLITE_OPEN_READ_ONLY"),
        "C5/C6: the validation Connection must be opened explicitly read-only"
    );
    assert!(
        validation.contains("&Connection"),
        "C5/C6: integrity and FK validation must consume the caller's live &Connection"
    );
    assert!(
        sources.windows_vfs.contains("SQLITE_FCNTL_FILE_POINTER")
            && sources.windows_vfs.contains("sqlite3_file_control"),
        "C5/C6 Windows: publication must derive its pinned native handle from the sqlite3_file owned by the validation Connection"
    );

    let prepared = section(
        &sources.windows_vfs,
        "pub(crate) struct PreparedCandidate",
        "pub(crate) fn replace_durable",
    );
    let preparation = section(
        &sources.windows_vfs,
        "pub(crate) fn prepare_candidate(connection: &Connection)",
        "\nimpl PreparedCandidate",
    );
    prepared_candidate_contract(preparation)
        .unwrap_or_else(|error| panic!("C5/C6 Windows: {error}"));
    assert!(
        prepared.contains("pub(crate) fn prepare_candidate")
            && prepared.contains("connection: &Connection")
            && prepared.contains("SQLITE_FCNTL_FILE_POINTER")
            && prepared.contains("sqlite3_file_control"),
        "C5/C6 Windows: prepare_candidate(&Connection) must derive an owned PreparedCandidate via SQLITE_FCNTL_FILE_POINTER"
    );
    assert!(
        prepared.contains("ReOpenFile("),
        "C5/C6 Windows: the sqlite3_file handle must be duplicated and owned across validation, pause, and rename"
    );
    assert!(
        prepared.contains("FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE"),
        "C6 Windows: the pinned candidate must share exactly read/write/delete publication semantics"
    );
    assert!(
        prepared.contains("impl PreparedCandidate")
            && prepared.contains("fn sync(")
            && prepared.contains("FlushFileBuffers("),
        "C5/C6 Windows: PreparedCandidate::sync must flush the owned handle with FlushFileBuffers"
    );

    assert_precedes(
        rebuild,
        "let validation_conn = schema::open_validation_connection(&next)?;",
        "prepare_candidate(&validation_conn)",
        "C5/C6 Windows: open read-only validation Connection before deriving its file object",
    );
    assert_precedes(
        rebuild,
        "prepare_candidate(&validation_conn)",
        "validate_database(&validation_conn)",
        "C5/C6 Windows: pin the exact file object before validating it",
    );
    assert_precedes(
        rebuild,
        "validate_database(&validation_conn)",
        "drop(validation_conn);",
        "C5/C6 Windows: integrity/FK must finish before closing their Connection",
    );
    assert_precedes(
        rebuild,
        "drop(validation_conn);",
        "candidate.sync()",
        "C5/C6 Windows: close SQLite before flushing the owned candidate handle",
    );
    assert_precedes(
        rebuild,
        "candidate.sync()",
        "pause_before_swap",
        "C5/C6 Windows: flush the candidate before exposing the pause seam",
    );
    assert_precedes(
        rebuild,
        "pause_before_swap",
        "self.swap_active(&next, candidate, candidate_standby)?;",
        "C5/C6 Windows: pass the same candidate explicitly across the pause into publication",
    );

    let swap = section(
        &sources.store,
        "    fn swap_active(",
        "\n    /// Reabre la conexión compartida",
    );
    assert!(
        swap.contains("candidate: windows_vfs::PreparedCandidate")
            && swap.contains("replace_durable(candidate, &active)"),
        "C5/C6 Windows: swap_active must receive and consume PreparedCandidate explicitly"
    );

    let replace = section(
        &sources.windows_vfs,
        "pub(crate) fn replace_durable",
        "\n/// Rust 1.80",
    );
    assert!(
        replace.contains("candidate: PreparedCandidate")
            && !replace.contains("CreateFileW(")
            && !replace.contains("ReOpenFile("),
        "C5/C6 Windows: durable publication must consume the pinned candidate, never reopen index.db.next by path"
    );
    for hidden_channel in ["thread_local!", "static ", "std::env", "var_os("] {
        assert!(
            !prepared.contains(hidden_channel)
                && !rebuild.contains(hidden_channel)
                && !swap.contains(hidden_channel),
            "C5/C6 Windows: PreparedCandidate ownership must be explicit; hidden channel `{hidden_channel}` is forbidden"
        );
    }
}

/// C5/C6 — guardas de mutación del contrato causal de `prepare_candidate`. Una implementación que
/// reabre otro objeto o que devuelve el handle original/no relacionado no debe satisfacer el
/// oráculo aunque conserve todos los nombres de API esperados.
#[test]
fn c5_c6_guardas_contrafactuales_rechazan_handle_reabierto_o_poseido_incorrecto() {
    let sources = normalized_sources();
    let preparation = section(
        &sources.windows_vfs,
        "pub(crate) fn prepare_candidate(connection: &Connection)",
        "\nimpl PreparedCandidate",
    );
    prepared_candidate_contract(preparation)
        .unwrap_or_else(|error| panic!("guarda anti-vacuidad: el fuente real no cumple: {error}"));

    let obsolete_share_mask = preparation.replacen(
        "FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,",
        "FILE_SHARE_READ | FILE_SHARE_DELETE,",
        1,
    );
    assert_ne!(
        obsolete_share_mask, preparation,
        "guarda anti-vacuidad: la mutación debe encontrar la máscara de compartición arbitrada"
    );
    assert_contract_rejected(
        prepared_candidate_contract(&obsolete_share_mask),
        "compartir exactamente lectura, escritura y borrado",
    );

    let another_connection = preparation.replacen(
        "\n            connection.handle(),",
        "\n            other_connection.handle(),",
        1,
    );
    assert_ne!(
        another_connection, preparation,
        "guarda anti-vacuidad: la mutación debe encontrar connection.handle()"
    );
    assert_contract_rejected(
        prepared_candidate_contract(&another_connection),
        "exactamente connection.handle()",
    );

    let unrelated_connection_handle = preparation.replacen(
        "\n            connection.handle(),",
        "\n            unrelated_handle,",
        1,
    );
    assert_ne!(
        unrelated_connection_handle, preparation,
        "guarda anti-vacuidad: la mutación debe sustituir el handle de la Connection"
    );
    assert_contract_rejected(
        prepared_candidate_contract(&unrelated_connection_handle),
        "exactamente connection.handle()",
    );

    let another_database = preparation.replacen("c\"main\".as_ptr(),", "c\"temp\".as_ptr(),", 1);
    assert_ne!(
        another_database, preparation,
        "guarda anti-vacuidad: la mutación debe encontrar el nombre main"
    );
    assert_contract_rejected(
        prepared_candidate_contract(&another_database),
        "exactamente la base main con un literal C idiomático",
    );

    let manual_c_string = preparation.replacen(
        "c\"main\".as_ptr(),",
        "b\"main\\0\".as_ptr().cast::<c_char>(),",
        1,
    );
    assert_ne!(
        manual_c_string, preparation,
        "guarda anti-vacuidad: la mutación debe encontrar el literal C main"
    );
    assert_contract_rejected(
        prepared_candidate_contract(&manual_c_string),
        "exactamente la base main con un literal C idiomático",
    );

    let unrelated_output = preparation.replacen(
        "ptr::addr_of_mut!(file).cast(),",
        "ptr::addr_of_mut!(other_file).cast(),",
        1,
    );
    assert_ne!(
        unrelated_output, preparation,
        "guarda anti-vacuidad: la mutación debe encontrar el output file"
    );
    assert_contract_rejected(
        prepared_candidate_contract(&unrelated_output),
        "exactamente en la variable file luego validada",
    );

    let unrelated_operand = preparation.replacen(
        "ReOpenFile(\n            original,",
        "ReOpenFile(\n            path_derived_handle,",
        1,
    );
    assert_ne!(
        unrelated_operand, preparation,
        "guarda anti-vacuidad: la mutación del operando debe encontrar el ReOpenFile real"
    );
    assert_contract_rejected(
        prepared_candidate_contract(&unrelated_operand),
        "ReOpenFile debe recibir exactamente original",
    );

    let wrongly_owned_original = preparation.replacen(
        "handle: OwnedHandle(handle),",
        "handle: OwnedHandle(original),",
        1,
    );
    assert_ne!(
        wrongly_owned_original, preparation,
        "guarda anti-vacuidad: la mutación de propiedad debe encontrar el PreparedCandidate real"
    );
    assert_contract_rejected(
        prepared_candidate_contract(&wrongly_owned_original),
        "PreparedCandidate debe poseer exactamente el handle devuelto por ReOpenFile",
    );

    let wrongly_owned_unrelated = preparation.replacen(
        "handle: OwnedHandle(handle),",
        "handle: OwnedHandle(unrelated_handle),",
        1,
    );
    assert_ne!(
        wrongly_owned_unrelated, preparation,
        "guarda anti-vacuidad: la segunda mutación de propiedad debe encontrar el PreparedCandidate real"
    );
    assert_contract_rejected(
        prepared_candidate_contract(&wrongly_owned_unrelated),
        "PreparedCandidate debe poseer exactamente el handle devuelto por ReOpenFile",
    );
}

/// Negativo C5/C6 — conservar la extracción correcta en el fuente no basta si un binding posterior
/// reemplaza `original`: `ReOpenFile` debe consumir sin sombreado el handle del sqlite3_file.
#[test]
fn c5_c6_contrafactual_rechaza_sombrear_original_antes_de_reopenfile() {
    let sources = normalized_sources();
    let preparation = section(
        &sources.windows_vfs,
        "pub(crate) fn prepare_candidate(connection: &Connection)",
        "\nimpl PreparedCandidate",
    );
    prepared_candidate_contract(preparation)
        .unwrap_or_else(|error| panic!("guarda anti-vacuidad: el fuente real no cumple: {error}"));

    let shadowed_original = preparation.replacen(
        "let original = unsafe { (*file.cast::<WinFilePrefix>()).handle };",
        "let original = unsafe { (*file.cast::<WinFilePrefix>()).handle };\n    let original = INVALID_HANDLE_VALUE;",
        1,
    );
    assert_ne!(
        shadowed_original, preparation,
        "guarda anti-vacuidad: la mutación debe insertar el sombreado después de extraer el handle validado"
    );
    assert!(
        shadowed_original.contains(
            "let original = INVALID_HANDLE_VALUE;\n    let handle = unsafe {\n        ReOpenFile(\n            original,"
        ),
        "guarda anti-vacuidad: el contrafactual debe hacer que ReOpenFile consuma el binding sombreado"
    );
    assert_contract_rejected(
        prepared_candidate_contract(&shadowed_original),
        "original no puede sombrearse",
    );
}

/// C5/C6 — native Windows evidence requires an absolute Win32 drive/UNC target, and rejects the
/// Object Manager prefix, cwd-relative basename and parent-handle forms. Publication preserves
/// that Win32 path with `RootDirectory=NULL`. There is one mutation and no alternate corridor.
#[test]
fn c5_c6_windows_publica_destino_win32_absoluto_en_un_unico_syscall() {
    let sources = normalized_sources();
    let rename = section(
        &sources.windows_vfs,
        "fn rename_handle_to(",
        "\nfn wide_path(path: &Path)",
    );

    assert_eq!(
        rename.matches("SetFileInformationByHandle(").count(),
        1,
        "C5/C6 Windows: publication must have exactly one native rename syscall"
    );
    assert!(
        rename.contains("wide_path(target)")
            && rename.contains("windows_rename_path::validate_win32_rename_path(&wide)")
            && rename.contains("(*info).RootDirectory = ptr::null_mut()"),
        "C5/C6 Windows: the sole syscall must receive the validated, preserved Win32-absolute target with RootDirectory=NULL"
    );
    assert!(
        rename.contains("FileRenameInfoEx")
            && sources
                .windows_vfs
                .contains("FILE_RENAME_FLAG_REPLACE_IF_EXISTS")
            && sources
                .windows_vfs
                .contains("FILE_RENAME_FLAG_POSIX_SEMANTICS"),
        "anti-vacuity guard: the single publication must retain FileRenameInfoEx POSIX replace semantics"
    );
    assert!(
        !rename.contains("(*info).RootDirectory = parent_handle")
            && !rename.contains("ERROR_INVALID_PARAMETER")
            && !rename.contains("CreateFileW(")
            && !rename.contains("target.parent()")
            && !rename.contains("target.file_name()")
            && !rename.contains("wide_path(Path::new(file_name))")
            && !rename.contains(r#"\??\"#),
        "C5/C6 Windows: no basename/cwd, parent-handle, Object Manager prefix or INVALID_PARAMETER fallback may remain"
    );
    assert_eq!(
        rename
            .matches("(*info).Anonymous.Flags = extended_flags.unwrap_or(0);")
            .count(),
        1,
        "C5 Windows: the requested strong flags must be materialized exactly once"
    );
    assert!(
        unique_position(
            rename,
            "SetFileInformationByHandle(handle, FileRenameInfoEx, info.cast(), buffer_size)",
        ) < unique_position(rename, "if renamed == 0 {")
            && rename.contains("let buffer_size = u32::try_from(total_bytes)"),
        "C5/C6 Windows: the sole syscall must precede its sole error gate"
    );

    let relative_counterfactual = rename.replacen(
        "(*info).RootDirectory = ptr::null_mut()",
        "(*info).RootDirectory = parent_handle",
        1,
    );
    assert_ne!(
        relative_counterfactual, rename,
        "anti-vacuity guard: the mutation must find the real RootDirectory assignment"
    );
    assert!(
        relative_counterfactual.contains("(*info).RootDirectory = parent_handle")
            && !relative_counterfactual.contains("(*info).RootDirectory = ptr::null_mut()"),
        "anti-vacuity guard: the counterfactual must remove the sole absolute-target anchor"
    );

    let object_manager_counterfactual = rename.replacen(
        "windows_rename_path::validate_win32_rename_path(&wide)",
        r#"Ok([r"\??\".encode_utf16().collect::<Vec<_>>(), wide].concat())"#,
        1,
    );
    assert_ne!(
        object_manager_counterfactual, rename,
        "anti-vacuity guard: the mutation must find the Win32 validation"
    );
    assert!(
        object_manager_counterfactual.contains(r#"\??\"#)
            && !object_manager_counterfactual
                .contains("windows_rename_path::validate_win32_rename_path(&wide)"),
        "anti-vacuity guard: the counterfactual must inject an Object Manager prefix into FileName"
    );
}
