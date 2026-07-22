//! Tests de integración de `lodestar-workspace` (E5): único escritor, commit con checkpoint, restore.

use lodestar_core::types::{Author, FrontmatterPatch, RelPath};
use lodestar_workspace::Workspace;

fn setup() -> (tempfile::TempDir, Workspace) {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Workspace::open(dir.path()).unwrap();
    ws.set_identity(Author {
        name: "Test".into(),
        email: "t@e.com".into(),
    });
    ws.init_vcs().unwrap();
    (dir, ws)
}

#[test]
fn crea_concept_y_lo_escribe_por_el_unico_escritor() {
    let (dir, ws) = setup();
    let p = RelPath::new("alfa.md").unwrap();
    let outcome = ws
        .create_concept(&p, "Nota", Some("Alfa"), "# H\n\ncuerpo\n", false)
        .unwrap();
    assert!(outcome.written);
    assert!(dir.path().join("alfa.md").is_file());
    // la workspace inyecta el timestamp de creación (paridad prototipo): el .md nace con él,
    // en formato ISO-8601 con precisión de segundos y sin generar warn FMT-TS.
    let escrito = std::fs::read_to_string(dir.path().join("alfa.md")).unwrap();
    assert!(
        escrito.contains("timestamp: "),
        "el .md creado no lleva timestamp: {escrito}"
    );
    let snap = ws.snapshot().unwrap();
    // el snapshot lo refleja
    assert!(snap
        .analysis
        .concepts
        .iter()
        .any(|c| c.as_str() == "alfa.md"));
    // ninguna página creada debe nacer con un warn de timestamp mal formado.
    assert!(
        !snap
            .analysis
            .per_file
            .get(&p)
            .map(|checks| checks.iter().any(|c| c.code.as_str() == "FMT-TS"))
            .unwrap_or(false),
        "la página creada dispara FMT-TS"
    );
}

#[test]
fn create_concept_no_conforme_no_escribe() {
    let (dir, ws) = setup();
    let p = RelPath::new("malo.md").unwrap();
    let outcome = ws
        .create_concept(&p, "", Some("Malo"), "# H\n", false)
        .unwrap();
    assert!(!outcome.written);
    assert!(outcome.rejected.is_some());
    assert!(!dir.path().join("malo.md").exists());
}

#[test]
fn merge_frontmatter_null_borra_y_escribe() {
    let (_dir, ws) = setup();
    let p = RelPath::new("x.md").unwrap();
    ws.create_concept(&p, "Nota", Some("X"), "# H\n", false)
        .unwrap();
    let mut patch = std::collections::BTreeMap::new();
    patch.insert("status".to_string(), None);
    patch.insert(
        "description".to_string(),
        Some(serde_yaml::Value::String("nueva".into())),
    );
    let outcome = ws.merge_frontmatter(&p, FrontmatterPatch(patch)).unwrap();
    assert!(outcome.raw.contains("description: nueva"));
}

#[test]
fn commit_devuelve_conformidad_post_commit() {
    let (_dir, ws) = setup();
    let p = RelPath::new("ok.md").unwrap();
    ws.create_concept(&p, "Nota", Some("Ok"), "# H\n\ncuerpo\n", false)
        .unwrap();
    let outcome = ws.commit("Añade Ok").unwrap();
    assert!(outcome.conformance.conform);
    // el log tiene el commit inicial + este
    assert!(ws.vcs_log(10).unwrap().len() >= 2);
}

#[test]
fn restore_hace_checkpoint_y_no_pierde_trabajo() {
    let (dir, ws) = setup();
    // commit 1: crea alfa
    let alfa = RelPath::new("alfa.md").unwrap();
    ws.create_concept(&alfa, "Nota", Some("Alfa"), "# H\n", false)
        .unwrap();
    let c1 = ws.commit("c1").unwrap();
    // cambios sin commitear: crea beta
    let beta = RelPath::new("beta.md").unwrap();
    ws.create_concept(&beta, "Nota", Some("Beta"), "# H\n", false)
        .unwrap();
    assert!(dir.path().join("beta.md").is_file());
    // restore al commit 1 → checkpoint automático preserva beta en el historial
    ws.restore(&c1.sha).unwrap();
    // beta ya no está en el working tree (restaurado a c1)...
    assert!(!dir.path().join("beta.md").exists());
    // ...pero el checkpoint lo dejó en el historial (no se perdió el trabajo).
    let log = ws.vcs_log(20).unwrap();
    assert!(log.iter().any(|c| c.message.contains("Checkpoint")));
}

