//! E35-H03 CI26/CI27/CI29/CI30/CI31 — recuperación disk-backed tras consumir el rename Windows.
//!
//! Antes del punto de no retorno, la generación vieja sigue siendo la recuperación correcta. Tras
//! el rename, conservar ese `standby` mientras se abre el target puede hacer que Windows/SQLite
//! resuelvan otra vez el objeto viejo. El protocolo actualizado exige un fallback read-only del
//! candidato, abierto antes de adquirir el handle `DELETE`, autenticado por FILE_ID antes del
//! rename y transportado explícitamente hasta el swap; después se cierra el standby viejo y toda
//! salida restaura el fallback candidato, nunca el placeholder in-memory.

const STORE_SOURCE: &str = include_str!("../src/lib.rs");

fn swap_protocol() -> &'static str {
    let start = STORE_SOURCE
        .find("    fn swap_active(")
        .expect("guarda anti-vacuidad: falta swap_active");
    let end = STORE_SOURCE[start..]
        .find("\n    #[cfg(windows)]\n    fn verify_published_document_count(")
        .expect("guarda anti-vacuidad: falta el límite posterior de swap_active");
    &STORE_SOURCE[start..start + end]
}

fn rebuild_protocol() -> &'static str {
    let start = STORE_SOURCE
        .find("fn rebuild_from_inventory_with_duration(")
        .expect("guarda anti-vacuidad: falta rebuild_from_inventory_with_duration");
    let end = STORE_SOURCE[start..]
        .find("\n    fn swap_active(")
        .expect("guarda anti-vacuidad: falta el límite posterior del rebuild");
    &STORE_SOURCE[start..start + end]
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

fn candidate_fallback_contract(rebuild: &str, swap: &str) -> Result<(), String> {
    let fallback_at = unique_position(
        rebuild,
        "let candidate_standby = schema::open_validation_connection(&next)",
        "C6 CI29: abrir candidate_standby por VFS antes de adquirir el handle DELETE",
    )?;
    let prepare_at = unique_position(
        rebuild,
        "let candidate = windows_vfs::prepare_candidate(&validation_conn)",
        "C5/C6: preparar el handle candidato desde la conexión validada",
    )?;
    let identity_at = unique_position(
        rebuild,
        "let candidate_identity = candidate.identity();",
        "C6: conservar la identidad del candidato",
    )?;
    let authenticate_at = unique_position(
        rebuild,
        "windows_vfs::connection_identity(&candidate_standby)",
        "C6: autenticar el fallback contra candidate_id",
    )?;
    let mismatch_at = unique_position(
        rebuild,
        "candidate_standby_identity != candidate_identity",
        "C6: rechazar un fallback de otro FILE_ID",
    )?;
    let validation_drop_at = unique_position(
        rebuild,
        "drop(validation_conn);",
        "C5/C6: cerrar la conexión de validación antes del sync",
    )?;
    let sync_at = unique_position(
        rebuild,
        "let candidate_sync = candidate.sync();",
        "C5/C6: sincronizar el handle candidato antes del swap",
    )?;
    let swap_at = unique_position(
        rebuild,
        "self.swap_active(&next, candidate, candidate_standby)?;",
        "C6 CI29: transportar candidate y candidate_standby hasta el swap",
    )?;

    if !(fallback_at < prepare_at
        && prepare_at < identity_at
        && identity_at < authenticate_at
        && authenticate_at < mismatch_at
        && mismatch_at < validation_drop_at
        && validation_drop_at < sync_at
        && sync_at < swap_at)
    {
        return Err(format!(
            "C6 CI29: candidate_standby debe abrirse antes de adquirir el handle DELETE y ambos deben autenticarse y transportarse en orden; fallback={fallback_at}, prepare={prepare_at}, identity={identity_at}, auth={authenticate_at}, mismatch={mismatch_at}, drop_validation={validation_drop_at}, sync={sync_at}, swap={swap_at}"
        ));
    }

    if !swap.contains("candidate_standby: Connection,") {
        return Err(
            "C6 CI29: swap_active debe recibir explícitamente candidate_standby disk-backed".into(),
        );
    }
    for forbidden in [
        "schema::open_validation_connection(next)",
        "schema::open_validation_connection(&next)",
    ] {
        if swap.contains(forbidden) {
            return Err(format!(
                "C6 CI29: swap_active no puede reabrir .next después de adquirir el handle DELETE; apareció `{forbidden}`"
            ));
        }
    }

    let rename_at = unique_position(
        swap,
        "replace_durable(candidate, &active)",
        "C6: rename del candidato",
    )?;
    let old_drop_at = swap[rename_at..]
        .find("drop(standby);")
        .map(|offset| rename_at + offset)
        .ok_or("C6: cerrar el standby viejo inmediatamente después del rename")?;
    let target_open_at = unique_position(
        swap,
        "let published = match open_sqlite(&active)",
        "C6: apertura autenticada del target",
    )?;

    if !(rename_at < old_drop_at && old_drop_at < target_open_at) {
        return Err(format!(
            "C6: orden post-rename inválido rename={rename_at}, drop_old={old_drop_at}, open_target={target_open_at}"
        ));
    }
    Ok(())
}

