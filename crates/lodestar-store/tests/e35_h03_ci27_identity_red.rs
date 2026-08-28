//! E35-H03 CI27 — autenticación causal de la generación publicada en Windows.
//!
//! El fallo remoto `candidate=4; pathname=0; root_state=0` demuestra que comparar solamente la
//! conexión recién abierta con el pathname puede autenticar dos vistas coherentes entre sí pero
//! ajenas al objeto que superó `integrity_check`. Estas guardas portables fijan el tercer vértice:
//! el `FILE_ID` del handle validado, conservado a través del rename y comparado después del swap.

const STORE_SOURCE: &str = include_str!("../src/lib.rs");
const WINDOWS_VFS_SOURCE: &str = include_str!("../src/windows_vfs.rs");

fn section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start_at = source
        .find(start)
        .unwrap_or_else(|| panic!("guarda anti-vacuidad: falta inicio `{start}`"));
    let remainder = &source[start_at..];
    let end_at = remainder
        .find(end)
        .unwrap_or_else(|| panic!("guarda anti-vacuidad: falta final `{end}`"));
    &remainder[..end_at]
}

fn unique_position(source: &str, needle: &str, reason: &str) -> Result<usize, String> {
    let positions: Vec<_> = source.match_indices(needle).map(|(at, _)| at).collect();
    if positions.len() == 1 {
        Ok(positions[0])
    } else {
        Err(format!(
            "{reason}: `{needle}` debe aparecer exactamente una vez; posiciones={positions:?}"
        ))
    }
}

fn precedes(source: &str, earlier: &str, later: &str, reason: &str) -> Result<(), String> {
    let earlier_at = unique_position(source, earlier, reason)?;
    let later_at = unique_position(source, later, reason)?;
    if earlier_at < later_at {
        Ok(())
    } else {
        Err(format!(
            "{reason}: `{earlier}`@{earlier_at} debe preceder `{later}`@{later_at}"
        ))
    }
}

fn candidate_identity_contract(source: &str) -> Result<(), String> {
    if !source.contains("pub(crate) type FileIdentity")
        && !source.contains("pub(crate) struct FileIdentity")
    {
        return Err("C5/C6: falta un tipo nativo FileIdentity compartido por candidato, pathname y conexión".into());
    }
    let prepared = section(
        source,
        "pub(crate) struct PreparedCandidate",
        "pub(crate) fn replace_durable",
    );
    if !prepared.contains("identity: FileIdentity") {
        return Err(
            "C5/C6: PreparedCandidate debe conservar el FILE_ID del handle validado".into(),
        );
    }

    let preparation = section(
        source,
        "pub(crate) fn prepare_candidate(connection: &Connection)",
        "\nimpl PreparedCandidate",
    );
    for forbidden in [
        "file_identity(original)",
        "path_identity(",
        "open_read_handle(",
    ] {
        if preparation.contains(forbidden) {
            return Err(format!(
                "C5/C6: la identidad candidata debe proceder del handle poseído por PreparedCandidate, no de `{forbidden}`"
            ));
        }
    }
    precedes(
        preparation,
        "if handle == INVALID_HANDLE_VALUE {",
        "let identity = file_identity(handle)",
        "C5/C6: no consultar identidad hasta validar ReOpenFile",
    )?;
    precedes(
        preparation,
        "let identity = file_identity(handle)",
        "Ok(PreparedCandidate {",
        "C5/C6: capturar el FILE_ID antes de construir PreparedCandidate",
    )?;
    if !preparation.contains("handle: OwnedHandle(handle)") || !preparation.contains("identity,") {
        return Err(
            "C5/C6: PreparedCandidate debe poseer juntos el handle reabierto y su identidad".into(),
        );
    }
    if !prepared.contains("pub(crate) fn identity(&self) -> FileIdentity")
        || !prepared.contains("self.identity")
    {
        return Err(
            "C5/C6: PreparedCandidate debe exponer su identidad sin reabrir por path".into(),
        );
    }
    Ok(())
}