#[test]
fn generate_index_aplica_por_el_unico_escritor() {
    let (dir, ws) = setup();
    let p = RelPath::new("alfa.md").unwrap();
    ws.create_concept(&p, "Concept", Some("Alfa"), "# H\n", false)
        .unwrap();
    let report = ws.generate_index("").unwrap();
    assert!(report.written >= 1);
    assert!(dir.path().join("index.md").is_file());
    // segunda vez: sin cambios.
    let report2 = ws.generate_index("").unwrap();
    assert_eq!(report2.written, 0);
}

#[test]
fn open_live_emite_evento_y_acelera_lecturas() {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Workspace::open(dir.path()).unwrap();
    ws.set_identity(Author {
        name: "Test".into(),
        email: "t@e.com".into(),
    });
    ws.init_vcs().unwrap();
    ws.enable_cache().unwrap();
    let rx = ws.subscribe().unwrap();

    // Escribir por el único escritor dispara el update optimista de la cache → IndexEvent.
    let p = RelPath::new("alfa.md").unwrap();
    ws.create_concept(&p, "Nota", Some("Alfa"), "# H\n\n[b](/beta.md)\n", false)
        .unwrap();
    let ev = rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("debe llegar un IndexEvent");
    assert!(ev.changed.contains(&p));

    // La cache responde consultas aceleradas coherentes con el core.
    let cache = ws.cache().unwrap();
    assert!(cache
        .dangling()
        .unwrap()
        .iter()
        .any(|d| d.as_str() == "beta.md"));
    assert!(cache.orphans().unwrap().contains(&p));
}

#[test]
fn switch_de_rama_por_el_unico_escritor() {
    let (dir, ws) = setup();
    let alfa = RelPath::new("alfa.md").unwrap();
    ws.create_concept(&alfa, "Nota", Some("Alfa"), "# H\n", false)
        .unwrap();
    ws.commit("main: alfa").unwrap();
    // rama nueva con un fichero extra
    ws.create_branch("feature", None).unwrap();
    ws.switch("feature").unwrap();
    let beta = RelPath::new("beta.md").unwrap();
    ws.create_concept(&beta, "Nota", Some("Beta"), "# H\n", false)
        .unwrap();
    ws.commit("feature: beta").unwrap();
    assert!(dir.path().join("beta.md").is_file());
    // volver a main → beta desaparece del working tree (aplicado por el único escritor)
    ws.switch("master").or_else(|_| ws.switch("main")).unwrap();
    assert!(!dir.path().join("beta.md").exists());
    assert!(dir.path().join("alfa.md").is_file());
}

#[test]
fn merge_fast_forward_por_workspace() {
    let (dir, ws) = setup();
    let alfa = RelPath::new("alfa.md").unwrap();
    ws.create_concept(&alfa, "Nota", Some("Alfa"), "# H\n", false)
        .unwrap();
    ws.commit("base").unwrap();
    let base_branch = ws
        .branches()
        .unwrap()
        .into_iter()
        .find(|b| b.is_head)
        .unwrap()
        .name;
    ws.create_branch("feature", None).unwrap();
    ws.switch("feature").unwrap();
    let beta = RelPath::new("beta.md").unwrap();
    ws.create_concept(&beta, "Nota", Some("Beta"), "# H\n", false)
        .unwrap();
    ws.commit("feature: beta").unwrap();
    ws.switch(&base_branch).unwrap();
    // Merge limpio (sin conflictos): beta se integra en la rama base. Puede ser ff o merge de
    // 3-vías según si la regeneración de index/tags dejó el árbol dirty (la ff pura se testea en vcs).
    let report = ws.merge("feature").unwrap();
    assert!(report.conflicted.is_empty());
    assert!(dir.path().join("beta.md").is_file());
}

#[test]
fn config_strictness_bloquea_avisos() {
    use lodestar_workspace::Config;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lodestar.toml"),
        "[gate]\nblock_warnings = true\n\n[identity]\nname = \"Ana\"\nemail = \"ana@x.com\"\n",
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert!(cfg.gate.block_warnings);
    assert_eq!(cfg.author().unwrap().name, "Ana");
    // un análisis con warns pero sin errores: bloquea solo si block_warnings.
    let analysis = lodestar_core::types::Analysis {
        warn_count: 2,
        ..Default::default()
    };
    assert!(cfg.gate_blocked(&analysis));
    assert!(!Config::default().gate_blocked(&analysis));
}