fn recovery_before_exit(branch: &str, exit: &str) -> Result<(), String> {
    let return_at = branch
        .find(exit)
        .ok_or_else(|| format!("guarda anti-vacuidad: el brazo no contiene la salida `{exit}`"))?;
    let prefix = &branch[..return_at];
    if !prefix.contains("*guard = candidate_standby;") {
        return Err(
            "C6: tras el rename debe reinstalar el fallback candidato disk-backed antes de retornar"
                .into(),
        );
    }
    if prefix.contains("*guard = standby;") {
        return Err("C6: no reinstalar la generación vieja después del rename".into());
    }
    Ok(())
}

fn implicit_exit_recovery_contract(protocol: &str) -> Result<(), String> {
    let activation_binding_at = unique_position(
        protocol,
        "let activation = activate_published_connection(&published);",
        "C6 CI30: conservar el resultado de activación hasta instalar el fallback",
    )?;
    let tail = &protocol[activation_binding_at..];
    let fallback_install_at = unique_position(
        tail,
        "*guard = candidate_standby;",
        "C6 CI30: instalar candidate_standby antes de las salidas implícitas",
    )?;
    let activation_exit_at = unique_position(
        tail,
        "activation?;",
        "C6 CI30: salida implícita de activación",
    )?;
    let identity_exit_at = unique_position(
        tail,
        "let identity = db_identity(&active).map_err(|error| StoreError::Io(error.to_string()))?;",
        "C6 CI30: salida implícita al obtener db_identity",
    )?;
    let published_install_at = unique_position(
        tail,
        "let candidate_standby = std::mem::replace(&mut *guard, published);",
        "C6 CI30: published solo sustituye al fallback después de las operaciones fallibles",
    )?;

    if !(fallback_install_at < activation_exit_at
        && activation_exit_at < identity_exit_at
        && identity_exit_at < published_install_at)
    {
        return Err(format!(
            "C6 CI30: candidate_standby debe estar en RootState antes de activation? y db_identity(...)?; published solo puede sustituirlo después; fallback={fallback_install_at}, activation={activation_exit_at}, identity={identity_exit_at}, published={published_install_at}"
        ));
    }
    Ok(())
}