fn published_identity_contract(store: &str, windows: &str) -> Result<(), String> {
    for required in [
        "pub(crate) fn connection_identity(",
        "pub(crate) fn path_identity(",
    ] {
        if !windows.contains(required) {
            return Err(format!(
                "C5/C6: falta el probe de identidad post-rename `{required}`"
            ));
        }
    }

    let rebuild = section(
        store,
        "fn rebuild_from_inventory_with_duration(",
        "\n    fn swap_active(",
    );
    precedes(
        rebuild,
        "let candidate_identity = candidate.identity();",
        "self.swap_active(&next, candidate)?;",
        "C5/C6: conservar candidate_id antes de consumir el handle en el rename",
    )?;
    precedes(
        rebuild,
        "self.swap_active(&next, candidate)?;",
        "self.verify_published_document_count(candidate_document_count, candidate_identity)?;",
        "C5/C6: autenticar candidate_id solo después del swap",
    )?;

    let verify = section(
        store,
        "fn verify_published_document_count(",
        "\n    /// Reabre la conexión compartida",
    );
    for required in [
        "candidate_identity: windows_vfs::FileIdentity",
        "windows_vfs::path_identity(&active)",
        "windows_vfs::connection_identity(&root_state)",
        "candidate_identity != pathname_identity",
        "candidate_identity != root_state_identity",
    ] {
        if !verify.contains(required) {
            return Err(format!(
                "C5/C6: autenticación triangular incompleta; falta `{required}`"
            ));
        }
    }
    if !verify.contains("candidate_id={candidate_identity:?}")
        || !verify.contains("pathname_id={pathname_identity:?}")
        || !verify.contains("root_state_id={root_state_identity:?}")
    {
        return Err(
            "C5/C6: el error debe distinguir candidate_id, pathname_id y root_state_id".into(),
        );
    }
    Ok(())
}

fn native_identity_probe_contract(windows: &str) -> Result<(), String> {
    let connection = section(
        windows,
        "pub(crate) fn connection_identity(",
        "\npub(crate) fn path_identity(",
    );
    if !connection.contains("file_identity(connection_main_handle(connection)?)") {
        return Err(
            "C5/C6: connection_identity debe derivar del handle main nativo de esa Connection"
                .into(),
        );
    }
    for forbidden in ["open_read_handle(", "path_identity("] {
        if connection.contains(forbidden) {
            return Err(format!(
                "C5/C6: connection_identity no puede sustituir la conexión por un pathname: apareció `{forbidden}`"
            ));
        }
    }

    let pathname = section(
        windows,
        "pub(crate) fn path_identity(",
        "\npub(crate) fn sidecar_diagnostics(",
    );
    precedes(
        pathname,
        "let path_handle = open_read_handle(path)?;",
        "file_identity(path_handle.0)",
        "C5/C6: path_identity debe identificar el handle nativo abierto desde el pathname",
    )?;
    if pathname.contains("connection_main_handle(") {
        return Err(
            "C5/C6: path_identity no puede reutilizar el handle interno de una Connection".into(),
        );
    }
    Ok(())
}

fn post_rename_open_order_contract(store: &str) -> Result<(), String> {
    let swap = section(
        store,
        "    fn swap_active(",
        "\n    #[cfg(windows)]\n    fn verify_published_document_count(",
    );
    let rename_at = unique_position(
        swap,
        "replace_durable(candidate, &active)",
        "C5 Windows: debe existir un único rename del PreparedCandidate",
    )?;
    let target_open_marker = "let published = match open_sqlite(&active)";
    let target_open_at = unique_position(
        swap,
        target_open_marker,
        "C5 Windows: debe existir una apertura post-rename del target",
    )?;
    if rename_at >= target_open_at {
        return Err("C5 Windows: el rename debe preceder la reapertura del target".into());
    }
    let corridor = &swap[rename_at..target_open_at];
    if !corridor.contains("drop(standby);") {
        return Err(
            "C5 Windows: standby de la generación vieja debe cerrarse tras el rename y antes de abrir/probar el target"
                .into(),
        );
    }
    for forbidden in [
        "connection_matches_path(",
        "path_identity(",
        "open_validation_connection(",
    ] {
        if corridor.contains(forbidden) {
            return Err(format!(
                "C5 Windows: no probar el target mientras standby puede aplazar la sustitución; apareció `{forbidden}`"
            ));
        }
    }
    Ok(())
}

