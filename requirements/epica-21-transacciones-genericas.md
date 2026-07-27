# E21 — Contrato MCP nuevo y operaciones transaccionales genéricas

> **Fase**: `§20.14` PRs 9 + 10 (`REFACTOR_PHASE_2 §Fase 11` y `§Fase 12`).
> **Objetivo de la épica**: el motor transaccional deja de hablar de relaciones tipadas y estados de
> OKF. Sus operaciones pasan a las **8 universales** de `§20.11`, se **eliminan las 5 semánticas**
> (`add_relation`, `remove_relation`, `transition_status`, `deprecate`, `replace_concept`), y se
> añaden las **selecciones masivas por consulta** (usando el lenguaje de E19). El contrato MCP queda
> sin terminología OKF. Al cerrar E21, una relación es un enlace Markdown y un estado es una
> propiedad arbitraria del frontmatter — nada más.
> Referencias maestras: `ARCHITECTURE.md §20.11`, `§20.10`; `docs/REFACTOR_PHASE_2 §Fase 11`,
> `§Fase 12`, `§Fase 13`; `CLAUDE.md` invariantes #1/#3/#4/#5.

**Principio rector**: `§Fase 13` — el motor transaccional (`WorkspaceRevision`, `DocumentRevision`,
staging, journal, escritura atómica, recovery, receipt, revert) **no cambia conceptualmente**. Lo
que cambia es *qué operaciones* normaliza y *qué valida*: de *"¿es conforme con OKF?"* a *"¿es
parseable? ¿queda dentro del workspace? ¿respeta la política de escritura? ¿introduce diagnósticos
nuevos? ¿coincide con las revisiones del plan?"*. **No se relitiga la mecánica** — es lo más valioso
del repo y está probada (staging → lock → backup → journal → renames → receipt → recovery).

**Herencia**: E19 (`Expression`/`parse`/`from_json`/`evaluate`) es la base de las selecciones
masivas; E17 (`resolve`/`Inventory`, la reescritura de enlaces relativos) es la base de
`move_document`. E21 los **consume**.

---

### E21-H01 — Retirar las 5 operaciones semánticas

- **Objetivo**: `NormalizedOperation` y la superficie de `change_plan` dejan de tener operaciones
  que asumen el modelo OKF.
- **Referencias**: `ARCHITECTURE.md §20.11` · `REFACTOR_PHASE_2 §Fase 12 (Eliminar operaciones
  semánticas específicas)` · `crates/lodestar-core/src/types.rs` (`NormalizedOperation`, las 11
  variantes), `crates/lodestar-core/src/plan.rs` (`apply_one`), `crates/lodestar-app/src/lib.rs`
  (`change_plan`), `crates/lodestar-mcp/src/tools.rs`.
- **Alcance**:
  - Eliminar de `NormalizedOperation` las variantes `AddRelation`, `RemoveRelation`,
    `TransitionStatus`. (`deprecate`/`replace_concept` viven hoy solo como `kind` de
    `impact_analyze.proposedOperation` / `SearchResult` — retirarlos ahí también.) Quedan las **8
    universales**: `Create`, `PatchFrontmatter`, `ReplaceBody`, `ReplaceText`, `EditSection`, `Move`,
    `Delete`, `ApplyFix`.
  - Retirar de `plan::apply_one` las ramas de esas 3 y su normalización; del `inputSchema`/despacho
    de `change_plan` los `op` correspondientes.
  - Retirar `impact_analyze`'s `proposedOperation.kind` de `transition_status`/`deprecate`/
    `change_relation`/`replace_concept` — deja solo `move`/`delete` (los que `§20.10` lista para
    impacto).
  - **Un `transition_status` es un `patch_frontmatter`** (`§Fase 12`); un `add_relation` es un
    enlace en el cuerpo (`replace_body`/`edit_section`/`replace_text`). No hay pérdida de capacidad:
    se expresa con las universales. Documentar la traducción.
