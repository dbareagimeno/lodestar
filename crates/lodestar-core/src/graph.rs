//! Grafo: `graph_model` y `neighborhood` (`ARCHITECTURE.md §4.1`, `§4.2`). Port de `buildGraphModel`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::document_set::DocumentSet;
use crate::types::{Analysis, Direction, Edge, GraphModel, GraphNode, Neighborhood, RelPath};

/// Destinos **internos** salientes de `p` (documentos y fantasmas), en orden de aparición y sin
/// repetir: la adyacencia del grafo derivada de [`Analysis::outgoing`] (E17-H04).
///
/// Es el filtro que separa la lista de enlaces del grafo: `outgoing` lleva **todos** los enlaces
/// del documento (externos, anchors, ficheros del proyecto, enlaces rotos a código), pero solo los
/// que conectan con otro documento —[`crate::types::LinkTarget::internal_path`], la única
/// definición— son aristas. La deduplicación mantiene el grafo como un conjunto de aristas: enlazar
/// dos veces al mismo documento no duplica la arista (sí duplica la referencia entrante, que es
/// una lista de enlaces, no de vecinos).
pub(crate) fn salientes(a: &Analysis, p: &RelPath) -> Vec<RelPath> {
    let mut vistos: BTreeSet<RelPath> = BTreeSet::new();
    let mut out: Vec<RelPath> = Vec::new();
    for link in a.outgoing.get(p).into_iter().flatten() {
        if let Some(t) = link.target.internal_path() {
            if vistos.insert(t.clone()) {
                out.push(t.clone());
            }
        }
    }
    out
}

/// Orígenes **distintos** que enlazan a `p`, derivados de [`Analysis::incoming`] (la inversa de
/// [`salientes`]). Deduplicados por la misma razón: un origen que enlaza dos veces es un solo
/// vecino en el grafo.
pub(crate) fn entrantes(a: &Analysis, p: &RelPath) -> Vec<RelPath> {
    let mut vistos: BTreeSet<RelPath> = BTreeSet::new();
    a.incoming
        .get(p)
        .into_iter()
        .flatten()
        .filter(|r| vistos.insert(r.from.clone()))
        .map(|r| r.from.clone())
        .collect()
}

/// Construye el `GraphModel` completo: nodos (documentos + ghosts) y aristas (con flag `dangling`).
pub(crate) fn graph_model(doc_set: &DocumentSet) -> GraphModel {
    let a = doc_set.analyze();
    let mut node_ids: BTreeSet<RelPath> = a.documents.iter().cloned().collect();
    let mut edges: Vec<Edge> = Vec::new();

    for src in &a.documents {
        for t in salientes(a, src) {
            // E16-H02: se retiró el quirk del prototipo (`buildGraphModel:1850`) que
            // descartaba las aristas/nodos a `index.md`/`log.md`. Todo destino INTERNO es una
            // arista; un fichero del proyecto, una URI externa o un enlace roto a algo que no
            // sería un documento Markdown no lo son (E17-H04, `§20.7`).
            let dangling = !doc_set.files().contains_key(&t);
            if dangling {
                node_ids.insert(t.clone()); // ghost
            }
            edges.push(Edge {
                source: src.clone(),
                target: t,
                dangling,
            });
        }
    }

    let nodes = node_ids
        .into_iter()
        .map(|id| node_for(doc_set, &id))
        .collect();
    GraphModel { nodes, edges }
}

/// Construye el [`GraphNode`] de `id`: su **título derivado** (E16-H03) y `ghost` si no hay
/// fichero para ese path.
///
/// Es la **única** definición de "qué es un nodo del grafo" (invariante #3, "una sola verdad
/// computada"): tanto `graph_model`/`neighborhood` del core como `App::graph_query` de la fachada
/// la reusan (esta última vía [`DocumentSet::node`]) en vez de reimplementar el criterio.
///
/// Un nodo **fantasma** no tiene ni frontmatter ni cuerpo del que derivar título, así que la
/// cadena de [`crate::model::derived_title`] se resuelve por su último eslabón: el nombre del
/// fichero sin `.md`.
pub fn node_for(doc_set: &DocumentSet, id: &RelPath) -> GraphNode {
    let parsed = doc_set.parsed(id);
    let title = match parsed {
        Some(p) => crate::model::derived_title(p.frontmatter.as_ref(), &p.body, id),
        None => crate::model::derived_title(None, "", id),
    };
    GraphNode {
        id: id.clone(),
        title,
        ghost: !doc_set.files().contains_key(id),
    }
}