fn fallback_authentication_contract(rebuild: &str) -> Result<(), String> {
    let fallback_binding = "let candidate_standby = schema::open_validation_connection(&next)";
    let fallback_at = unique_position(
        rebuild,
        fallback_binding,
        "C6 CI30: candidate_standby debe tener un único binding disk-backed",
    )?;
    let identity_binding =
        "let candidate_standby_identity = windows_vfs::connection_identity(&candidate_standby)";
    let identity_at = unique_position(
        rebuild,
        identity_binding,
        "C6 CI30: autenticar exactamente una vez el binding candidate_standby",
    )?;
    let mismatch = "if candidate_standby_identity != candidate_identity {";
    let mismatch_at = unique_position(
        rebuild,
        mismatch,
        "C6 CI30: branch causal de identidad fallback distinta",
    )?;
    let swap = "self.swap_active(&next, candidate, candidate_standby)?;";
    let swap_at = unique_position(
        rebuild,
        swap,
        "C6 CI30: transportar el mismo binding autenticado hasta swap_active",
    )?;
    if !(fallback_at < identity_at && identity_at < mismatch_at && mismatch_at < swap_at) {
        return Err(format!(
            "C6 CI30: orden inválido al autenticar y transportar candidate_standby; fallback={fallback_at}, identity={identity_at}, mismatch={mismatch_at}, swap={swap_at}"
        ));
    }

    let mismatch_tail = &rebuild[mismatch_at + mismatch.len()..];
    let mismatch_end = mismatch_tail
        .find("\n        }")
        .ok_or("guarda anti-vacuidad: no se pudo delimitar el branch de mismatch del fallback")?;
    let mismatch_branch = &mismatch_tail[..mismatch_end];
    if !mismatch_branch.contains("return Err(StoreError::Io(format!(")
        || !mismatch_branch.contains("validated candidate fallback identity mismatch")
        || !mismatch_branch.contains("candidate_id={candidate_identity:?}")
        || !mismatch_branch.contains("fallback_id={candidate_standby_identity:?}")
    {
        return Err(
            "C6 CI30: un FILE_ID fallback distinto debe retornar Err causal con ambas identidades"
                .into(),
        );
    }

    // Desde la creación hasta el consumo en swap solo puede viajar el binding original. Vigilar
    // recién desde `connection_identity` dejaría fuera precisamente un shadow interpuesto justo
    // antes de autenticar, haciendo que la comprobación legitimase otro handle.
    let authenticated_transport = &rebuild[fallback_at..swap_at + swap.len()];
    if authenticated_transport.matches(fallback_binding).count() != 1
        || authenticated_transport
            .matches("schema::open_validation_connection(&next)")
            .count()
            != 1
        || authenticated_transport
            .matches("candidate_standby =")
            .count()
            != 1
        || authenticated_transport.contains("candidate_standby=")
        || authenticated_transport.contains("open_sqlite(&next)")
    {
        return Err(
            "C6 CI31: candidate_standby no puede sombrearse, reasignarse ni reabrirse entre su creación y el swap"
                .into(),
        );
    }
    Ok(())
}

fn ci30_transport_contract(rebuild: &str) -> Result<(), String> {
    let fallback_binding = "let candidate_standby = schema::open_validation_connection(&next)";
    let identity_binding =
        "let candidate_standby_identity = windows_vfs::connection_identity(&candidate_standby)";
    let identity_at = unique_position(
        rebuild,
        identity_binding,
        "guarda histórica CI30: identity del fallback",
    )?;
    let swap = "self.swap_active(&next, candidate, candidate_standby)?;";
    let swap_at = unique_position(rebuild, swap, "guarda histórica CI30: swap")?;
    let authenticated_transport = &rebuild[identity_at..swap_at + swap.len()];
    if authenticated_transport.matches(fallback_binding).count() != 0
        || authenticated_transport
            .matches("schema::open_validation_connection(&next)")
            .count()
            != 0
        || authenticated_transport.contains("candidate_standby =")
        || authenticated_transport.contains("candidate_standby=")
    {
        return Err("CI30 detectó rebinding después de connection_identity".into());
    }
    Ok(())
}

fn post_rename_recovery_contract(rebuild: &str, protocol: &str) -> Result<(), String> {
    candidate_fallback_contract(rebuild, protocol)?;
    let rename_at = protocol
        .find("replace_durable(candidate, &active)")
        .expect("checked above");
    let post_rename = &protocol[rename_at..];
    if post_rename.contains("*guard = standby;") {
        return Err(
            "C6: ninguna salida posterior al rename puede restaurar el standby viejo".into(),
        );
    }
    if post_rename.contains("Connection::open_in_memory()") {
        return Err("C6: no crear otro placeholder tras el punto de no retorno".into());
    }
    // El rename ya consumido empieza en `directory_sync`: el `return Err(` anterior pertenece al
    // fallo del propio replace y no es todavía una salida post-publicación. Las cuatro salidas
    // explícitas reales usan `publication_error`, no `Err`, y por eso deben enumerarse por su
    // forma efectiva en producción. El cursor separa cada corredor: una restauración anterior no
    // puede satisfacer por accidente dos salidas posteriores.
    let post_publication = &post_rename[post_rename
        .find("let directory_sync =")
        .ok_or("guarda anti-vacuidad: falta el inicio post-rename directory_sync")?..];
    let exits: Vec<_> = post_publication
        .match_indices("return publication_error(")
        .map(|(at, _)| at)
        .collect();
    if exits.len() != 4 {
        return Err(format!(
            "C6: deben observarse las cuatro salidas reales post-rename mediante publication_error; observadas={exits:?}"
        ));
    }
    let expected_context = [
        "if let Err(error) = directory_sync",
        "let published = match open_sqlite(&active)",
        "windows_vfs::connection_identity(&published)",
        "published_identity != candidate_identity",
    ];
    let mut corridor_start = 0;
    for (ordinal, return_at) in exits.into_iter().enumerate() {
        let corridor =
            &post_publication[corridor_start..return_at + "return publication_error(".len()];
        if !corridor.contains(expected_context[ordinal]) {
            return Err(format!(
                "C6: salida post-rename #{} no corresponde al contexto esperado `{}`",
                ordinal + 1,
                expected_context[ordinal]
            ));
        }
        recovery_before_exit(corridor, "return publication_error(")
            .map_err(|error| format!("C6: salida post-rename #{}: {error}", ordinal + 1))?;
        corridor_start = return_at + "return publication_error(".len();
    }
    Ok(())
}

