//! E35-H03 CI19/review3 — orden durable de validacion y publicacion.
//!
//! La persistencia fisica de `fsync` no ofrece un observable determinista y portatil desde una
//! integracion. Este test lee exactamente los fuentes que compila `lodestar-store` y fija el
//! protocolo ratificado completo: una conexion de validacion read-only ejecuta ambas validaciones
//! SQLite, se cierra antes de sincronizar el candidato ya preparado, y esa sincronizacion precede
//! al rename y al fsync inmediato del directorio.

const STORE_SOURCE: &str = include_str!("../src/lib.rs");
const SCHEMA_SOURCE: &str = include_str!("../src/schema.rs");

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
        "guarda anti-vacuidad: `{needle}` debe identificar exactamente un paso real; posiciones={positions:?}"
    );
    positions[0]
}

fn assert_precedes(protocol: &str, earlier: &str, later: &str, criterion: &str) {
    let earlier_at = unique_position(protocol, earlier);
    let later_at = unique_position(protocol, later);
    assert!(
        earlier_at < later_at,
        "{criterion}; orden observado `{later}`@{later_at} antes de `{earlier}`@{earlier_at}"
    );
}

/// C5/C6 + ARCH §20.12.2 + REFACTOR_PHASE_2 — una `.next` solo puede llegar al rename tras
/// completar `integrity_check` y `foreign_key_check`, cerrar la conexion que los ejecuta y
/// sincronizar el fichero candidato. Despues del rename debe sincronizarse el directorio.
#[test]
fn c5_c6_integridad_y_fk_preceden_sync_generacion_rename_y_fsync_directorio() {
    let validation = section(
        SCHEMA_SOURCE,
        "pub(crate) fn validate_database(",
        "\npub(crate) fn read_user_version(",
    );
    let rebuild = section(
        STORE_SOURCE,
        "    fn rebuild_iter<I>(",
        "\n    fn swap_active(",
    );
    let swap = section(
        STORE_SOURCE,
        "    fn swap_active(",
        "\n    /// Reabre la conexión compartida",
    );

    // Guardas anti-vacuidad: el helper recibe la conexion viva preparada por el rebuild y contiene
    // las dos comprobaciones normativas, en ese orden.
    assert!(
        validation.contains("&Connection"),
        "C5/C6: validate_database debe operar sobre la misma &Connection de validacion"
    );
    assert_precedes(
        validation,
        "\"PRAGMA integrity_check\"",
        "\"PRAGMA foreign_key_check\"",
        "C5/C6: integrity_check debe preceder foreign_key_check",
    );
    assert_precedes(
        validation,
        "\"PRAGMA foreign_key_check\"",
        "    Ok(())",
        "C5/C6: validate_database no puede devolver exito antes de foreign_key_check",
    );

    // Cadena causal del candidato: la conexion read-only se abre y produce el candidato preparado;
    // esa misma conexion ejecuta ambos PRAGMA, se cierra y solo entonces se sincroniza el handle.
    assert_precedes(
        rebuild,
        "let validation_conn = schema::open_validation_connection(&next)?;",
        "prepare_candidate(&validation_conn)",
        "C5/C6: debe abrirse la conexion de validacion antes de preparar su candidato",
    );
    assert_precedes(
        rebuild,
        "prepare_candidate(&validation_conn)",
        "validate_database(&validation_conn)",
        "C5/C6: el candidato debe derivarse de la misma conexion antes de integrity/FK",
    );
    assert_precedes(
        rebuild,
        "validate_database(&validation_conn)",
        "drop(validation_conn);",
        "C5/C6: integrity_check + foreign_key_check deben completarse antes de cerrar la conexion",
    );
    assert_precedes(
        rebuild,
        "drop(validation_conn);",
        "candidate.sync()",
        "C5/C6: la conexion de validacion debe cerrarse antes de FlushFileBuffers del candidato",
    );
    assert_precedes(
        rebuild,
        "candidate.sync()",
        "pause_before_swap",
        "C5/C6: el candidato debe sincronizarse antes de la pausa pre-publicacion",
    );
    assert_precedes(
        rebuild,
        "pause_before_swap",
        "self.swap_active(&next, candidate, candidate_standby)?;",
        "C5/C6: la pausa debe preceder la publicacion del mismo candidato preparado",
    );

    // Cadena causal de publicacion: el candidato se pasa de forma explicita al unico rename y el
    // fsync del directorio aparece como el siguiente paso con efecto, antes de reabrir o confirmar.
    assert_precedes(
        swap,
        "if let Err(error) = replace_durable(candidate, &active)",
        "sync_directory(active.parent().expect(\"cache directory\"))?",
        "C5/C6: el rename debe preceder al fsync del directorio",
    );
    let rename_at = unique_position(
        swap,
        "if let Err(error) = replace_durable(candidate, &active)",
    );
    let after_rename = &swap[rename_at..];
    let directory_sync_at = unique_position(
        after_rename,
        "sync_directory(active.parent().expect(\"cache directory\"))?",
    );
    for forbidden in [
        "open_sqlite(",
        "activate_published_connection(",
        ".commit()",
    ] {
        if let Some(at) = after_rename.find(forbidden) {
            assert!(
                directory_sync_at < at,
                "C5/C6: el fsync del directorio debe ser inmediato tras rename, antes de `{forbidden}`"
            );
        }
    }
}
