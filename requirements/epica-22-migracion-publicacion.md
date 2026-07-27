# E22 — Migración de repos OKF y limpieza pública

> **Fase**: `§20.14` PR 11 (`REFACTOR_PHASE_2 §Fase 14` y `§Fase 1`).
> **Objetivo de la épica**: cerrar la migración de cara al usuario. Un repo OKF existente sigue
> siendo Markdown válido —sin migración destructiva— y se le ofrece un diagnóstico opcional
> `migrate-from-okf --dry-run`; el README y la documentación de producto dejan de describir OKF, git
> y la UI; y se declara la versión **incompatible** v0.3.0.
> Referencias maestras: `ARCHITECTURE.md §20.13`, `§20.1`; `docs/REFACTOR_PHASE_2 §Fase 14`,
> `§Definición final del producto`.

**Principio rector**: `§Fase 14` — *"no modificar destructivamente documentos anteriores"*. `type:
decision`/`status: accepted` se conservan **exactamente** y siguen siendo metadata consultable;
`index.md` y los índices de tags sobreviven como documentos normales; `okf_version` se conserva como
metadata desconocida y se **ofrece como recomendación de limpieza, no como error**.

---

### E22-H01 — `migrate-from-okf --dry-run`

- **Objetivo**: un comando de diagnóstico que detecta convenciones OKF legadas y las reporta **sin
  modificar ningún fichero**.
- **Referencias**: `ARCHITECTURE.md §20.13` · `REFACTOR_PHASE_2 §Fase 14 (Comando de diagnóstico
  opcional)` · `crates/lodestar-cli/src/main.rs`, `commands.rs`.
- **Alcance**:
  - Subcomando CLI `migrate-from-okf` (solo `--dry-run` en v0.3; sin `--dry-run` puede ser error de
    uso o alias del dry-run — decidir). Recorre el workspace y reporta lo que `§Fase 14` lista:
    `index.md` raíz, índices anidados, metadata `okf_version`, índices de tags generados.
  - **No modifica ningún fichero** (invariante de la historia). La salida es informativa, con las
    recomendaciones de `§Fase 14` («trata los índices como navegación opcional», «elimina
    `okf_version` cuando convenga», «revisa los índices de tags antes de borrarlos»).
  - Reusa el descubrimiento (E15) y el parseo (E16) — no reimplementa detección de frontmatter.
- **Criterios de aceptación**:
  - **Dado** un workspace con `index.md` raíz + `okf_version` + un índice de tags, **Cuando** se
    corre `migrate-from-okf --dry-run`, **Entonces** los detecta y los lista → `dry_run_detecta`.
  - **Dado** ese workspace, **Cuando** se corre, **Entonces** **ningún fichero cambia** (hash del
    árbol idéntico antes/después) → `dry_run_no_modifica`.
  - **Dado** un workspace **sin** convenciones OKF, **Cuando** se corre, **Entonces** reporta que no
    hay nada que migrar, exit 0 → `dry_run_workspace_limpio`.
- **Dependencias**: E21 completa.
- **Pruebas**: `crates/lodestar-cli/tests/`.
- **Frontera (mcp.yml)**: no (es CLI).

### E22-H02 — Documentación de producto: README y arquitectura

- **Objetivo**: el README y la doc de producto describen el producto de `§Definición final`, no OKF.
- **Referencias**: `ARCHITECTURE.md §20.1`, `§20` (entero) · `REFACTOR_PHASE_2 §Definición final del
  producto` · `README.md`, `CHANGELOG.md`.
- **Alcance**:
  - Reescribir `README.md`: hoy abre con *"Motor headless … para bases de conocimiento en formato
    **OKF**"* y describe la app de escritorio congelada y el crate `vcs` dormido — ambos **retirados**
    en la migración. La definición nueva es la de `§20.1`: *"un motor local y transaccional para que
    agentes de IA descubran, consulten, comprendan y modifiquen de forma segura una red arbitraria de
    documentos Markdown"*. Actualizar el listado de tools (10, con `metadata_inspect`), los comandos
    (`check`/`reindex`/`migrate-from-okf`, sin `init`/`index`/`tags`/`export`/`import`), y el arranque
    (`cd proyecto && lodestar-mcp`).
  - Retirar de `README`/docs las menciones a OKF como formato obligatorio, a la app de escritorio y a
    git como capacidad (el crate `vcs` ya no existe).
  - **Retirar los fixtures OKF heredados** de `crates/lodestar-fixtures/src/lib.rs` (`conformant`,
    `with_issues` con `okf_version`, `synthetic`) **si ya no tienen consumidores** tras E16–E21 — o
    dejarlos si algún test de paridad los usa; verificar.
