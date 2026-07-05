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
        store
            .in_index()
            .unwrap()
            .into_iter()
            .collect::<BTreeSet<_>>(),
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
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let mut mirror: FileMap = FileMap::new();

    let names = ["index.md", "a.md", "b.md", "c.md", "d.md", "e.md"];

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
        .upsert(
            &rp("a.md"),
            "---\ntype: C\ntitle: A\n---\n\n# H\n\nx\n",
            0,
            0,
        )
        .unwrap();
    // Operadores/comillas de FTS5 no deben inyectar ni provocar error.
    for expr in ["\" OR 1=1 --", "a*", "NEAR(", "\"\"\"", "col:val"] {
        assert!(
            store.fts_candidates(expr).is_ok(),
            "expr {expr:?} rompió FTS"
        );
    }
}

#[test]
fn blast_radius_igual_neighborhood_in() {
    // a -> b -> c ; blast-radius de c (In) = {c, b, a}
    let mut files = FileMap::new();
    files.insert(
        rp("a.md"),
        "---\ntype: C\ntitle: A\n---\n\n# H\n\n[b](/b.md)\n".into(),
    );
    files.insert(
        rp("b.md"),
        "---\ntype: C\ntitle: B\n---\n\n# H\n\n[c](/c.md)\n".into(),
    );
    files.insert(
        rp("c.md"),
        "---\ntype: C\ntitle: C\n---\n\n# H\n\nfin\n".into(),
    );

    let dir = tempfile::tempdir().unwrap();
    write_all(dir.path(), &files);
    let store = Store::open_and_build(dir.path()).unwrap();

    let bundle = Bundle::from_files(files.clone());
    let nb = bundle.neighborhood(&rp("c.md"), 5, Direction::In);
    let core_set: BTreeSet<RelPath> = nb.nodes.iter().map(|n| n.id.clone()).collect();
    let sql_set: BTreeSet<RelPath> = store
        .blast_radius(&rp("c.md"), 5)
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(sql_set, core_set);
}

#[test]
fn bus_emite_indexevent_en_cambio() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let rx = store.subscribe();
    store
        .upsert(
            &rp("a.md"),
            "---\ntype: C\ntitle: A\n---\n\n# H\n\nx\n",
            0,
            0,
        )
        .unwrap();
    let ev = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(ev.changed, vec![rp("a.md")]);
    assert!(ev.removed.is_empty());
    // Un no-op no emite: el canal queda vacío.
    store
        .upsert(
            &rp("a.md"),
            "---\ntype: C\ntitle: A\n---\n\n# H\n\nx\n",
            0,
            0,
        )
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
    let nuevo_raw =
        "---\ntype: Concept\ntitle: Gamma\ndescription: d\n---\n\n# H\n\n[a](/alfa.md)\n";
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

// --- Regresiones de la revisión profunda -------------------------------------

#[test]
fn fichero_no_utf8_no_congela_la_cache() {
    // Antes, UN .md no-UTF8 abortaba TODO walk_disk → ninguna reconciliación volvía a aplicar.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("buena.md"),
        "---\ntype: N\ntitle: B\ndescription: d\n---\n\n# H\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("latin1.md"),
        b"---\ntype: N\n---\n\n# a\xf1o\n",
    )
    .unwrap();
    let store = Store::open_and_build(dir.path()).unwrap();
    // La buena está indexada; la ilegible se saltó con diagnóstico (no venenó el walk).
    assert!(store.concepts().unwrap().contains(&rp("buena.md")));
    // Y el reconcile sigue vivo (repara drift a pesar del fichero no-UTF8).
    std::fs::write(
        dir.path().join("nueva.md"),
        "---\ntype: N\ntitle: N2\ndescription: d\n---\n\n# H\n",
    )
    .unwrap();
    let ev = store.reconcile_all().unwrap();
    assert!(ev.changed.contains(&rp("nueva.md")));
}

#[test]
fn search_unicode_case_insensitive_como_el_core() {
    // El LIKE de SQLite plegaba solo ASCII: «PROGRAMACIÓN» no casaba con «programación».
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("curso.md"),
        "---\ntype: N\ntitle: Curso\ndescription: d\n---\n\n# H\n\nPROGRAMACI\u{d3}N avanzada\n",
    )
    .unwrap();
    let store = Store::open_and_build(dir.path()).unwrap();
    let hits = store.search("programaci\u{f3}n").unwrap();
    assert!(hits.contains(&rp("curso.md")), "hits: {hits:?}");
    // Y el texto suelto también cubre basename y valores de frontmatter (semántica del core).
    assert!(store.search("curso").unwrap().contains(&rp("curso.md")));
}

#[test]
fn esquema_viejo_con_misma_version_se_reconstruye() {
    // Regresión: un build antiguo dejó `user_version=1` pero una tabla `files` SIN la columna
    // `hash`. Como `create_schema` es `IF NOT EXISTS`, la tabla vieja sobrevivía y el upsert
    // reventaba con «table files has no column named hash». `schema_is_current` lo detecta y
    // fuerza el rebuild limpio pese a coincidir la versión.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.md"),
        "---\ntype: N\ntitle: A\ndescription: d\n---\n\n# H\n",
    )
    .unwrap();

    // Fabrica a mano una cache con esquema viejo (solo `path`/`kind`, sin `hash`).
    let db_dir = dir.path().join(".lodestar");
    std::fs::create_dir_all(&db_dir).unwrap();
    {
        let conn = rusqlite::Connection::open(db_dir.join("index.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE files (path TEXT PRIMARY KEY, kind TEXT);
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }

    // open + rebuild + upsert deben funcionar sin error pese al esquema derivado.
    let store = Store::open(dir.path()).unwrap();
    store.rebuild().unwrap();
    store
        .upsert(
            &rp("b.md"),
            "---\ntype: N\ntitle: B\ndescription: d\n---\n\n# H\n",
            0,
            0,
        )
        .unwrap();
    assert!(store.concepts().unwrap().contains(&rp("a.md")));
    assert!(store.concepts().unwrap().contains(&rp("b.md")));

    // Y la tabla `files` reconstruida ya tiene la columna `hash`.
    let conn = rusqlite::Connection::open(db_dir.join("index.db")).unwrap();
    assert!(
        conn.prepare("SELECT hash FROM files LIMIT 0").is_ok(),
        "la tabla files debe tener la columna hash tras el rebuild limpio"
    );
}

#[test]
fn cache_corrupta_se_recrea_sola() {
    // La cache es desechable: un index.db corrupto no puede dejar open() fallando para siempre.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.md"),
        "---\ntype: N\ntitle: A\ndescription: d\n---\n\n# H\n",
    )
    .unwrap();
    let db_dir = dir.path().join(".lodestar");
    std::fs::create_dir_all(&db_dir).unwrap();
    std::fs::write(db_dir.join("index.db"), b"esto no es una base sqlite").unwrap();
    let store = Store::open_and_build(dir.path()).unwrap();
    assert!(store.concepts().unwrap().contains(&rp("a.md")));
}
