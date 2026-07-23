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

    // MIGRADO en E17-H04: `hard_fail`/`warn_count` son métodos derivados de `diagnostics`.
    let (hf, wc) = store.validation_counts().unwrap();
    assert_eq!(hf, a.hard_fail(), "hard_fail difiere (gana el core)");
    assert_eq!(wc, a.warn_count(), "warn_count difiere (gana el core)");

    assert_eq!(
        sorted(store.isolated().unwrap()),
        sorted(a.isolated.clone()),
        "isolated difiere"
    );
    // MIGRADO en E17-H04: `Analysis::dangling` es una lista de ENLACES rotos (origen + destino +
    // href), y la síntesis SQL sigue devolviendo los destinos fantasma. Se comparan los destinos,
    // deduplicados, que es lo que ambas vistas dicen del grafo.
    let mut destinos_colgantes: Vec<RelPath> =
        a.dangling.iter().map(|d| d.target.clone()).collect();
    destinos_colgantes.sort();
    destinos_colgantes.dedup();
    assert_eq!(
        sorted(store.dangling().unwrap()),
        destinos_colgantes,
        "dangling difiere"
    );
    assert_eq!(
        sorted(store.documents().unwrap()),
        sorted(a.documents.clone()),
        "el inventario de documentos difiere"
    );
    for p in &a.documents {
        // MIGRADO en E17-H04: `inn` pasó a `incoming`, con una entrada por ENLACE; el store
        // sintetiza vecinos (`SELECT DISTINCT src`), así que se comparan los orígenes distintos.
        let mut expected: Vec<RelPath> = a
            .incoming
            .get(p)
            .into_iter()
            .flatten()
            .map(|r| r.from.clone())
            .collect();
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
        from_store.analyze().hard_fail(),
        from_disk.analyze().hard_fail()
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

    // Y la tabla `documents` reconstruida ya tiene la columna `content_hash` (store v2, E18-H01:
    // `files.hash` → `documents.content_hash`).
    let conn = rusqlite::Connection::open(db_dir.join("index.db")).unwrap();
    assert!(
        conn.prepare("SELECT content_hash FROM documents LIMIT 0")
            .is_ok(),
        "la tabla documents debe tener la columna content_hash tras el rebuild limpio"
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

// ===========================================================================
// E18 — Store v2 (`ARCHITECTURE.md §20.12`, `REFACTOR_PHASE_2 §Fase 9`)
// ===========================================================================
//
// Las tablas del store son INTERNAS (el store es su dueño único, la historia no abre frontera
// MCP), así que estos tests las consultan por SQL con una segunda conexión, igual que ya hacen
// `cache_v2_se_reconstruye` y `esquema_viejo_con_misma_version_se_reconstruye`. No se añade API
// pública para observarlas: el DDL es el sujeto del test.
//
// DECISIONES DE CRITERIO PROPIO que estos tests fijan (la historia deja la forma exacta abierta):
//
//   1. `value_type` es un **catálogo cerrado de 6**: `string|number|boolean|null|array|object`.
//      Son los tipos que `§20.8` prohíbe coercionar entre sí («sin coerción implícita entre
//      string/número, string/booleano, escalar/lista, lista/objeto»), así que son exactamente los
//      que E19 necesita para decidir que `priority >= "high"` es un error de tipo y los que E20
//      necesita para comunicar heterogeneidad en `inferredTypes`.
//
//   2. **Una lista es UNA fila**, con el array entero en `value_json` — no una fila por elemento.
//      Razón: `FieldPath` no direcciona posiciones (`§20.8` es dot-notation sobre mapas), así que
//      `owners.0` sería un path que `ParsedFrontmatter::get` NO resuelve; materializarlo obligaría
//      a un segundo navegador del `Value` que no es la única verdad de acceso (invariante #3), que
//      es justo lo que la historia prohíbe. Y no hace falta: `owners contains "security"` (E19) y
//      el recuento de valores frecuentes (E20) operan sobre el valor de la lista, que está entero
//      en `value_json`. Es el mismo reparto que ya practica `synth.rs` — SQL sirve las filas, el
//      core dictamina.
//
//   3. **Los mapas intermedios SÍ tienen fila** (`service` además de `service.name`): son
//      propiedades direccionables por `get`, y `has(service)` (E19) y el catálogo de propiedades
//      (E20) las necesitan. La regla queda entonces en una sola frase, verificable:
//      **hay exactamente una fila por propiedad direccionable por `ParsedFrontmatter::get`**
//      (equivalentemente: por cada par de `ParsedFrontmatter::walk`, ver
//      `crates/lodestar-core/tests/documento.rs`).
//
//   4. `target_kind` es el **discriminante serde de `LinkTarget`** (`document`, `workspaceFile`,
//      `externalUri`, `selfAnchor`, `missing`, `escapesWorkspace`): la etiqueta que el propio enum
//      ya define para el wire (`#[serde(tag = "kind", …, rename_all = "camelCase")]`). No se
//      inventa un vocabulario paralelo de la cache — la columna es la proyección a texto de la
//      clasificación del core, y los tests lo comprueban **derivándola del enum**, no copiándola.

/// Conexión de solo lectura a la cache de `root` (las tablas son internas: se consultan por SQL).
fn conexion(root: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(root.join(".lodestar").join("index.db")).unwrap()
}

/// Los 6 valores admitidos de `metadata.value_type` (decisión 1 de la cabecera).
const TIPOS_VALIDOS: [&str; 6] = ["string", "number", "boolean", "null", "array", "object"];

/// Una fila de la tabla `metadata` del store v2 (`§20.12`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct FilaMetadata {
    field_path: String,
    value_json: String,
    value_type: String,
}

impl FilaMetadata {
    /// El `value_json` ya parseado (se compara el VALOR, no el texto: el orden de las claves de un
    /// objeto serializado no es parte del contrato).
    fn valor(&self) -> serde_json::Value {
        serde_json::from_str(&self.value_json).unwrap_or_else(|e| {
            panic!(
                "`value_json` de `{}` no es JSON válido ({e}): {:?}",
                self.field_path, self.value_json
            )
        })
    }
}

/// Filas de `metadata` de un documento, ordenadas por `field_path`.
fn filas_metadata(root: &Path, doc: &str) -> Vec<FilaMetadata> {
    let conn = conexion(root);
    let mut stmt = conn
        .prepare(
            "SELECT field_path, value_json, value_type FROM metadata \
             WHERE document_path = ?1 ORDER BY field_path",
        )
        .expect(
            "el store v2 debe materializar la tabla `metadata(document_path, field_path, \
             value_json, value_type)` (`§20.12`, E18-H01)",
        );
    let filas = stmt
        .query_map([doc], |r| {
            Ok(FilaMetadata {
                field_path: r.get(0)?,
                value_json: r.get(1)?,
                value_type: r.get(2)?,
            })
        })
        .unwrap();
    filas.map(|f| f.unwrap()).collect()
}

/// La fila de un `field_path` concreto (falla con un mensaje útil si no está).
fn fila_de<'a>(filas: &'a [FilaMetadata], field_path: &str) -> &'a FilaMetadata {
    filas
        .iter()
        .find(|f| f.field_path == field_path)
        .unwrap_or_else(|| {
            panic!(
                "falta la fila `{field_path}`; hay: {:?}",
                filas.iter().map(|f| &f.field_path).collect::<Vec<_>>()
            )
        })
}

