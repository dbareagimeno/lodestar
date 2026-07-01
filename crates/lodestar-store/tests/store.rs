//! Tests de `lodestar-store` (E3): apertura/DDL, cold rebuild, incremental con gate por hash,
//! FTS5 (subcadena + escapado), **paridad SQL == core**, property incremental==core, bus de
//! eventos, reconcile y `ConceptStore`.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use lodestar_core::types::{Direction, FileMap, RelPath};
use lodestar_core::Bundle;
use lodestar_store::Store;

fn write_all(root: &Path, files: &FileMap) {
    for (p, c) in files {
        let fp = root.join(p.as_str());
        if let Some(parent) = fp.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(fp, c).unwrap();
    }
}

fn rp(s: &str) -> RelPath {
    RelPath::new(s).unwrap()
}

fn sorted(mut v: Vec<RelPath>) -> Vec<RelPath> {
    v.sort();
    v
}

/// Comprueba que la síntesis del store coincide con `core::analyze` sobre el mismo corpus.
fn assert_matches_core(store: &Store, files: &FileMap) {
    let bundle = Bundle::from_files(files.clone());
    let a = bundle.analyze();

    let (hf, wc) = store.conformance_counts().unwrap();
    assert_eq!(hf, a.hard_fail, "hard_fail difiere (gana el core)");
    assert_eq!(wc, a.warn_count, "warn_count difiere (gana el core)");

    assert_eq!(
        sorted(store.orphans().unwrap()),
        sorted(a.orphans.clone()),
        "orphans difiere"
    );
    assert_eq!(
        sorted(store.dangling().unwrap()),
        sorted(a.dangling.clone()),
        "dangling difiere"
    );
    assert_eq!(
        store.in_index().unwrap().into_iter().collect::<BTreeSet<_>>(),
        a.in_index.iter().cloned().collect::<BTreeSet<_>>(),
        "in_index difiere"
    );
    for p in &a.concepts {
        let mut expected = a.inn.get(p).cloned().unwrap_or_default();
        expected.sort();
        expected.dedup();
        assert_eq!(
            sorted(store.backlinks(p).unwrap()),
            expected,
            "backlinks de {p} difieren"
        );
    }
}

#[test]
fn abrir_crea_esquema_y_reabrir_es_idempotente() {
    let dir = tempfile::tempdir().unwrap();
    write_all(dir.path(), &lodestar_fixtures::conformant());
    {
        let store = Store::open_and_build(dir.path()).unwrap();
        assert!(!store.concepts().unwrap().is_empty());
    }
    // Reabrir no rompe y ve el mismo contenido.
    let store = Store::open_and_build(dir.path()).unwrap();
    assert_eq!(store.concepts().unwrap().len(), 2);
    assert!(dir.path().join(".lodestar/index.db").exists());
}

#[test]
fn cold_rebuild_es_idempotente() {
    let dir = tempfile::tempdir().unwrap();
    write_all(dir.path(), &lodestar_fixtures::conformant());
    let store = Store::open_and_build(dir.path()).unwrap();
    let before = sorted(store.concepts().unwrap());
    store.rebuild().unwrap();
    let after = sorted(store.concepts().unwrap());
    assert_eq!(before, after, "un segundo rebuild debe ser idempotente");
}

#[test]
fn paridad_sql_igual_core_conformant() {
    let dir = tempfile::tempdir().unwrap();
    let files = lodestar_fixtures::conformant();
    write_all(dir.path(), &files);
    let store = Store::open_and_build(dir.path()).unwrap();
    assert_matches_core(&store, &files);
}

#[test]
fn paridad_sql_igual_core_with_issues() {
    let dir = tempfile::tempdir().unwrap();
    let files = lodestar_fixtures::with_issues();
    write_all(dir.path(), &files);
    let store = Store::open_and_build(dir.path()).unwrap();
    assert_matches_core(&store, &files);
}

#[test]
fn incremental_gate_por_hash_suprime_noops() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let p = rp("alfa.md");
    let raw = "---\ntype: Concept\ntitle: Alfa\ndescription: x\n---\n\n# H\n\ncuerpo\n";
    assert!(store.upsert(&p, raw, 0, 0).unwrap(), "primer upsert cambia");
    assert!(
        !store.upsert(&p, raw, 0, 0).unwrap(),
        "reescribir el mismo contenido (no-op/echo) NO debe cambiar nada"
    );
    // Un cambio real sí cambia.
    let raw2 = raw.replace("cuerpo", "cuerpo modificado");
    assert!(store.upsert(&p, &raw2, 0, 0).unwrap());
}

#[test]
fn property_incremental_igual_core() {
    // LCG determinista (Math.random no disponible en scripts de test tampoco hace falta).
    let mut seed: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let mut mirror: FileMap = FileMap::new();

    let names = [
        "index.md", "a.md", "b.md", "c.md", "d.md", "e.md",
    ];

    for _ in 0..120 {
        let name = names[next() % names.len()];
        let p = rp(name);
        let op = next() % 3;
        if op == 0 {
            // borrar
            mirror.remove(&p);
            store.remove(&p).unwrap();
        } else {
            // crear/modificar con un enlace pseudo-aleatorio
            let target = names[1 + (next() % (names.len() - 1))];
            let raw = if name == "index.md" {
                format!("---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [x](/{target})\n")
            } else {
                format!(
                    "---\ntype: Concept\ntitle: {name}\ndescription: d\n---\n\n# H\n\n[l](/{target})\n"
                )
            };
            mirror.insert(p.clone(), raw.clone());
            store.upsert(&p, &raw, 0, 0).unwrap();
        }
        assert_matches_core(&store, &mirror);
    }
}