#[test]
fn conformance_cache_por_tree_oid() {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Workspace::open(dir.path()).unwrap();
    ws.set_identity(Author {
        name: "Test".into(),
        email: "t@e.com".into(),
    });
    ws.init_vcs().unwrap();
    ws.enable_cache().unwrap();
    let p = RelPath::new("ok.md").unwrap();
    ws.create_concept(&p, "Nota", Some("Ok"), "# H\n\ncuerpo\n", false)
        .unwrap();
    let c = ws.commit("añade ok").unwrap();
    // primera lectura computa y cachea; segunda debe salir de la cache (mismo resultado).
    let a = ws.conformance_of(&c.sha).unwrap();
    let b = ws.conformance_of(&c.sha).unwrap();
    assert_eq!(a, b);
    assert!(a.conform);
}

#[test]
fn diff_working_vs_head() {
    let (_dir, ws) = setup();
    let p = RelPath::new("alfa.md").unwrap();
    ws.create_concept(&p, "Nota", Some("Alfa"), "# H\n", false)
        .unwrap();
    ws.commit("c1").unwrap();
    // edita sin commitear
    ws.merge_frontmatter(&p, {
        let mut m = std::collections::BTreeMap::new();
        m.insert(
            "status".to_string(),
            Some(serde_yaml::Value::String("review".into())),
        );
        FrontmatterPatch(m)
    })
    .unwrap();
    let diff = ws.diff_working().unwrap();
    assert!(diff
        .status_changes
        .iter()
        .any(|s| s.to.as_deref() == Some("review")));
}

// --- Regresiones de la revisión profunda: ciclo de merge y commits -----------

#[test]
fn merge_tres_vias_limpio_se_concluye_con_dos_padres() {
    // base → dos ramas con ficheros DISJUNTOS pero divergentes (no ff) → merge → commit.
    // Antes: MERGE_HEAD quedaba para siempre (RepoBusy eterno) y el commit tenía 1 padre.
    let (_dir, ws) = setup();
    let alfa = RelPath::new("alfa.md").unwrap();
    ws.create_concept(&alfa, "Nota", Some("Alfa"), "# H\n", false)
        .unwrap();
    ws.commit("base").unwrap();
    let base_branch = ws
        .branches()
        .unwrap()
        .into_iter()
        .find(|b| b.is_head)
        .unwrap()
        .name;
    ws.create_branch("feature", None).unwrap();
    ws.switch("feature").unwrap();
    let beta = RelPath::new("beta.md").unwrap();
    ws.create_concept(&beta, "Nota", Some("Beta"), "# H\n", false)
        .unwrap();
    ws.commit("feature: beta").unwrap();
    ws.switch(&base_branch).unwrap();
    let gamma = RelPath::new("gamma.md").unwrap();
    ws.create_concept(&gamma, "Nota", Some("Gamma"), "# H\n", false)
        .unwrap();
    ws.commit("base: gamma").unwrap();

    let report = ws.merge("feature").unwrap();
    assert!(report.conflicted.is_empty());
    assert!(!report.fast_forward);
    // Concluir el merge NO puede estar bloqueado (RepoBusy) — es como funciona git.
    let outcome = ws.commit("merge feature").unwrap();
    let head = &ws.vcs_log(1).unwrap()[0];
    assert_eq!(head.parents.len(), 2, "el commit de merge ratifica §13.6.3");
    assert_eq!(head.id, outcome.sha);
    // Y el estado del repo queda limpio: el siguiente commit normal funciona.
    let delta = RelPath::new("delta.md").unwrap();
    ws.create_concept(&delta, "Nota", Some("Delta"), "# H\n", false)
        .unwrap();
    ws.commit("post-merge").unwrap();
}

