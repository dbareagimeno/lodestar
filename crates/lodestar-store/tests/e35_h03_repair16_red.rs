//! Regresión conductual de E35-H03 repair16 específica de Windows.
//!
//! La métrica RSS sintética que antes compartía este fichero ya queda cubierta por la observación
//! independiente de `e35_h03_repair17_red`; aquí no se duplica ni se inspecciona código fuente.

#![cfg(windows)]

use std::fs::{FileTimes, OpenOptions};

use lodestar_discovery::filesystem_fingerprint;

/// §20.12.2 — una mutación de mismo tamaño con LastWriteTime restaurado debe ser detectable
/// también en Windows. Se restaura explícitamente `LastWriteTime`, de modo que el cambio solo puede
/// detectarse mediante la identidad/ChangeTime reales que expone la huella pública.
#[test]
fn c6_windows_fingerprint_detecta_mutacion_mismo_tamano_con_mtime_restaurado() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fingerprint.md");
    std::fs::write(&path, b"first-payload").unwrap();
    let original_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    let before = filesystem_fingerprint(&path, false).unwrap();

    std::fs::write(&path, b"other-payload").unwrap();
    assert_eq!(
        b"first-payload".len(),
        b"other-payload".len(),
        "anti-vacuidad: la mutación debe conservar el tamaño"
    );
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(original_mtime))
        .unwrap();

    let after = filesystem_fingerprint(&path, false).unwrap();
    assert_eq!(
        before.kind, after.kind,
        "sigue siendo el mismo tipo de entrada"
    );
    assert_eq!(before.size, after.size, "la mutación conserva el tamaño");
    assert_eq!(
        before.mtime_ns, after.mtime_ns,
        "la guarda restaura realmente LastWriteTime"
    );
    assert_eq!(
        before.identity, after.identity,
        "la escritura no reemplaza la identidad del fichero"
    );
    assert_ne!(before.identity, 0, "la identidad Windows debe ser real");
    assert_ne!(before.ctime_ns, 0, "ChangeTime inicial debe ser real");
    assert_ne!(after.ctime_ns, 0, "ChangeTime final debe ser real");
    assert_ne!(
        before, after,
        "§20.12.2: la huella pública debe detectar bytes distintos aunque tamaño y LastWriteTime coincidan"
    );
}