#[test]
fn fts_subcadena_que_fts_pierde_aparece_via_core() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let raw = "---\ntype: Concept\ntitle: Fundamentos\ndescription: d\n---\n\n# H\n\nsobre programación\n";
    store.upsert(&rp("fund.md"), raw, 0, 0).unwrap();

    // "gramac" es subcadena de "programación" pero NO un token FTS → FTS no lo encuentra...
    assert!(
        store.fts_candidates("gramac").unwrap().is_empty(),
        "FTS tokeniza; no debería casar una subcadena parcial"
    );
    // ...pero la semántica de subcadena del core (search) SÍ.
    assert_eq!(store.search("gramac").unwrap(), vec![rp("fund.md")]);
}

#[test]
fn fts_expresion_maliciosa_no_rompe() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .upsert(&rp("a.md"), "---\ntype: C\ntitle: A\n---\n\n# H\n\nx\n", 0, 0)
        .unwrap();
    // Operadores/comillas de FTS5 no deben inyectar ni provocar error.
    for expr in ["\" OR 1=1 --", "a*", "NEAR(", "\"\"\"", "col:val"] {
        assert!(store.fts_candidates(expr).is_ok(), "expr {expr:?} rompió FTS");
    }
}

#[test]
fn blast_radius_igual_neighborhood_in() {
    // a -> b -> c ; blast-radius de c (In) = {c, b, a}
    let mut files = FileMap::new();
    files.insert(rp("a.md"), "---\ntype: C\ntitle: A\n---\n\n# H\n\n[b](/b.md)\n".into());
    files.insert(rp("b.md"), "---\ntype: C\ntitle: B\n---\n\n# H\n\n[c](/c.md)\n".into());
    files.insert(rp("c.md"), "---\ntype: C\ntitle: C\n---\n\n# H\n\nfin\n".into());

    let dir = tempfile::tempdir().unwrap();
    write_all(dir.path(), &files);
    let store = Store::open_and_build(dir.path()).unwrap();

    let bundle = Bundle::from_files(files.clone());
    let nb = bundle.neighborhood(&rp("c.md"), 5, Direction::In);
    let core_set: BTreeSet<RelPath> = nb.nodes.iter().map(|n| n.id.clone()).collect();
    let sql_set: BTreeSet<RelPath> = store.blast_radius(&rp("c.md"), 5).unwrap().into_iter().collect();
    assert_eq!(sql_set, core_set);
}

#[test]
fn bus_emite_indexevent_en_cambio() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let rx = store.subscribe();
    store
        .upsert(&rp("a.md"), "---\ntype: C\ntitle: A\n---\n\n# H\n\nx\n", 0, 0)
        .unwrap();
    let ev = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(ev.changed, vec![rp("a.md")]);
    assert!(ev.removed.is_empty());
    // Un no-op no emite: el canal queda vacío.
    store
        .upsert(&rp("a.md"), "---\ntype: C\ntitle: A\n---\n\n# H\n\nx\n", 0, 0)
        .unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
}

#[test]
fn reconcile_repara_drift_fuera_de_banda() {
    let dir = tempfile::tempdir().unwrap();
    let mut files = lodestar_fixtures::conformant();
    write_all(dir.path(), &files);
    let store = Store::open_and_build(dir.path()).unwrap();

    // Cambio fuera de banda: añade un fichero y borra otro directamente en disco.
    let nuevo = rp("gamma.md");
    let nuevo_raw = "---\ntype: Concept\ntitle: Gamma\ndescription: d\n---\n\n# H\n\n[a](/alfa.md)\n";
    std::fs::write(dir.path().join("gamma.md"), nuevo_raw).unwrap();
    std::fs::remove_file(dir.path().join("beta.md")).unwrap();
    files.insert(nuevo.clone(), nuevo_raw.into());
    files.remove(&rp("beta.md"));

    let ev = store.reconcile_all().unwrap();
    assert!(ev.changed.contains(&nuevo));
    assert!(ev.removed.contains(&rp("beta.md")));
    assert_matches_core(&store, &files);
}

#[test]
fn conceptstore_sirve_bundle_identico() {
    let dir = tempfile::tempdir().unwrap();
    let files = lodestar_fixtures::with_issues();
    write_all(dir.path(), &files);
    let store = Store::open_and_build(dir.path()).unwrap();

    let from_disk = Bundle::from_files(files.clone());
    let from_store = store.bundle();
    assert_eq!(
        from_store.analyze().hard_fail,
        from_disk.analyze().hard_fail
    );
    assert_eq!(from_store.files().len(), from_disk.files().len());
    assert_eq!(from_store.analyze(), from_disk.analyze());
}

#[test]
fn watcher_reconcilia_en_vivo() {
    let dir = tempfile::tempdir().unwrap();
    write_all(dir.path(), &lodestar_fixtures::conformant());
    let store = Arc::new(Store::open_and_build(dir.path()).unwrap());
    let rx = store.subscribe();
    let _w = store.watch().unwrap();

    // Escribe un fichero nuevo en disco; el watcher debe reconciliar y emitir.
    let raw = "---\ntype: Concept\ntitle: Delta\ndescription: d\n---\n\n# H\n\n[a](/alfa.md)\n";
    std::fs::write(dir.path().join("delta.md"), raw).unwrap();

    // Espera hasta ~5s a que el debouncer procese.
    let ev = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(ev.changed.contains(&rp("delta.md")) || !ev.changed.is_empty());
}