/// El valor YAML de `v` convertido a JSON, que es la forma canónica con la que se compara
/// `value_json` (parseado, no como texto).
fn a_json(v: &serde_yaml::Value) -> serde_json::Value {
    serde_json::to_value(v).expect("el frontmatter de estos tests es representable en JSON")
}

/// **Cruce con la única verdad de acceso (invariante #3)**: cada fila de `metadata` debe ser
/// exactamente lo que `ParsedFrontmatter::get` devuelve para ese `field_path`, y el conjunto de
/// filas debe cubrir **todas** las propiedades direccionables — ni una de más (paths inventados
/// que `get` no resuelve) ni una de menos.
///
/// Es la aserción que impide que el store acabe con un segundo navegador del `Value` en SQL, que
/// es lo que la historia prohíbe explícitamente.
fn assert_metadata_coincide_con_el_core(raw: &str, filas: &[FilaMetadata]) {
    let parsed = lodestar_core::model::parse_file("x.md", raw);
    let fm = parsed
        .frontmatter
        .expect("el documento de este test tiene frontmatter");

    for fila in filas {
        let path = lodestar_core::types::FieldPath::parse(&fila.field_path)
            .unwrap_or_else(|e| panic!("`{}` no es un FieldPath válido: {e:?}", fila.field_path));
        let valor = fm.get(&path).unwrap_or_else(|| {
            panic!(
                "la cache materializó `{}`, pero `ParsedFrontmatter::get` no lo resuelve: el \
                 recorrido sería un segundo navegador del `Value` (invariante #3)",
                fila.field_path
            )
        });
        assert_eq!(
            fila.valor(),
            a_json(valor),
            "el `value_json` de `{}` no es el valor que devuelve el core",
            fila.field_path
        );
        assert!(
            TIPOS_VALIDOS.contains(&fila.value_type.as_str()),
            "`value_type` fuera del catálogo cerrado {TIPOS_VALIDOS:?}: {:?}",
            fila.value_type
        );
    }
}

// --- E18-H01: DDL v2 (`documents` y `metadata`) ------------------------------

/// `metadata_indexa_paths_anidados` — **Dado** un documento con `service: {name: auth, tier:
/// critical}`, **Cuando** se indexa, **Entonces** hay filas `service.name` y `service.tier` con su
/// valor y su tipo.
///
/// Además del criterio, fija la regla completa del recorrido (decisión 3): el mapa intermedio
/// `service` **también** tiene fila, y no hay ninguna otra — el conjunto de filas es exactamente
/// el de propiedades direccionables por `ParsedFrontmatter::get`.
///
/// Fase ROJA: la tabla `metadata` no existe (hoy el frontmatter se materializa entero como texto
/// en `files.frontmatter_json`, sin indexar por field path).
#[test]
fn metadata_indexa_paths_anidados() {
    let dir = tempfile::tempdir().unwrap();
    let raw = "---\nservice:\n  name: auth\n  tier: critical\n---\n\n# Servicio de auth\n";
    std::fs::write(dir.path().join("svc.md"), raw).unwrap();

    let store = Store::open_and_build(dir.path()).unwrap();
    assert!(
        store.documents().unwrap().contains(&rp("svc.md")),
        "guarda anti-vacuidad: el documento está indexado"
    );

    let filas = filas_metadata(dir.path(), "svc.md");

    // (1) El criterio: las dos propiedades anidadas, con su valor y su tipo.
    assert_eq!(
        fila_de(&filas, "service.name").valor(),
        serde_json::json!("auth")
    );
    assert_eq!(fila_de(&filas, "service.name").value_type, "string");
    assert_eq!(
        fila_de(&filas, "service.tier").valor(),
        serde_json::json!("critical")
    );
    assert_eq!(fila_de(&filas, "service.tier").value_type, "string");

    // (2) El mapa intermedio es una propiedad direccionable y tiene fila propia, con el objeto
    //     entero (lo necesitan `has(service)` de E19 y el catálogo de propiedades de E20).
    assert_eq!(
        fila_de(&filas, "service").valor(),
        serde_json::json!({"name": "auth", "tier": "critical"})
    );
    assert_eq!(fila_de(&filas, "service").value_type, "object");

    // (3) Y NADA más: el recorrido no inventa paths (ni `service.name.x`, ni la raíz).
    let paths: Vec<&str> = filas.iter().map(|f| f.field_path.as_str()).collect();
    assert_eq!(paths, vec!["service", "service.name", "service.tier"]);

    // (4) Cruce con la única verdad de acceso.
    assert_metadata_coincide_con_el_core(raw, &filas);
}

/// `metadata_conserva_el_tipo` — **Dado** un documento con `priority: 2` y otro con
/// `priority: "alta"`, **Cuando** se indexan, **Entonces** las dos filas conservan tipos distintos
/// (`number` y `string`).
///
/// Extiende el criterio al **catálogo cerrado de 6** (decisión 1 de la cabecera): sin `boolean`,
/// `null`, `array` y `object` distinguibles, E19 no puede rechazar `priority >= "high"` como error
/// de tipo ni E20 comunicar la heterogeneidad de una propiedad.
///
/// Fase ROJA: no hay tabla `metadata` (hoy el tipo solo sobrevive dentro del JSON del frontmatter
/// completo, no como columna consultable).
#[test]
fn metadata_conserva_el_tipo() {
    let dir = tempfile::tempdir().unwrap();
    let raw_a = "---\npriority: 2\n---\n\n# A\n";
    let raw_b = "---\npriority: \"alta\"\n---\n\n# B\n";
    let raw_c = "---\nactivo: true\nvacio: null\nowners: [platform]\nservice:\n  tier: critical\n---\n\n# C\n";
    std::fs::write(dir.path().join("a.md"), raw_a).unwrap();
    std::fs::write(dir.path().join("b.md"), raw_b).unwrap();
    std::fs::write(dir.path().join("c.md"), raw_c).unwrap();

    let store = Store::open_and_build(dir.path()).unwrap();
    assert_eq!(
        store.documents().unwrap().len(),
        3,
        "guarda anti-vacuidad: los tres documentos están indexados"
    );

    // (1) El criterio: la MISMA propiedad con dos tipos distintos en dos documentos.
    let a = filas_metadata(dir.path(), "a.md");
    let b = filas_metadata(dir.path(), "b.md");
    assert_eq!(fila_de(&a, "priority").value_type, "number");
    assert_eq!(fila_de(&a, "priority").valor(), serde_json::json!(2));
    assert_eq!(fila_de(&b, "priority").value_type, "string");
    assert_eq!(fila_de(&b, "priority").valor(), serde_json::json!("alta"));
    assert_ne!(
        fila_de(&a, "priority").value_type,
        fila_de(&b, "priority").value_type,
        "sin tipos distintos, `priority: 2` y `priority: \"alta\"` serían el mismo dato y la \
         comparación sin coerción de `§20.8` no podría existir"
    );

    // (2) El resto del catálogo cerrado, cada uno con su valor JSON.
    let c = filas_metadata(dir.path(), "c.md");
    assert_eq!(fila_de(&c, "activo").value_type, "boolean");
    assert_eq!(fila_de(&c, "activo").valor(), serde_json::json!(true));
    // Una clave presente con valor `null` NO es una clave ausente (`§20.4`): tiene fila, con tipo
    // propio.
    assert_eq!(fila_de(&c, "vacio").value_type, "null");
    assert_eq!(fila_de(&c, "vacio").valor(), serde_json::Value::Null);
    assert_eq!(fila_de(&c, "owners").value_type, "array");
    assert_eq!(fila_de(&c, "service").value_type, "object");
    assert_eq!(fila_de(&c, "service.tier").value_type, "string");

    // (3) El catálogo es CERRADO: ninguna fila de la cache usa un tipo fuera de los seis.
    let conn = conexion(dir.path());
    let mut stmt = conn
        .prepare("SELECT DISTINCT value_type FROM metadata ORDER BY value_type")
        .expect("la tabla `metadata` debe existir");
    let tipos: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|t| t.unwrap())
        .collect();
    for t in &tipos {
        assert!(
            TIPOS_VALIDOS.contains(&t.as_str()),
            "`value_type` fuera del catálogo cerrado {TIPOS_VALIDOS:?}: {t:?}"
        );
    }
    let mut esperados = TIPOS_VALIDOS.map(String::from);
    esperados.sort();
    assert_eq!(
        tipos,
        esperados.to_vec(),
        "el fixture cubre los seis tipos del catálogo cerrado; tipos observados: {tipos:?}"
    );

    assert_metadata_coincide_con_el_core(raw_c, &c);
}

