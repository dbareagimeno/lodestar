//! E35-H01 — fase roja del scalar YAML semántico de `performance.maxMemory`.
//!
//! Estos tests observan el valor deserializado, no el texto incidental del documento. El
//! whitespace separado del scalar por YAML es válido; el whitespace que forma parte del scalar
//! sigue siendo inválido y debe conservar el camino de error accionable.

use std::path::Path;

use lodestar_workspace::WorkspaceConfig;

const BYTES_256_MIB: u64 = 256 * 1024 * 1024;

fn escribe_yaml(root: &Path, yaml: &str) {
    let dir = root.join(".lodestar");
    std::fs::create_dir_all(&dir).expect("crear .lodestar");
    std::fs::write(dir.join("config.yaml"), yaml).expect("escribir config.yaml");
}

fn bytes_de_config(yaml: &str) -> u64 {
    let dir = tempfile::tempdir().expect("tempdir");
    escribe_yaml(dir.path(), yaml);
    WorkspaceConfig::load(dir.path())
        .expect("la configuración YAML válida debe cargar")
        .performance
        .max_memory_bytes()
}

fn error_de_config(yaml: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    escribe_yaml(dir.path(), yaml);
    WorkspaceConfig::load(dir.path()).expect_err("la configuración inválida debe rechazarse")
}

/// C2 — cinco formas YAML distintas deben producir exactamente el mismo scalar semántico.
#[test]
fn scalar_yaml_acepta_whitespace_sintactico_y_observa_bytes_exactos() {
    let casos = [
        (
            "block plain con espacios antes de newline",
            "performance:\n  maxMemory: 256MiB   \n",
        ),
        (
            "plain con espacios antes de comentario",
            "performance:\n  maxMemory: 256MiB   # comentario\n",
        ),
        (
            "flow plain con espacio antes de cierre",
            "performance: {maxMemory: 256MiB }\n",
        ),
        (
            "flow plain con whitespace sintáctico antes de coma final",
            "performance: {maxMemory: 256MiB ,}\n",
        ),
        (
            "clave quoted con scalar plain y espacio antes de cierre",
            "performance: {\"maxMemory\": 256MiB }\n",
        ),
    ];

    for (forma, yaml) in casos {
        assert_eq!(
            bytes_de_config(yaml),
            BYTES_256_MIB,
            "{forma}: el scalar semántico debe observar exactamente 256MiB"
        );
    }
}

/// C3 — el mínimo es inclusivo y ambas clases de overflow devuelven errores accionables.
#[test]
fn c3_minimo_inclusivo_y_overflows_de_magnitud_o_conversion_son_checked() {
    let err = error_de_config("performance:\n  maxMemory: 63MiB\n");
    assert!(
        err.contains("performance.maxMemory")
            && err.contains("63MiB")
            && err.contains("mínimo es 64MiB")
            && err.contains("67108864"),
        "63MiB debe rechazar por el mínimo con perilla, valor y regla: {err}"
    );

    assert_eq!(
        bytes_de_config("performance:\n  maxMemory: 64MiB\n"),
        64 * 1024_u64 * 1024,
        "64MiB es válido y debe observarse en bytes exactos"
    );

    for (valor, regla) in [
        (
            "18446744073709551616MiB",
            "desbordamiento de u64 al convertir la magnitud",
        ),
        (
            "18446744073709551615GiB",
            "desbordamiento de u64 al convertir GiB",
        ),
    ] {
        let err = error_de_config(&format!("performance:\n  maxMemory: {valor}\n"));
        assert!(
            err.contains("performance.maxMemory") && err.contains(valor) && err.contains(regla),
            "{valor}: overflow debe ser un error accionable de la conversión checked: {err}"
        );
        assert!(
            !err.contains("256MiB debe estar validado"),
            "{valor}: overflow no puede caer a default ni acceder a una config inválida: {err}"
        );
    }
}

/// C2 — espacios dentro del contenido del scalar nunca pueden caer al default ni normalizarse.
#[test]
fn scalar_yaml_rechaza_espacios_en_contenido_con_perilla_valor_y_regla() {
    for (forma, yaml, recibido) in [
        (
            "quoted con espacio final",
            "performance:\n  maxMemory: \"256MiB \"\n",
            "256MiB ",
        ),
        (
            "quoted con espacio inicial",
            "performance:\n  maxMemory: \" 256MiB\"\n",
            " 256MiB",
        ),
        (
            "plain con espacio interior",
            "performance:\n  maxMemory: 256 MiB\n",
            "256 MiB",
        ),
    ] {
        let err = error_de_config(yaml);
        assert!(
            err.contains("performance.maxMemory"),
            "{forma}: el error debe nombrar la perilla: {err}"
        );
        assert!(
            err.contains(recibido),
            "{forma}: el error debe conservar el valor recibido {recibido:?}: {err}"
        );
        assert!(
            err.contains("incumple la gramática"),
            "{forma}: el rechazo debe atribuirse a la gramática exacta: {err}"
        );
    }
}

/// C2/C3 — los negativos ratificados fallan en la perilla correcta y no se convierten en default.
#[test]
fn scalar_yaml_rechaza_negativos_ratificados_con_regla_accionable() {
    for (valor, regla) in [
        ("0MiB", "incumple la gramática"),
        ("064MiB", "incumple la gramática"),
        ("1.5GiB", "incumple la gramática"),
        ("256mib", "incumple la gramática"),
        ("256MB", "incumple la gramática"),
    ] {
        let yaml = format!("performance:\n  maxMemory: {valor}\n");
        let err = error_de_config(&yaml);
        assert!(
            err.contains("performance.maxMemory"),
            "{valor}: el error debe nombrar la perilla: {err}"
        );
        assert!(
            err.contains(valor),
            "{valor}: el error debe conservar el valor recibido: {err}"
        );
        assert!(
            err.contains(regla),
            "{valor}: el error debe nombrar la regla incumplida ({regla}): {err}"
        );
    }
}

/// C8 — serde mantiene la superficie estricta: no se inventan knobs ni presets de performance.
#[test]
fn serde_rechaza_knobs_y_presets_de_performance_desconocidos() {
    for knob in [
        "sqliteQuota",
        "wTinyLfuQuota",
        "cachePreset",
        "sqlitePreset",
        "wTinyLfuPreset",
    ] {
        let yaml = format!("performance: {{maxMemory: 256MiB, {knob}: 1MiB}}\n");
        let err = error_de_config(&yaml);
        assert!(
            err.contains(knob),
            "serde debe nombrar la clave de performance desconocida {knob}: {err}"
        );
        assert!(
            !err.contains("256MiB debe estar validado"),
            "{knob}: el rechazo debe ocurrir al deserializar, no por una ruta posterior: {err}"
        );
    }
}