fn assert_rejected(result: Result<(), String>, expected: &str) {
    let error = result.expect_err("la mutación contrafactual debía romper el contrato");
    assert!(
        error.contains(expected),
        "la mutación falló por otra razón: esperada `{expected}`, observada `{error}`"
    );
}

/// C6 / §20.12.2 — la conexión read-only de respaldo debe abrirse antes de que
/// `prepare_candidate` adquiera el handle exclusivo `DELETE`; su FILE_ID se compara después contra
/// el candidato preparado y ambos recursos llegan vivos al swap. Reabrir `.next` dentro del swap
/// reproduce exactamente el `unable to open database file` observado en Windows CI29.
#[test]
fn c6_windows_abre_fallback_antes_del_handle_delete_y_lo_transporta_hasta_swap() {
    let rebuild = rebuild_protocol();
    candidate_fallback_contract(rebuild, swap_protocol())
        .unwrap_or_else(|error| panic!("rojo causal CI29: {error}"));

    // Contrafactual causal: conserva todas las operaciones pero mueve la apertura RO después de
    // prepare_candidate. El oráculo debe rechazarlo porque el handle DELETE ya niega ese open.
    let fallback = "let candidate_standby = schema::open_validation_connection(&next)";
    let prepare = "let candidate = windows_vfs::prepare_candidate(&validation_conn)";
    let moved_after_prepare = rebuild
        .replacen(fallback, "__CI29_FALLBACK__", 1)
        .replacen(prepare, fallback, 1)
        .replacen("__CI29_FALLBACK__", prepare, 1);
    assert_ne!(
        moved_after_prepare, rebuild,
        "guarda anti-vacuidad: el contrafactual debe intercambiar open RO y handle DELETE"
    );
    assert_rejected(
        candidate_fallback_contract(&moved_after_prepare, swap_protocol()),
        "antes de adquirir el handle DELETE",
    );
}

