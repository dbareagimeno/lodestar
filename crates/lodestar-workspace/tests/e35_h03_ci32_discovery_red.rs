//! E35-H03 CI32 — una sola política de configuración y de admisión Markdown.
//!
//! Estos tests fijan dos divergencias observadas por la revisión arquitectónica: el consumidor
//! directo `Store` no puede validar menos configuración que `Workspace`, y el guard de escritura
//! no puede discrepar del inventario canónico por la capitalización de la extensión Markdown.

use lodestar_core::types::RelPath;
use lodestar_store::Store;
use lodestar_workspace::Workspace;

fn root_con_config(yaml: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("crear workspace temporal");
    std::fs::create_dir_all(root.path().join(".lodestar")).expect("crear directorio de control");
    std::fs::write(root.path().join(".lodestar/config.yaml"), yaml)
        .expect("escribir configuración del caso");
    root
}

fn error_workspace(root: &std::path::Path, clave: &str) -> String {
    match Workspace::open(root) {
        Err(error) => {
            let mensaje = error.to_string();
            assert!(
                mensaje.contains(clave),
                "Workspace debe nombrar la clave desconocida {clave:?}; mensaje={mensaje:?}"
            );
            mensaje
        }
        Ok(workspace) => panic!(
            "Workspace no puede aceptar {clave:?} ni degradar a defaults; policy={:?}",
            workspace.discovery_policy()
        ),
    }
}

fn error_store(root: &std::path::Path, clave: &str) -> String {
    match Store::open(root) {
        Err(error) => {
            let mensaje = error.to_string();
            assert!(
                mensaje.contains(clave),
                "Store debe nombrar la misma clave desconocida {clave:?}; mensaje={mensaje:?}"
            );
            mensaje
        }
        Ok(_) => panic!(
            "Store::open aceptó {clave:?} aunque Workspace::open la rechaza: el consumidor directo \
             no puede caer a la política de discovery por defecto"
        ),
    }
}

/// Criterio CI32-A — una clave desconocida de nivel superior se rechaza tanto desde `Workspace`
/// como desde el consumidor directo `Store`.
///
/// Anti-vacuidad: primero se abre una configuración válida que contiene una sección no-discovery
/// (`workspace`) y se demuestra que el `Store` conserva el `include` declarado. Por tanto, el
/// arreglo no puede consistir en rechazar todas las claves que su parser reducido no conoce ni en
/// ignorar el fichero entero.
#[test]
fn store_y_workspace_rechazan_clave_desconocida_de_nivel_superior() {
    let valido = root_con_config(
        "workspace:\n  writableRoots: [notas]\n\
         discovery:\n  include: [\"notas/**/*.MD\"]\n",
    );
    std::fs::create_dir_all(valido.path().join("notas")).unwrap();
    std::fs::write(valido.path().join("notas/admitida.md"), "# Admitida\n").unwrap();
    std::fs::write(valido.path().join("fuera.md"), "# Fuera\n").unwrap();

    let workspace = Workspace::open(valido.path())
        .expect("las secciones conocidas completas deben seguir siendo válidas");
    let rutas_workspace: Vec<String> = workspace
        .document_set()
        .unwrap()
        .files()
        .keys()
        .map(|path| path.as_str().to_string())
        .collect();
    assert_eq!(
        rutas_workspace,
        vec!["notas/admitida.md"],
        "anti-vacuidad: Workspace debe aplicar el include declarado, no defaults"
    );

    let store = Store::open(valido.path()).expect(
        "Store debe aceptar claves conocidas ajenas a discovery al validar la configuración completa",
    );
    store
        .rebuild()
        .expect("reconstruir con la policy capturada");
    let rutas_store: Vec<String> = store
        .documents()
        .expect("listar inventario derivado")
        .into_iter()
        .map(|path| path.as_str().to_string())
        .collect();
    assert_eq!(
        rutas_store,
        vec!["notas/admitida.md"],
        "anti-vacuidad: Store debe aplicar el include válido, no degradarlo a defaults"
    );

    let invalido = root_con_config(
        "discovry:\n  include: [\"nada/**/*.md\"]\n\
         workspace:\n  writableRoots: [notas]\n",
    );
    let workspace_error = error_workspace(invalido.path(), "discovry");
    let store_error = error_store(invalido.path(), "discovry");
    assert!(
        workspace_error.contains("config.yaml") && store_error.contains("discovry"),
        "ambas aperturas deben rechazar de forma explicable la misma configuración"
    );
}

