//! E35-H01 — C4, proyección MCP del error de configuración.
//!
//! La configuración inválida debe conservar `WorkspaceError::Io`, mapear a
//! `INTERNAL_IO_ERROR` y llevar al wire el nombre de la perilla, valor y regla accionable. No se
//! acepta `INVALID_SCHEMA`, éxito silencioso ni una variante nueva del contrato MCP.

use lodestar_app::{App, AppError};
use lodestar_core::types::ErrorCode;
use std::process::{Command, Stdio};

fn escribe_config(root: &std::path::Path, value: &str) {
    let dir = root.join(".lodestar");
    std::fs::create_dir_all(&dir).expect("crear .lodestar");
    std::fs::write(
        dir.join("config.yaml"),
        format!("performance:\n  maxMemory: {value}\n"),
    )
    .expect("escribir config.yaml");
}

/// C4 — el fallo de maxMemory se proyecta por el camino existente y no introduce un código MCP.
#[test]
fn error_max_memory_se_proyecta_como_internal_io_error_con_mensaje_accionable() {
    let dir = tempfile::tempdir().expect("tempdir");
    escribe_config(dir.path(), "63MiB");
    let err: AppError = match App::open(dir.path()).map_err(AppError::from) {
        Ok(_) => panic!("63MiB debe impedir la apertura por debajo del mínimo C3"),
        Err(err) => err,
    };
    assert_eq!(
        err.code,
        ErrorCode::InternalIoError,
        "la fachada conserva INTERNAL_IO_ERROR para errores de config"
    );
    let wire = err.to_string();
    assert!(
        wire.starts_with("INTERNAL_IO_ERROR: "),
        "el wire debe empezar por el código exacto seguido de ': ': {wire}"
    );
    let message = &err.message;
    assert!(
        message.contains("performance.maxMemory"),
        "mensaje accionable: {message}"
    );
    assert!(
        message.contains("63MiB"),
        "mensaje debe conservar el valor: {message}"
    );
    assert!(
        message.contains("mínimo es 64MiB") && message.contains("67108864"),
        "mensaje debe explicar la regla C3 con el límite exacto: {message}"
    );
    assert_ne!(err.code, ErrorCode::InvalidSchema);
}

/// Guarda anti-vacuidad de C4: un workspace válido no se convierte en error MCP.
#[test]
fn max_memory_valido_no_se_proyecta_como_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    escribe_config(dir.path(), "64MiB");
    let app = App::open(dir.path()).expect("64MiB debe abrir");
    assert_eq!(app.workspace().root(), dir.path());
}

/// C4 — el binario real rechaza la configuración antes de abrir una sesión MCP.
#[test]
fn proceso_real_max_memory_invalido_sale_con_internal_io_error_sin_sesion() {
    let dir = tempfile::tempdir().expect("tempdir");
    escribe_config(dir.path(), "63MiB");

    let salida = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
        .arg(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("ejecutar el proceso real lodestar-mcp");
    let stdout = String::from_utf8_lossy(&salida.stdout);
    let stderr = String::from_utf8_lossy(&salida.stderr);

    assert_eq!(
        salida.status.code(),
        Some(3),
        "la configuración inválida impide iniciar el proceso (stderr: {stderr})"
    );
    assert!(
        stdout.is_empty(),
        "un fallo antes del lifecycle no puede emitir sesión ni envelope en stdout: {stdout:?}"
    );
    assert!(
        stderr.contains("INTERNAL_IO_ERROR:"),
        "stderr debe conservar el código estable y su separador: {stderr:?}"
    );
    assert!(
        stderr.contains("performance.maxMemory"),
        "el mensaje debe nombrar la perilla inválida: {stderr:?}"
    );
    assert!(
        stderr.contains("63MiB"),
        "el mensaje debe conservar el valor recibido: {stderr:?}"
    );
    assert!(
        stderr.contains("mínimo es 64MiB"),
        "el mensaje debe explicar la regla/mínimo accionable: {stderr:?}"
    );
    assert!(
        !stderr.contains("INVALID_SCHEMA"),
        "un fallo de apertura no puede proyectarse como INVALID_SCHEMA: {stderr:?}"
    );
}