/// `metadata_roundtrip_json` — **Dado** un documento con listas y objetos anidados en listas,
/// **Cuando** se indexa, **Entonces** el `value_json` permite reconstruir el valor original.
///
/// «Reconstruir» se juzga en serio: con las filas de **primer nivel** se rearma el frontmatter
/// entero y se compara con el que devuelve el core. Eso es lo que fija la decisión 2 (una lista es
/// una fila, con el array entero): si la cache aplanara `contacts` en filas por elemento, la
/// reconstrucción perdería la forma y este test lo vería.
///
/// Fase ROJA: no hay tabla `metadata`.
#[test]
fn metadata_roundtrip_json() {
    let dir = tempfile::tempdir().unwrap();
    let raw = concat!(
        "---\n",
        "owners: [platform, security]\n",
        "contacts:\n",
        "  - nombre: Ana\n",
        "    rol: sre\n",
        "  - nombre: Bea\n",
        "matriz:\n",
        "  - [1, 2]\n",
        "  - [3]\n",
        "service:\n",
        "  name: auth\n",
        "  equipos:\n",
        "    - core\n",
        "---\n",
        "\n",
        "# Documento con listas\n",
    );
    std::fs::write(dir.path().join("listas.md"), raw).unwrap();
    let store = Store::open_and_build(dir.path()).unwrap();
    assert!(store.documents().unwrap().contains(&rp("listas.md")));

    let filas = filas_metadata(dir.path(), "listas.md");

    // (1) La lista de mapas viaja ENTERA y con su forma: es el caso que tienta a aplanar.
    assert_eq!(fila_de(&filas, "contacts").value_type, "array");
    assert_eq!(
        fila_de(&filas, "contacts").valor(),
        serde_json::json!([{"nombre": "Ana", "rol": "sre"}, {"nombre": "Bea"}]),
        "el objeto anidado dentro de la lista debe poder reconstruirse desde `value_json`"
    );
    // Listas de listas incluidas.
    assert_eq!(
        fila_de(&filas, "matriz").valor(),
        serde_json::json!([[1, 2], [3]])
    );
    // Una lista colgando de un mapa sigue siendo una hoja, con su propio field path.
    assert_eq!(
        fila_de(&filas, "service.equipos").valor(),
        serde_json::json!(["core"])
    );

    // (2) No hay filas POR DENTRO de una lista (`contacts.0.nombre` no es direccionable).
    for f in &filas {
        assert!(
            !f.field_path.starts_with("contacts.")
                && !f.field_path.starts_with("owners.")
                && !f.field_path.starts_with("matriz.")
                && !f.field_path.starts_with("service.equipos."),
            "`{}` desciende por dentro de una lista: `FieldPath` no direcciona posiciones",
            f.field_path
        );
    }

    // (3) ROUND-TRIP de verdad: con las filas de primer nivel se rearma el frontmatter entero.
    let mut reconstruido = serde_json::Map::new();
    for f in &filas {
        if !f.field_path.contains('.') {
            reconstruido.insert(f.field_path.clone(), f.valor());
        }
    }
    let esperado = a_json(
        &lodestar_core::model::parse_file("listas.md", raw)
            .frontmatter
            .expect("tiene frontmatter")
            .value,
    );
    assert_eq!(
        serde_json::Value::Object(reconstruido),
        esperado,
        "las filas de `metadata` deben permitir reconstruir el frontmatter original"
    );

    // (4) Y cada fila sigue siendo lo que dice el core.
    assert_metadata_coincide_con_el_core(raw, &filas);
}

/// `PRAGMA user_version` que estampó **v0.3** (la versión de E16-H02: sin `files.kind` ni
/// `links.src_is_index`, todavía con la tabla `tags` y con las columnas OKF promovidas).
///
/// Dato **histórico**, como `USER_VERSION_V02`: el test lo usa para fabricar una cache antigua y
/// nunca asevera contra el número nuevo (que lee de disco), así que sobrevive a cualquier bump.
const USER_VERSION_V03: i64 = 3;

