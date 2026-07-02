//! Arnés diferencial JS-vs-Rust (E1-H18, `ARCHITECTURE.md §12`).
//!
//! La red de seguridad de la paridad: ejecuta las funciones PURAS del prototipo (`prototype/harness/`,
//! vía node) como ORÁCULO y compara su salida normalizada con `lodestar-core` sobre las mismas
//! fixtures. Cubre analyze (conformidad/links/orphans/dangling/in_index), query, generadores y grafo.
//!
//! `OKF-CONFLICT` (adición ratificada del core ausente en el prototipo) se filtra antes de comparar.
//! Los mensajes de conformidad NO se comparan (la UI localiza por `code`, §12); se comparan `level:code`.
//! Si `node` no está en el PATH, el test se SALTA (no falla) para no romper CI sin node.

use std::path::PathBuf;
use std::process::Command;

use lodestar_core::types::{FileMap, RelPath, Severity};
use lodestar_core::Bundle;
use serde_json::{json, Value};

fn fm(pairs: &[(&str, &str)]) -> FileMap {
    pairs
        .iter()
        .map(|(p, c)| (RelPath::new(p).unwrap(), (*c).to_string()))
        .collect()
}

fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Pass => "pass",
        Severity::Info => "info",
        Severity::Warn => "warn",
        Severity::Err => "err",
    }
}

fn sorted_paths(v: &[RelPath]) -> Vec<String> {
    let mut out: Vec<String> = v.iter().map(|p| p.as_str().to_string()).collect();
    out.sort();
    out
}

/// Construye la salida normalizada del lado Rust, idéntica en forma a `proto.mjs::analyzeFixture`.
fn rust_normalize(files: &FileMap, queries: &[&str]) -> Value {
    let bundle = Bundle::from_files(files.clone());
    let a = bundle.analyze();

    // out / inn con los vectores ordenados.
    let mut out = serde_json::Map::new();
    for (p, ts) in &a.out {
        out.insert(p.as_str().to_string(), json!(sorted_paths(ts)));
    }
    let mut inn = serde_json::Map::new();
    for (p, ss) in &a.inn {
        inn.insert(p.as_str().to_string(), json!(sorted_paths(ss)));
    }

    // perFile como listas ordenadas de "level:code", filtrando OKF-CONFLICT (ausente en el prototipo).
    let mut per_file = serde_json::Map::new();
    let mut hard_fail = 0usize;
    let mut warn_count = 0usize;
    for (p, checks) in &a.per_file {
        let kept: Vec<&lodestar_core::types::Check> = checks
            .iter()
            .filter(|c| c.code != lodestar_core::types::CheckCode::OkfConflict)
            .collect();
        if kept.iter().any(|c| c.level == Severity::Err) {
            hard_fail += 1;
        }
        warn_count += kept.iter().filter(|c| c.level == Severity::Warn).count();
        let mut codes: Vec<String> = kept
            .iter()
            .map(|c| format!("{}:{}", severity_str(c.level), c.code.as_str()))
            .collect();
        codes.sort();
        per_file.insert(p.as_str().to_string(), json!(codes));
    }

    // query.
    let mut query = serde_json::Map::new();
    for q in queries {
        query.insert(q.to_string(), json!(sorted_paths(&bundle.query(q))));
    }

    // generadores.
    let gen_index_root = bundle
        .gen_index("")
        .writes
        .into_iter()
        .next()
        .map(|(_, c)| c)
        .unwrap_or_default();

    let tag_mut = bundle.gen_tag_indexes();
    let mut tag_writes = serde_json::Map::new();
    for (p, c) in &tag_mut.writes {
        tag_writes.insert(p.as_str().to_string(), json!(c));
    }
    let tag_deletes = sorted_paths(&tag_mut.deletes);

    // grafo (nodos {id,ghost} ordenados por id; aristas {source,target,dangling} por (source,target)).
    let g = bundle.graph_model();
    let mut nodes: Vec<Value> = g
        .nodes
        .iter()
        .map(|n| json!({ "id": n.id.as_str(), "ghost": n.ghost }))
        .collect();
    nodes.sort_by(|x, y| x["id"].as_str().cmp(&y["id"].as_str()));
    let mut edges: Vec<Value> = g
        .edges
        .iter()
        .map(|e| {
            json!({ "source": e.source.as_str(), "target": e.target.as_str(), "dangling": e.dangling })
        })
        .collect();
    edges.sort_by(|x, y| {
        (x["source"].as_str(), x["target"].as_str())
            .cmp(&(y["source"].as_str(), y["target"].as_str()))
    });

    let mut in_index: Vec<String> = a.in_index.iter().map(|p| p.as_str().to_string()).collect();
    in_index.sort();

    json!({
        "concepts": sorted_paths(&a.concepts),
        "out": Value::Object(out),
        "inn": Value::Object(inn),
        "inIndex": in_index,
        "dangling": sorted_paths(&a.dangling),
        "orphans": sorted_paths(&a.orphans),
        "perFile": Value::Object(per_file),
        "hardFail": hard_fail,
        "warnCount": warn_count,
        "query": Value::Object(query),
        "genIndexRoot": gen_index_root,
        "genTagIndexes": { "writes": Value::Object(tag_writes), "deletes": tag_deletes },
        "graph": { "nodes": nodes, "edges": edges },
    })
}

