//! E35-H03 CI23 review repair — portable guards for the Windows publication protocol.
//!
//! These tests run on every host and inspect the Windows implementation that the crate ships.
//! They pin relationships that cannot be exercised on a Unix runner: the validated SQLite file
//! object must remain the publication source, and SMB-incompatible relative renames get one
//! narrowly conditioned absolute retry.

const STORE_SOURCE: &str = include_str!("../src/lib.rs");
const SCHEMA_SOURCE: &str = include_str!("../src/schema.rs");
const WINDOWS_VFS_SOURCE: &str = include_str!("../src/windows_vfs.rs");

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

fn braced_block_from<'a>(source: &'a str, marker_at: usize, marker: &str) -> &'a str {
    assert!(
        source[marker_at..].starts_with(marker),
        "anti-vacuity guard: expected `{marker}` at byte {marker_at}"
    );
    let open_at = marker_at
        + source[marker_at..]
            .find('{')
            .unwrap_or_else(|| panic!("anti-vacuity guard: `{marker}` has no opening brace"));
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[open_at..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth
                    .checked_sub(1)
                    .unwrap_or_else(|| panic!("anti-vacuity guard: unbalanced `{marker}` block"));
                if depth == 0 {
                    return &source[marker_at..=open_at + offset];
                }
            }
            _ => {}
        }
    }
    panic!("anti-vacuity guard: unterminated `{marker}` block")
}

fn unique_braced_block_at<'a>(source: &'a str, marker: &str) -> &'a str {
    braced_block_from(source, unique_position(source, marker), marker)
}