/// DDL exacto que escribía v0.3 (`schema.rs` antes de E18-H01).
const DDL_V03: &str = r#"
    CREATE TABLE files (
        path TEXT PRIMARY KEY, type TEXT, title TEXT, description TEXT, status TEXT,
        resource TEXT, frontmatter_json TEXT NOT NULL DEFAULT '{}',
        body TEXT NOT NULL DEFAULT '', raw TEXT NOT NULL DEFAULT '', hash BLOB NOT NULL,
        mtime INTEGER NOT NULL DEFAULT 0, size INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE links (src TEXT NOT NULL, dst TEXT NOT NULL, href TEXT NOT NULL);
    CREATE TABLE tags (path TEXT NOT NULL, tag TEXT NOT NULL);
    CREATE TABLE diagnostics (
        path TEXT NOT NULL, code TEXT NOT NULL, level TEXT NOT NULL, msg TEXT NOT NULL,
        targets_json TEXT NOT NULL DEFAULT '[]'
    );
    CREATE VIRTUAL TABLE files_fts USING fts5(path UNINDEXED, title, description, body);
"#;

/// `cache_v3_se_reconstruye` — **Dado** una cache de v0.3 con el DDL viejo, **Cuando** se abre,
/// **Entonces** se detecta antigua y se reconstruye.
///
/// **Sin acoplarse al número de versión nuevo** (igual que `cache_v2_se_reconstruye`): estampa la
/// versión histórica de v0.3 y asevera que, tras abrir, la cache quedó estampada con una
/// **distinta**. Cualquier bump futuro lo mantiene verde.
///
/// Verifica además el **criterio de salida de la épica** sobre el esquema nuevo: ni columnas OKF
/// promovidas ni tabla `tags`, y `documents` con la forma de `§20.12` — incluido que `title` es el
/// **título derivado** (`§20.4`), no el campo `title` del usuario.
///
/// Fase ROJA: hoy `USER_VERSION == 3 == USER_VERSION_V03` y `schema_is_current` acepta el DDL de
/// v0.3 tal cual (es el vigente), así que no se detecta antigua; y la cache nueva sigue trayendo
/// `tags` y `files` con las columnas OKF.
#[test]
fn cache_v3_se_reconstruye() {
    // (1) Un proyecto con un `.md` SIN `title:` en el frontmatter (su título derivado sale del H1)
    //     y una cache fabricada a mano con el esquema y la versión de v0.3.
    let dir = tempfile::tempdir().unwrap();
    let raw = "---\ntipo: nota\n---\n\n# Título del H1\n\nCuerpo.\n";
    std::fs::write(dir.path().join("a.md"), raw).unwrap();
    let db_dir = dir.path().join(".lodestar");
    std::fs::create_dir_all(&db_dir).unwrap();
    {
        let conn = rusqlite::Connection::open(db_dir.join("index.db")).unwrap();
        conn.execute_batch(DDL_V03).unwrap();
        // Una fila de la tabla OKF `tags`, para que la cache de v0.3 sea genuina.
        conn.execute("INSERT INTO tags (path, tag) VALUES ('a.md', 'nota')", [])
            .unwrap();
        conn.pragma_update(None, "user_version", USER_VERSION_V03)
            .unwrap();
    }
    assert_eq!(
        user_version_de(dir.path()),
        USER_VERSION_V03,
        "guarda anti-vacuidad: la cache de partida está estampada con la versión de v0.3"
    );

    // (2) Abrir con el código nuevo: sin error y sirviendo el contenido actual de los `.md`.
    let store =
        Store::open_and_build(dir.path()).expect("una cache v0.3 no puede romper la apertura");
    assert!(
        store.documents().unwrap().contains(&rp("a.md")),
        "tras la reconstrucción, la cache debe servir el contenido actual de los `.md`"
    );

    // (3) Se detectó como antigua: quedó estampada con una versión DISTINTA de la de v0.3.
    assert_ne!(
        user_version_de(dir.path()),
        USER_VERSION_V03,
        "la cache de v0.3 debe detectarse antigua (bump de `USER_VERSION`) y reconstruirse"
    );

    // (4) El esquema nuevo tiene la forma de `§20.12`…
    let conn = conexion(dir.path());
    assert!(
        conn.prepare(
            "SELECT path, title, body, raw, frontmatter_json, content_hash FROM documents LIMIT 0"
        )
        .is_ok(),
        "el DDL v2 debe crear `documents(path, title, body, raw, frontmatter_json, content_hash)`"
    );
    // …y NINGUNA columna promovida de OKF ni la tabla `tags` (criterio de salida de la épica: el
    // frontmatter es metadata arbitraria, `tags` es una propiedad como cualquier otra).
    for columna in ["type", "status", "description", "resource"] {
        assert!(
            conn.prepare(&format!("SELECT {columna} FROM documents LIMIT 0"))
                .is_err(),
            "`documents` no debe promover el campo OKF `{columna}` a columna"
        );
    }
    assert!(
        conn.prepare("SELECT tag FROM tags LIMIT 0").is_err(),
        "la tabla `tags` era el índice del campo OKF `tags`: se retira (ahora es metadata)"
    );

    // (5) `title` es el TÍTULO DERIVADO (`§20.4`), no el campo del usuario: este documento no
    //     tiene `title:` en el frontmatter y aun así la columna trae el H1.
    let title: String = conn
        .query_row("SELECT title FROM documents WHERE path = 'a.md'", [], |r| {
            r.get(0)
        })
        .expect("`documents` debe tener la fila del documento indexado");
    let parsed = lodestar_core::model::parse_file("a.md", raw);
    let derivado =
        lodestar_core::model::derived_title(parsed.frontmatter.as_ref(), &parsed.body, &rp("a.md"));
    assert_eq!(
        title, derivado,
        "`documents.title` debe ser el título derivado del core, no el campo `title` del usuario"
    );
    assert_eq!(title, "Título del H1");
}

// --- E18-H02: `links` y `diagnostics` genéricos ------------------------------

/// Una fila de la tabla `links` del store v2 (`§20.12`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct FilaEnlace {
    raw_href: String,
    target_kind: String,
    target_path: Option<String>,
    fragment: Option<String>,
    resolved: i64,
}

/// Filas de `links` de un documento origen, ordenadas por `raw_href`.
fn filas_enlaces(root: &Path, source: &str) -> Vec<FilaEnlace> {
    let conn = conexion(root);
    let mut stmt = conn
        .prepare(
            "SELECT raw_href, target_kind, target_path, fragment, resolved FROM links \
             WHERE source_path = ?1 ORDER BY raw_href",
        )
        .expect(
            "el store v2 debe materializar `links(source_path, raw_href, target_kind, \
             target_path, fragment, resolved)` (`§20.12`, E18-H02)",
        );
    let filas = stmt
        .query_map([source], |r| {
            Ok(FilaEnlace {
                raw_href: r.get(0)?,
                target_kind: r.get(1)?,
                target_path: r.get(2)?,
                fragment: r.get(3)?,
                resolved: r.get(4)?,
            })
        })
        .unwrap();
    filas.map(|f| f.unwrap()).collect()
}

/// El discriminante serde de un [`lodestar_core::types::LinkTarget`] — la etiqueta `kind` que el
/// enum ya define para el wire. Es la fuente de la que sale `links.target_kind`: la cache proyecta
/// la clasificación del core, no inventa un vocabulario propio.
fn kind_de(target: &lodestar_core::types::LinkTarget) -> String {
    serde_json::to_value(target).unwrap()["kind"]
        .as_str()
        .expect("`LinkTarget` va etiquetado con `kind`")
        .to_string()
}

/// Documento con un enlace de **cada** clase de `LinkTarget` (`§20.6`), más los ficheros a los que
/// apuntan. Devuelve el `FileMap` de documentos; el fichero de proyecto se escribe aparte.
fn ws_seis_clases() -> FileMap {
    let mut files = FileMap::new();
    files.insert(
        rp("raiz.md"),
        concat!(
            "# Raíz\n",
            "\n",
            "Documento: [d](docs/auth.md).\n",
            "Fichero del proyecto: [c](src/auth/token_service.rs).\n",
            "Externo: [e](https://example.com/x).\n",
            "Anchor propio: [s](#raiz).\n",
            "Inexistente: [m](no-existe.md).\n",
            "Escape: [x](../../../etc/passwd).\n",
        )
        .into(),
    );
    files.insert(
        rp("docs/auth.md"),
        "# Auth\n\n## La sección\n\nContenido.\n".into(),
    );
    files
}