fn sidecar_diagnostic_contract(store: &str, windows: &str) -> Result<(), String> {
    if !windows.contains("pub(crate) fn sidecar_diagnostics(") {
        return Err(
            "C6: falta diagnóstico WAL/SHM cuando los conteos divergen tras el swap".into(),
        );
    }
    let diagnostics = section(
        windows,
        "pub(crate) fn sidecar_diagnostics(",
        "\nfn connection_main_handle(",
    );
    for required in [
        "[\"-wal\", \"-shm\"]",
        "path_identity(&sidecar)",
        "ErrorKind::NotFound",
    ] {
        if !diagnostics.contains(required) {
            return Err(format!(
                "C6: el diagnóstico debe separar sidecars ausentes/presentes por identidad; falta `{required}`"
            ));
        }
    }

    let verify = section(
        store,
        "fn verify_published_document_count(",
        "\n    /// Reabre la conexión compartida",
    );
    if !verify.contains("windows_vfs::sidecar_diagnostics(&active)")
        || !verify.contains("sidecars={sidecars}")
    {
        return Err(
            "C6: un mismatch post-swap debe informar identidades de main y estado de WAL/SHM"
                .into(),
        );
    }
    Ok(())
}

fn assert_rejected(result: Result<(), String>, expected: &str) {
    let error = result.expect_err("el contrafactual debía romper el contrato");
    assert!(
        error.contains(expected),
        "el contrafactual falló por otra causa: esperada `{expected}`, observada `{error}`"
    );
}

/// C5/C6 + §20.12.2 — el oráculo previo (`connection == pathname`) no identifica el objeto que
/// pasó integridad. El handle reabierto desde `SQLITE_FCNTL_FILE_POINTER` debe capturar su FILE_ID
/// y `PreparedCandidate` debe conservarlo aunque el rename consuma el handle.
#[test]
fn c5_c6_windows_prepared_candidate_conserva_file_id_del_handle_validado() {
    candidate_identity_contract(WINDOWS_VFS_SOURCE)
        .unwrap_or_else(|error| panic!("rojo causal CI27: {error}"));

    let wrong_object = WINDOWS_VFS_SOURCE.replacen(
        "let identity = file_identity(handle)",
        "let identity = file_identity(original)",
        1,
    );
    assert_ne!(
        wrong_object, WINDOWS_VFS_SOURCE,
        "guarda anti-vacuidad: debe existir la captura del FILE_ID candidato"
    );
    assert_rejected(
        candidate_identity_contract(&wrong_object),
        "no de `file_identity(original)`",
    );
}

