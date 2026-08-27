//! E35-H03 CI19/review3 — orden durable de validacion y publicacion.
//!
//! La persistencia fisica de `fsync` no ofrece un observable determinista y portatil desde una
//! integracion. Este test lee exactamente los fuentes que compila `lodestar-store` y fija el
//! protocolo ratificado completo: ambas validaciones SQLite terminan antes de sincronizar la
//! generacion, y esa sincronizacion precede al rename y al fsync del directorio.

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
        "pub(crate) fn validate_database(path: &std::path::Path) -> Result<(), StoreError> {",
        "\npub(crate) fn read_user_version(",
    );
    let rebuild = section(
        STORE_SOURCE,
        "    fn rebuild_iter<I>(",
        "\n    fn swap_active(&self, next: &Path)",
    );
    let swap = section(
        STORE_SOURCE,
        "    fn swap_active(&self, next: &Path) -> Result<(), StoreError> {",
        "\n    /// Reabre la conexión compartida",
    );

    // Guardas anti-vacuidad: el helper invocado por el rebuild contiene las dos comprobaciones
    // normativas, en ese orden, y solo devuelve exito despues de ambas. Al retornar, su `conn`
    // local ya queda fuera de alcance antes de que el caller pueda sincronizar el fichero.
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

    // Cadena causal del candidato: cerrar la conexion de carga no basta. La llamada que ejecuta
    // ambos PRAGMA debe terminar antes de la unica sincronizacion de `.next`, y esta antes de
    // entrar en la ruta que publica el nombre activo.
    assert_precedes(
        rebuild,
        "drop(next_conn);",
        "let check = schema::validate_database(&next);",
        "guarda anti-vacuidad: la conexion de carga `.next` debe estar cerrada antes de validar",
    );
    assert_precedes(
        rebuild,
        "let check = schema::validate_database(&next);",
        "sync_generation(&next)?;",
        "C5/C6: integrity_check + foreign_key_check deben completarse antes de sincronizar `.next`",
    );
    assert_precedes(
        rebuild,
        "sync_generation(&next)?;",
        "self.swap_active(&next)?;",
        "C5/C6: la generacion candidata debe sincronizarse antes de publicarla",
    );

    // Cadena causal de publicacion: `replace_durable` es el unico rename del protocolo comun y
    // el fsync del directorio debe ocurrir inmediatamente despues, antes de cualquier exito.
    assert_precedes(
        swap,
        "if let Err(error) = replace_durable(next, &active)",
        "sync_directory(active.parent().expect(\"cache directory\"))?",
        "C5/C6: el rename debe preceder al fsync del directorio",
    );
}