- **Fuera de alcance**: las selecciones masivas (E21-H02); `move_document` con reescritura (E21-H03).
- **Criterios de aceptación**:
  - **Dado** un `change_plan` con `op: "transition_status"`, **Cuando** se procesa, **Entonces** es
    un error (op desconocida) → `transition_status_retirada`.
  - **Dado** `impact_analyze` con `proposedOperation.kind: "deprecate"`, **Entonces** error →
    `impact_sin_ops_semanticas`.
  - **Dado** un `change_plan` con `op: "patch_frontmatter"` que fija `status: "accepted"`,
    **Entonces** funciona (la capacidad de «transición» sobrevive como patch) →
    `patch_hace_de_transicion`.
  - Estructural: `grep -rn "AddRelation\|TransitionStatus\|replace_concept" crates/*/src` no
    encuentra las variantes → checklist.
- **Dependencias**: E20 completa.
- **Pruebas**: `crates/lodestar-app/tests/`, `crates/lodestar-mcp/tests/`.
- **Frontera (mcp.yml)**: **sí**.

### E21-H02 — Selecciones masivas por consulta

- **Objetivo**: seleccionar documentos con el lenguaje de E19 y aplicarles una operación en un solo
  plan.
- **Referencias**: `ARCHITECTURE.md §20.11` · `REFACTOR_PHASE_2 §Fase 12 (Operaciones masivas
  basadas en consulta)` · `crates/lodestar-app/src/lib.rs` (`change_plan`).
- **Alcance**:
  - `change_plan` acepta una forma de **selección** `{selection: {where|filter}, operation: {…}}`
    (`§Fase 12`): la consulta E19 selecciona los documentos, y la operación se expande a una
    `NormalizedOperation` por documento seleccionado.
  - El flujo de `§Fase 12` se respeta: `query → documentos → snapshot de revisiones → semantic diff
    → impact → validation → change plan`. Cada documento seleccionado captura su `DocumentRevision`.
  - Solo las operaciones que tienen sentido en masa (`patch_frontmatter`, `replace_text`, `delete`
    con su política, `apply_fix`) — `create` no aplica a una selección de documentos existentes.
- **Criterios de aceptación**:
  - **Dado** `{selection: {where: "type = \"decision\" and status = \"draft\""}, operation:
    {patch_frontmatter: {status: "review"}}}`, **Cuando** se planifica, **Entonces** el plan tiene una
    op por documento que casa la consulta → `seleccion_masiva_patch`.
  - **Dado** una selección que no casa ningún documento, **Entonces** el plan es vacío (sin cambios),
    sin error → `seleccion_vacia`.
  - **Dado** una selección masiva, **Entonces** cada documento del plan lleva su `DocumentRevision`
    capturada → `seleccion_captura_revisiones`.
- **Dependencias**: E21-H01, E19.
- **Pruebas**: `crates/lodestar-app/tests/`.
- **Frontera (mcp.yml)**: **sí**.

### E21-H03 — `move_document` con reescritura de backlinks

- **Objetivo**: mover un documento reescribiendo los enlaces entrantes, como una transacción lógica.
- **Referencias**: `ARCHITECTURE.md §20.11` · `REFACTOR_PHASE_2 §Fase 12 (Movimiento de documentos,
  Eliminación)` · `crates/lodestar-core/src/plan.rs` (`rewrite_body_links`, que E17 dejó sobre
  `LINK_REWRITE_RE`), `crates/lodestar-core/src/links.rs`.
- **Alcance**:
  - `move_document` con `rewrite_inbound_links`: (1) encuentra los backlinks (de `Analysis::incoming`,
    E17-H04), (2) calcula el **nuevo enlace relativo** desde cada origen, (3) reescribe **solo el
    destino** conservando label y fragmento, (4) muestra todos los documentos modificados, (5)
    verifica que no aparecen enlaces rotos, (6) aplica todo como una **única transacción lógica**.
  - **Reescritura por el `span` del enlace** (E17-H01 lo materializó): sustituir el destino en su
    rango de bytes exacto, no por regex — cubre también los enlaces de referencia (`[id]: destino`).
    Es la mejora que E17 dejó anotada como propia de `move_document`.
  - `delete_document` exige **política explícita** (`§Fase 12`): rechazar si hay backlinks · permitir
    enlaces rotos · eliminar referencias · sustituir referencias. `InboundLinksPolicy` ya existe en
    `types.rs`; conectar su semántica.