/// Localiza `prototype/harness/run.mjs` desde el manifest del crate.
fn harness_runner() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../prototype/harness/run.mjs")
        .canonicalize()
        .expect("ruta del arnés existe")
}

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Ejecuta el oráculo JS sobre el fixture y devuelve su salida normalizada.
fn proto_normalize(name: &str, files: &FileMap, queries: &[&str]) -> Value {
    let files_obj: serde_json::Map<String, Value> = files
        .iter()
        .map(|(p, c)| (p.as_str().to_string(), Value::String(c.clone())))
        .collect();
    let input = json!({ "files": files_obj, "queries": queries });
    let tmp = std::env::temp_dir().join(format!("lodestar-diff-{name}.json"));
    std::fs::write(&tmp, serde_json::to_vec(&input).unwrap()).unwrap();
    let out = Command::new("node")
        .arg(harness_runner())
        .arg(&tmp)
        .output()
        .expect("node ejecuta el arnés");
    let _ = std::fs::remove_file(&tmp);
    assert!(
        out.status.success(),
        "el arnés JS falló en «{name}»: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "salida del arnés no es JSON en «{name}»: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// Compara Rust vs prototipo sobre un fixture; si difieren, señala la primera clave divergente.
fn assert_parity(name: &str, files: &FileMap, queries: &[&str]) {
    let rust = rust_normalize(files, queries);
    let proto = proto_normalize(name, files, queries);
    if rust != proto {
        let (ro, po) = (rust.as_object().unwrap(), proto.as_object().unwrap());
        for k in ro.keys() {
            if ro.get(k) != po.get(k) {
                panic!(
                    "DIVERGENCIA en «{name}» → clave «{k}»:\n  rust  = {}\n  proto = {}",
                    serde_json::to_string(&ro[k]).unwrap(),
                    serde_json::to_string(po.get(k).unwrap_or(&Value::Null)).unwrap(),
                );
            }
        }
        panic!("DIVERGENCIA en «{name}» (estructura distinta)");
    }
}

const PROBES: &[&str] = &[
    "",
    "is:orphan",
    "is:invalid",
    "is:linked",
    "is:reserved",
    "has:tags",
    "no:description",
    "type:concept",
    "type=metric",
    "status:draft",
    "-is:orphan",
    "body:resumen",
    "alfa",
    "\"primer concept\"",
];

#[test]
fn diferencial_fixture_conforme() {
    if !node_available() {
        eprintln!("SKIP diferencial: `node` no está en el PATH");
        return;
    }
    assert_parity("conforme", &lodestar_fixtures::conformant(), PROBES);
}

#[test]
fn diferencial_fixture_con_issues() {
    if !node_available() {
        eprintln!("SKIP diferencial: `node` no está en el PATH");
        return;
    }
    // `OKF-CONFLICT` es una adición ratificada del core ausente en el prototipo; su fichero se excluye
    // del oráculo (se testea aparte en `core.rs::conformidad_dispara_cada_codigo`). Si no, el core
    // marca `conflicto.md` como `is:invalid` y el prototipo no → divergencia esperada, no un bug.
    let mut files = lodestar_fixtures::with_issues();
    files.remove(&RelPath::new("conflicto.md").unwrap());
    assert_parity("issues", &files, PROBES);
}

#[test]
fn diferencial_sintetico() {
    if !node_available() {
        eprintln!("SKIP diferencial: `node` no está en el PATH");
        return;
    }
    assert_parity("sintetico", &lodestar_fixtures::synthetic(8), PROBES);
}

#[test]
fn diferencial_tags_numericos_y_nfc() {
    if !node_available() {
        eprintln!("SKIP diferencial: `node` no está en el PATH");
        return;
    }
    // Ejercita el orden numérico de items (fix E) y el slug NFC (fix D) en los índices de tags.
    let files = fm(&[
        ("index.md", "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n"),
        (
            "doc-2.md",
            "---\ntype: Nota\ntitle: D2\ndescription: d\ntags: [café, serie]\n---\n\n# H\n",
        ),
        (
            "doc-10.md",
            "---\ntype: Nota\ntitle: D10\ndescription: d\ntags: [serie]\n---\n\n# H\n",
        ),
        (
            "doc-1.md",
            "---\ntype: Nota\ntitle: D1\ndescription: d\ntags: [serie]\n---\n\n# H\n",
        ),
        (
            "cafe-nfc.md",
            "---\ntype: Nota\ntitle: C\ndescription: d\ntags: [cafe\u{0301}]\n---\n\n# H\n",
        ),
    ]);
    assert_parity("tags", &files, PROBES);
}

#[test]
fn diferencial_grafo_reservados_y_relativos() {
    if !node_available() {
        eprintln!("SKIP diferencial: `node` no está en el PATH");
        return;
    }
    // Ejercita la exclusión de aristas a reservados (fix F), enlaces relativos y colgantes.
    let files = fm(&[
        ("index.md", "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [A](a.md)\n"),
        (
            "a.md",
            "---\ntype: Concept\ntitle: A\ndescription: d\n---\n\n# H\n\n[idx](/index.md), [b](/b.md), [falta](/no.md), [rel](./c.md)\n",
        ),
        ("b.md", "---\ntype: Concept\ntitle: B\ndescription: d\n---\n\n# H\n\nvuelve a [A](/a.md)\n"),
        ("c.md", "---\ntype: Concept\ntitle: C\ndescription: d\n---\n\n# H\n"),
        ("log.md", "# Update Log\n\n## 2024-01-01\n* nota\n"),
    ]);
    assert_parity("grafo", &files, PROBES);
}

#[test]
fn diferencial_campo_null_y_extras() {
    if !node_available() {
        eprintln!("SKIP diferencial: `node` no está en el PATH");
        return;
    }
    // Campo extra con valor null (fix C: has: lo cuenta como presente) + claves de productor.
    let files = fm(&[
        ("index.md", "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n"),
        (
            "x.md",
            "---\ntype: Nota\ntitle: X\ndescription: d\ncustom:\nzulu: 1\nalpha: 2\n---\n\n# H\n",
        ),
    ]);
    let probes = &["has:custom", "has:zulu", "no:custom", "custom:1", "zulu:1"];
    assert_parity("nullextra", &files, probes);
}