/// `links_materializa_las_5_clases` — **Dado** un documento con enlaces de los 5 tipos, **Cuando**
/// se indexa, **Entonces** cada uno tiene su `target_kind` y los externos/anchors conservan su
/// `raw_href`.
///
/// Cubre las **seis** variantes de `LinkTarget` (`§20.6` define 6: la historia dice «5 tipos»
/// contando las clases del fixture heredado; cubrirlas todas es un superconjunto estricto).
///
/// **Cierra la asimetría declarada al cerrar E17** (`IMPLEMENTATION_STATUS`, aviso 2): hoy la
/// cache resuelve con `Inventory::default()`, así que TODO destino interno sale `Missing` y el
/// fichero de proyecto ni siquiera se materializa. Con `target_kind` en la tabla eso deja de ser
/// invisible: `document` vs `missing` vs `workspaceFile` son tres filas distintas y el store
/// necesita el inventario real (documentos **y** `other_files`) para escribirlas. Se juzga aquí y
/// no en H04 porque sin inventario el criterio «cada uno tiene su `target_kind`» es inalcanzable:
/// no hay 5 clases que materializar, hay 2.
///
/// Fase ROJA: la tabla `links` no tiene esas columnas (hoy es `(src, dst, href)`) y solo guarda
/// aristas internas.
#[test]
fn links_materializa_las_5_clases() {
    let dir = tempfile::tempdir().unwrap();
    let files = ws_seis_clases();
    write_all(dir.path(), &files);
    // Fichero del proyecto que NO es documento: el destino de un enlace `workspaceFile`.
    std::fs::create_dir_all(dir.path().join("src/auth")).unwrap();
    std::fs::write(
        dir.path().join("src/auth/token_service.rs"),
        "// Destino de un enlace WorkspaceFile.\n",
    )
    .unwrap();

    let store = Store::open_and_build(dir.path()).unwrap();
    assert_eq!(
        sorted(store.documents().unwrap()),
        vec![rp("docs/auth.md"), rp("raiz.md")],
        "guarda anti-vacuidad: el `.rs` no es documento, los dos `.md` sí"
    );

    let filas = filas_enlaces(dir.path(), "raiz.md");

    // (1) Las seis clases, con su `target_kind`, su `target_path` y el `raw_href` intacto (los
    //     externos y los anchors NO tienen path: la columna es NULL, no una cadena vacía).
    let observado: Vec<(&str, &str, Option<&str>)> = filas
        .iter()
        .map(|f| {
            (
                f.raw_href.as_str(),
                f.target_kind.as_str(),
                f.target_path.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        observado,
        vec![
            ("#raiz", "selfAnchor", None),
            ("../../../etc/passwd", "escapesWorkspace", None),
            ("docs/auth.md", "document", Some("docs/auth.md")),
            ("https://example.com/x", "externalUri", None),
            ("no-existe.md", "missing", Some("no-existe.md")),
            (
                "src/auth/token_service.rs",
                "workspaceFile",
                Some("src/auth/token_service.rs")
            ),
        ],
        "cada enlace debe materializarse con su clasificación y su href original"
    );

    // (2) El vocabulario de `target_kind` NO es de la cache: es el discriminante serde del
    //     `LinkTarget` que computa el core con el MISMO inventario (documentos + other_files).
    let ds = DocumentSet::with_other_files(files, [rp("src/auth/token_service.rs")]);
    let salientes = ds
        .analyze()
        .outgoing
        .get(&rp("raiz.md"))
        .expect("`raiz.md` tiene salientes")
        .clone();
    assert_eq!(
        salientes.len(),
        filas.len(),
        "la cache debe materializar TODOS los enlaces del documento, no solo las aristas del grafo"
    );
    for link in &salientes {
        let fila = filas
            .iter()
            .find(|f| f.raw_href == link.href)
            .unwrap_or_else(|| panic!("falta la fila del enlace `{}`", link.href));
        assert_eq!(
            fila.target_kind,
            kind_de(&link.target),
            "`target_kind` de `{}` debe ser el discriminante del `LinkTarget` del core",
            link.href
        );
    }

    // (3) `resolved` en los dos casos en los que la palabra no admite lectura: un destino que
    //     existe está resuelto; un destino que no existe, no. (El resto de clases queda abierto:
    //     no lo fija ningún criterio de la historia.)
    let resolved_de = |href: &str| {
        filas
            .iter()
            .find(|f| f.raw_href == href)
            .unwrap_or_else(|| panic!("falta la fila de `{href}`"))
            .resolved
    };
    assert_eq!(
        resolved_de("docs/auth.md"),
        1,
        "un enlace a un documento que existe está resuelto"
    );
    assert_eq!(
        resolved_de("no-existe.md"),
        0,
        "un enlace a un destino inexistente NO está resuelto"
    );
}

/// `links_separa_el_fragmento` — **Dado** un enlace con fragmento, **Cuando** se indexa,
/// **Entonces** `fragment` está poblado y `target_path` no lo incluye.
///
/// Fase ROJA: la tabla `links` no tiene columnas `fragment`/`target_path`.
#[test]
fn links_separa_el_fragmento() {
    let dir = tempfile::tempdir().unwrap();
    let mut files = FileMap::new();
    files.insert(
        rp("raiz.md"),
        concat!(
            "# Raíz\n",
            "\n",
            "A una sección de otro documento: [f](docs/auth.md#la-seccion).\n",
            "A una sección propia: [s](#raiz).\n",
        )
        .into(),
    );
    files.insert(
        rp("docs/auth.md"),
        "# Auth\n\n## La sección\n\nContenido.\n".into(),
    );
    write_all(dir.path(), &files);
    let store = Store::open_and_build(dir.path()).unwrap();
    assert!(store.documents().unwrap().contains(&rp("raiz.md")));

    let filas = filas_enlaces(dir.path(), "raiz.md");

    // (1) El criterio: el fragmento va en su columna y el destino no lo arrastra…
    let con_fragmento = filas
        .iter()
        .find(|f| f.raw_href == "docs/auth.md#la-seccion")
        .unwrap_or_else(|| panic!("falta la fila del enlace con fragmento; hay: {filas:?}"));
    assert_eq!(con_fragmento.fragment.as_deref(), Some("la-seccion"));
    assert_eq!(
        con_fragmento.target_path.as_deref(),
        Some("docs/auth.md"),
        "`target_path` es el destino normalizado SIN el fragmento"
    );
    // …y el `raw_href` sigue siendo el href original, byte a byte (lo necesita `move_document`).
    assert_eq!(con_fragmento.raw_href, "docs/auth.md#la-seccion");
    assert_eq!(
        con_fragmento.target_kind, "document",
        "el fragmento no cambia la clasificación del destino"
    );

    // (2) Un anchor propio es fragmento puro: sin `target_path`.
    let anchor = filas
        .iter()
        .find(|f| f.raw_href == "#raiz")
        .unwrap_or_else(|| panic!("falta la fila del anchor propio; hay: {filas:?}"));
    assert_eq!(anchor.fragment.as_deref(), Some("raiz"));
    assert_eq!(anchor.target_path, None);
    assert_eq!(anchor.target_kind, "selfAnchor");

    // (3) Ningún destino materializado lleva almohadilla: el fragmento vive SOLO en su columna.
    for f in &filas {
        assert!(
            !f.target_path.as_deref().unwrap_or("").contains('#'),
            "`target_path` no puede incluir el fragmento: {f:?}"
        );
    }
}

/// Una fila de la tabla `diagnostics` del store v2 (`§20.12`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct FilaDiag {
    code: String,
    severity: String,
    message: String,
    range_json: Option<String>,
}

/// Filas de `diagnostics` de un documento, ordenadas por `code`.
fn filas_diagnostics(root: &Path, doc: &str) -> Vec<FilaDiag> {
    let conn = conexion(root);
    let mut stmt = conn
        .prepare(
            "SELECT code, severity, message, range_json FROM diagnostics \
             WHERE document_path = ?1 ORDER BY code",
        )
        .expect(
            "el store v2 debe materializar `diagnostics(document_path, code, severity, message, \
             range_json)` (`§20.12`, E18-H02)",
        );
    let filas = stmt
        .query_map([doc], |r| {
            Ok(FilaDiag {
                code: r.get(0)?,
                severity: r.get(1)?,
                message: r.get(2)?,
                range_json: r.get(3)?,
            })
        })
        .unwrap();
    filas.map(|f| f.unwrap()).collect()
}

/// `diagnostics_conserva_el_rango` — **Dado** un `FM-YAML-INVALID`, **Cuando** se indexa,
/// **Entonces** su `range_json` sobrevive al round-trip.
///
/// El round-trip se juzga contra el `Check` del **core** (autoridad): el `Range` deserializado de
/// la columna debe ser el mismo objeto que el core produce, no un texto parecido. Y un
/// diagnóstico **sin** rango deja la columna a `NULL`, que es lo que distingue «no se conoce la
/// posición» de «la posición es la línea 0».
///
/// Fase ROJA: la tabla `diagnostics` no tiene `range_json` (ni las columnas renombradas
/// `document_path`/`severity`/`message`), así que el rango que el core ya calcula desde E16-H05 se
/// pierde al materializar.
#[test]
fn diagnostics_conserva_el_rango() {
    let dir = tempfile::tempdir().unwrap();
    let mut files = FileMap::new();
    // YAML sintácticamente inválido (una lista sin cerrar) → `FM-YAML-INVALID` con rango.
    files.insert(
        rp("malo.md"),
        "---\ntitulo: [sin cerrar\notro: 1\n---\n\n# Cuerpo\n".into(),
    );
    // Marcadores de merge → `DOC-CONFLICT-MARKER`, que NO lleva rango.
    files.insert(
        rp("conflicto.md"),
        "# Conflicto\n\n<<<<<<< HEAD\nmío\n=======\ntuyo\n>>>>>>> otra\n".into(),
    );
    write_all(dir.path(), &files);
    let store = Store::open_and_build(dir.path()).unwrap();
    assert_eq!(store.documents().unwrap().len(), 2);

    // El veredicto de referencia es el del core (una sola verdad computada).
    let ds = DocumentSet::from_files(files.clone());
    let esperado = ds
        .analyze()
        .diagnostics
        .get(&rp("malo.md"))
        .and_then(|cs| cs.first().cloned())
        .expect("el core debe diagnosticar el frontmatter inválido");
    assert_eq!(esperado.code.as_str(), "FM-YAML-INVALID");
    let rango_core = esperado
        .range
        .expect("`FM-YAML-INVALID` trae rango desde E16-H05");

    let filas = filas_diagnostics(dir.path(), "malo.md");
    assert_eq!(filas.len(), 1, "un solo diagnóstico local: {filas:?}");
    let fila = &filas[0];
    assert_eq!(fila.code, "FM-YAML-INVALID");
    assert_eq!(
        fila.severity,
        serde_json::to_value(esperado.level)
            .unwrap()
            .as_str()
            .unwrap(),
        "`severity` es el valor de wire del `Severity` del core"
    );
    assert_eq!(
        fila.message, esperado.msg,
        "`message` es el mensaje del core"
    );

    // (1) El criterio: el rango sobrevive al round-trip por la columna.
    let texto = fila
        .range_json
        .as_deref()
        .expect("`range_json` debe estar poblado para `FM-YAML-INVALID`");
    let rango: lodestar_core::types::Range = serde_json::from_str(texto)
        .unwrap_or_else(|e| panic!("`range_json` no deserializa a `Range` ({e}): {texto:?}"));
    assert_eq!(
        rango, rango_core,
        "el rango materializado debe ser el que calcula el core"
    );
    // Guarda anti-vacuidad: el rango señala las líneas del bloque de frontmatter, no un 0..0.
    assert_eq!((rango.start_line, rango.end_line), (2, 3));

    // (2) Un diagnóstico sin rango deja la columna a NULL (ausencia, no un rango falso).
    let sin_rango = filas_diagnostics(dir.path(), "conflicto.md");
    assert_eq!(sin_rango.len(), 1, "{sin_rango:?}");
    assert_eq!(sin_rango[0].code, "DOC-CONFLICT-MARKER");
    assert_eq!(
        sin_rango[0].range_json, None,
        "sin rango conocido, `range_json` es NULL"
    );
}

// --- E18-H03: FTS sin campos privilegiados -----------------------------------
//
// El índice de texto deja de depender de `type`/`status`/`tags`/`description`: ha de indexar
// path, título derivado, body y los VALORES TEXTUALES del frontmatter genérico (`§20.12`). Los
// tests apuntan a `fts_candidates` (el índice FTS5 en sí), NO a `search` — `search` ya cubre el
// frontmatter por la vía del core (`query::loose_text_match`), así que no distinguiría un FTS que
// privilegia `description` de uno que indexa metadata genérica. `fts_candidates` sí.
//
// Fase ROJA: hoy el FTS es `files_fts(path, title, description, body)` (index.rs), así que una
// palabra que solo vive en un campo de frontmatter que no sea `description` NO es indexable.

/// `fts_encuentra_valores_de_frontmatter` — **Dado** un documento con `owners: [platform,
/// security]`, **Cuando** se busca «security», **Entonces** aparece.
///
/// La palabra vive **solo** en el valor de un campo de frontmatter arbitrario (ni en el título
/// derivado ni en el body): si el FTS solo indexa `description`, no la encuentra.
///
/// Fase ROJA: `security` no está en ninguna columna FTS actual (`title`/`description`/`body`).
#[test]
fn fts_encuentra_valores_de_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    let raw = "---\nowners: [platform, security]\n---\n\n# Servicio\n\nCuerpo sin la palabra clave.\n";
    std::fs::write(dir.path().join("svc.md"), raw).unwrap();
    let store = Store::open_and_build(dir.path()).unwrap();
    assert!(
        store.documents().unwrap().contains(&rp("svc.md")),
        "guarda anti-vacuidad: el documento está indexado"
    );

    // El FTS (acelerador) debe casar la palabra que vive en el valor de `owners`.
    let hits = store.fts_candidates("security").unwrap();
    assert!(
        hits.contains(&rp("svc.md")),
        "el FTS debe indexar los valores textuales del frontmatter, no solo `description`; \
         hits: {hits:?}"
    );
}

