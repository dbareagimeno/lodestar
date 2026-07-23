//! Test de integración de **E15-H02**: `change_apply` **deja de regenerar** `index`/`tags`.
//!
//! E13-H11 metió la auto-regeneración de los generados dentro de la transacción de publicación
//! (`crates/lodestar-workspace/src/transaction.rs::augment_with_regenerated`). E15 borra los
//! generadores (`core::generate`, `Bundle::gen_index`/`gen_tag_indexes`) porque «ningún fichero
//! tiene semántica de catálogo» — así que la transacción debe publicar **exactamente** lo que pide
//! el change set y nada más: un `index.md` del proyecto es un documento más, propiedad del usuario,
//! que Lodestar no reescribe por su cuenta.
//!
//! Este fichero **sustituye** a `regen.rs` (que fija el comportamiento contrario y lo retira el
//! implementador). Deliberadamente **no** usa `Bundle::gen_index`/`gen_tag_indexes` para montar el
//! estado de partida: esas funciones desaparecen en E15-H02 y el test debe seguir compilando.
//!
//! ## Fase ROJA (documentada)
//! El test compila hoy (toda la API que usa ya existe); el rojo es de **aserción**: hoy
//! `apply_transaction` aumenta el resultado con la regeneración, así que el `index.md` escrito a
//! mano se sustituye por el canónico y aparece en `changedPaths` junto a `beta.md`.

use std::path::Path;

use lodestar_app::App;
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::RelPath;

/// Escribe un `.md` (creando los directorios intermedios) dentro del workspace temporal.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Política permisiva: el criterio no depende del veredicto de conformidad (desde E16-H02 un
/// documento sin enlaces ni siquiera genera diagnóstico: el aislamiento es una propiedad).
fn policy_permisiva() -> PlanPolicy {
    PlanPolicy {
        require_conformant_result: false,
        allow_warnings: true,
    }
}

/// `changedPaths` del ÚNICO receipt persistido del apply (`.lodestar/runtime/receipts/*.json`).
fn receipt_changed_paths(root: &Path) -> Vec<String> {
    let dir = root.join(".lodestar").join("runtime").join("receipts");
    let jsons: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            panic!(
                "el directorio de receipts {} debe existir: {e}",
                dir.display()
            )
        })
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    assert_eq!(
        jsons.len(),
        1,
        "tras un apply debe haber exactamente un receipt, hay {}: {jsons:?}",
        jsons.len(),
    );
    let contenido = std::fs::read_to_string(&jsons[0]).unwrap();
    let valor: serde_json::Value = serde_json::from_str(&contenido)
        .unwrap_or_else(|e| panic!("el receipt debe ser JSON válido: {e}\n{contenido}"));
    valor["changedPaths"]
        .as_array()
        .unwrap_or_else(|| panic!("el receipt debe llevar `changedPaths` (array): {valor}"))
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// `apply_no_regenera_indices` — **Dado** un workspace con un `index.md` desactualizado (no lista
/// el documento que se va a crear, y ni siquiera tiene el formato que produciría un generador),
/// **Cuando** se aplica un `change_plan` que crea un documento, **Entonces** el receipt lista
/// **solo** el documento creado — ningún índice regenerado — y el `index.md` de disco queda byte a
/// byte idéntico (`requirements/epica-15-workspace-universal.md` § E15-H02).
#[test]
fn apply_no_regenera_indices() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // (1) Un `index.md` escrito a mano y DESACTUALIZADO: menciona `alfa.md`, no menciona el
    //     documento que se creará, y su prosa no es la que emitiría ningún generador.
    let index_original = "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Mi proyecto\n\n\
                          Índice mantenido a mano.\n\n* [Alfa](/alfa.md)\n";
    escribe(root, "index.md", index_original);
    escribe(
        root,
        "alfa.md",
        "---\ntype: Nota\ntitle: Alfa\ndescription: Primer documento\n---\n\n# Resumen\n\ncuerpo\n",
    );

    let app = App::open(root).expect("el workspace temporal debe abrir");

    // (2) Plan: crear un documento en la raíz (el ámbito del `index.md`).
    let ops = serde_json::json!([
        { "op": "create", "path": "beta.md", "type": "Nota", "title": "Beta",
          "body": "# Resumen\n\ncuerpo del documento beta\n" },
    ]);
    let plan = app
        .change_plan(None, &ops, policy_permisiva())
        .expect("el `change_plan` del create debe tener éxito");

    // (3) Apply.
    let apply = app
        .change_apply(&plan.change_set_id, None)
        .expect("el `change_apply` debe publicar la transacción");

    // (a) `changedPaths` del resultado = SOLO el documento creado.
    let beta = RelPath::new("beta.md").unwrap();
    assert_eq!(
        apply.changed_paths,
        vec![beta],
        "la transacción debe publicar solo el documento del plan, sin índices regenerados",
    );

    // (b) Lo mismo en el receipt persistido (es el contrato que ve el agente).
    let receipt = receipt_changed_paths(root);
    assert_eq!(
        receipt,
        vec!["beta.md".to_string()],
        "el receipt debe listar solo `beta.md`: {receipt:?}",
    );

    // (c) El `index.md` del usuario queda intacto byte a byte: nadie lo regeneró.
    let index_final = std::fs::read_to_string(root.join("index.md")).unwrap();
    assert_eq!(
        index_final, index_original,
        "el `index.md` del proyecto no debe reescribirse: es un documento más, no un catálogo",
    );

    // (d) Y no se ha inventado ningún árbol de índices de tags.
    assert!(
        !root.join("tags").exists(),
        "el apply no debe materializar un árbol `tags/` de índices generados",
    );
}
