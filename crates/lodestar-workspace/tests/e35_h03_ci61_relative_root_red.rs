//! E35-H03 CI61 — una raíz relativa se fija al abrir y autentica el mismo rebuild que la absoluta.
//!
//! `ARCHITECTURE.md §20.5` permite obtener la raíz de `--root` y exige canonicalizarla al arranque,
//! manteniéndola fija durante la sesión. C1/C7 exigen además que discovery y SQLite compartan esa
//! misma autoridad. El caso relativo cambia temporalmente el cwd del proceso bajo un guard RAII;
//! este binario de integración contiene un único test, por lo que el cambio no compite con otros
//! tests del harness.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use lodestar_core::types::RelPath;
use lodestar_workspace::Workspace;

static CWD_LOCK: Mutex<()> = Mutex::new(());

struct RestoreCurrentDir(PathBuf);

impl RestoreCurrentDir {
    fn enter(path: &Path) -> Self {
        let previous = std::env::current_dir().expect("capturar cwd original");
        std::env::set_current_dir(path).expect("entrar en el padre del workspace relativo");
        Self(previous)
    }
}

impl Drop for RestoreCurrentDir {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.0)
            .unwrap_or_else(|error| panic!("restaurar cwd original {}: {error}", self.0.display()));
    }
}

fn write(root: &Path, relative: &str, contents: impl AsRef<[u8]>) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("crear directorio de fixture");
    }
    std::fs::write(path, contents).expect("escribir fixture");
}

fn materialize_fixture(root: &Path) {
    std::fs::create_dir_all(root).expect("crear raíz de fixture");
    write(root, ".lodestarignore", "ignored.md\n");
    write(
        root,
        "alpha.md",
        "---\ntitle: Árbol CI61\nmeta:\n  nested: sí\n---\n# Alpha\n\n[beta](nested/beta.md) [asset](asset.txt) [missing](missing.md)\n",
    );
    write(
        root,
        "nested/beta.md",
        "# Beta Unicode 🧭\n\naguja_ci61_rebuild_relativo\n",
    );
    write(root, "asset.txt", "activo enlazable\n");
    write(
        root,
        "ignored.md",
        "# Excluded\n\naguja_ci61_no_debe_indexarse\n",
    );
}

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("RelPath de fixture válido")
}