/// `fts_no_privilegia_campos` — **Dado** dos documentos con la misma palabra en el body y en el
/// frontmatter, **Cuando** se buscan, **Entonces** ambos aparecen.
///
/// El body de A y el campo de frontmatter arbitrario de B deben pesar **igual** en el índice: la
/// misma palabra los encuentra a los dos.
///
/// Fase ROJA: A aparece (el body está indexado) pero B no (su palabra vive en `plataforma`, un
/// campo que el FTS actual no indexa) — el índice privilegia `description`/`body` sobre la
/// metadata genérica.
#[test]
fn fts_no_privilegia_campos() {
    let dir = tempfile::tempdir().unwrap();
    // Doc A: la palabra en el BODY.
    std::fs::write(
        dir.path().join("a.md"),
        "---\ntitle: Alfa\n---\n\n# Alfa\n\nDesplegamos sobre kubernetes en el cuerpo.\n",
    )
    .unwrap();
    // Doc B: la MISMA palabra en un valor de frontmatter arbitrario (ni `title` ni `description`).
    std::fs::write(
        dir.path().join("b.md"),
        "---\nplataforma: kubernetes\n---\n\n# Beta\n\nCuerpo sin la palabra clave.\n",
    )
    .unwrap();
    let store = Store::open_and_build(dir.path()).unwrap();
    assert_eq!(
        store.documents().unwrap().len(),
        2,
        "guarda anti-vacuidad: los dos documentos están indexados"
    );

    let hits: BTreeSet<RelPath> = store
        .fts_candidates("kubernetes")
        .unwrap()
        .into_iter()
        .collect();
    // (1) El body de A se encuentra (esto ya vale hoy: no-vacuidad).
    assert!(
        hits.contains(&rp("a.md")),
        "la palabra en el body debe encontrarse; hits: {hits:?}"
    );
    // (2) El frontmatter de B pesa lo mismo: también aparece.
    assert!(
        hits.contains(&rp("b.md")),
        "la misma palabra en un campo de frontmatter arbitrario debe encontrarse igual que en el \
         body; hits: {hits:?}"
    );
}