#[test]
fn merge_con_conflicto_se_resuelve_y_concluye() {
    let (dir, ws) = setup();
    let f = RelPath::new("f.md").unwrap();
    ws.create_concept(&f, "Nota", Some("F"), "# H\n\nbase\n", false)
        .unwrap();
    ws.commit("base").unwrap();
    let base_branch = ws
        .branches()
        .unwrap()
        .into_iter()
        .find(|b| b.is_head)
        .unwrap()
        .name;
    ws.create_branch("feature", None).unwrap();
    ws.switch("feature").unwrap();
    ws.write_concept(
        &f,
        "---\ntype: Nota\ntitle: F\ndescription: d\n---\n\n# H\n\nfeature\n",
        true,
    )
    .unwrap();
    ws.commit("feature").unwrap();
    ws.switch(&base_branch).unwrap();
    ws.write_concept(
        &f,
        "---\ntype: Nota\ntitle: F\ndescription: d\n---\n\n# H\n\nmain\n",
        true,
    )
    .unwrap();
    ws.commit("main").unwrap();

    let report = ws.merge("feature").unwrap();
    assert!(
        report.conflicted.contains(&f),
        "debe conflictar: {report:?}"
    );
    let raw = std::fs::read_to_string(dir.path().join("f.md")).unwrap();
    assert!(raw.contains("<<<<<<<"), "marcadores para OKF-CONFLICT");
    // Resuelve y concluye: el commit lleva 2 padres y el repo queda limpio.
    ws.write_concept(
        &f,
        "---\ntype: Nota\ntitle: F\ndescription: d\n---\n\n# H\n\nresuelto\n",
        true,
    )
    .unwrap();
    ws.commit("resuelve el merge").unwrap();
    assert_eq!(ws.vcs_log(1).unwrap()[0].parents.len(), 2);
    let delta = RelPath::new("post.md").unwrap();
    ws.create_concept(&delta, "Nota", Some("Post"), "# H\n", false)
        .unwrap();
    ws.commit("post").unwrap(); // ya sin estado Merging
}