/// Criterio CI32-A (anidado) — una clave desconocida dentro de una sección conocida tampoco puede
/// quedar invisible para el `Store`. `writeableRoots` es un typo de una política de escritura:
/// aceptarlo mientras `Workspace` lo rechaza haría que cada fachada interpretase un fichero
/// distinto.
///
/// El control válido del test anterior garantiza que `workspace` no se rechaza por ser irrelevante
/// para el store; aquí se exige validar su forma completa aunque el store solo consuma discovery.
#[test]
fn store_y_workspace_rechazan_clave_desconocida_en_seccion_conocida() {
    let invalido = root_con_config(
        "workspace:\n  writeableRoots: [notas]\n\
         discovery:\n  include: [\"**/*.md\"]\n",
    );

    let workspace_error = error_workspace(invalido.path(), "writeableRoots");
    let store_error = error_store(invalido.path(), "writeableRoots");
    assert!(
        workspace_error.contains("config.yaml") && store_error.contains("writeableRoots"),
        "ambos consumidores deben atribuir el rechazo a la configuración y a la clave exacta"
    );
}

/// Criterio CI32-B — la admisión de una ruta candidata usa exactamente la misma semántica de
/// casing Markdown que el inventario compartido. Con `include: **/*.MD`, un `a.md` ya forma parte
/// de `document_set` y del rebuild del store; el guard de escritura debe admitir esa misma ruta.
///
/// Contrafactual: `a.txt` no entra en ninguno de los inventarios y el guard sí lo rechaza, de modo
/// que el test no pasa convirtiendo `assert_discoverable` en un `Ok(())` incondicional.
#[test]
fn include_mayusculo_admite_markdown_minusculo_en_inventario_y_guard() {
    let root = root_con_config("discovery:\n  include: [\"**/*.MD\"]\n");
    std::fs::write(root.path().join("a.md"), "# Documento\n").unwrap();
    std::fs::write(root.path().join("a.txt"), "fuera\n").unwrap();

    let workspace = Workspace::open(root.path()).expect("abrir workspace con include mayúsculo");
    let documentos: Vec<String> = workspace
        .document_set()
        .unwrap()
        .files()
        .keys()
        .map(|path| path.as_str().to_string())
        .collect();
    assert_eq!(
        documentos,
        vec!["a.md"],
        "precondición: el inventario canónico admite la extensión Markdown con casing equivalente"
    );

    let store = Store::open(root.path()).expect("Store acepta la misma config");
    store
        .rebuild()
        .expect("rebuild desde el inventario canónico");
    let indexados: Vec<String> = store
        .documents()
        .expect("listar documentos reconstruidos")
        .into_iter()
        .map(|path| path.as_str().to_string())
        .collect();
    assert_eq!(
        indexados,
        vec!["a.md"],
        "precondición: Store también deriva a.md desde el inventario canónico"
    );

    let markdown = RelPath::new("a.md").unwrap();
    workspace
        .assert_discoverable(&markdown)
        .unwrap_or_else(|error| {
            panic!(
            "a.md está en document_set y en Store, por lo que el guard de escritura debe admitir \
             exactamente la misma ruta con include **/*.MD: {error}"
        )
        });

    let no_markdown = RelPath::new("a.txt").unwrap();
    assert!(
        workspace.assert_discoverable(&no_markdown).is_err(),
        "contrafactual: igualar casing de extensiones Markdown no puede admitir a.txt"
    );
}
