//! Tests de integración de **E13-H11**: auto-regeneración de `index`/`tags` DENTRO de
//! `change_apply`, en la MISMA transacción/receipt (decisión **D6a**, `ARCHITECTURE.md §19.6`).
//!
//! `App::change_apply` (E13-H08) YA publica el change set del plan por el único escritor, pero NO
//! toca los generados: si un cambio crea/mueve/borra conceptos o altera tags, los `index.md` y los
//! índices de tags quedan **obsoletos** hasta que el usuario corra `lodestar index`/`tags`. Esta
//! historia añade que `change_apply`, cuando el cambio afecta la estructura, **incluya en el mismo
//! lote** la regeneración de los `index.md`/índices de tags afectados (una `Mutation` añadida al
//! staging antes de publicar → mismo staging/journal/receipt).
//!
//! Los generadores son puros y ya existen en `lodestar-core`: [`Bundle::gen_index`] y
//! [`Bundle::gen_tag_indexes`] (`bundle.rs:420/425`, `§10` fila 12). Estos tests los usan para
//! **montar** el estado generado inicial (así el `index.md`/los índices de tags de partida son
//! genuinamente los que produciría `lodestar index`/`tags`), y luego verifican que TRAS el apply el
//! disco canónico refleja la estructura nueva.
//!
//! ## Cómo se verifica que la regeneración fue en la MISMA transacción
//! El [`ApplyResult::changed_paths`] es el conjunto de paths que la transacción
//! creó/modificó/borró (lo que se persiste tal cual en el `ChangeReceipt.changedPaths`, un único
//! receipt por apply — `crates/lodestar-workspace/src/receipts.rs`). Si el `index.md`/el índice de
//! tags obsoleto aparece ahí, se publicó en el MISMO lote que el `.md` del concepto (no en un
//! segundo pase). Se asevera además leyendo el receipt persistido (`.lodestar/runtime/receipts/`).
//!
//! ## Fase ROJA (documentada)
//! `change_apply` YA existe y compila, así que estos tests **compilan**; el rojo es de **aserción**
//! (regla 2), no de compilación: hoy `change_apply` publica solo las ops del plan, así que
//!   - `apply_regenera_index`: el `index.md` en disco NO incluye el concepto nuevo y `changedPaths`
//!     NO contiene `index.md` → las aserciones fallan.
//!   - `apply_regenera_tags`: el índice del tag que quedó huérfano (`tags/azul/index.md`) SIGUE en
//!     disco y NO está en `changedPaths` → las aserciones fallan.
//!
//! Cuando el implementador enganche la auto-regen en `change_apply`, ambos pasan a verde.
//!
//! NO se toca producción: la implementación de la auto-regen es trabajo del implementador.

use std::path::Path;

use lodestar_app::{App, ApplyResult};
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::{FileMap, Mutation, RelPath};
use lodestar_core::Bundle;

/// Escribe un `.md` (creando los directorios intermedios) dentro del bundle temporal.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Materializa en disco todos los `writes` de una [`Mutation`] (los generados). Usa `RelPath`
/// validado como único chokepoint de path (invariante #6): nada de strings crudos como ruta.
fn materializa(root: &Path, mutation: &Mutation) {
    for (path, contenido) in &mutation.writes {
        escribe(root, path.as_str(), contenido);
    }
}

/// Política permisiva: no exige resultado conforme y admite warnings, para que el plan sea siempre
/// aplicable (el criterio de esta historia no depende del veredicto de conformidad; un concepto
/// recién creado que aún no está listado por su `index` es a lo sumo un *huérfano* → warning).
fn policy_permisiva() -> PlanPolicy {
    PlanPolicy {
        require_conformant_result: false,
        allow_warnings: true,
    }
}

/// Lee el `index.md` canónico de `dir` (p. ej. `""` para el raíz) tras el apply.
fn lee_index(root: &Path, dir: &str) -> String {
    let ruta = root.join(format!("{dir}index.md"));
    std::fs::read_to_string(&ruta)
        .unwrap_or_else(|e| panic!("el `index.md` de «{dir}» debe existir en disco: {e}"))
}

