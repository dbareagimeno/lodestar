//! Grafo: `graph_model` y `neighborhood` (`ARCHITECTURE.md §4.1`, `§4.2`). Port de `buildGraphModel`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::bundle::Bundle;
use crate::types::{Direction, Edge, GraphModel, GraphNode, Neighborhood, RelPath};

/// Construye el `GraphModel` completo: nodos (concepts + ghosts) y aristas (con flag `dangling`).
pub(crate) fn graph_model(bundle: &Bundle) -> GraphModel {
    let a = bundle.analyze();
    let mut node_ids: BTreeSet<RelPath> = a.concepts.iter().cloned().collect();
    let mut edges: Vec<Edge> = Vec::new();

    for src in &a.concepts {
        if let Some(targets) = a.out.get(src) {
            for t in targets {
                let dangling = !bundle.files().contains_key(t);
                if dangling {
                    node_ids.insert(t.clone()); // ghost
                }
                edges.push(Edge {
                    source: src.clone(),
                    target: t.clone(),
                    dangling,
                });
            }
        }
    }

    let nodes = node_ids
        .into_iter()
        .map(|id| node_for(bundle, &id))
        .collect();
    GraphModel { nodes, edges }
}

fn node_for(bundle: &Bundle, id: &RelPath) -> GraphNode {
    let exists = bundle.files().contains_key(id);
    let fm = bundle.parsed(id).and_then(|p| p.fm.as_ref());
    GraphNode {
        id: id.clone(),
        ghost: !exists,
        r#type: fm.and_then(|f| f.r#type.clone()),
        status: fm.and_then(|f| f.status.clone()),
    }
}

/// Subgrafo dirigido a profundidad `depth` desde `root`.
/// `Out`=dependencias salientes · `In`=blast-radius (aristas inversas) · `Both`=ambas.
pub(crate) fn neighborhood(
    bundle: &Bundle,
    root: &RelPath,
    depth: u32,
    dir: Direction,
) -> Neighborhood {
    let a = bundle.analyze();
    // Adyacencia según dirección.
    let out = &a.out;
    let inn = &a.inn;

    let mut visited: BTreeSet<RelPath> = BTreeSet::new();
    let mut queue: VecDeque<(RelPath, u32)> = VecDeque::new();
    queue.push_back((root.clone(), 0));
    visited.insert(root.clone());

    let mut edge_set: BTreeMap<(RelPath, RelPath), bool> = BTreeMap::new();

    while let Some((cur, d)) = queue.pop_front() {
        if d >= depth {
            continue;
        }
        let mut neighbors: Vec<RelPath> = Vec::new();
        if matches!(dir, Direction::Out | Direction::Both) {
            if let Some(ts) = out.get(&cur) {
                for t in ts {
                    let dangling = !bundle.files().contains_key(t);
                    edge_set.insert((cur.clone(), t.clone()), dangling);
                    neighbors.push(t.clone());
                }
            }
        }
        if matches!(dir, Direction::In | Direction::Both) {
            if let Some(ss) = inn.get(&cur) {
                for s in ss {
                    edge_set.insert((s.clone(), cur.clone()), false);
                    neighbors.push(s.clone());
                }
            }
        }
        for nb in neighbors {
            if visited.insert(nb.clone()) {
                queue.push_back((nb, d + 1));
            }
        }
    }

    let nodes = visited.iter().map(|id| node_for(bundle, id)).collect();
    let edges = edge_set
        .into_iter()
        .map(|((source, target), dangling)| Edge {
            source,
            target,
            dangling,
        })
        .collect();
    Neighborhood {
        root: root.clone(),
        nodes,
        edges,
    }
}