/// C6 / §20.12.2 — si el primer open o su autenticación fallan después del rename, RootState debe
/// recibir el fallback candidato; restaurar el standby viejo volvería a exponer snapshot=0 y dejar
/// el placeholder produciría `no such table` en la siguiente apertura.
#[test]
fn c6_windows_error_post_rename_restaura_candidate_standby() {
    let compliant = r#"Err(error) => {
        *guard = candidate_standby;
        return Err(StoreError::Io(error.to_string()));
    }"#;
    recovery_before_exit(compliant, "return Err(")
        .expect("guarda anti-vacuidad: el fallback candidato debe aceptarse");

    let wrong_generation = compliant.replace("candidate_standby", "standby");
    assert_rejected(
        recovery_before_exit(&wrong_generation, "return Err("),
        "fallback candidato disk-backed",
    );
    let placeholder = compliant.replace("*guard = candidate_standby;", "drop(candidate_standby);");
    assert_rejected(
        recovery_before_exit(&placeholder, "return Err("),
        "fallback candidato disk-backed",
    );

    post_rename_recovery_contract(rebuild_protocol(), swap_protocol())
        .unwrap_or_else(|error| panic!("rojo causal CI29: {error}"));

    // Mutation test local: borrar CADA restauración que protege una salida real debe ser
    // observado por esa salida, no quedar oculto por otra asignación anterior del protocolo.
    let protocol = swap_protocol();
    let post_start = protocol
        .find("let directory_sync =")
        .expect("guarda anti-vacuidad: inicio post-rename");
    let post = &protocol[post_start..];
    let mut cursor = 0;
    for ordinal in 1..=4 {
        let relative_return = post[cursor..]
            .find("return publication_error(")
            .unwrap_or_else(|| panic!("guarda anti-vacuidad: falta salida real #{ordinal}"));
        let return_at = cursor + relative_return;
        let restoration = post[cursor..return_at]
            .rfind("*guard = candidate_standby;")
            .map(|offset| cursor + offset)
            .unwrap_or_else(|| panic!("guarda anti-vacuidad: falta restauración real #{ordinal}"));
        let absolute = post_start + restoration;
        let mut mutant = protocol.to_owned();
        mutant.replace_range(
            absolute..absolute + "*guard = candidate_standby;".len(),
            "drop(candidate_standby);",
        );
        assert_rejected(
            post_rename_recovery_contract(rebuild_protocol(), &mutant),
            &format!("salida post-rename #{ordinal}"),
        );
        cursor = return_at + "return publication_error(".len();
    }
}

/// Negativo C6 — el contrato previo retenía `standby` hasta comparar conexión↔pathname. Ese orden
/// es precisamente el que puede autenticar dos vistas viejas. La guarda rechaza explícitamente
/// cualquier restauración de la generación anterior después de consumir el rename.
#[test]
fn c6_windows_post_rename_no_reinstala_standby_viejo() {
    let compliant = r#"Err(error) => {
        *guard = candidate_standby;
        return Err(error);
    }"#;
    recovery_before_exit(compliant, "return Err(")
        .expect("guarda anti-vacuidad: recuperación candidata conforme");
    let old = compliant.replace("candidate_standby", "standby");
    assert_rejected(
        recovery_before_exit(&old, "return Err("),
        "fallback candidato disk-backed",
    );

    post_rename_recovery_contract(rebuild_protocol(), swap_protocol())
        .unwrap_or_else(|error| panic!("rojo causal CI29: {error}"));
}

/// C6 / §20.12.2 — `activation?` y `db_identity(...)?` también son salidas post-rename aunque no
/// contengan un `return` explícito. RootState debe poseer el fallback candidato antes de ambas, y
/// la conexión publicada solo puede sustituirlo cuando las dos operaciones ya han terminado.
#[test]
fn c6_windows_salidas_implicitas_conservan_fallback_hasta_identidad_persistida() {
    let protocol = swap_protocol();
    implicit_exit_recovery_contract(protocol)
        .unwrap_or_else(|error| panic!("rojo causal CI30: {error}"));

    let activation_binding = "let activation = activate_published_connection(&published);";
    let activation_at = protocol
        .find(activation_binding)
        .expect("guarda anti-vacuidad: binding de activation");
    let (prefix, tail) = protocol.split_at(activation_at);
    let deleted_tail = tail.replacen("*guard = candidate_standby;", "drop(candidate_standby);", 1);
    assert_ne!(deleted_tail, tail, "guarda anti-vacuidad: borrar fallback");
    let deleted = format!("{prefix}{deleted_tail}");

    // Evidencia del gap previo: el oráculo CI29 solo enumeraba los cuatro `return` explícitos y
    // aceptaba que estas dos salidas dejaran instalado el placeholder.
    post_rename_recovery_contract(rebuild_protocol(), &deleted)
        .expect("guarda histórica: CI29 no observaba las salidas implícitas");
    assert_rejected(
        implicit_exit_recovery_contract(&deleted),
        "instalar candidate_standby",
    );

    let moved_tail = tail
        .replacen("*guard = candidate_standby;", "", 1)
        .replacen(
            "activation?;",
            "activation?;\n            *guard = candidate_standby;",
            1,
        );
    assert_ne!(moved_tail, tail, "guarda anti-vacuidad: mover fallback");
    let moved = format!("{prefix}{moved_tail}");
    post_rename_recovery_contract(rebuild_protocol(), &moved)
        .expect("guarda histórica: CI29 aceptaba instalar fallback tras activation?");
    assert_rejected(
        implicit_exit_recovery_contract(&moved),
        "antes de activation? y db_identity",
    );

    let published_install = "let candidate_standby = std::mem::replace(&mut *guard, published);";
    let identity_exit =
        "let identity = db_identity(&active).map_err(|error| StoreError::Io(error.to_string()))?;";
    let published_too_early = protocol
        .replacen(published_install, "__CI30_PUBLISHED_INSTALL__", 1)
        .replacen(
            identity_exit,
            &format!("{published_install}\n        {identity_exit}"),
            1,
        )
        .replace("__CI30_PUBLISHED_INSTALL__", "");
    assert_ne!(
        published_too_early, protocol,
        "guarda anti-vacuidad: adelantar published"
    );
    assert_rejected(
        implicit_exit_recovery_contract(&published_too_early),
        "published solo puede sustituirlo después",
    );
}