/// `true` si el `apply` publicó `rel` en su lote (aparece en `changedPaths`). Esa es la evidencia
/// de que la regeneración ocurrió en la MISMA transacción que el cambio del concepto.
fn changed_contiene(apply: &ApplyResult, rel: &str) -> bool {
    let objetivo = RelPath::new(rel).expect("rel de test válido");
    apply.changed_paths.contains(&objetivo)
}

/// Lee EL receipt persistido del apply (`.lodestar/runtime/receipts/*.json`) — hay exactamente uno
/// por apply en estos tests. Devuelve su `changedPaths` como conjunto de strings, para aseverar que
/// el generado regenerado quedó en el MISMO receipt.
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

/// `apply_regenera_index` — Dado un `create` de un concepto en un directorio con `index.md`
/// generado, Cuando se aplica, Entonces el `index.md` regenerado incluye el nuevo concepto **en el
/// mismo receipt**.
///
/// Montaje del `index` generado: se construye un `Bundle` con el único concepto de partida
/// (`alfa.md`) y se materializa su `index.md` raíz con [`Bundle::gen_index`] (`""` = raíz) — así el
/// index de partida es EXACTAMENTE el que produciría `lodestar index`, y lista `alfa.md` pero NO el
/// concepto que se creará. Tras el apply se comprueba: (a) el `index.md` canónico incluye ya un
/// enlace al nuevo `.md`; y (b) que la regeneración fue transaccional — `beta.md` **y** `index.md`
/// están ambos en `changedPaths` del [`ApplyResult`] y en el `changedPaths` del receipt persistido.
#[test]
fn apply_regenera_index() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // (1) Concepto de partida conforme.
    let alfa =
        "---\ntype: Nota\ntitle: Alfa\ndescription: Primer concepto\n---\n\n# Resumen\n\ncuerpo\n";
    escribe(root, "alfa.md", alfa);

    // (2) `index.md` raíz GENERADO por el core sobre {alfa.md} (mismo output que `lodestar index`).
    let mut files: FileMap = FileMap::new();
    files.insert(RelPath::new("alfa.md").unwrap(), alfa.to_string());
    let bundle = Bundle::from_files(files);
    let index_mut = bundle.gen_index("");
    materializa(root, &index_mut);

    // El index de partida lista alfa pero todavía NO el concepto que se creará (guarda anti-vacuidad).
    let index_inicial = lee_index(root, "");
    assert!(
        index_inicial.contains("alfa.md"),
        "el index generado de partida debe listar el concepto existente: {index_inicial:?}",
    );
    assert!(
        !index_inicial.contains("beta.md"),
        "el index de partida NO debe listar aún el concepto que se creará: {index_inicial:?}",
    );

    let app = App::open(root).expect("el bundle temporal debe abrir");

    // (3) Plan: crear un concepto conforme en el mismo ámbito (raíz) que el `index.md`.
    let ops = serde_json::json!([
        { "op": "create", "path": "beta.md", "type": "Nota", "title": "Beta",
          "body": "# Resumen\n\ncuerpo del concepto beta\n" },
    ]);
    let plan = app
        .change_plan(None, &ops, policy_permisiva())
        .expect("el `change_plan` del create debe tener éxito");

    // (4) Apply: publica el create y —esta historia— la regeneración del index en el mismo lote.
    let apply = app
        .change_apply(&plan.change_set_id, None)
        .expect("el `change_apply` debe publicar la transacción");

    // (a) El `index.md` canónico ya referencia el concepto nuevo (fue regenerado).
    let index_final = lee_index(root, "");
    assert!(
        index_final.contains("beta.md"),
        "tras el apply, el `index.md` regenerado debe incluir un enlace al concepto nuevo \
         (`beta.md`); index actual: {index_final:?}",
    );

    // (b) Misma transacción: el `.md` nuevo Y el `index.md` regenerado están ambos en `changedPaths`.
    assert!(
        changed_contiene(&apply, "beta.md"),
        "el `.md` creado debe estar en changedPaths: {:?}",
        apply.changed_paths,
    );
    assert!(
        changed_contiene(&apply, "index.md"),
        "el `index.md` regenerado debe publicarse en el MISMO lote (changedPaths del ApplyResult): \
         {:?}",
        apply.changed_paths,
    );

    // (b') Y quedan ambos en el MISMO receipt persistido (un único receipt por apply).
    let receipt = receipt_changed_paths(root);
    assert!(
        receipt.iter().any(|p| p == "beta.md") && receipt.iter().any(|p| p == "index.md"),
        "el receipt del apply debe listar tanto `beta.md` como el `index.md` regenerado: {receipt:?}",
    );
}

