//! E35-H03 C2/C4 — la ruta canónica no debe leer cuerpos durante el inventario.
//!
//! `discover_inventory` conserva paths y metadata sin abrir candidatos; el store abre cada cuerpo
//! una sola vez para validarlo y proyectarlo. El informe cuenta esa operación
//! (`documents_read = N`), y este test la contrasta con una señal de acceso al cuerpo entre ambas
//! fases y con el seam de auditoría de la lectura de proyección.

use std::fs::{self, File, FileTimes};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use lodestar_core::types::RelPath;
use lodestar_discovery::{discover_inventory, DiscoveryPolicy};
use lodestar_store::Store;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn write(root: &Path, path: &str, contents: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(target, contents).unwrap();
}

fn large_utf8_markdown() -> String {
    let mut body = String::from("---\ntitle: repair9\n---\n\n# Repair 9\n\n");
    for _ in 0..45_000 {
        body.push_str("cuerpo UTF-8 grande: áéíóú · 東京 · 🚀\n");
    }
    body
}

fn access_time_ns(path: &Path) -> u128 {
    fs::metadata(path)
        .expect("guard: el cuerpo admitido existe")
        .accessed()
        .expect("guard: el filesystem expone atime")
        .duration_since(UNIX_EPOCH)
        .expect("guard: atime no es anterior a epoch")
        .as_nanos()
}

fn reset_access_time(path: &Path) {
    let file = File::open(path).expect("abrir cuerpo para preparar el observador atime");
    file.set_times(FileTimes::new().set_accessed(SystemTime::UNIX_EPOCH))
        .expect("el filesystem debe permitir fijar atime para la prueba");
}

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("RelPath válido")
}

/// C2/C4 — **Dado** un documento UTF-8 grande admitido por discovery, **cuando** se ejecuta la
/// ruta canónica `discover_inventory` + `rebuild_from_discovered_inventory`, **entonces** el
/// cuerpo debe abrirse una sola vez, en la segunda pasada, y el inventario no debe producir ningún
/// acceso observable al payload.
#[test]
fn c2_c4_canonical_inventory_and_rebuild_read_each_admitted_body_once() {
    let _env = env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let root = tempfile::tempdir().unwrap();
    let document = root.path().join("docs/large-utf8.md");
    let payload = large_utf8_markdown();
    assert!(
        payload.len() > 1_000_000,
        "guard anti-vacuidad: el cuerpo debe ser grande y UTF-8, no un fixture vacío"
    );
    write(root.path(), "docs/large-utf8.md", &payload);

    // El sidecar vive en el plano de control, ya existente antes del snapshot, para que su
    // escritura de auditoría no introduzca una entrada nueva en la frontera canónica.
    let store = Store::open(root.path()).expect("abrir cache derivada vacía");
    let audit = root.path().join(".lodestar/h03-repair9-read-audit.ndjson");
    write(root.path(), ".lodestar/h03-repair9-read-audit.ndjson", b"");

    reset_access_time(&document);
    let before_discovery = access_time_ns(&document);
    let discovered = discover_inventory(root.path(), &DiscoveryPolicy::default())
        .expect("discovery canónico debe completar");
    assert_eq!(
        discovered.documents,
        vec![rp("docs/large-utf8.md")],
        "guard anti-vacuidad: el cuerpo grande debe ser un documento admitido"
    );
    let after_discovery = access_time_ns(&document);
    let discovery_body_read = after_discovery > before_discovery;
    assert!(
        !discovery_body_read,
        "C2/C4: discovery no debe abrir/leer el cuerpo admitido (atime: {before_discovery}->{after_discovery})"
    );

    // Conserva una única línea temporal de atime: el siguiente cambio solo puede provenir de la
    // lectura de proyección posterior al inventario descubierto.
    let before_rebuild = after_discovery;
    std::env::set_var("LODESTAR_H03_TEST_READ_AUDIT", &audit);
    let report = store
        .rebuild_from_discovered_inventory(&discovered)
        .expect("rebuild canónico con el snapshot descubierto");
    std::env::remove_var("LODESTAR_H03_TEST_READ_AUDIT");
    let after_rebuild = access_time_ns(&document);
    let rebuild_body_read = after_rebuild > before_rebuild;
    assert!(
        rebuild_body_read,
        "guard del observador: la segunda pasada no produjo una señal de lectura del cuerpo"
    );

    let events: Vec<serde_json::Value> = fs::read_to_string(&audit)
        .expect("seam H03 de auditoría de lectura real")
        .lines()
        .map(|line| serde_json::from_str(line).expect("evento NDJSON válido"))
        .collect();
    assert_eq!(
        events.len(),
        1,
        "la proyección debe registrar una lectura del cuerpo"
    );
    assert_eq!(events[0]["event"].as_str(), Some("payload_read"));
    assert_eq!(events[0]["path"].as_str(), Some("docs/large-utf8.md"));
    assert_eq!(events[0]["bytes"].as_u64(), Some(payload.len() as u64));
    assert_eq!(
        report["documents_read"].as_u64(),
        Some(1),
        "guard anti-vacuidad: el informe debe admitir exactamente un cuerpo"
    );

    let observed_body_reads = u64::from(discovery_body_read) + u64::from(rebuild_body_read);
    assert_eq!(
        observed_body_reads,
        1,
        "C2/C4: el cuerpo admitido fue abierto/leído {observed_body_reads} veces (discovery atime: {before_discovery}->{after_discovery}; rebuild atime: {before_rebuild}->{after_rebuild}; documents_read={}; audit_events={events:?})",
        report["documents_read"]
    );
}
