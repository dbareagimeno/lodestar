//! E35-H03 C1/C6 — el Store debe preservar la autoridad de policy y el plano de control.
//!
//! Reproducciones CI58 del juicio final: un inventario válido para otra `DiscoveryPolicy` no es
//! canónico para el Store abierto, y `.lodestar` no puede redirigir la cache fuera del workspace.

use std::path::Path;

use lodestar_core::types::RelPath;
use lodestar_discovery::{discover_inventory, DiscoveryPolicy};
use lodestar_store::Store;

fn write(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).expect("CI58 crear directorio de fixture");
    }
    std::fs::write(target, bytes).expect("CI58 escribir fixture");
}

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("CI58 RelPath válido")
}

#[test]
fn c1_discovered_inventory_de_otra_policy_se_rechaza_y_preserva_activo_canonico() {
    let root = tempfile::tempdir().expect("CI58 workspace temporal");
    let active = rp("docs/active.md");
    let secret = rp("docs/secret.md");
    write(
        root.path(),
        active.as_str(),
        "# Active CI58\n\nci58-active-canonical-needle\n",
    );
    write(
        root.path(),
        secret.as_str(),
        "# Secret CI58\n\nci58-secret-forbidden-needle\n",
    );

    let mut restrictive = DiscoveryPolicy::default();
    restrictive.exclude.push("docs/secret.md".into());
    // Abrir primero materializa el plano de control; ambos inventarios se capturan después para
    // que el rojo mida la autoridad de policy, no un fingerprint anterior a crear `.lodestar`.
    let store = Store::open_with_policy(root.path(), restrictive.clone())
        .expect("CI58 abrir Store con policy restrictiva");
    let canonical = discover_inventory(root.path(), &restrictive)
        .expect("CI58 inventario canónico bajo la policy del Store");
    assert_eq!(
        canonical.documents,
        vec![active.clone()],
        "CI58 anti-vacuidad: la policy restrictiva debe excluir secret.md"
    );

    store
        .rebuild_from_discovered_inventory(&canonical)
        .expect("CI58 positivo: el inventario de la misma policy debe publicarse");
    assert_eq!(
        store.documents().expect("CI58 snapshot activo inicial"),
        vec![active.clone()],
        "CI58 anti-vacuidad: existe una generación canónica anterior distinta"
    );
    assert_eq!(
        store
            .fts_candidates("ci58-active-canonical-needle")
            .expect("CI58 FTS activo inicial"),
        vec![active.clone()],
        "CI58 anti-vacuidad: el activo anterior es consultable por FTS"
    );

    let permissive = DiscoveryPolicy::default();
    let foreign = discover_inventory(root.path(), &permissive)
        .expect("CI58 inventario válido bajo una policy permisiva distinta");
    assert_eq!(
        foreign.documents,
        vec![active.clone(), secret.clone()],
        "CI58 anti-vacuidad: el inventario ajeno debe contener realmente secret.md"
    );

    let rebuild = store.rebuild_from_discovered_inventory(&foreign);
    let active_after = store
        .documents()
        .expect("CI58 consultar activo tras inventario ajeno");
    let active_fts_after = store
        .fts_candidates("ci58-active-canonical-needle")
        .expect("CI58 consultar FTS canónico tras inventario ajeno");
    let forbidden_fts_after = store
        .fts_candidates("ci58-secret-forbidden-needle")
        .expect("CI58 consultar FTS prohibido tras inventario ajeno");

    assert!(
        rebuild.is_err()
            && active_after == vec![active.clone()]
            && active_fts_after == vec![active]
            && forbidden_fts_after.is_empty(),
        "C1 CI58: el Store debe rechazar un DiscoveredInventory generado con otra policy y preservar documentos/FTS canónicos; rebuild={rebuild:?}, documents={active_after:?}, active_fts={active_fts_after:?}, forbidden_fts={forbidden_fts_after:?}"
    );
}

