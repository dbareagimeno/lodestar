//! E35-H03 CI26/CI27 — recuperación disk-backed tras consumir el rename Windows.
//!
//! Antes del punto de no retorno, la generación vieja sigue siendo la recuperación correcta. Tras
//! el rename, conservar ese `standby` mientras se abre el target puede hacer que Windows/SQLite
//! resuelvan otra vez el objeto viejo. El protocolo actualizado exige un fallback read-only del
//! candidato, autenticado por FILE_ID antes del rename; después se cierra el standby viejo y toda
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

fn candidate_fallback_contract(protocol: &str) -> Result<(), String> {
    let identity_at = unique_position(
        protocol,
        "let candidate_identity = candidate.identity();",
        "C6: conservar la identidad del candidato",
    )?;
    let fallback_at = unique_position(
        protocol,
        "let candidate_standby = schema::open_validation_connection(next)",
        "C6: preparar fallback disk-backed del candidato",
    )?;
    let authenticate_at = unique_position(
        protocol,
        "windows_vfs::connection_identity(&candidate_standby)",
        "C6: autenticar el fallback contra candidate_id",
    )?;
    let mismatch_at = unique_position(
        protocol,
        "candidate_standby_identity != candidate_identity",
        "C6: rechazar un fallback de otro FILE_ID",
    )?;
    let rename_at = unique_position(
        protocol,
        "replace_durable(candidate, &active)",
        "C6: rename del candidato",
    )?;
    let old_drop_at = protocol[rename_at..]
        .find("drop(standby);")
        .map(|offset| rename_at + offset)
        .ok_or("C6: cerrar el standby viejo inmediatamente después del rename")?;
    let target_open_at = unique_position(
        protocol,
        "let published = match open_sqlite(&active)",
        "C6: apertura autenticada del target",
    )?;

    if !(identity_at < fallback_at
        && fallback_at < authenticate_at
        && authenticate_at < mismatch_at
        && mismatch_at < rename_at
        && rename_at < old_drop_at
        && old_drop_at < target_open_at)
    {
        return Err(format!(
            "C6: orden inválido identity={identity_at}, fallback={fallback_at}, auth={authenticate_at}, mismatch={mismatch_at}, rename={rename_at}, drop_old={old_drop_at}, open_target={target_open_at}"
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

fn post_rename_recovery_contract(protocol: &str) -> Result<(), String> {
    candidate_fallback_contract(protocol)?;
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

/// C6 / §20.12.2 — antes de soltar el único handle viejo debe existir una conexión read-only al
/// candidato y su FILE_ID debe coincidir con el capturado durante integridad. Así cualquier error
/// post-rename dispone de recuperación real sin mantener vivo el objeto que retrasa el replace.
#[test]
fn c6_windows_prepara_fallback_candidato_autenticado_antes_del_rename() {
    candidate_fallback_contract(swap_protocol())
        .unwrap_or_else(|error| panic!("rojo causal CI27: {error}"));
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

    post_rename_recovery_contract(swap_protocol())
        .unwrap_or_else(|error| panic!("rojo causal CI27: {error}"));

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
            post_rename_recovery_contract(&mutant),
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

    post_rename_recovery_contract(swap_protocol())
        .unwrap_or_else(|error| panic!("rojo causal CI27: {error}"));
}