- **Criterios de aceptación**:
  - Estructural: `README.md` no describe OKF como formato obligatorio ni la app de escritorio como
    parte del producto → checklist.
  - **Dado** el README, **Cuando** se lee el bloque de arranque, **Entonces** documenta
    `lodestar-mcp` sin argumentos desde el cwd → checklist.
- **Dependencias**: E22-H01.
- **Pruebas**: — (documentación; verificación por checklist/juez).
- **Frontera (mcp.yml)**: no.

### E22-H03 — Publicación de la versión incompatible v0.3.0

- **Objetivo**: declarar v0.3.0, incompatible con v0.2.x, en el versionado y el CHANGELOG.
- **Referencias**: `ARCHITECTURE.md §20.1` (v0.3.0 incompatible) · `REFACTOR_PHASE_2 §Orden de
  implementación PR 11` · `Cargo.toml` (`workspace.package.version`), `CHANGELOG.md`, `RELEASING.md`.
- **Alcance**:
  - Bump de `workspace.package.version` `0.2.0` → `0.3.0` y de las `workspace.dependencies` internas
    (`lodestar-core = { …, version = "0.3.0" }`, etc.).
  - Entrada de `CHANGELOG.md` para v0.3.0 que resuma el giro: modelo Markdown universal, retirada de
    OKF/git/UI/generadores, lenguaje de consulta tipado, `metadata_inspect`, y la **incompatibilidad**
    con v0.2.x (el store se reconstruye, la superficie MCP cambia).
  - `IMPLEMENTATION_STATUS.md`: marcar la migración E15–E22 como completa.
- **Criterios de aceptación**:
  - `cargo build --workspace` con la versión nueva compila y `cargo test --workspace` pasa →
    checklist.
  - `CHANGELOG.md` tiene una entrada v0.3.0 que declara la incompatibilidad → checklist.
- **Dependencias**: E22-H01, H02.
- **Pruebas**: la suite entera (regresión).
- **Frontera (mcp.yml)**: no.

### E22-H04 — Verificación end-to-end de la migración completa

- **Objetivo**: demostrar, sobre un workspace real, que los **criterios de aceptación finales** de
  `REFACTOR_PHASE_2` se cumplen de punta a punta por la superficie MCP/CLI.
- **Referencias**: `REFACTOR_PHASE_2 §Criterios de aceptación`, `§Resultado esperado`.
- **Alcance**: un test e2e (o un guion reproducible en `crates/lodestar-mcp/tests/`) que, sobre un
  proyecto arbitrario **sin** `.lodestar/`/`index.md`/frontmatter obligatorio, recorra el flujo del
  documento: descubrir → `workspace_status` → `knowledge_search` (con `where`) → `knowledge_get` →
  `metadata_inspect` → `graph_query` → `change_plan` (una selección masiva) → `change_apply` →
  `knowledge_check` → `change_revert`. Cada paso verificado contra el criterio del documento.
- **Criterios de aceptación**: los 29 de `§Criterios de aceptación` cubiertos por el guion, con foco
  en los que aún no se probaron e2e (selección masiva por consulta, `move_document` con reescritura,
  `metadata_inspect`, la equivalencia `where`/`filter`).
- **Dependencias**: E21 completa, E22-H01…H03.
- **Pruebas**: `crates/lodestar-mcp/tests/e2e_migracion.rs` (nuevo).
- **Frontera (mcp.yml)**: no.

---

## Orden de construcción

```
H01 (migrate-from-okf) ─→ H02 (docs) ─→ H03 (v0.3.0) ─→ H04 (e2e final)
```

## Criterio de salida

Un repo OKF existente se abre sin migración y `migrate-from-okf --dry-run` lo diagnostica sin tocarlo;
el README describe el producto de Markdown universal; v0.3.0 está declarada incompatible; y el guion
e2e recorre el flujo completo del documento sobre un workspace arbitrario, cumpliendo los criterios
de aceptación finales.