/// C5/C6 — validation, the pause seam, and the atomic rename must all refer to one pinned file
/// object. Extracting SQLite's `sqlite3_file` and duplicating its native handle prevents a path
/// reopen from selecting a different `.next`; excluding `FILE_SHARE_WRITE` prevents an external
/// writer from changing the validated candidate before publication.
#[test]
fn c5_c6_windows_publica_el_mismo_handle_validado_sin_reabrir_next_por_path() {
    let rebuild = section(
        STORE_SOURCE,
        "fn rebuild_from_inventory_with_duration(",
        "\n    fn swap_active(",
    );
    let validation = section(
        SCHEMA_SOURCE,
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
        SCHEMA_SOURCE.contains("open_validation_connection")
            && SCHEMA_SOURCE.contains("OpenFlags::SQLITE_OPEN_READ_ONLY"),
        "C5/C6: the validation Connection must be opened explicitly read-only"
    );
    assert!(
        validation.contains("&Connection"),
        "C5/C6: integrity and FK validation must consume the caller's live &Connection"
    );
    assert!(
        WINDOWS_VFS_SOURCE.contains("SQLITE_FCNTL_FILE_POINTER")
            && WINDOWS_VFS_SOURCE.contains("sqlite3_file_control"),
        "C5/C6 Windows: publication must derive its pinned native handle from the sqlite3_file owned by the validation Connection"
    );

    let prepared = section(
        WINDOWS_VFS_SOURCE,
        "pub(crate) struct PreparedCandidate",
        "pub(crate) fn replace_durable",
    );
    let preparation = section(
        WINDOWS_VFS_SOURCE,
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
        prepared.contains("FILE_SHARE_READ | FILE_SHARE_DELETE")
            && !prepared.contains("FILE_SHARE_WRITE"),
        "C6 Windows: the pinned candidate must deny write sharing while allowing read/delete publication semantics"
    );
    assert!(
        prepared.contains("impl PreparedCandidate")
            && prepared.contains("fn sync(")
            && prepared.contains("FlushFileBuffers("),
        "C5/C6 Windows: PreparedCandidate::sync must flush the owned handle with FlushFileBuffers"
    );

    assert_precedes(
        rebuild,
        "open_validation_connection(&next)",
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
        "self.swap_active(&next, candidate)",
        "C5/C6 Windows: pass the same candidate explicitly across the pause into publication",
    );

    let swap = section(
        STORE_SOURCE,
        "    fn swap_active(",
        "\n    /// Reabre la conexión compartida",
    );
    assert!(
        swap.contains("candidate: windows_vfs::PreparedCandidate")
            && swap.contains("replace_durable(candidate, &active)"),
        "C5/C6 Windows: swap_active must receive and consume PreparedCandidate explicitly"
    );

    let replace = section(
        WINDOWS_VFS_SOURCE,
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
    let preparation = section(
        WINDOWS_VFS_SOURCE,
        "pub(crate) fn prepare_candidate(connection: &Connection)",
        "\nimpl PreparedCandidate",
    );
    prepared_candidate_contract(preparation)
        .unwrap_or_else(|error| panic!("guarda anti-vacuidad: el fuente real no cumple: {error}"));

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
    let preparation = section(
        WINDOWS_VFS_SOURCE,
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

/// C5/C6 — some SMB servers reject the relative `RootDirectory` form with
/// `ERROR_INVALID_PARAMETER`. Only that pre-mutation failure may retry with an absolute target;
/// the retry must retain the extended atomic replace flags and no successful first rename may be
/// followed by another mutation.
#[test]
fn c5_c6_windows_smb_reintenta_absoluto_solo_tras_invalid_parameter() {
    let rename = section(
        WINDOWS_VFS_SOURCE,
        "fn rename_handle_to(",
        "\nfn wide_path(path: &Path)",
    );

    let failed_gate = "if renamed == 0 {";
    let failed_gate_positions: Vec<_> = rename
        .match_indices(failed_gate)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        failed_gate_positions.len(),
        2,
        "anti-vacuity guard: primary and fallback calls must each test their result once; positions={failed_gate_positions:?}"
    );
    let failed_gate_at = failed_gate_positions[0];
    let primary = &rename[..failed_gate_at];
    let failed_attempt = braced_block_from(rename, failed_gate_at, failed_gate);
    let invalid_parameter_gate =
        "if error.raw_os_error() != Some(ERROR_INVALID_PARAMETER as i32) {";
    let rejected_other_error = unique_braced_block_at(failed_attempt, invalid_parameter_gate);
    let absolute_at = unique_position(failed_attempt, "let mut wide = wide_path(target);");
    let fallback = &failed_attempt[absolute_at..];

    assert!(
        primary.contains("target.parent()")
            && primary.contains("target.file_name()")
            && primary.contains("(*info).RootDirectory = parent_handle"),
        "anti-vacuity guard: the primary attempt must remain the reparse-safe relative RootDirectory rename"
    );
    assert!(
        primary.contains("FileRenameInfoEx")
            && WINDOWS_VFS_SOURCE.contains("FILE_RENAME_FLAG_REPLACE_IF_EXISTS")
            && WINDOWS_VFS_SOURCE.contains("FILE_RENAME_FLAG_POSIX_SEMANTICS"),
        "anti-vacuity guard: the primary publication must retain FileRenameInfoEx POSIX replace semantics"
    );
    assert!(
        failed_attempt.contains("ERROR_INVALID_PARAMETER"),
        "C5/C6 Windows SMB: the absolute fallback must be gated specifically by ERROR_INVALID_PARAMETER"
    );
    assert!(
        !primary.contains("wide_path(target)")
            && !primary.contains("RootDirectory = ptr::null_mut()"),
        "C5/C6 Windows SMB: the primary attempt cannot construct or use the absolute fallback"
    );
    assert!(
        rejected_other_error.contains("return Err(error);")
            && unique_position(failed_attempt, "return Err(error);") < absolute_at,
        "C5/C6 Windows SMB: every non-INVALID_PARAMETER error must return before constructing the absolute destination"
    );
    assert!(
        fallback.contains("(*info).RootDirectory = ptr::null_mut()")
            && fallback.contains("wide_path(target)"),
        "C5/C6 Windows SMB: only the failed INVALID_PARAMETER branch may rebuild FILE_RENAME_INFO with NULL RootDirectory and an absolute destination"
    );

    assert_eq!(
        failed_attempt.find(failed_gate),
        Some(0),
        "C5/C6 Windows SMB: the fallback protocol must be nested under the failed primary result"
    );
    assert!(
        unique_position(
            primary,
            "SetFileInformationByHandle(handle, FileRenameInfoEx, info.cast(), total_bytes as u32)",
        ) < failed_gate_at,
        "C5/C6 Windows SMB: the primary syscall must precede its zero-result gate"
    );
    assert!(
        unique_position(
            failed_attempt,
            "let error = std::io::Error::last_os_error();"
        ) > failed_gate.len(),
        "C5/C6 Windows SMB: the zero-result gate must precede capture of the OS error"
    );
    assert_precedes(
        failed_attempt,
        "let error = std::io::Error::last_os_error();",
        invalid_parameter_gate,
        "C5/C6 Windows SMB: raw_os_error may only classify the captured failed syscall",
    );
    assert_precedes(
        failed_attempt,
        invalid_parameter_gate,
        "let mut wide = wide_path(target);",
        "C5/C6 Windows SMB: the exact INVALID_PARAMETER gate must precede absolute path construction",
    );
    assert_eq!(
        primary.matches("SetFileInformationByHandle(").count(),
        1,
        "C5/C6 Windows SMB: the primary branch must issue exactly one rename"
    );
    assert_eq!(
        fallback.matches("SetFileInformationByHandle(").count(),
        1,
        "C5/C6 Windows SMB: the failed INVALID_PARAMETER branch must issue exactly one fallback rename"
    );
    assert_eq!(
        primary
            .matches("(*info).Anonymous.Flags = extended_flags.unwrap_or(0);")
            .count(),
        1,
        "C5 Windows SMB: the primary attempt must use the requested extended flags exactly once"
    );
    assert_eq!(
        fallback
            .matches("(*info).Anonymous.Flags = extended_flags.unwrap_or(0);")
            .count(),
        1,
        "C5 Windows SMB: the fallback must retain the same extended flags exactly once"
    );
    assert_eq!(
        primary.matches("FileRenameInfoEx").count(),
        1,
        "C5 Windows SMB: the primary attempt must use FileRenameInfoEx"
    );
    assert_eq!(
        fallback.matches("FileRenameInfoEx").count(),
        1,
        "C5 Windows SMB: the fallback must use FileRenameInfoEx rather than weakening the protocol"
    );
    assert!(
        failed_attempt.contains("let mut wide = wide_path(target);")
            && !rename[failed_gate_at + failed_attempt.len()..].contains("wide_path(target)"),
        "C5/C6 Windows SMB: successful primary publication must bypass all absolute-fallback construction"
    );
}