// ===========================================================================
// E18-H04 — Paridad core ↔ store bajo el modelo nuevo (`§20.12`, `§10` fila 1)
// ===========================================================================
//
// La garantía del invariante #3 sobre el modelo GENÉRICO: la `Analysis` que sirve la cache desde
// su DDL v2 debe coincidir con la que computa el core puro. La comparación NO reusa
// `assert_matches_core` (que construye el core con `DocumentSet::from_files`, SIN `other_files`):
// una paridad honesta del modelo nuevo tiene que declarar los `other_files` en AMBOS lados, o el
// `WorkspaceFile` se degradaría a `Missing` en el core de referencia y la asimetría quedaría
// invisible. Por eso hay un helper nuevo, `assert_paridad_con_inventario`.
//
// COLISIÓN DE NOMBRE (resuelta): ya existe un `property_incremental_igual_core` (arriba) que hace
// 120 ediciones sobre corpus OKF y compara vía `assert_matches_core`. NO se puede duplicar el
// nombre ni editar el existente. El test incremental de H04 lleva otro nombre
// —`paridad_incremental_con_enlaces_clasificados`— y ejercita lo que el existente NO puede: la
// clasificación de enlaces (`WorkspaceFile` a un fichero de proyecto estable) y la metadata
// ANIDADA bajo ediciones incrementales, comparando la `Analysis` completa con `other_files`
// declarados.
//
// STUB de la fase roja: `Store::outgoing_links` (firma + `todo!()`, declarado en `lib.rs`) es la
// pieza que faltaba — hoy el store no expone la clasificación de `Analysis::outgoing` desde su
// tabla `links` materializada.

/// El `target_path` que el store escribe para un [`lodestar_core::types::LinkTarget`] (la columna
/// `links.target_path`): el path sin fragmento de los destinos con path, `None` para los demás. Se
/// **deriva del enum** del core (no se copia un vocabulario paralelo de la cache), igual que
/// `kind_de`.
fn target_path_de(t: &lodestar_core::types::LinkTarget) -> Option<String> {
    use lodestar_core::types::LinkTarget;
    match t {
        LinkTarget::Document(p) | LinkTarget::WorkspaceFile(p) | LinkTarget::Missing(p) => {
            Some(p.as_str().to_string())
        }
        LinkTarget::ExternalUri(_) | LinkTarget::SelfAnchor(_) | LinkTarget::EscapesWorkspace => {
            None
        }
    }
}

/// Compara la `Analysis` **completa** del modelo nuevo entre el store y el core, declarando los
/// mismos `other_files` en ambos lados (la clave de la paridad honesta con `WorkspaceFile`).
///
/// Cubre las seis piezas de `Analysis` (`§20.7`) por la superficie pública del store:
/// - `documents` → `Store::documents`
/// - `diagnostics` (agregados) → `Store::validation_counts` (`hard_fail`/`warn_count`)
/// - `isolated` → `Store::isolated`
/// - `dangling` → `Store::dangling` (destinos markdown ausentes; `§20.7`)
/// - `incoming` → `Store::backlinks` (orígenes distintos)
/// - `outgoing` **con su clasificación de `target_kind`** → `Store::outgoing_links` (STUB)
fn assert_paridad_con_inventario(store: &Store, files: &FileMap, other_files: &[RelPath]) {
    let ds = DocumentSet::with_other_files(files.clone(), other_files.iter().cloned());
    let a = ds.analyze();

    // diagnostics (agregados): gana el core.
    let (hf, wc) = store.validation_counts().unwrap();
    assert_eq!(hf, a.hard_fail(), "hard_fail difiere (gana el core)");
    assert_eq!(wc, a.warn_count(), "warn_count difiere (gana el core)");

    // documentos.
    assert_eq!(
        sorted(store.documents().unwrap()),
        sorted(a.documents.clone()),
        "el inventario de documentos difiere"
    );

    // aislados.
    assert_eq!(
        sorted(store.isolated().unwrap()),
        sorted(a.isolated.clone()),
        "isolated difiere"
    );

    // colgantes: la síntesis SQL devuelve los destinos markdown ausentes (`is_edge = 1`, fantasmas
    // del grafo). Se comparan contra los destinos `Missing` del core que serían documentos.
    let mut destinos_colgantes: Vec<RelPath> = a
        .dangling
        .iter()
        .filter(|d| d.target.is_markdown())
        .map(|d| d.target.clone())
        .collect();
    destinos_colgantes.sort();
    destinos_colgantes.dedup();
    assert_eq!(
        sorted(store.dangling().unwrap()),
        destinos_colgantes,
        "dangling difiere"
    );

    for p in &a.documents {
        // incoming: orígenes distintos (el store sintetiza `SELECT DISTINCT source_path`).
        let mut expected: Vec<RelPath> = a
            .incoming
            .get(p)
            .into_iter()
            .flatten()
            .map(|r| r.from.clone())
            .collect();
        expected.sort();
        expected.dedup();
        assert_eq!(
            sorted(store.backlinks(p).unwrap()),
            expected,
            "backlinks de {p} difieren"
        );

        // outgoing CON su clasificación: `(href, target_kind, target_path, fragment)`. El
        // `target_kind` sale del discriminante serde del `LinkTarget` del core (`kind_de`), no de
        // un vocabulario propio de la cache.
        let mut core_out: Vec<(String, String, Option<String>, Option<String>)> = a
            .outgoing
            .get(p)
            .into_iter()
            .flatten()
            .map(|l| {
                (
                    l.href.clone(),
                    kind_de(&l.target),
                    target_path_de(&l.target),
                    l.fragment.clone(),
                )
            })
            .collect();
        core_out.sort();
        let mut store_out = store.outgoing_links(p).unwrap();
        store_out.sort();
        assert_eq!(
            store_out, core_out,
            "los enlaces salientes clasificados de {p} difieren (gana el core)"
        );
    }
}