/// C6 / §20.12.2 — el fallback de recuperación solo es válido si un FILE_ID distinto produce un
/// error causal y el mismo binding autenticado llega a `swap_active`, sin shadow, reasignación ni
/// un segundo open que invalide la comprobación.
#[test]
fn c6_windows_fallback_distinto_falla_y_binding_autenticado_no_se_reabre() {
    let rebuild = rebuild_protocol();
    fallback_authentication_contract(rebuild)
        .unwrap_or_else(|error| panic!("rojo causal CI30: {error}"));

    let identity_binding =
        "let candidate_standby_identity = windows_vfs::connection_identity(&candidate_standby)";
    let shadow_before_identity = rebuild.replacen(
        identity_binding,
        &format!("let candidate_standby = open_sqlite(&next)?;\n        {identity_binding}"),
        1,
    );
    assert_ne!(
        shadow_before_identity, rebuild,
        "guarda anti-vacuidad: insertar shadow exacto antes de connection_identity"
    );
    candidate_fallback_contract(&shadow_before_identity, swap_protocol())
        .expect("guarda histórica: el contrato base acepta el shadow previo a identity");
    ci30_transport_contract(&shadow_before_identity)
        .expect("guarda histórica: CI30 empezaba a vigilar después del shadow");
    assert_rejected(
        fallback_authentication_contract(&shadow_before_identity),
        "entre su creación y el swap",
    );

    let mismatch_at = rebuild
        .find("if candidate_standby_identity != candidate_identity {")
        .expect("guarda anti-vacuidad: branch mismatch");
    let return_offset = rebuild[mismatch_at..]
        .find("return Err(StoreError::Io(format!(")
        .expect("guarda anti-vacuidad: Err causal mismatch");
    let return_at = mismatch_at + return_offset;
    let mut inert_branch = rebuild.to_owned();
    inert_branch.replace_range(
        return_at..return_at + "return Err".len(),
        "let _ignored = Err",
    );
    candidate_fallback_contract(&inert_branch, swap_protocol())
        .expect("guarda histórica: CI29 aceptaba un branch mismatch inerte");
    assert_rejected(
        fallback_authentication_contract(&inert_branch),
        "debe retornar Err causal",
    );

    let drop_validation = "drop(validation_conn);";
    let shadowed = rebuild.replacen(
        drop_validation,
        "let candidate_standby = reopen_candidate_fallback(&next)?;\n        drop(validation_conn);",
        1,
    );
    assert_ne!(shadowed, rebuild, "guarda anti-vacuidad: insertar shadow");
    candidate_fallback_contract(&shadowed, swap_protocol())
        .expect("guarda histórica: CI29 aceptaba sombrear el binding autenticado");
    assert_rejected(
        fallback_authentication_contract(&shadowed),
        "no puede sombrearse, reasignarse ni reabrirse",
    );

    let reopened = rebuild.replacen(
        drop_validation,
        "let reopened_candidate = schema::open_validation_connection(&next)?;\n        drop(validation_conn);",
        1,
    );
    assert_ne!(reopened, rebuild, "guarda anti-vacuidad: insertar reopen");
    candidate_fallback_contract(&reopened, swap_protocol())
        .expect("guarda histórica: CI29 aceptaba reabrir .next tras autenticar");
    assert_rejected(
        fallback_authentication_contract(&reopened),
        "no puede sombrearse, reasignarse ni reabrirse",
    );
}