/// Subgrafo dirigido a profundidad `depth` desde `root`.
/// `Out`=dependencias salientes · `In`=blast-radius (aristas inversas) · `Both`=ambas.
pub(crate) fn neighborhood(
    doc_set: &DocumentSet,
    root: &RelPath,
    depth: u32,
    dir: Direction,
) -> Neighborhood {
    let a = doc_set.analyze();

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
            for t in salientes(a, &cur) {
                let dangling = !doc_set.files().contains_key(&t);
                edge_set.insert((cur.clone(), t.clone()), dangling);
                neighbors.push(t);
            }
        }
        if matches!(dir, Direction::In | Direction::Both) {
            for s in entrantes(a, &cur) {
                edge_set.insert((s.clone(), cur.clone()), false);
                neighbors.push(s);
            }
        }
        for nb in neighbors {
            if visited.insert(nb.clone()) {
                queue.push_back((nb, d + 1));
            }
        }
    }

    let nodes = visited.iter().map(|id| node_for(doc_set, id)).collect();
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

// ---------------------------------------------------------------------------
// E11-H02: operaciones estructurales del grafo (`path_between`/`cycles`/`components`).
//
// Todas parten de la MISMA representación que `graph_model`/`neighborhood` (invariante #3): el
// `GraphModel` completo, con sus nodos (documentos + ghosts) y sus aristas. No se construye ningún
// grafo paralelo. (El quirk de ficheros reservados que aquí se filtraba murió con E16-H02.)
// ---------------------------------------------------------------------------

/// Adyacencia **dirigida** derivada de un [`GraphModel`]: conjunto de nodos y, por cada nodo, sus
/// destinos salientes. Reusa las aristas ya computadas por `graph_model` (misma inclusión de
/// ghosts que el resto del módulo).
fn adyacencia_dirigida(
    model: &GraphModel,
) -> (BTreeSet<RelPath>, BTreeMap<RelPath, BTreeSet<RelPath>>) {
    let nodes: BTreeSet<RelPath> = model.nodes.iter().map(|n| n.id.clone()).collect();
    let mut out: BTreeMap<RelPath, BTreeSet<RelPath>> = BTreeMap::new();
    for e in &model.edges {
        out.entry(e.source.clone())
            .or_default()
            .insert(e.target.clone());
    }
    (nodes, out)
}

/// Camino más corto **dirigido** de `a` a `b` siguiendo aristas salientes, incluyendo ambos
/// extremos (`[a, .., b]`). Devuelve `vec![]` si no hay camino o si algún extremo no es un nodo del
/// grafo — **nunca** un error. BFS sobre la adyacencia dirigida del `GraphModel`; los vecinos se
/// recorren en orden de `RelPath` (`BTreeSet`), así el camino elegido entre varios igual de cortos
/// es determinista.
pub(crate) fn path_between(doc_set: &DocumentSet, a: &RelPath, b: &RelPath) -> Vec<RelPath> {
    let model = graph_model(doc_set);
    let (nodes, out) = adyacencia_dirigida(&model);
    if !nodes.contains(a) || !nodes.contains(b) {
        return Vec::new();
    }
    if a == b {
        return vec![a.clone()];
    }

    let mut prev: BTreeMap<RelPath, RelPath> = BTreeMap::new();
    let mut visited: BTreeSet<RelPath> = BTreeSet::new();
    let mut queue: VecDeque<RelPath> = VecDeque::new();
    visited.insert(a.clone());
    queue.push_back(a.clone());

    while let Some(cur) = queue.pop_front() {
        if let Some(targets) = out.get(&cur) {
            for t in targets {
                if visited.insert(t.clone()) {
                    prev.insert(t.clone(), cur.clone());
                    if t == b {
                        // Reconstruye el camino de `b` a `a` y lo invierte.
                        let mut path = vec![b.clone()];
                        let mut node = b.clone();
                        while &node != a {
                            node = prev[&node].clone();
                            path.push(node.clone());
                        }
                        path.reverse();
                        return path;
                    }
                    queue.push_back(t.clone());
                }
            }
        }
    }
    Vec::new()
}

/// Ciclos **dirigidos** del grafo de enlaces. Cada ciclo es el conjunto de nodos de una componente
/// fuertemente conexa (SCC) no trivial: una SCC de más de un nodo, o un único nodo con arista a sí
/// mismo. Los nodos acíclicos no aparecen. Los ciclos, y los nodos dentro de cada uno, se devuelven
/// ordenados por `RelPath` (determinista).
pub(crate) fn cycles(doc_set: &DocumentSet) -> Vec<Vec<RelPath>> {
    let model = graph_model(doc_set);
    let (nodes, out) = adyacencia_dirigida(&model);

    let mut result: Vec<Vec<RelPath>> = Vec::new();
    for scc in tarjan_sccs(&nodes, &out) {
        let es_ciclo = scc.len() > 1
            || (scc.len() == 1 && out.get(&scc[0]).is_some_and(|ts| ts.contains(&scc[0])));
        if es_ciclo {
            let mut c = scc;
            c.sort();
            result.push(c);
        }
    }
    result.sort();
    result
}

