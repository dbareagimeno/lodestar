//! Tests de la **cuarentena** del plano de control (E25-H02) que fijan lo que la batería original
//! de `tests/transactions.rs` dejó sin morder, según la pasada de mutantes de
//! `decisiones/16-deuda-auditoria-e25-e26.md §16(l)`.
//!
//! `Workspace::quarantine_transaction` (`src/recovery.rs`) **mueve** —no depura— el material de una
//! transacción cuya recuperación falló a `.lodestar/runtime/journal/quarantine/<txnId>/`: el
//! journal, el árbol de copias **y el sidecar de huellas**. Dos mutaciones sobrevivían a la suite:
//!
//! - **mutante S**: reemplazar el `rename` del sidecar de huellas por un `remove_file`. La
//!   cuarentena seguía existiendo y con el journal y las copias dentro, así que todos los tests de
//!   E25-H02 seguían verdes — pero el sidecar es lo que dice **con qué se iba a verificar** cada
//!   copia (tamaño y revisión blake3). Sin él, el árbol cuarentenado es un montón de bytes sin
//!   oráculo: no se puede decidir cuál de las copias es la que no verificaba. La cuarentena existe
//!   para no perder material forense; borrar una de sus tres piezas es exactamente lo que no puede
//!   pasar.
//! - **mutante N**: sustituir el bucle que busca el primer nombre libre (`<txnId>.2`, `<txnId>.3`…)
//!   por el nombre a secas. Una segunda cuarentena del **mismo** `txnId` escribía dentro del
//!   directorio de la primera, mezclando dos incidentes en un solo árbol y pisando el journal y el
//!   sidecar del primero. Ningún test cuarentenaba dos veces el mismo id, así que nadie se enteraba.
//!
//! Los dos criterios se aseveran sobre **comportamiento observable en disco** (qué ficheros quedan
//! y con qué bytes), no sobre la forma del código.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lodestar_core::plan;
use lodestar_core::types::{
    ChangeSet, ChangeSetId, FileMap, NormalizedOperation, PlanHash, RelPath, RiskAssessment,
    SemanticDiff, ValidationReport,
};
use lodestar_workspace::Workspace;

// ---------------------------------------------------------------------------
// Arnés: sembrar, componer un change set y dejar en disco una transacción interrumpida cuyo
// material NO verifica (que es lo que manda a cuarentena).
// ---------------------------------------------------------------------------

/// Siembra `root` con un `.md` por nombre y devuelve el `Workspace` abierto.
fn siembra(root: &Path, nombres: &[&str]) -> Workspace {
    let ws = Workspace::open(root).unwrap();
    for n in nombres {
        ws.create_document(
            &RelPath::new(&format!("{n}.md")).unwrap(),
            "Nota",
            Some(n),
            &format!("# {n}\n\ncuerpo original\n"),
            false,
        )
        .unwrap();
    }
    ws
}

/// Change set que sustituye el cuerpo de cada `<n>.md`, anclado a la revisión ACTUAL.
fn cs_modifica(ws: &Workspace, id: &str, paths: &[&str]) -> ChangeSet {
    ChangeSet {
        id: ChangeSetId(id.to_string()),
        base_revision: ws.workspace_revision().unwrap(),
        operations: paths
            .iter()
            .map(|p| NormalizedOperation::ReplaceBody {
                path: RelPath::new(p).unwrap(),
                body: format!("# {p}\n\ncuerpo NUEVO\n"),
            })
            .collect(),
        plan_hash: PlanHash("blake3:test".to_string()),
        risk: RiskAssessment::default(),
        semantic_diff: SemanticDiff::default(),
        validation: ValidationReport::default(),
        expires_at: "0".to_string(),
    }
}

/// `FileMap` de los `.md` canónicos (excluye `.lodestar/`).
fn canonico(root: &Path) -> FileMap {
    let mut out = FileMap::new();
    fn walk(d: &Path, base: &Path, out: &mut FileMap) {
        let Ok(entries) = std::fs::read_dir(d) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            let nombre = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if nombre.starts_with('.') {
                continue;
            }
            if p.is_dir() {
                walk(&p, base, out);
            } else if p.extension().is_some_and(|x| x == "md") {
                let rel = p
                    .strip_prefix(base)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(
                    RelPath::new(&rel).unwrap(),
                    std::fs::read_to_string(&p).unwrap_or_default(),
                );
            }
        }
    }
    walk(root, root, &mut out);
    out
}

/// Paths afectados por llevar `antes` a `despues`, con el criterio y el orden del orquestador.
fn afectados(antes: &FileMap, despues: &FileMap) -> Vec<RelPath> {
    let mut set: std::collections::BTreeSet<RelPath> = std::collections::BTreeSet::new();
    for (rel, c) in despues {
        if antes.get(rel) != Some(c) {
            set.insert(rel.clone());
        }
    }
    for rel in antes.keys() {
        if !despues.contains_key(rel) {
            set.insert(rel.clone());
        }
    }
    set.into_iter().collect()
}