#[test]
fn c1_inventario_autentico_mutado_despues_de_discovery_se_rechaza_y_preserva_activo_fts() {
    let root = tempfile::tempdir().expect("CI59 workspace temporal");
    let active = rp("docs/active.md");
    let omitted = rp("docs/omitted.md");
    write(
        root.path(),
        active.as_str(),
        "# Active CI59\n\nci59-active-canonical-needle\n",
    );
    write(
        root.path(),
        omitted.as_str(),
        "# Omitted CI59\n\nci59-omitted-forbidden-needle\n",
    );

    let policy = DiscoveryPolicy::default();
    let store = Store::open_with_policy(root.path(), policy.clone())
        .expect("CI59 abrir Store con policy canónica");
    let canonical = discover_inventory(root.path(), &policy)
        .expect("CI59 inventario genuino para el mismo root y policy");
    assert_eq!(
        canonical.documents,
        vec![active.clone(), omitted.clone()],
        "CI59 anti-vacuidad: el walker genuino debe admitir los dos documentos"
    );

    store
        .rebuild_from_discovered_inventory(&canonical)
        .expect("CI59 positivo: el inventario genuino sin mutar debe publicarse");
    assert_eq!(
        store
            .fts_candidates("ci59-omitted-forbidden-needle")
            .expect("CI59 FTS inicial"),
        vec![omitted.clone()],
        "CI59 anti-vacuidad: el documento que se intentará omitir está en el activo anterior"
    );

    let mut forged = canonical.clone();
    forged.documents.retain(|path| path != &omitted);
    assert_eq!(
        forged.documents,
        vec![active.clone()],
        "CI59 anti-vacuidad: la copia pública fue mutada realmente después de discovery"
    );
    assert!(
        forged.entry_fingerprints.contains_key(&omitted),
        "CI59 anti-vacuidad: no se recapturó un inventario; solo se adulteró su lista pública"
    );

    let rebuild = store.rebuild_from_discovered_inventory(&forged);
    let active_after = store
        .documents()
        .expect("CI59 consultar activo tras inventario adulterado");
    let active_fts_after = store
        .fts_candidates("ci59-active-canonical-needle")
        .expect("CI59 consultar FTS activo tras inventario adulterado");
    let omitted_fts_after = store
        .fts_candidates("ci59-omitted-forbidden-needle")
        .expect("CI59 consultar FTS omitido tras inventario adulterado");

    assert!(
        rebuild.is_err()
            && active_after == vec![active.clone(), omitted.clone()]
            && active_fts_after == vec![active]
            && omitted_fts_after == vec![omitted],
        "C1 CI59: el Store debe rechazar una copia de su inventario canónico mutada mediante campos públicos y preservar documentos/FTS activos; rebuild={rebuild:?}, documents={active_after:?}, active_fts={active_fts_after:?}, omitted_fts={omitted_fts_after:?}"
    );
}

#[cfg(unix)]
#[test]
fn c6_plano_de_control_symlink_exterior_se_rechaza_sin_crear_cache_fuera() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("CI58 workspace temporal");
    let exterior = tempfile::tempdir().expect("CI58 destino exterior temporal");
    let markdown = b"# Canonical CI58\n\nci58-markdown-must-survive\n";
    write(root.path(), "docs/canonical.md", markdown);
    symlink(exterior.path(), root.path().join(".lodestar"))
        .expect("CI58 crear symlink de plano de control al exterior");
    assert!(
        std::fs::symlink_metadata(root.path().join(".lodestar"))
            .expect("CI58 metadata del plano de control")
            .file_type()
            .is_symlink(),
        "CI58 anti-vacuidad: .lodestar debe ser un symlink real"
    );
    assert_eq!(
        std::fs::read_dir(exterior.path())
            .expect("CI58 listar exterior antes")
            .count(),
        0,
        "CI58 anti-vacuidad: el destino exterior comienza vacío"
    );

    let opened = Store::open(root.path());
    let open_error = opened.as_ref().err().map(ToString::to_string);
    drop(opened);
    let exterior_entries: Vec<_> = std::fs::read_dir(exterior.path())
        .expect("CI58 listar exterior después")
        .map(|entry| entry.expect("CI58 entrada exterior").file_name())
        .collect();
    let markdown_after = std::fs::read(root.path().join("docs/canonical.md"))
        .expect("CI58 releer Markdown canónico");

    assert!(
        open_error.is_some()
            && exterior_entries.is_empty()
            && !exterior.path().join("h03-writer.lock").exists()
            && !exterior.path().join("index.db").exists()
            && !exterior.path().join("index.db-wal").exists()
            && !exterior.path().join("index.db-shm").exists()
            && markdown_after == markdown,
        "C6 CI58: Store::open debe rechazar .lodestar symlink antes de crear lock/db/sidecars fuera y preservar Markdown; error={open_error:?}, exterior={exterior_entries:?}, markdown_preservado={} ",
        markdown_after == markdown
    );
}

#[cfg(unix)]
#[test]
fn c6_plano_de_control_symlink_se_rechaza_antes_de_leer_config_exterior() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("CI63 workspace temporal");
    let exterior = tempfile::tempdir().expect("CI63 destino exterior temporal");
    std::fs::write(
        exterior.path().join("config.yaml"),
        "discovery:\n  include: contenido-exterior-invalido\n",
    )
    .expect("CI63 escribir config exterior inválida");
    symlink(exterior.path(), root.path().join(".lodestar"))
        .expect("CI63 crear symlink del plano de control");

    let error = match Store::open(root.path()) {
        Ok(_) => panic!("CI63 debe rechazar el plano de control antes de cargar config"),
        Err(error) => error.to_string(),
    };

    assert!(
        error.contains("control plane must be a real directory")
            && !error.contains("contenido-exterior-invalido")
            && !error.contains("config.yaml inválido"),
        "C6 CI63: el rechazo del symlink debe preceder cualquier lectura de config exterior; error={error:?}"
    );
}
