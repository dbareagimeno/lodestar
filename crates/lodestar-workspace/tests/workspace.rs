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
    // el snapshot lo refleja
    let snap = ws.snapshot().unwrap();
    assert!(snap
        .analysis
        .concepts
        .iter()
        .any(|c| c.as_str() == "alfa.md"));
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