#[test]
fn switch_no_deja_suciedad_fantasma_ni_checkpoints_vacios() {
    let (_dir, ws) = setup();
    let alfa = RelPath::new("alfa.md").unwrap();
    ws.create_concept(&alfa, "Nota", Some("Alfa"), "# H\n", false)
        .unwrap();
    ws.commit("base").unwrap();
    let base_branch = ws
        .branches()
        .unwrap()
        .into_iter()
        .find(|b| b.is_head)
        .unwrap()
        .name;
    ws.create_branch("feature", None).unwrap();
    ws.switch("feature").unwrap();
    let beta = RelPath::new("beta.md").unwrap();
    ws.create_concept(&beta, "Nota", Some("Beta"), "# H\n", false)
        .unwrap();
    ws.commit("feature: beta").unwrap();
    let n_before = ws.vcs_log(50).unwrap().len();
    // Ida y vuelta sin tocar nada, terminando en la MISMA rama: NO deben aparecer commits
    // nuevos (checkpoints espurios por el index desincronizado tras el switch).
    ws.switch(&base_branch).unwrap();
    ws.switch("feature").unwrap();
    ws.switch(&base_branch).unwrap();
    ws.switch("feature").unwrap();
    let log = ws.vcs_log(50).unwrap();
    assert_eq!(
        log.len(),
        n_before,
        "checkpoints espurios: {:?}",
        log.iter().map(|c| c.message.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn init_bundle_scaffold_e_idempotente() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("nuevo");
    // Primer arranque: crea directorio + index raíz + git con commit inicial.
    let ws = Workspace::init_bundle(&root).unwrap();
    assert!(root.join("index.md").is_file());
    assert!(root.join(".git").is_dir());
    assert!(ws.has_vcs());
    assert!(!ws.vcs_log(5).unwrap().is_empty(), "commit inicial");
    // Idempotente: sobre un bundle existente no duplica ni rompe nada.
    let n = ws.vcs_log(10).unwrap().len();
    let ws2 = Workspace::init_bundle(&root).unwrap();
    assert_eq!(ws2.vcs_log(10).unwrap().len(), n);
    // Y el bundle recién creado es conforme (abrible por open_bundle del escritorio).
    assert_eq!(ws2.analyze().unwrap().hard_fail, 0);
}

#[test]
fn escritorio_crear_workspace_con_cache_vieja_funciona() {
    // Regresión e2e del flujo del escritorio: un build antiguo dejó en `.lodestar/index.db`
    // un esquema viejo (tabla `files` SIN la columna `hash`) pero con `user_version=1`. Como
    // `create_schema` es `IF NOT EXISTS`, la tabla vieja sobrevivía y al abrir/crear el
    // workspace la app reventaba con «error de la cache: sqlite: table files has no column
    // named hash». Ahora el store detecta el drift y reconstruye el esquema limpio.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // (1) Cache vieja fabricada a mano, ANTES de cualquier arranque.
    let db_dir = root.join(".lodestar");
    std::fs::create_dir_all(&db_dir).unwrap();
    {
        let conn = rusqlite::Connection::open(db_dir.join("index.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE files (path TEXT PRIMARY KEY, kind TEXT NOT NULL);
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }

    // (2) Scaffold del bundle (first-run del escritorio). Idempotente.
    Workspace::init_bundle(root).unwrap();
    assert!(root.join("index.md").is_file());

    // (3) Apertura en vivo: cache incremental + watcher (lo que hace la app al abrir).
    let ws = Workspace::open_live(root).unwrap();

    // (4) El snapshot funciona pese a la cache vieja.
    let snap = ws.snapshot().unwrap();
    assert!(snap.files.keys().any(|p| p.as_str() == "index.md"));

    // (5) Crear un concept nuevo funciona (este era el flujo que reventaba).
    let p = RelPath::new("nuevo.md").unwrap();
    let outcome = ws
        .create_concept(&p, "Nota", Some("Nuevo"), "# H\n\ncuerpo\n", false)
        .unwrap();
    assert!(outcome.written);
    assert!(root.join("nuevo.md").is_file());
}

// ---------------------------------------------------------------------------
// E9-H05 — Config nueva `.lodestar/config.yaml` (`WorkspaceConfig::load`).
//
// Fase ROJA: `WorkspaceConfig` y su loader YAML todavía NO existen en producción.
// Estos tests fijan el contrato objetivo (ARCHITECTURE.md §19.4, DECISIONES D4/D5):
//   WorkspaceConfig { workspace: { writable_roots: Vec<RelPath>, reference_roots: Vec<RelPath>,
//                                  ignored: Vec<String> }, gate, transactions }
// cargado con `WorkspaceConfig::load(root)` desde `.lodestar/config.yaml` (YAML, claves camelCase).
// Los defaults son seguros: un bundle sin `config.yaml` NO es error.
// ---------------------------------------------------------------------------

/// Escribe `<root>/.lodestar/config.yaml` con el contenido dado (crea `.lodestar/` si falta).
fn escribe_config_yaml(root: &std::path::Path, contenido: &str) {
    let dir = root.join(".lodestar");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.yaml"), contenido).unwrap();
}

/// Criterio: `.lodestar/config.yaml` con `writableRoots: [knowledge]` →
/// `writable_roots == [RelPath("knowledge")]`.
#[test]
fn carga_writable_roots() {
    use lodestar_workspace::WorkspaceConfig;
    let dir = tempfile::tempdir().unwrap();
    escribe_config_yaml(dir.path(), "workspace:\n  writableRoots: [knowledge]\n");

    let cfg = WorkspaceConfig::load(dir.path()).unwrap();

    assert_eq!(
        cfg.workspace.writable_roots,
        vec![RelPath::new("knowledge").unwrap()],
        "writableRoots del YAML debe deserializarse a Vec<RelPath> validado"
    );
}

/// Criterio: bundle SIN `config.yaml` → defaults seguros (NO error) y `ignored` contiene
/// `.lodestar/runtime`. (No aseveramos el valor exacto de `writable_roots` por defecto: la
/// representación del root "todo el bundle" es una decisión de diseño del implementador —
/// `RelPath::new(".")` es inválido — y no debemos cerrarla desde el test.)
#[test]
fn defaults_sin_config() {
    use lodestar_workspace::WorkspaceConfig;
    let dir = tempfile::tempdir().unwrap();
    // Deliberadamente NO escribimos `.lodestar/config.yaml`.

    let cfg = WorkspaceConfig::load(dir.path())
        .expect("un bundle sin config.yaml debe cargar defaults seguros, no fallar");

    assert!(
        cfg.workspace
            .ignored
            .iter()
            .any(|p| p == ".lodestar/runtime"),
        "los defaults de `ignored` deben incluir siempre `.lodestar/runtime`; eran: {:?}",
        cfg.workspace.ignored
    );
}

/// Criterio: `writableRoots: [../escape]` → error de validación (`RelPath` rechaza `..`).
/// La carga NO debe silenciar el traversal ni caer a defaults: debe ser `Err`.
#[test]
fn roots_rechazan_traversal() {
    use lodestar_workspace::WorkspaceConfig;
    let dir = tempfile::tempdir().unwrap();
    escribe_config_yaml(dir.path(), "workspace:\n  writableRoots: [\"../escape\"]\n");

    let res = WorkspaceConfig::load(dir.path());

    assert!(
        res.is_err(),
        "un writableRoot con `..` debe ser rechazado por RelPath y propagarse como error, \
         no devolver una config; se obtuvo: {res:?}"
    );
}

/// Criterio: `config.yaml` malformado → error explícito (NO defaults silenciosos).
#[test]
fn config_malformada_es_error() {
    use lodestar_workspace::WorkspaceConfig;
    let dir = tempfile::tempdir().unwrap();
    // Secuencia de flujo YAML sin cerrar: parseo inválido garantizado.
    escribe_config_yaml(dir.path(), "workspace:\n  writableRoots: [knowledge\n");

    let res = WorkspaceConfig::load(dir.path());

    assert!(
        res.is_err(),
        "un config.yaml malformado debe ser error explícito, nunca caer a defaults silenciosos; \
         se obtuvo: {res:?}"
    );
}

/// Criterio EXTRA (cierra reserva de E9-H05): el usuario especifica su propia lista `ignored`
/// y los obligatorios `.lodestar/runtime` y `.git` SIGUEN en el conjunto efectivo (no se pierden
/// por el reemplazo de serde). La garantía es "los obligatorios se inyectan siempre al consumir
/// `ignored`" (config.rs: «Los defaults **siempre** incluyen `.lodestar/runtime` y `.git`, se
/// especifique o no `ignored` en el YAML»).
///
/// Fase ROJA: hoy `#[serde(default)]` sobre `ignored: Vec<String>` REEMPLAZA la lista completa
/// cuando el YAML la trae, así que `[node_modules]` borra los obligatorios. El implementador debe
/// inyectarlos al cargar/consumir (o exponer un `effective_ignored()`); este test asevera el
/// conjunto efectivo servido en `cfg.workspace.ignored`, coherente con `defaults_sin_config`.
#[test]
fn ignored_conserva_obligatorios() {
    use lodestar_workspace::WorkspaceConfig;
    let dir = tempfile::tempdir().unwrap();
    // El usuario declara SOLO `node_modules`: no menciona los obligatorios.
    escribe_config_yaml(dir.path(), "workspace:\n  ignored: [node_modules]\n");

    let cfg = WorkspaceConfig::load(dir.path()).unwrap();
    let ig = &cfg.workspace.ignored;

    assert!(
        ig.iter().any(|s| s == "node_modules"),
        "la lista del usuario debe conservarse; eran: {ig:?}"
    );
    assert!(
        ig.iter().any(|s| s == ".lodestar/runtime"),
        "obligatorio `.lodestar/runtime` perdido al especificar `ignored`; eran: {ig:?}"
    );
    assert!(
        ig.iter().any(|s| s == ".git"),
        "obligatorio `.git` perdido al especificar `ignored`; eran: {ig:?}"
    );
}

// ---------------------------------------------------------------------------
// E9-H06 — Separación canónico vs runtime (`.lodestar/runtime/` + gitignore ajustado).
//
// Fase ROJA (ARCHITECTURE.md §19.4, REFACTOR §4.1/§14, DECISIONES D5):
//   - El `.gitignore` deja de ignorar `.lodestar/` ENTERO: ignora solo `index.db` + `runtime/`,
//     dejando `config.yaml`/`schema.yaml`/`templates/` VERSIONADOS.
//   - El watcher/carga NO indexan `.lodestar/runtime/` (desechable), sí los canónicos.
//   - En un repo ya adoptado (con `.lodestar/` trackeado entero) la apertura ajusta el
//     `.gitignore` de forma idempotente.
// La escritura del `.gitignore` pasa a hacerse como texto plano desde `lodestar-workspace`
// (sin git2; `vcs` queda dormido, §19.2).
// ---------------------------------------------------------------------------

/// Construye un matcher de gitignore a partir del `<root>/.gitignore` real.
fn gitignore_de(root: &std::path::Path) -> ignore::gitignore::Gitignore {
    let (gi, err) = ignore::gitignore::Gitignore::new(root.join(".gitignore"));
    assert!(err.is_none(), "gitignore ilegible: {err:?}");
    gi
}

/// `true` si `rel` (relativa al root) queda ignorada por el `.gitignore` dado, considerando
/// también sus directorios padre (semántica real de git).
fn esta_ignorado(gi: &ignore::gitignore::Gitignore, rel: &str, es_dir: bool) -> bool {
    gi.matched_path_or_any_parents(rel, es_dir).is_ignore()
}

/// Criterio: bundle recién abierto → el `.gitignore` ignora `.lodestar/index.db` y
/// `.lodestar/runtime/` pero **no** `.lodestar/config.yaml`.
///
/// Fase ROJA: hoy `Vcs::init` escribe `/.lodestar/\n*.db…`, que ignora `.lodestar/` ENTERO
/// (incluido `config.yaml`) → la aserción "config.yaml NO ignorado" falla.
#[test]
fn gitignore_parte_lodestar() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bundle");
    // Scaffold de un bundle nuevo (la ruta canónica de "recién abierto/creado").
    Workspace::init_bundle(&root).unwrap();

    let gi = gitignore_de(&root);

    assert!(
        esta_ignorado(&gi, ".lodestar/index.db", false),
        "el `.gitignore` debe ignorar la cache `.lodestar/index.db`"
    );
    assert!(
        esta_ignorado(&gi, ".lodestar/runtime/plans/x.json", false),
        "el `.gitignore` debe ignorar el runtime desechable `.lodestar/runtime/`"
    );
    assert!(
        !esta_ignorado(&gi, ".lodestar/config.yaml", false),
        "el `.gitignore` NO debe ignorar la config VERSIONADA `.lodestar/config.yaml`"
    );
}

/// Criterio: un fichero en `.lodestar/runtime/plans/x.json` no genera un `IndexEvent` de
/// conocimiento cuando el watcher procesa eventos.
///
/// Nota de fase: hoy el store excluye `.lodestar/` entero e indexa solo `.md`, así que este test
/// ya pasa — es el **guardián de regresión** de H06: cuando la carga empiece a INCLUIR los
/// canónicos de `.lodestar/` (config/schema/templates), el runtime debe seguir excluido. El
/// control positivo (un `.md` de conocimiento real SÍ emite evento) garantiza que la ausencia de
/// evento del runtime no es un bus muerto.
#[test]
fn runtime_no_indexa() {
    let dir = tempfile::tempdir().unwrap();
    // Apertura EN VIVO (cache + watcher + bus). Sin git: no hace falta para observar el bus.
    let ws = Workspace::open_live(dir.path()).unwrap();
    let rx = ws.subscribe().unwrap();

    // (1) Escribe runtime DESECHABLE directamente en disco (json y md, el caso peligroso).
    let rt = dir.path().join(".lodestar/runtime/plans");
    std::fs::create_dir_all(&rt).unwrap();
    std::fs::write(rt.join("x.json"), "{\"plan\":1}\n").unwrap();
    std::fs::write(
        rt.join("nota.md"),
        "---\ntype: Nota\ntitle: R\n---\n\n# R\n",
    )
    .unwrap();

    // (2) Control positivo: un `.md` de CONOCIMIENTO real por el único escritor debe emitir evento.
    let real = RelPath::new("real.md").unwrap();
    ws.create_concept(&real, "Nota", Some("Real"), "# H\n\ncuerpo\n", false)
        .unwrap();

    // (3) Drena el bus una ventana amplia (cubre el debounce ~250 ms). NINGÚN evento puede
    // referenciar el runtime; el evento de `real.md` debe llegar.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2000);
    let mut vio_real = false;
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(ev) => {
                for p in ev.changed.iter().chain(ev.removed.iter()) {
                    assert!(
                        !p.as_str().contains("runtime"),
                        "un fichero de `.lodestar/runtime/` generó un IndexEvent de conocimiento: {p:?}"
                    );
                }
                if ev.changed.iter().any(|p| p.as_str() == "real.md") {
                    vio_real = true;
                }
            }
            Err(_) if vio_real => break,
            Err(_) => {}
        }
    }
    assert!(
        vio_real,
        "control positivo: un `.md` de conocimiento real debe emitir un IndexEvent (bus vivo)"
    );
}

/// Criterio: un repo ya adoptado con `.lodestar/` trackeado ENTERO (su `.gitignore` no lo ignora)
/// → al abrir se ofrece/aplica ignorar solo `index.db` + `runtime/`, de forma idempotente.
///
/// Fase ROJA: hoy la apertura solo toca `.git/info/exclude` (vía `ensure_cache_ignored`), nunca el
/// `.gitignore` versionado del repo → el `.gitignore` sigue sin ignorar `index.db`/`runtime/`.
#[test]
fn adopcion_ajusta_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // Repo adoptado: el usuario ya tiene su `.gitignore` propio SIN mención de `.lodestar/`
    // (es decir, `.lodestar/` entero está trackeado) y sus ficheros canónicos versionados.
    std::fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
    std::fs::create_dir_all(root.join(".lodestar")).unwrap();
    std::fs::write(root.join(".lodestar/config.yaml"), "workspace: {}\n").unwrap();

    // Abrir el workspace debe ajustar el `.gitignore` (texto plano, sin git2).
    let _ws = Workspace::open_live(root).unwrap();
    let tras_primera = std::fs::read_to_string(root.join(".gitignore")).unwrap();

    let gi = gitignore_de(root);
    assert!(
        esta_ignorado(&gi, ".lodestar/index.db", false),
        "tras adoptar, el `.gitignore` debe ignorar `.lodestar/index.db`; era:\n{tras_primera}"
    );
    assert!(
        esta_ignorado(&gi, ".lodestar/runtime/plans/x.json", false),
        "tras adoptar, el `.gitignore` debe ignorar `.lodestar/runtime/`; era:\n{tras_primera}"
    );
    assert!(
        !esta_ignorado(&gi, ".lodestar/config.yaml", false),
        "el ajuste NO debe ignorar la config versionada `.lodestar/config.yaml`; era:\n{tras_primera}"
    );
    assert!(
        tras_primera.lines().any(|l| l.trim() == "node_modules/"),
        "el ajuste debe preservar el `.gitignore` propio del usuario; era:\n{tras_primera}"
    );

    // Idempotencia: una segunda apertura no vuelve a mutar el `.gitignore`.
    let _ws2 = Workspace::open_live(root).unwrap();
    let tras_segunda = std::fs::read_to_string(root.join(".gitignore")).unwrap();
    assert_eq!(
        tras_primera, tras_segunda,
        "el ajuste del `.gitignore` debe ser idempotente (sin líneas duplicadas)"
    );
}

