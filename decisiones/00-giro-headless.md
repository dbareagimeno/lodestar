---
id: 0
titulo: "Giro a motor headless de integridad semántica"
estado: "ratificada"
prioridad: 2
etiquetas: ["arquitectura", "mcp", "transacciones", "git"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-22"
cerrada_en: "2026-07-22"
revisada_en: "2026-07-23"
epica: "E9"
historias: ["E9-H01", "E9-H02", "E9-H04"]
subpuntos: ["D0", "D1", "D3", "D4", "D5", "D6", "D-CheckCode", "D-check"]
relacionadas: [3, 6, 8]
---

# §0 — Giro a motor headless de integridad semántica

- **Contexto**: `docs/history/REFACTOR.md` redefine Lodestar como **motor headless de integridad semántica**
  (busca/comprende/valida/modifica conocimiento vía cambios planificados y recuperables, sin editor,
  sin GUI y sin git). Propuesta de diseño en `docs/history/REFACTOR_DISENO_PROPUESTA.md`; diseño ratificado en
  **`ARCHITECTURE.md §19`** (supersede §13 en superficie de producto). Descomposición en
  `requirements/epica-09-*.md` … `epica-14-*.md`.
- **Sub-decisiones cerradas** (puerta 1 de `/planificar`):
  - **D0** — Adenda como **§19 nueva** + nota de cabecera en §13 ("superada en superficie; crate `vcs`
    y mecánica §13.2–§13.6 conservados como dormidos") + anotación en §10 (filas de git ciertas sobre
    el crate, exposición revertida).
  - **D1** — Capas nuevas: **Opción C (híbrido)** — mecánica transaccional en `lodestar-workspace`
    (único escritor); crate nuevo **`lodestar-app`** fino como servicios de caso de uso que comparten
    mcp/cli.
  - **D3** — Envelope en `lodestar-app`; **códigos de error** en `core::types`.
  - **D4** — Config migra a **`.lodestar/config.yaml`** YAML unificado
    (`workspace.{writableRoots,referenceRoots,ignored}` + `gate` + `transactions`; `identity` dormida).
  - **D5** — `.lodestar/{config,schema}.yaml` + `templates/` **versionados**; `.lodestar/runtime/` +
    `index.db` **gitignored**; `WorkspaceRevision` **excluye todo `.lodestar/`**.
  - **D6** — (a) generadores **solo CLI** + auto-regen dentro de `change_apply`; (b) transporte
    **stdio + `outputSchema` vía `schemars`**, `rmcp` **diferido**.
  - **D-CheckCode** — Familias estáticas acotadas de `CheckCode` (`SCHEMA-REQFIELD`, `SCHEMA-STATUS`,
    `REL-TARGET`, `REL-CARD`, `REL-TYPE`), i18n keyed por código.
  - **D-check** — `lodestar check` sigue como puerta de CI sobre el working tree;
    `--staged`/`--rev`/`--range` **diferidos** con el crate `vcs` dormido.
- **Confirmadas** (se declaran en §19, sin criterio adicional): `core::schema` en el core **puro**;
  modelo transaccional en `workspace`; reutilización de `OkfDiff`/`blast_radius`/`neighborhood`/
  `Mutation`/`RelPath`/blake3; seguridad §14 (simplificada al no haber git/red/exec en la superficie).
- **Cierres colaterales**: la parte de **git** de este documento queda **superada por §19** ([`§6`](06-semantica-merge-local.md) semántica
  de `merge` local, y la exposición de git en fachadas): el crate `vcs` se conserva dormido pero su
  superficie no se implementa en v2. [`§3`](03-transporte-mcp-rmcp.md) (rmcp) se reafina a "**stdio + `outputSchema`, `rmcp` diferido**".