/// C1/C7 + §20.5 — abrir desde el cwd padre con `relative-root` debe fijar una raíz absoluta y
/// reconstruir exactamente los documentos, FTS y clasificaciones que el caso absoluto positivo.
#[test]
fn c1_c7_root_relativo_reconstruye_cache_con_la_misma_autoridad_que_root_absoluto() {
    let _cwd_serial = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let outer = tempfile::tempdir().expect("sandbox CI61");
    let absolute_root = outer.path().join("absolute-root");
    let relative_root_absolute = outer.path().join("relative-root");
    materialize_fixture(&absolute_root);
    materialize_fixture(&relative_root_absolute);

    let absolute = Workspace::open_live(&absolute_root)
        .expect("guarda positiva: una raíz absoluta válida activa y reconstruye la cache");
    let absolute_set = absolute.document_set().expect("core absoluto");
    let absolute_core = absolute_set.analyze();
    let absolute_cache = absolute.cache().expect("cache absoluta activa");
    let absolute_documents = absolute_cache.documents().expect("documentos absolutos");
    let absolute_search = absolute_cache
        .search("aguja_ci61_rebuild_relativo")
        .expect("búsqueda absoluta");
    let absolute_links = absolute_cache
        .outgoing_links(&rp("alpha.md"))
        .expect("enlaces absolutos");

    assert_eq!(
        absolute_documents,
        vec![rp("alpha.md"), rp("nested/beta.md")],
        "guarda anti-vacuidad: el positivo absoluto indexa ambos documentos admitidos"
    );
    assert_eq!(
        absolute_documents, absolute_core.documents,
        "guarda anti-vacuidad: el positivo absoluto conserva paridad core↔cache"
    );
    assert_eq!(
        absolute_search,
        vec![rp("nested/beta.md")],
        "guarda anti-vacuidad: la búsqueda consulta un payload real, no solo conteos"
    );
    assert!(
        absolute_links.iter().any(|(_, kind, path, _)| {
            kind == "document" && path.as_deref() == Some("nested/beta.md")
        }),
        "guarda anti-vacuidad: beta se clasifica como document"
    );
    assert!(
        absolute_links.iter().any(|(_, kind, path, _)| {
            kind == "workspaceFile" && path.as_deref() == Some("asset.txt")
        }),
        "guarda anti-vacuidad: el asset admitido se clasifica como workspaceFile"
    );
    assert!(
        absolute_links.iter().any(|(_, kind, path, _)| {
            kind == "missing" && path.as_deref() == Some("missing.md")
        }),
        "guarda anti-vacuidad: el enlace roto conserva su clasificación"
    );
    assert!(
        absolute_cache
            .search("aguja_ci61_no_debe_indexarse")
            .expect("búsqueda del excluido")
            .is_empty(),
        "guarda anti-vacuidad: .lodestarignore excluye el centinela negativo"
    );

    let relative = {
        let _restore_cwd = RestoreCurrentDir::enter(outer.path());
        let input = Path::new("relative-root");
        assert!(
            !input.is_absolute(),
            "guarda anti-vacuidad: la API recibe realmente una raíz relativa"
        );
        Workspace::open_live(input).expect(
            "rojo causal CI61: la raíz relativa válida debe autenticar discovery y activar la cache",
        )
    };

    assert_eq!(
        relative.root(),
        std::fs::canonicalize(&relative_root_absolute).expect("raíz esperada canonicalizada"),
        "§20.5: la raíz relativa se canonicaliza al arrancar y queda fija tras restaurar el cwd"
    );
    let relative_set = relative
        .document_set()
        .expect("core relativo tras restaurar cwd");
    let relative_core = relative_set.analyze();
    let relative_cache = relative.cache().expect("cache relativa activa");
    assert_eq!(
        relative_cache.documents().expect("documentos relativos"),
        absolute_documents,
        "C1/C7: root relativo y absoluto publican exactamente los mismos documentos"
    );
    assert_eq!(
        relative_core.documents, absolute_core.documents,
        "C1/C7: el core conserva la misma política canónica tras fijar la raíz relativa"
    );
    assert_eq!(
        relative_cache
            .search("aguja_ci61_rebuild_relativo")
            .expect("búsqueda relativa"),
        absolute_search,
        "C7: el payload consultable coincide, no solo el número de filas"
    );
    assert_eq!(
        relative_cache
            .outgoing_links(&rp("alpha.md"))
            .expect("enlaces relativos"),
        absolute_links,
        "C7: las clasificaciones document/workspaceFile/missing coinciden exactamente"
    );
}

#[cfg(unix)]
#[test]
fn c6_workspace_rechaza_plano_de_control_symlink_antes_de_leer_config_exterior() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("CI65 workspace temporal");
    let exterior = tempfile::tempdir().expect("CI65 exterior temporal");
    std::fs::write(
        exterior.path().join("config.yaml"),
        "discovery:\n  include: contenido-exterior-invalido\n",
    )
    .expect("CI65 escribir config exterior inválida");
    symlink(exterior.path(), root.path().join(".lodestar"))
        .expect("CI65 enlazar plano de control al exterior");

    let error = match Workspace::open_live(root.path()) {
        Ok(_) => panic!("CI65 debe rechazar el plano de control antes de cargar config"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("control plane must be a real directory")
            && !error.contains("contenido-exterior-invalido")
            && !error.contains("config.yaml inválido"),
        "C6 CI65: Workspace debe rechazar el symlink antes de leer config exterior; error={error:?}"
    );
}

#[cfg(unix)]
#[test]
fn c1_workspace_fija_la_raiz_canonica_cuando_el_root_es_symlink() {
    use std::os::unix::fs::symlink;

    let sandbox = tempfile::tempdir().expect("CI67 sandbox");
    let target = sandbox.path().join("real-workspace");
    std::fs::create_dir(&target).expect("CI67 crear workspace real");
    std::fs::write(target.join("doc.md"), "# CI67\n").expect("CI67 escribir Markdown");
    let alias = sandbox.path().join("workspace-alias");
    symlink(&target, &alias).expect("CI67 crear root symlink");

    let workspace = Workspace::open(&alias).expect("CI67 abrir root symlink legítimo");
    assert_eq!(
        workspace.root(),
        std::fs::canonicalize(&target).expect("CI67 canonical target"),
        "§20.5: la raíz queda canonicalizada al arrancar, no solo convertida en absoluta"
    );
}