/// Componentes fuertemente conexas (Tarjan, iterativo para no arriesgar desbordamiento de pila en
/// grafos grandes). Devuelve cada SCC como un `Vec<RelPath>` (sin ordenar internamente; el llamante
/// ordena si lo necesita). El orden de recorrido de nodos y vecinos es por `RelPath`, así que la
/// partición en SCCs es determinista.
fn tarjan_sccs(
    nodes: &BTreeSet<RelPath>,
    out: &BTreeMap<RelPath, BTreeSet<RelPath>>,
) -> Vec<Vec<RelPath>> {
    let mut index_of: BTreeMap<RelPath, usize> = BTreeMap::new();
    let mut lowlink: BTreeMap<RelPath, usize> = BTreeMap::new();
    let mut on_stack: BTreeSet<RelPath> = BTreeSet::new();
    let mut tarjan_stack: Vec<RelPath> = Vec::new();
    let mut next_index: usize = 0;
    let mut sccs: Vec<Vec<RelPath>> = Vec::new();

    for start in nodes {
        if index_of.contains_key(start) {
            continue;
        }
        // Pila de trabajo explícita: (nodo, índice del próximo vecino a visitar).
        index_of.insert(start.clone(), next_index);
        lowlink.insert(start.clone(), next_index);
        next_index += 1;
        tarjan_stack.push(start.clone());
        on_stack.insert(start.clone());
        let mut work: Vec<(RelPath, usize)> = vec![(start.clone(), 0)];

        while let Some((v, i)) = work.last().cloned() {
            // Próximo vecino de `v` (orden `RelPath`), si queda alguno.
            let siguiente = out.get(&v).and_then(|ts| ts.iter().nth(i)).cloned();
            match siguiente {
                Some(w) => {
                    work.last_mut().expect("work no vacío").1 = i + 1;
                    if !index_of.contains_key(&w) {
                        index_of.insert(w.clone(), next_index);
                        lowlink.insert(w.clone(), next_index);
                        next_index += 1;
                        tarjan_stack.push(w.clone());
                        on_stack.insert(w.clone());
                        work.push((w, 0));
                    } else if on_stack.contains(&w) {
                        let iw = index_of[&w];
                        let lv = lowlink[&v];
                        lowlink.insert(v.clone(), lv.min(iw));
                    }
                }
                None => {
                    // Todos los vecinos visitados: si `v` es raíz de una SCC, extráela.
                    if lowlink[&v] == index_of[&v] {
                        let mut scc: Vec<RelPath> = Vec::new();
                        loop {
                            let w = tarjan_stack.pop().expect("tarjan_stack no vacío");
                            on_stack.remove(&w);
                            let es_v = w == v;
                            scc.push(w);
                            if es_v {
                                break;
                            }
                        }
                        sccs.push(scc);
                    }
                    work.pop();
                    // Propaga el lowlink de `v` a su padre en la pila de trabajo.
                    if let Some((parent, _)) = work.last() {
                        let lp = lowlink[parent];
                        let lv = lowlink[&v];
                        lowlink.insert(parent.clone(), lp.min(lv));
                    }
                }
            }
        }
    }
    sccs
}

/// Componentes conexas por conectividad **no dirigida** del grafo de enlaces. Cada componente es el
/// conjunto de sus nodos (documentos + ghosts). Los nodos aislados forman su propia componente
/// unitaria. BFS sobre la adyacencia simetrizada; componentes y nodos ordenados por `RelPath`.
pub(crate) fn components(doc_set: &DocumentSet) -> Vec<Vec<RelPath>> {
    let model = graph_model(doc_set);
    let nodes: BTreeSet<RelPath> = model.nodes.iter().map(|n| n.id.clone()).collect();

    // Adyacencia no dirigida: cada arista conecta en ambos sentidos.
    let mut adj: BTreeMap<RelPath, BTreeSet<RelPath>> = BTreeMap::new();
    for n in &nodes {
        adj.entry(n.clone()).or_default();
    }
    for e in &model.edges {
        adj.entry(e.source.clone())
            .or_default()
            .insert(e.target.clone());
        adj.entry(e.target.clone())
            .or_default()
            .insert(e.source.clone());
    }

    let mut visited: BTreeSet<RelPath> = BTreeSet::new();
    let mut comps: Vec<Vec<RelPath>> = Vec::new();
    for start in &nodes {
        if visited.contains(start) {
            continue;
        }
        let mut comp: BTreeSet<RelPath> = BTreeSet::new();
        let mut queue: VecDeque<RelPath> = VecDeque::new();
        visited.insert(start.clone());
        queue.push_back(start.clone());
        while let Some(cur) = queue.pop_front() {
            comp.insert(cur.clone());
            if let Some(ns) = adj.get(&cur) {
                for nb in ns {
                    if visited.insert(nb.clone()) {
                        queue.push_back(nb.clone());
                    }
                }
            }
        }
        comps.push(comp.into_iter().collect());
    }
    comps.sort();
    comps
}