/// C5/C6 + BDD-A4 — después del rename, tanto el pathname como la conexión que se instalará en
/// `RootState` deben compararse contra candidate_id. Que pathname y conexión coincidan entre sí no
/// basta: ambos pueden seguir viendo la generación anterior, como demostró el rojo Windows CI27.
#[test]
fn c5_c6_post_rename_autentica_pathname_y_rootstate_contra_candidate_id() {
    native_identity_probe_contract(WINDOWS_VFS_SOURCE)
        .unwrap_or_else(|error| panic!("rojo causal CI28: {error}"));
    published_identity_contract(STORE_SOURCE, WINDOWS_VFS_SOURCE)
        .unwrap_or_else(|error| panic!("rojo causal CI27: {error}"));

    let pathname_as_connection = WINDOWS_VFS_SOURCE.replacen(
        "file_identity(connection_main_handle(connection)?)",
        "path_identity(std::path::Path::new(\"index.db\"))",
        1,
    );
    assert_ne!(
        pathname_as_connection, WINDOWS_VFS_SOURCE,
        "guarda anti-vacuidad: debe existir la derivación desde connection_main_handle"
    );
    assert_rejected(
        native_identity_probe_contract(&pathname_as_connection),
        "handle main nativo de esa Connection",
    );

    let wrong_comparison = STORE_SOURCE.replacen(
        "candidate_identity != root_state_identity",
        "pathname_identity != root_state_identity",
        1,
    );
    assert_ne!(
        wrong_comparison, STORE_SOURCE,
        "guarda anti-vacuidad: debe existir la comparación candidate↔RootState"
    );
    assert_rejected(
        published_identity_contract(&wrong_comparison, WINDOWS_VFS_SOURCE),
        "candidate_identity != root_state_identity",
    );
}

/// C5 Windows — el handle `standby` pertenece a la generación sustituida. Aunque sirve de
/// recuperación antes del punto de no retorno, mantenerlo vivo durante el primer open/probe del
/// target permite que Windows/SQLite resuelvan todavía la generación vieja por nombre. Debe
/// cerrarse después del rename durable y antes de cualquier autenticación del pathname.
#[test]
fn c5_windows_cierra_standby_antes_de_abrir_target_post_rename() {
    post_rename_open_order_contract(STORE_SOURCE)
        .unwrap_or_else(|error| panic!("rojo causal CI27: {error}"));

    let swap = section(
        STORE_SOURCE,
        "    fn swap_active(",
        "\n    #[cfg(windows)]\n    fn verify_published_document_count(",
    );
    let rename_at = swap
        .find("replace_durable(candidate, &active)")
        .expect("guarda anti-vacuidad: rename candidato");
    let open_at = swap
        .find("let published = match open_sqlite(&active)")
        .expect("guarda anti-vacuidad: open post-rename");
    let drop_at = swap[rename_at..open_at]
        .find("drop(standby);")
        .map(|offset| rename_at + offset)
        .expect("guarda anti-vacuidad: cierre ordenado de standby");
    let without_ordered_drop = format!(
        "{}{}",
        &STORE_SOURCE[..STORE_SOURCE.find(swap).unwrap() + drop_at],
        &STORE_SOURCE[STORE_SOURCE.find(swap).unwrap() + drop_at + "drop(standby);".len()..]
    );
    assert_rejected(
        post_rename_open_order_contract(&without_ordered_drop),
        "standby de la generación vieja debe cerrarse",
    );
}

/// Negativo C6 — si los tres FILE_ID coinciden pero divergen los conteos, el fallo ya no apunta a
/// un rename del objeto equivocado sino a la vista SQLite (WAL/SHM o caché compartida por nombre).
/// El diagnóstico debe enumerar ambos sidecars y sus identidades para que el siguiente arreglo sea
/// causal y no otro retry ciego.
#[test]
fn c6_mismatch_post_swap_diagnostica_file_ids_y_sidecars_wal_shm() {
    sidecar_diagnostic_contract(STORE_SOURCE, WINDOWS_VFS_SOURCE)
        .unwrap_or_else(|error| panic!("rojo causal CI27: {error}"));

    let without_shm = WINDOWS_VFS_SOURCE.replacen("[\"-wal\", \"-shm\"]", "[\"-wal\"]", 1);
    assert_ne!(
        without_shm, WINDOWS_VFS_SOURCE,
        "guarda anti-vacuidad: el diagnóstico debe declarar WAL y SHM"
    );
    assert_rejected(
        sidecar_diagnostic_contract(STORE_SOURCE, &without_shm),
        "[\"-wal\", \"-shm\"]",
    );
}