/// Directorio `recovery/<txnId>/`.
fn recovery_de(root: &Path, txn_id: &str) -> PathBuf {
    root.join(".lodestar")
        .join("runtime")
        .join("recovery")
        .join(txn_id)
}

/// Sidecar de huellas `recovery/<txnId>.digests.json`.
fn digests_de(root: &Path, txn_id: &str) -> PathBuf {
    root.join(".lodestar")
        .join("runtime")
        .join("recovery")
        .join(format!("{txn_id}.digests.json"))
}

/// Directorio de cuarentena `journal/quarantine/<nombre>/`.
fn cuarentena_de(root: &Path, nombre: &str) -> PathBuf {
    root.join(".lodestar")
        .join("runtime")
        .join("journal")
        .join("quarantine")
        .join(nombre)
}

/// Todos los ficheros bajo `dir` (recursivo) como `ruta relativa POSIX -> bytes`.
fn ficheros_bajo(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(d: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(d) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
                continue;
            }
            let rel = p
                .strip_prefix(base)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, std::fs::read(&p).unwrap_or_default());
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Deja en disco el estado durable de una transacción **interrumpida** (copias → journal → un
/// rename), y corrompe la copia de recuperación del path publicado para que la recuperación
/// **falle** y el material acabe en cuarentena.
///
/// Devuelve el `txn_id` usado.
fn transaccion_irrecuperable(ws: &Workspace, root: &Path, txn_id: &str) {
    let cs = cs_modifica(ws, txn_id, &["uno.md", "dos.md", "tres.md"]);
    let antes = canonico(root);
    let resultado =
        plan::apply_normalized_ops(&antes, &cs.operations).expect("prever el resultado del plan");
    let afectados = afectados(&antes, &resultado);
    assert!(
        !afectados.is_empty(),
        "precondición del arnés: la transacción debe afectar a algún path"
    );

    let base = ws.workspace_revision().unwrap();
    let result_rev = lodestar_core::types::workspace_revision(&resultado, &[] as &[RelPath]);

    ws.backup_originals(txn_id, &afectados)
        .expect("preparar las copias de recuperación");
    let mut journal = ws
        .create_journal(txn_id, &afectados, &base, &result_rev)
        .expect("crear el write-ahead journal");

    // Un rename ya hecho y anotado: la restauración es NECESARIA, así que ninguna implementación
    // puede saltarse la copia y pasar el test vacuamente.
    let publicado = &afectados[0];
    std::fs::write(
        root.join(publicado.as_str()),
        resultado.get(publicado).unwrap(),
    )
    .unwrap();
    journal
        .mark_applied(publicado)
        .expect("anotar el rename en el journal");

    // La copia de recuperación de ese path se vuelve ILEGIBLE (bytes que no son UTF-8): la
    // recuperación no puede verificarla y manda el material a cuarentena. Es el mismo mecanismo que
    // usa `journal_irrecuperable_no_encalla_el_workspace` en `tests/transactions.rs`, y no depende
    // de `chmod` ni del usuario que corra los tests.
    std::fs::write(
        recovery_de(root, txn_id).join(publicado.as_str()),
        [0xff, 0xfe, 0x00, 0x01],
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// Los criterios.
// ---------------------------------------------------------------------------

/// **`decisiones §16(l)`, mutante S** — el **sidecar de huellas** se MUEVE a la cuarentena, no se
/// borra.
///
/// El sidecar (`recovery/<txnId>.digests.json`) es el oráculo con el que se verifica cada copia:
/// tamaño y revisión blake3 de cada original respaldado. Sin él, el árbol cuarentenado es un montón
/// de bytes del que nadie puede decir cuál era la copia rota — que es precisamente la pregunta que
/// el material forense existe para responder.
///
/// El mutante que sobrevivía cambiaba el `rename` del sidecar por un `remove_file`: la cuarentena
/// quedaba con journal y copias, todos los tests de E25-H02 seguían verdes, y la pieza que da
/// sentido a las otras dos desaparecía en silencio.
#[test]
fn la_cuarentena_conserva_el_sidecar_de_huellas() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let ws = siembra(root, &["uno", "dos", "tres"]);

    let id = "mut16l-sidecar";
    transaccion_irrecuperable(&ws, root, id);

    // Los bytes EXACTOS del sidecar antes de cuarentenar: es lo que tiene que sobrevivir.
    let sidecar_origen = digests_de(root, id);
    let huellas = std::fs::read(&sidecar_origen)
        .expect("precondición: `backup_originals` debe haber escrito el sidecar de huellas");
    assert!(
        !huellas.is_empty(),
        "precondición: el sidecar de huellas no puede estar vacío (si no, el test sería vacuo)"
    );
    drop(ws);

    // La recuperación falla y manda el material a cuarentena.
    let ws2 = Workspace::open(root).unwrap();
    let _ = ws2.recover();
    assert!(
        !ws2.recovery_pending(),
        "precondición: tras la cuarentena el workspace vuelve a ser escribible"
    );

    let cuarentena = cuarentena_de(root, id);
    assert!(
        cuarentena.is_dir(),
        "precondición: el material debe haber ido a la cuarentena {}",
        cuarentena.display()
    );

    // (1) El sidecar ya no está en su sitio de origen: se MOVIÓ.
    assert!(
        !sidecar_origen.exists(),
        "el sidecar de huellas no puede quedarse en `recovery/`: se mueve entero a la cuarentena"
    );

    // (2) Y está dentro de la cuarentena, byte a byte.
    let dentro = ficheros_bajo(&cuarentena);
    let sidecar_cuarentenado = dentro
        .iter()
        .find(|(nombre, _)| nombre.ends_with(".digests.json"))
        .map(|(_, bytes)| bytes.clone())
        .unwrap_or_else(|| {
            panic!(
                "la cuarentena debe CONSERVAR el sidecar de huellas: es el oráculo con el que se \
                 verifica cada copia (tamaño + blake3), y sin él el árbol cuarentenado no permite \
                 decidir cuál era la copia rota. Ficheros en la cuarentena: {:?}",
                dentro.keys().collect::<Vec<_>>()
            )
        });
    assert_eq!(
        sidecar_cuarentenado, huellas,
        "y lo conserva byte a byte: la cuarentena MUEVE material forense, no lo reescribe"
    );
}

/// **`decisiones §16(l)`, mutante N** — cuarentenar **dos veces el mismo `txnId`** no pisa la
/// cuarentena anterior: la segunda va al primer nombre libre (`<txnId>.2`).
///
/// El bucle que numera existe justamente para esto, y ningún test lo ejercía: todos cuarentenaban
/// un id distinto. El mutante que sobrevivía lo eliminaba, con lo que el segundo incidente escribía
/// **dentro** del árbol del primero — journal contra journal, sidecar contra sidecar — y el
/// material forense de la primera cuarentena quedaba mezclado y parcialmente pisado.
///
/// Repetir el mismo `txnId` no es rebuscado: el id lo elige quien planifica, y una recuperación que
/// falla dos veces sobre el mismo plan (dos reintentos del mismo agente) llega aquí con el mismo
/// nombre.
#[test]
fn cuarentenar_dos_veces_el_mismo_id_no_pisa_la_primera() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let ws = siembra(root, &["uno", "dos", "tres"]);

    let id = "mut16l-repetida";

    // --- Primer incidente ---
    transaccion_irrecuperable(&ws, root, id);
    drop(ws);
    let ws2 = Workspace::open(root).unwrap();
    let _ = ws2.recover();
    let primera = cuarentena_de(root, id);
    assert!(
        primera.is_dir(),
        "precondición: el primer incidente debe estar en {}",
        primera.display()
    );
    let contenido_primera = ficheros_bajo(&primera);
    assert!(
        !contenido_primera.is_empty(),
        "precondición: la primera cuarentena no puede estar vacía"
    );

    // --- Segundo incidente, con el MISMO txnId ---
    transaccion_irrecuperable(&ws2, root, id);
    drop(ws2);
    let ws3 = Workspace::open(root).unwrap();
    let _ = ws3.recover();
    assert!(
        !ws3.recovery_pending(),
        "precondición: el segundo incidente también desencalla el workspace"
    );

    // (1) La primera cuarentena sigue EXACTAMENTE como estaba, byte a byte.
    let despues_primera = ficheros_bajo(&primera);
    let pisados: Vec<&String> = contenido_primera
        .iter()
        .filter(|(nombre, bytes)| despues_primera.get(*nombre) != Some(bytes))
        .map(|(nombre, _)| nombre)
        .collect();
    assert!(
        pisados.is_empty(),
        "la segunda cuarentena del mismo `txnId` no puede tocar la primera: mezclar dos incidentes \
         en un árbol —journal contra journal, sidecar contra sidecar— destruye el material forense \
         que la cuarentena existe para preservar. Ficheros de la primera que cambiaron o \
         desaparecieron: {pisados:?}"
    );

    // (2) Y el segundo incidente está en el primer nombre libre, con material propio.
    let segunda = cuarentena_de(root, &format!("{id}.2"));
    assert!(
        segunda.is_dir(),
        "la segunda cuarentena del mismo `txnId` debe ir al primer nombre libre `{id}.2` en {}",
        segunda.display()
    );
    assert!(
        !ficheros_bajo(&segunda).is_empty(),
        "y llevarse su propio material: una cuarentena vacía no preserva nada"
    );
}