/// `paridad_con_edge_cases` — **Dado** el fixture `with_edge_cases()` (5 clases de enlace, mismo
/// basename, capitalización), **Cuando** se compara core vs store, **Entonces** las dos `Analysis`
/// son idénticas.
///
/// Es el test que cierra el invariante #3 sobre el modelo genérico: `with_edge_cases()` tiene las
/// cinco clases de destino, dos documentos con el **mismo basename** en árboles distintos y un
/// enlace con **capitalización errónea**, más un enlace a un fichero de proyecto (`.rs`) que solo
/// es `WorkspaceFile` si el store declara `other_files` como el core. La paridad se juzga con
/// `DocumentSet::with_other_files` en el lado del core (declarando el `.rs`) contra la `Analysis`
/// que el store sirve desde su DDL nuevo.
///
/// Fase ROJA: el store no expone hoy la clasificación de `Analysis::outgoing` (`Store::outgoing_links`
/// es un stub), y la síntesis de diagnósticos de enlace (`synth::link_diagnostics`) reconstruye el
/// `DocumentSet` **sin** `other_files`, así que `warn_count` diverge (el `.rs` cae como
/// `LINK-TARGET-MISSING` en la cache y como `WorkspaceFile` silencioso en el core).
#[test]
fn paridad_con_edge_cases() {
    let dir = tempfile::tempdir().unwrap();
    let files = lodestar_fixtures::with_edge_cases();
    lodestar_fixtures::materialize(&files, dir.path()).unwrap();
    // El fichero de proyecto (no-`.md`) al que apunta uno de los enlaces de `raiz.md`: es lo que
    // hace `WorkspaceFile` (y no `Missing`) a ese enlace. El store lo recoge en `other_files` al
    // recorrer el disco; el core lo recibe declarado abajo.
    let rs = rp("src/auth/token_service.rs");
    std::fs::create_dir_all(dir.path().join("src/auth")).unwrap();
    std::fs::write(
        dir.path().join(rs.as_str()),
        "// Destino de un enlace WorkspaceFile.\n",
    )
    .unwrap();

    let store = Store::open_and_build(dir.path()).unwrap();
    assert_eq!(
        store.documents().unwrap().len(),
        5,
        "guarda anti-vacuidad: los 5 `.md` del fixture son documentos; el `.rs` no"
    );

    assert_paridad_con_inventario(&store, &files, &[rs]);
}

/// `paridad_incremental_con_enlaces_clasificados` — **Dado** N ediciones aleatorias deterministas
/// sobre documentos con metadata **anidada** y enlaces a un fichero de proyecto **estable**,
/// **Cuando** se aplican incrementalmente, **Entonces** el store coincide con el core en cada paso.
///
/// Es el criterio incremental de H04 con OTRO nombre (el `property_incremental_igual_core` de
/// arriba no se puede duplicar ni editar) y ejercitando lo que aquel no cubre: la clasificación de
/// enlaces (`WorkspaceFile`) y la metadata anidada bajo `upsert`/`remove` incrementales. Los
/// destinos de los enlaces son **estables** a propósito (un `.rs` de `other_files` que nunca se
/// borra, un `index.md` que siempre existe, un `no-existe.md` que nunca existe, un externo y un
/// anchor): así el `target_kind` materializado nunca queda obsoleto por la limitación de cascada
/// de los `upsert` incrementales, y la única divergencia posible es de modelo, no de refresco.
///
/// Fase ROJA: misma que `paridad_con_edge_cases` — falta `Store::outgoing_links` y la síntesis de
/// diagnósticos ignora `other_files` (el enlace al `.rs` infla `warn_count` en la cache).
#[test]
fn paridad_incremental_con_enlaces_clasificados() {
    // LCG determinista (semilla distinta de la del property test existente).
    let mut seed: u64 = 0xD1B54A32D192ED03;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };

    let dir = tempfile::tempdir().unwrap();
    // Fichero de proyecto ESTABLE (no-`.md`): destino `WorkspaceFile` que nunca cambia.
    let rs = rp("src/lib.rs");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join(rs.as_str()), "// código estable\n").unwrap();
    // Documento raíz ESTABLE (nunca se churnea): destino `Document` siempre presente, para que los
    // enlaces `/index.md` no oscilen entre `document` y `missing`.
    let index_raw = "---\nkind: index\n---\n\n# Índice\n";
    std::fs::write(dir.path().join("index.md"), index_raw).unwrap();

    let store = Store::open_and_build(dir.path()).unwrap();
    let mut mirror: FileMap = FileMap::new();
    mirror.insert(rp("index.md"), index_raw.to_string());

    // Documentos que se churnean (crean/modifican/borran). NO se enlazan entre sí: así ningún
    // `target_kind` queda obsoleto por la ausencia de cascada.
    let churn = ["a.md", "b.md", "docs/c.md"];

    for _ in 0..80 {
        let name = churn[next() % churn.len()];
        let p = rp(name);
        let op = next() % 3;
        if op == 0 {
            mirror.remove(&p);
            store.remove(&p).unwrap();
        } else {
            let tier = ["critical", "normal", "low"][next() % 3];
            // Frontmatter con metadata ANIDADA (`service.name`/`service.tier`) y una lista, y
            // cuerpo con enlaces a destinos estables de cada clase.
            let raw = format!(
                "---\nservice:\n  name: auth\n  tier: {tier}\nowners: [platform, security]\n---\n\n\
                 # H\n\n\
                 A la raíz: [i](/index.md).\n\
                 A código del proyecto: [c](/src/lib.rs).\n\
                 Externo: [e](https://example.com).\n\
                 Anchor propio: [a](#h).\n\
                 Inexistente: [m](/no-existe.md).\n"
            );
            mirror.insert(p.clone(), raw.clone());
            store.upsert(&p, &raw, 0, 0).unwrap();
        }

        // (1) La `Analysis` completa coincide, con `other_files` declarados en ambos lados.
        assert_paridad_con_inventario(&store, &mirror, &[rs.clone()]);

        // (2) La metadata anidada materializada sigue siendo la del core (`walk`) tras cada
        //     edición: ni una fila inventada ni un valor coercionado.
        for (path, raw) in &mirror {
            let filas = filas_metadata(dir.path(), path.as_str());
            assert_metadata_coincide_con_el_core(raw, &filas);
        }
    }
}