// ---------------------------------------------------------------------------
// E10-H05 — Loader de esquemas: `.lodestar/schema.yaml` → `lodestar_core::schema::Schema`.
//
// Fase ROJA (ARCHITECTURE.md §19.2, REFACTOR §4/§9.4, patrón `WorkspaceConfig::load`):
// el TIPO `Schema` vive en el CORE (puro); el LOADER (I/O) vive en `workspace` y NUNCA
// deja que el core abra ficheros. API objetivo asumida (consistente con
// `WorkspaceConfig::load`):
//
//     lodestar_workspace::WorkspaceSchema::load(root: &Path)
//         -> Result<lodestar_core::schema::Schema, String>
//
// lee `<root>/.lodestar/schema.yaml`; ausencia de fichero ⇒ `Schema` vacío/permisivo
// (NO error), igual que `Config::load`/`WorkspaceConfig::load`.
// ---------------------------------------------------------------------------

/// Escribe `<root>/.lodestar/schema.yaml` con el contenido dado (crea `.lodestar/` si falta).
fn escribe_schema_yaml(root: &std::path::Path, contenido: &str) {
    let dir = root.join(".lodestar");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("schema.yaml"), contenido).unwrap();
}

/// Criterio `sin_schema_permisivo`: un bundle SIN `.lodestar/schema.yaml` → `Schema` vacío
/// permisivo (types vacío) y **sin error** (compat con bundles OKF actuales).
#[test]
fn sin_schema_permisivo() {
    use lodestar_workspace::WorkspaceSchema;
    let dir = tempfile::tempdir().unwrap();
    // Deliberadamente NO escribimos `.lodestar/schema.yaml`.

    let schema = WorkspaceSchema::load(dir.path())
        .expect("un bundle sin schema.yaml debe cargar un Schema permisivo, no fallar");

    assert!(
        schema.types.is_empty(),
        "sin schema.yaml, `types` debe estar vacío (permisivo); eran: {:?}",
        schema.types.keys().collect::<Vec<_>>()
    );
}

/// Criterio extra (evita vacuidad — ejercita el loader real de I/O): con un
/// `.lodestar/schema.yaml` presente que declara un `DocType` `decision`, el loader lo
/// deserializa a `Schema` y expone sus `required_fields`.
#[test]
fn loader_carga_schema_yaml() {
    use lodestar_workspace::WorkspaceSchema;
    let dir = tempfile::tempdir().unwrap();
    escribe_schema_yaml(
        dir.path(),
        "\
version: \"1\"
types:
  decision:
    name: decision
    requiredFields: [title, status, rationale]
    allowedStatuses: [proposed, accepted]
",
    );

    let schema =
        WorkspaceSchema::load(dir.path()).expect("un schema.yaml válido debe cargar sin error");

    let decision = schema
        .types
        .get("decision")
        .expect("el loader debe deserializar el DocType `decision` del schema.yaml");
    assert_eq!(
        decision.required_fields,
        vec![
            "title".to_string(),
            "status".to_string(),
            "rationale".to_string()
        ],
        "el loader debe preservar `requiredFields` del wire camelCase"
    );
}
