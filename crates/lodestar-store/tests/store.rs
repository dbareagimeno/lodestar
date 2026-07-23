//! Tests de `lodestar-store` (E3): apertura/DDL, cold rebuild, incremental con gate por hash,
//! FTS5 (subcadena + escapado), **paridad SQL == core**, property incremental==core, bus de
//! eventos, reconcile y `DocumentStore`.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use lodestar_core::types::{Direction, FileMap, RelPath};
use lodestar_core::DocumentSet;
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
    let doc_set = DocumentSet::from_files(files.clone());
    let a = doc_set.analyze();

    let (hf, wc) = store.validation_counts().unwrap();
    assert_eq!(hf, a.hard_fail, "hard_fail difiere (gana el core)");
    assert_eq!(wc, a.warn_count, "warn_count difiere (gana el core)");

    assert_eq!(
        sorted(store.isolated().unwrap()),
        sorted(a.isolated.clone()),
        "isolated difiere"
    );
    assert_eq!(
        sorted(store.dangling().unwrap()),
        sorted(a.dangling.clone()),
        "dangling difiere"
    );
    assert_eq!(
        sorted(store.documents().unwrap()),
        sorted(a.documents.clone()),
        "el inventario de documentos difiere"
    );
    for p in &a.documents {
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
        assert!(!store.documents().unwrap().is_empty());
    }
    // Reabrir no rompe y ve el mismo contenido.
    let store = Store::open_and_build(dir.path()).unwrap();
    // 3 = los 3 `.md` del fixture: desde E16-H02 `index.md` es un documento más del inventario.
    assert_eq!(store.documents().unwrap().len(), 3);
    assert!(dir.path().join(".lodestar/index.db").exists());
}

