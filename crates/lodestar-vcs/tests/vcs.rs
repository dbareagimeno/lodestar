//! Tests de integración de `lodestar-vcs` (E4): init/commit/log/tree_files/conformidad/ramas.

use lodestar_core::types::{Author, RepoState, Sha};
use lodestar_vcs::Vcs;

fn author() -> Author {
    Author {
        name: "Test".into(),
        email: "test@example.com".into(),
    }
}

fn write(root: &std::path::Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

#[test]
fn discover_techo_en_root_no_engancha_ancestro() {
    let parent = tempfile::tempdir().unwrap();
    // repo en el ancestro
    Vcs::init(parent.path(), &author()).unwrap();
    // un subdirectorio SIN su propio .git
    let child = parent.path().join("sub");
    std::fs::create_dir_all(&child).unwrap();
    // discover(child) no debe enganchar el repo del ancestro (techo en el root)
    assert!(Vcs::discover(&child).unwrap().is_none());
}

#[test]
fn init_crea_repo_con_commit_inicial_y_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    let vcs = Vcs::init(dir.path(), &author()).unwrap();
    assert!(dir.path().join(".gitignore").is_file());
    let contenido = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert!(contenido.contains(".lodestar/"));
    let log = vcs.log(10).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].message, "Commit inicial");
    assert_eq!(log[0].author.name, "Test");
}

#[test]
fn commit_y_tree_files_reconstruye_el_arbol() {
    let dir = tempfile::tempdir().unwrap();
    let vcs = Vcs::init(dir.path(), &author()).unwrap();
    write(
        dir.path(),
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n",
    );
    let sha = vcs.commit("Añade A", &author()).unwrap();
    let files = vcs.tree_files(&sha).unwrap();
    assert!(files.keys().any(|k| k.as_str() == "a.md"));
    // .gitignore no es .md → no aparece en el file-map
    assert!(files.keys().all(|k| k.as_str().ends_with(".md")));
}

#[test]
fn conformidad_por_commit() {
    let dir = tempfile::tempdir().unwrap();
    let vcs = Vcs::init(dir.path(), &author()).unwrap();
    // commit conforme
    write(
        dir.path(),
        "ok.md",
        "---\ntype: Nota\ntitle: Ok\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    let bueno = vcs.commit("ok", &author()).unwrap();
    assert!(vcs.conformance(&bueno).unwrap().conform);
    // commit no conforme (sin frontmatter)
    write(dir.path(), "malo.md", "# sin frontmatter\n");
    let malo = vcs.commit("malo", &author()).unwrap();
    let conf = vcs.conformance(&malo).unwrap();
    assert!(!conf.conform);
    assert_eq!(conf.hard_fail, 1);
    // last_conforming debe volver al commit bueno
    assert_eq!(vcs.last_conforming().unwrap(), Some(bueno));
}

#[test]
fn ramas_crear_y_listar() {
    let dir = tempfile::tempdir().unwrap();
    let vcs = Vcs::init(dir.path(), &author()).unwrap();
    vcs.create_branch("feature", None).unwrap();
    let branches = vcs.branches().unwrap();
    assert!(branches.iter().any(|b| b.name == "feature"));
    assert!(branches.iter().any(|b| b.is_head)); // la rama por defecto es HEAD
}

#[test]
fn repo_state_limpio_tras_init() {
    let dir = tempfile::tempdir().unwrap();
    let vcs = Vcs::init(dir.path(), &author()).unwrap();
    assert_eq!(vcs.repo_state(), RepoState::Clean);
}

#[test]
fn dirty_paths_detecta_cambios_sin_commitear() {
    let dir = tempfile::tempdir().unwrap();
    let vcs = Vcs::init(dir.path(), &author()).unwrap();
    write(dir.path(), "nuevo.md", "contenido\n");
    let dirty = vcs.dirty_paths().unwrap();
    assert!(dirty.iter().any(|p| p.as_str() == "nuevo.md"));
}

#[test]
fn sha_invalido_no_cruza_la_frontera() {
    // Sha::new valida hex; un oid inválido se rechaza antes de tocar git2.
    assert!(Sha::new("zzz").is_err());
}