- **Criterios de aceptación**:
  - **Dado** `docs/auth.md` con 3 backlinks desde distintas profundidades, **Cuando** se mueve a
    `docs/security/auth.md` con `rewriteInboundLinks`, **Entonces** los 3 orígenes tienen el enlace
    recalculado (relativo correcto) conservando su label → `move_reescribe_backlinks`.
  - **Dado** un backlink que es un enlace de **referencia** (`[x][id]` + `[id]: ../auth.md`),
    **Cuando** se mueve, **Entonces** la **definición** se reescribe → `move_reescribe_referencia`.
  - **Dado** un `delete_document` sobre un documento con backlinks y política `reject`, **Entonces**
    se rechaza con `INBOUND_LINKS_EXIST` → `delete_rechaza_con_backlinks`.
  - **Dado** el mismo delete con política `remove_links`, **Entonces** los enlaces entrantes se
    eliminan y el plan lo refleja → `delete_remove_links`.
  - **[Añadido tras la fase roja — gap de `§Fase 12`]** **Dado** un `delete_document` **sin**
    `inboundLinksPolicy` sobre un documento con backlinks, **Entonces** se rechaza pidiendo una
    política explícita — **no** se elige `reject` en silencio (`§Fase 12`: *"No elegir una política
    automáticamente"*). Hoy defaultea a `reject`: hay que convertir la omisión en error →
    `delete_exige_politica_explicita`.
- **Dependencias**: E21-H01, E17.
- **Pruebas**: `crates/lodestar-core/tests/`, `crates/lodestar-app/tests/`.
- **Frontera (mcp.yml)**: **sí**.

### E21-H04 — Limpieza del contrato y `SemanticDiff`

- **Objetivo**: `contracts/mcp.yml` y los tipos del wire quedan sin terminología OKF; `OkfDiff`
  pasa a su nombre neutro.
- **Referencias**: `ARCHITECTURE.md §20.3` (terminología), `§20.11` · `REFACTOR_PHASE_2 §Fase 1` ·
  `crates/lodestar-core/src/diff.rs` (`OkfDiff`, `diff_snap`), `contracts/mcp.yml`.
- **Alcance**:
  - `core::diff::OkfDiff` → `SemanticDiff` (el nombre que `§20.3` manda y que E16-H06 difirió aquí).
    Ajustar sus consumidores.
  - Repaso final de `contracts/mcp.yml`: ningún ejemplo, descripción ni tipo con vocabulario OKF
    (`concept`, `conformance`, `bundle`). Cierra `DECISIONES.md §13` si se decide completar
    `Conformant → Valid` (o se documenta el aplazamiento).
  - `/contrato --check` limpio: `contracts/mcp.yml` ↔ `tools::list()`/`call()` ↔ `core::types`.
- **Criterios de aceptación**:
  - Estructural: `grep -rn "OkfDiff\|Concept\|conformance" crates/*/src` solo en comentarios
    históricos → checklist.
  - `/contrato --check` no reporta discrepancias → checklist.
  - **Dado** la suite, **Cuando** se ejecuta, **Entonces** pasa (renombre puramente léxico) →
    regresión.
- **Dependencias**: E21-H01, H02, H03.
- **Pruebas**: `crates/lodestar-mcp/tests/`.
- **Frontera (mcp.yml)**: **sí**.

---

## Orden de construcción

```
H01 (retirar semánticas) ─→ H02 (selecciones masivas) ─→ H03 (move/delete) ─→ H04 (contrato + diff)
```

## Criterio de salida

`change_plan` solo conoce las 8 operaciones universales; una selección por consulta genera un plan
de una operación por documento; `move_document` reescribe los backlinks relativos (incluidas las
definiciones de referencia) por el `span`, no por regex; y `/contrato --check` está limpio sin
terminología OKF en el wire.