#[test]
fn cold_rebuild_es_idempotente() {
    let dir = tempfile::tempdir().unwrap();
    write_all(dir.path(), &lodestar_fixtures::conformant());
    let store = Store::open_and_build(dir.path()).unwrap();
    let before = sorted(store.documents().unwrap());
    store.rebuild().unwrap();
    let after = sorted(store.documents().unwrap());
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
                format!("---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [x](/{target})\n")
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

    let doc_set = DocumentSet::from_files(files.clone());
    let nb = doc_set.neighborhood(&rp("c.md"), 5, Direction::In);
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
fn documentstore_sirve_workspace_identico() {
    let dir = tempfile::tempdir().unwrap();
    let files = lodestar_fixtures::with_issues();
    write_all(dir.path(), &files);
    let store = Store::open_and_build(dir.path()).unwrap();

    let from_disk = DocumentSet::from_files(files.clone());
    let from_store = store.document_set();
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
    assert!(store.documents().unwrap().contains(&rp("buena.md")));
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
    assert!(store.documents().unwrap().contains(&rp("a.md")));
    assert!(store.documents().unwrap().contains(&rp("b.md")));

    // Y la tabla `files` reconstruida ya tiene la columna `hash`.
    let conn = rusqlite::Connection::open(db_dir.join("index.db")).unwrap();
    assert!(
        conn.prepare("SELECT hash FROM files LIMIT 0").is_ok(),
        "la tabla files debe tener la columna hash tras el rebuild limpio"
    );
}

// ---------------------------------------------------------------------------
// E15-H01 — Borrar el crate `lodestar-vcs` y su cableado (cara del store)
// ---------------------------------------------------------------------------

/// `PRAGMA user_version` que estampó **v0.2** (la versión con la tabla `commit_conformance`).
/// Es un dato **histórico**, no la versión vigente: el test lo usa solo para fabricar una cache
/// antigua, y nunca asevera contra el número nuevo (que lee de disco), así que sigue valiendo
/// después de cualquier bump futuro.
const USER_VERSION_V02: i64 = 1;

/// DDL exacto que escribía v0.2 (incluida la tabla git `commit_conformance`).
const DDL_V02: &str = r#"
    CREATE TABLE files (
        path TEXT PRIMARY KEY, kind TEXT NOT NULL, type TEXT, title TEXT, description TEXT,
        status TEXT, resource TEXT, frontmatter_json TEXT NOT NULL DEFAULT '{}',
        body TEXT NOT NULL DEFAULT '', raw TEXT NOT NULL DEFAULT '', hash BLOB NOT NULL,
        mtime INTEGER NOT NULL DEFAULT 0, size INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE links (
        src TEXT NOT NULL, dst TEXT NOT NULL, href TEXT NOT NULL,
        src_is_index INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE tags (path TEXT NOT NULL, tag TEXT NOT NULL);
    CREATE TABLE diagnostics (
        path TEXT NOT NULL, code TEXT NOT NULL, level TEXT NOT NULL, msg TEXT NOT NULL,
        targets_json TEXT NOT NULL DEFAULT '[]'
    );
    CREATE VIRTUAL TABLE files_fts USING fts5(path UNINDEXED, title, description, body);
    CREATE TABLE commit_conformance (
        tree_oid TEXT PRIMARY KEY, hard_fail INTEGER NOT NULL, warn_count INTEGER NOT NULL,
        conform INTEGER NOT NULL
    );
"#;

/// Lee el `PRAGMA user_version` de la cache de `root` (sin pasar por el store).
fn user_version_de(root: &Path) -> i64 {
    let conn = rusqlite::Connection::open(root.join(".lodestar").join("index.db")).unwrap();
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

/// `cache_v2_se_reconstruye` — **Dado** una cache `.lodestar/index.db` escrita por v0.2 (con
/// `commit_conformance` y su `user_version`), **Cuando** se abre con el código nuevo, **Entonces**
/// se detecta que la versión no es la vigente y se reconstruye limpia, sin error y sirviendo el
/// contenido actual de los `.md` (`requirements/epica-15-workspace-universal.md` § E15-H01).
///
/// **Sin acoplarse al número de versión nuevo**: el test estampa la versión *histórica* de v0.2 y
/// asevera que, tras abrir, la cache quedó estampada con una versión **distinta** — es decir, que
/// el bump de `USER_VERSION` (`schema.rs`) ocurrió y disparó la reconstrucción. Cualquier bump
/// futuro lo mantiene verde.
///
/// Fase ROJA: hoy `USER_VERSION == 1 == USER_VERSION_V02` y `schema_is_current` acepta el esquema
/// v0.2 tal cual (es idéntico al vigente), así que la cache **no** se detecta antigua: la versión
/// sigue siendo la de v0.2 y una cache nueva sigue trayendo la tabla `commit_conformance`. Las dos
/// aserciones de retirada fallan.
#[test]
fn cache_v2_se_reconstruye() {
    // (1) Un proyecto con un `.md` y una cache fabricada a mano con el esquema y la versión de v0.2.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.md"),
        "---\ntype: N\ntitle: A\ndescription: d\n---\n\n# H\n",
    )
    .unwrap();
    let db_dir = dir.path().join(".lodestar");
    std::fs::create_dir_all(&db_dir).unwrap();
    {
        let conn = rusqlite::Connection::open(db_dir.join("index.db")).unwrap();
        conn.execute_batch(DDL_V02).unwrap();
        // Una fila de la tabla git, para que la cache v0.2 sea genuina (no un esquema vacío).
        conn.execute(
            "INSERT INTO commit_conformance (tree_oid, hard_fail, warn_count, conform) \
             VALUES ('deadbeef', 0, 0, 1)",
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", USER_VERSION_V02)
            .unwrap();
    }
    // Guarda anti-vacuidad: la cache de partida está estampada con la versión de v0.2.
    assert_eq!(user_version_de(dir.path()), USER_VERSION_V02);

    // (2) Abrir con el código nuevo: sin error y sirviendo el contenido actual de los `.md`.
    let store =
        Store::open_and_build(dir.path()).expect("una cache v0.2 no puede romper la apertura");
    assert!(
        store.documents().unwrap().contains(&rp("a.md")),
        "tras la reconstrucción, la cache debe servir el contenido actual de los `.md`"
    );

    // (3) Se detectó como antigua: la cache quedó estampada con una versión DISTINTA de la de v0.2.
    assert_ne!(
        user_version_de(dir.path()),
        USER_VERSION_V02,
        "la cache de v0.2 debe detectarse antigua (bump de `USER_VERSION`) y reconstruirse"
    );

    // (4) Y el esquema nuevo es limpio: una cache recién creada NO trae la tabla git
    //     `commit_conformance` (se retira del DDL en E15-H01).
    let limpio = tempfile::tempdir().unwrap();
    let _fresca = Store::open(limpio.path()).unwrap();
    let conn =
        rusqlite::Connection::open(limpio.path().join(".lodestar").join("index.db")).unwrap();
    assert!(
        conn.prepare("SELECT tree_oid FROM commit_conformance LIMIT 0")
            .is_err(),
        "el DDL vigente no debe crear la tabla git `commit_conformance`"
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
    assert!(store.documents().unwrap().contains(&rp("a.md")));
}

// --- E11-H05: `impact_analyze` reusa el blast-radius; verificado idéntico al core ------------
//
// UBICACIÓN (documentada): el criterio `impacto_paridad_core` de la historia E11-H05 es una
// paridad **store vs core** — el `Store::blast_radius` (CTE recursivo, el bloque que
// `transitivelyAffected` de `impact_analyze` reusa) debe alcanzar EXACTAMENTE el mismo conjunto de
// paths que `DocumentSet::neighborhood(In)` del core (invariante #3: "gana el core"; SQLite solo
// acelera). No tiene superficie de wire ni depende de la tool MCP, así que vive junto al bloque que
// verifica (`lodestar-store`), no en `crates/lodestar-mcp/tests/`. El parent autorizó explícitamente
// esta ubicación ("puede vivir en crates/lodestar-store/tests/ … elige dónde encaja; documenta").
//
// NOTA SOBRE EL COLOR (honestidad de fase): este test es **VERDE desde ya**, no rojo. Verifica un
// invariante que YA se sostiene porque `Store::blast_radius` y `DocumentSet::neighborhood` YA existen
// (E11-H05 los REUSA, no los crea). Su valor es de guarda de regresión: sella que el bloque que
// `impact_analyze` reusa es paridad-exacta con el core antes de construir la tool encima. El ROJO de
// la historia lo aportan los dos criterios de comportamiento (`impacto_move_30`,
// `impacto_delete_bloqueos`) en `crates/lodestar-mcp/tests/mcp.rs`, que sí necesitan la tool/servicio
// inexistentes. Es una elección deliberada frente a duplicar el nombre en MCP con un rojo artificial:
// aquí la aserción es real y no vacua (topología no trivial, ver abajo), no un placeholder.
//
// NO-VACUIDAD: usa un grafo NO lineal (diamante A→{B,C}→D, más una rama larga D→E→F y un nodo
// DESCONECTADO `z.md`) para que la igualdad de conjuntos sea informativa: el blast-radius(In) de F
// debe reunir a {F,E,D,B,C,A} y EXCLUIR a `z.md`. Un CTE mal escrito (o un core divergente) rompería
// en esta topología aunque pasara el caso lineal preexistente (`blast_radius_igual_neighborhood_in`).

#[test]
fn impacto_paridad_core() {
    // Diamante + rama + desconectado:
    //   A ─▶ B ─┐
    //   A ─▶ C ─┴▶ D ─▶ E ─▶ F        (aristas dirigidas por enlaces de cuerpo)
    //   z (aislado, sin aristas)
    // blast-radius(In) de F = {F, E, D, B, C, A}; `z` queda fuera.
    let mut files = FileMap::new();
    files.insert(
        rp("a.md"),
        "---\ntype: C\ntitle: A\n---\n\n# H\n\n[b](/b.md) y [c](/c.md)\n".into(),
    );
    files.insert(
        rp("b.md"),
        "---\ntype: C\ntitle: B\n---\n\n# H\n\n[d](/d.md)\n".into(),
    );
    files.insert(
        rp("c.md"),
        "---\ntype: C\ntitle: C\n---\n\n# H\n\n[d](/d.md)\n".into(),
    );
    files.insert(
        rp("d.md"),
        "---\ntype: C\ntitle: D\n---\n\n# H\n\n[e](/e.md)\n".into(),
    );
    files.insert(
        rp("e.md"),
        "---\ntype: C\ntitle: E\n---\n\n# H\n\n[f](/f.md)\n".into(),
    );
    files.insert(
        rp("f.md"),
        "---\ntype: C\ntitle: F\n---\n\n# H\n\nfin\n".into(),
    );
    files.insert(
        rp("z.md"),
        "---\ntype: C\ntitle: Z\n---\n\n# H\n\naislado\n".into(),
    );

    let dir = tempfile::tempdir().unwrap();
    write_all(dir.path(), &files);
    let store = Store::open_and_build(dir.path()).unwrap();

    let doc_set = DocumentSet::from_files(files.clone());
    // Profundidad grande para alcanzar todo el alcance transitivo, no solo el vecindario inmediato.
    let core_set: BTreeSet<RelPath> = doc_set
        .neighborhood(&rp("f.md"), 10, Direction::In)
        .nodes
        .iter()
        .map(|n| n.id.clone())
        .collect();
    let sql_set: BTreeSet<RelPath> = store
        .blast_radius(&rp("f.md"), 10)
        .unwrap()
        .into_iter()
        .collect();

    assert_eq!(
        sql_set, core_set,
        "el blast-radius del store debe ser idéntico a neighborhood(In) del core (gana el core)"
    );
    // Sanidad de la topología (no vacuo): el alcance transitivo cubre el diamante y excluye el nodo
    // desconectado. Si el core cambiara y ambos conjuntos degeneraran juntos, esto lo cazaría.
    let esperado: BTreeSet<RelPath> = ["f.md", "e.md", "d.md", "b.md", "c.md", "a.md"]
        .into_iter()
        .map(rp)
        .collect();
    assert_eq!(core_set, esperado, "el core debe alcanzar todo el diamante");
    assert!(
        !core_set.contains(&rp("z.md")),
        "el nodo desconectado no debe estar en el blast-radius"
    );
}
