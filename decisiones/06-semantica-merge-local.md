---
id: 6
titulo: "Semántica de merge local"
estado: "obsoleta"
prioridad: 1
etiquetas: ["git"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
cerrada_en: "2026-07-23"
revisada_en: "2026-07-23"
epica: "E15"
historias: ["E15-H01"]
superada_por: "ARCHITECTURE.md §20.13"
relacionadas: [0, 7]
---

# §6 — Semántica de `merge` local

> **Cerrada por §20 (2026-07-23)**: la migración a workspaces Markdown universales **borra** el crate
> `lodestar-vcs` (E15-H01), no lo deja dormido. Ya no hay `merge` que decidir: si git volviera algún
> día a la superficie, se rediseñaría desde cero. Se conserva el registro histórico.
>
> **Superada antes por §0/§19 (2026-07-22)**: git sale de la superficie de producto; el crate `vcs` (con su
> `merge` a nivel de árbol) se conserva **dormido**, sin fachadas que lo expongan. Esta decisión queda
> como diseño de referencia por si git vuelve.


- **Estado**: `merge` se implementa a **nivel de árbol** (`merge_trees` de libgit2): el vcs **no
  escribe el working tree**; devuelve el `FileMap` resultante para que la workspace lo aplique por el
  único escritor. En conflicto, los ficheros llevan marcadores `<<<<<<< / ======= / >>>>>>>` (los
  detecta `OKF-CONFLICT`) y se deja `MERGE_HEAD` → `repo_state() = Merging` bloquea el commit hasta
  resolver. Fast-forward y up-to-date resueltos aparte.
- **Por qué está abierta**: es una elección de UX. La alternativa sería delegar el merge al binario
  `git` (con su resolución/hooks), lo que rompería el invariante «vcs no escribe el working tree en
  local» y el modelo de único escritor.
- **Qué decidir**: ¿confirmas el merge a nivel de árbol por el único escritor (recomendado, coherente
  con §16) o prefieres delegar en el binario `git`?
- **Recomendación**: confirmar el enfoque actual.