/// `apply_regenera_tags` — Dado un cambio de tags, Cuando se aplica, Entonces los índices de tags
/// obsoletos se purgan **en la misma transacción**.
///
/// Montaje de los índices de tags generados: dos conceptos con un tag distinto cada uno
/// (`alfa` → `rojo`, `beta` → `azul`); se materializan los índices con [`Bundle::gen_tag_indexes`]
/// (`tags/index.md`, `tags/rojo/index.md`, `tags/azul/index.md`), igual que `lodestar tags`.
///
/// Cómo se provoca el cambio de tags: un `patch_frontmatter` sobre `beta.md` que reemplaza su tag
/// `azul` por `rojo`. Tras el cambio, ningún concepto tiene el tag `azul` → su índice
/// `tags/azul/index.md` queda **obsoleto**. Se asevera que tras el apply ese índice ha desaparecido
/// del disco (purgado) y que la purga fue transaccional (`tags/azul/index.md` en `changedPaths`).
#[test]
fn apply_regenera_tags() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // (1) Dos conceptos, cada uno con su tag; más un `index.md` raíz mínimo (marcador de bundle).
    let alfa = "---\ntype: Nota\ntitle: Alfa\ntags:\n  - rojo\n---\n\n# Resumen\n\ncuerpo\n";
    let beta = "---\ntype: Nota\ntitle: Beta\ntags:\n  - azul\n---\n\n# Resumen\n\ncuerpo\n";
    escribe(
        root,
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    escribe(root, "alfa.md", alfa);
    escribe(root, "beta.md", beta);

    // (2) Índices de tags GENERADOS por el core (mismo output que `lodestar tags`).
    let mut files: FileMap = FileMap::new();
    files.insert(RelPath::new("alfa.md").unwrap(), alfa.to_string());
    files.insert(RelPath::new("beta.md").unwrap(), beta.to_string());
    let bundle = Bundle::from_files(files);
    let tags_mut = bundle.gen_tag_indexes();
    materializa(root, &tags_mut);

    // El índice del tag `azul` existe de partida (guarda anti-vacuidad).
    let azul = root.join("tags").join("azul").join("index.md");
    assert!(
        azul.is_file(),
        "el índice del tag `azul` debe existir de partida (generado): {}",
        azul.display(),
    );

    let app = App::open(root).expect("el bundle temporal debe abrir");

    // (3) Plan: reemplazar el tag `azul` de `beta.md` por `rojo` → `azul` queda sin conceptos.
    let ops = serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "beta.md" },
          "patch": { "tags": ["rojo"] } },
    ]);
    let plan = app
        .change_plan(None, &ops, policy_permisiva())
        .expect("el `change_plan` del cambio de tags debe tener éxito");

    // (4) Apply: publica el cambio y —esta historia— purga el índice de tag obsoleto en el mismo lote.
    let apply = app
        .change_apply(&plan.change_set_id, None)
        .expect("el `change_apply` debe publicar la transacción");

    // (a) El índice del tag obsoleto se purgó del disco canónico.
    assert!(
        !azul.is_file(),
        "tras el apply, el índice del tag obsoleto `tags/azul/index.md` debe haberse purgado: {}",
        azul.display(),
    );

    // (b) Misma transacción: la purga aparece en `changedPaths` del apply y del receipt.
    assert!(
        changed_contiene(&apply, "tags/azul/index.md"),
        "la purga del índice de tag obsoleto debe publicarse en el MISMO lote (changedPaths): {:?}",
        apply.changed_paths,
    );
    let receipt = receipt_changed_paths(root);
    assert!(
        receipt.iter().any(|p| p == "tags/azul/index.md"),
        "el receipt del apply debe listar la purga de `tags/azul/index.md`: {receipt:?}",
    );
}
