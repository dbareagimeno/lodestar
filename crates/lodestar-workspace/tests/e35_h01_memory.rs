//! E35-H01 — fase roja independiente para configuración y contabilidad de memoria.
//!
//! Mapeo: C1 `default_y_valores_validos_observan_bytes`; C2
//! `gramatica_estricta_rechaza_valor_y_explica_regla`; C3
//! `minimo_y_overflow_son_accionables_y_checked`; C5
//! `default_es_independiente_del_host`; C6
//! `una_apertura_entrega_un_unico_budget_y_dos_aperturas_no_comparten_owner`; C7
//! `particion_exacta_preserva_residuo_en_work`; C8
//! `solo_max_memory_es_publico_y_no_habilita_consumidores`.
//!
//! C4 (proyección MCP) vive en `lodestar-mcp/tests/e35_h01_memory.rs` y C9 (documentación y
//! trazabilidad) vive en `lodestar-fixtures/tests/e35_h01_docs.rs`, para mantener cada frontera
//! en su crate de integración.
//!
//! La spec ratificada exige bytes observables, no solo que el texto parezca válido. Los accesores
//! usados por C1/C5/C6/C7 son la API pública objetivo que debe entregar el implementador:
//! `WorkspaceConfig::performance.max_memory_bytes()`, `Workspace::memory_budget()` y los tres
//! subpresupuestos de `MemoryBudget`. Antes de E35-H01 esos símbolos no existen; ese es el rojo
//! específico de la ausencia del comportamiento, no un fallo de fixture.

use std::path::Path;

use lodestar_workspace::{MemoryBudget, Workspace, WorkspaceConfig};

fn escribe_config(root: &Path, yaml: &str) {
    let dir = root.join(".lodestar");
    std::fs::create_dir_all(&dir).expect("crear .lodestar");
    std::fs::write(dir.join("config.yaml"), yaml).expect("escribir config.yaml");
}

fn config_con_max_memory(root: &Path, valor: &str) {
    escribe_config(root, &format!("performance:\n  maxMemory: {valor}\n"));
}

/// C1 — ausencia, 64MiB, 256MiB y 2GiB deben devolver bytes efectivos distintos y exactos.
#[test]
fn default_y_valores_validos_observan_bytes() {
    let casos = [
        (None, 256 * 1024_u64 * 1024),
        (Some("64MiB"), 64 * 1024_u64 * 1024),
        (Some("256MiB"), 256 * 1024_u64 * 1024),
        (Some("2GiB"), 2 * 1024_u64 * 1024 * 1024),
    ];

    for (valor, esperado) in casos {
        let dir = tempfile::tempdir().expect("tempdir");
        if let Some(valor) = valor {
            config_con_max_memory(dir.path(), valor);
        }
        let cfg = WorkspaceConfig::load(dir.path()).expect("config válida debe cargar");
        assert_eq!(
            cfg.performance.max_memory_bytes(),
            esperado,
            "el valor efectivo de {valor:?} debe observarse en bytes, no solo validarse como texto"
        );
    }
}

/// C5 — el default no consulta RSS/cgroup/host: la misma config produce el mismo número.
#[test]
fn default_es_independiente_del_host() {
    let a = tempfile::tempdir().expect("tempdir");
    let b = tempfile::tempdir().expect("tempdir");
    let wa = Workspace::open(a.path()).expect("workspace A sin config");
    let wb = Workspace::open(b.path()).expect("workspace B sin config");
    assert_eq!(
        wa.config().performance.max_memory_bytes(),
        256 * 1024_u64 * 1024,
        "la config antigua sin performance usa exactamente 256MiB"
    );
    assert_eq!(
        wa.config().performance.max_memory_bytes(),
        wb.config().performance.max_memory_bytes(),
        "el valor no puede depender de RSS/cgroup o de una sonda del host"
    );
}

/// C6 — cada apertura crea exactamente un owner y ningún presupuesto se comparte globalmente.
#[test]
fn una_apertura_entrega_un_unico_budget_y_dos_aperturas_no_comparten_owner() {
    let a = tempfile::tempdir().expect("tempdir");
    let b = tempfile::tempdir().expect("tempdir");
    config_con_max_memory(a.path(), "64MiB");
    config_con_max_memory(b.path(), "64MiB");
    let wa = Workspace::open(a.path()).expect("workspace A");
    let wb = Workspace::open(b.path()).expect("workspace B");
    let wa_again = Workspace::open(a.path()).expect("una segunda apertura del workspace A");

    let first = wa.memory_budget();
    assert!(
        std::ptr::eq(first, wa.memory_budget()),
        "una apertura debe conservar un único MemoryBudget, no construir uno por accessor"
    );
    assert!(
        !std::ptr::eq(first, wb.memory_budget()),
        "dos workspaces no pueden compartir un presupuesto global"
    );
    assert!(
        !std::ptr::eq(first, wa_again.memory_budget()),
        "cada llamada a Workspace::open debe obtener su propio owner"
    );
}

/// C8 — abrir para leer no conecta SQLite ni activa un consumidor de cache/runtime.
#[test]
fn apertura_sin_consumidores_de_memoria_mantiene_cache_none_y_no_crea_runtime() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = Workspace::open(dir.path()).expect("workspace sin config");

    assert!(
        workspace.cache().is_none(),
        "Workspace::open no debe activar SQLite/W-TinyLFU ni otro consumidor"
    );
    assert!(
        !dir.path().join(".lodestar/index.db").exists(),
        "abrir no debe crear el artefacto de cache"
    );
    assert!(
        !dir.path().join(".lodestar/runtime").exists(),
        "abrir no debe activar ni materializar runtime"
    );
}

/// C7 — 101/199 distinguen residuo protegido de `floor(50*N/100)` y la suma agota N.
#[test]
fn particion_exacta_preserva_residuo_en_work() {
    for (n, sqlite, wtlfu, work) in [
        (101, 30, 20, 51),
        (199, 59, 39, 101),
        (64 * 1024 * 1024, 20_132_659, 13_421_772, 33_554_433),
    ] {
        let budget = MemoryBudget::from_bytes(n).expect("N positivo debe poder particionarse");
        assert_eq!(
            budget.total_bytes(),
            n,
            "el presupuesto total debe conservar exactamente N"
        );
        assert_eq!(budget.sqlite_bytes(), sqlite);
        assert_eq!(budget.w_tiny_lfu_bytes(), wtlfu);
        assert_eq!(budget.work_bytes(), work);
        assert_eq!(
            budget.sqlite_bytes() + budget.w_tiny_lfu_bytes() + budget.work_bytes(),
            n,
            "las tres reservas deben agotar exactamente N"
        );
        assert!(budget.work_bytes() >= n / 2, "Work es la reserva protegida");
    }
}
