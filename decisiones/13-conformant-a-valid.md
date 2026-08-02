---
id: 13
titulo: "Conformant a Valid: la mitad que falta de la tabla de terminología"
estado: "cerrada"
prioridad: 1
etiquetas: ["contrato", "mcp", "terminologia"]
origen: "hallazgo-de-implementacion"
abierta_en: "2026-07-23"
cerrada_en: "2026-07-25"
revisada_en: "2026-07-25"
epica: "E23"
historias: ["E23-H14", "E21-H04"]
relacionadas: [18]
---

# §13 — `Conformant → Valid`: la mitad que falta de `§20.3`

- **Contexto** (detectado al implementar E16-H06): la tabla de terminología de `§20.3` manda dos
  sustituciones emparejadas, `Conformance → Validation` y `Conformant → Valid`. La primera está
  hecha (`ApplyConformance` → `ApplyValidation`, `Store::conformance_counts` →
  `validation_counts`…). La segunda **no**, y el resultado es una asimetría visible en el wire:
  `ApplyValidation { conformant, errors, warnings }` — el contenedor habla de *validación* y el
  veredicto sigue hablando de *conformidad*.
- **Por qué no se cerró**: `conformant` no aparece solo como campo. Está en
  `ErrorCode::NonconformantResult` (wire `NONCONFORMANT_RESULT`), que es **una de las 16 filas
  congeladas** del catálogo de errores de `§19.3`, y en `PlanPolicy::requireConformantResult` y
  `allowNonconformant`, que son superficie de `change_plan`. No es un renombre léxico: toca el
  contrato de errores.
- **Qué decidir**: (a) completar la pareja y aceptar el cambio de wire
  (`NONCONFORMANT_RESULT` → `INVALID_RESULT`, `requireConformantResult` → `requireValidResult`),
  aprovechando que v0.3 ya es incompatible; (b) dejar `conformant` como término del **veredicto**
  y documentar en `§20.3` que la sustitución solo aplica al *sustantivo*, no al *adjetivo*.
- **Recomendación**: **(a)**, y hacerlo en **E21**, que es cuando se toca el motor transaccional y
  su contrato de errores. Hacerlo ahora significaría abrir el catálogo congelado dos veces.
  Mientras tanto la asimetría es fea pero inocua: `core::types::ValidationReport` ya tenía esa
  misma forma antes de la migración, así que no se ha introducido una discrepancia nueva.
- **Estado (E21-H04, cierre de E21)**: **APLAZADA, no cerrada**. E21-H04 hizo el renombre neutro
  que sí era léxico y sin riesgo (`core::diff::OkfDiff` → `SnapshotDiff`) y repasó el contrato: en
  la superficie viva de `mcp.yml` no queda vocabulario OKF salvo `conformant`/
  `requireConformantResult`/`NONCONFORMANT_RESULT`, que son exactamente esta decisión. Completar la
  opción (a) exige tocar `ErrorCode::NonconformantResult` —**una de las 16 filas congeladas** del
  catálogo de errores (`§19.3`)— y la superficie de `change_plan`; eso **no** es un renombre léxico
  y queda **fuera** del alcance de E21-H04 (la historia acota su repaso a lo que no abre el catálogo
  congelado). La asimetría sigue siendo fea pero inocua. Se retoma en la historia que decida abrir
  el catálogo de errores; hasta entonces `conformant`/`NONCONFORMANT_RESULT` se conservan tal cual y
  su presencia en la superficie activa de `mcp.yml` es **deuda documentada**, no una discrepancia de
  contrato.
- **Resolución (E23-H14, 2026-07-25): (a)**, decidida por el usuario. Se abre el catálogo de 16
  códigos —**la única vez**— porque el momento es ahora o nunca: v0.3.0 ya es incompatible con
  v0.2.x, así que romper el wire cuesta **cero** hoy y deja de costarlo en cuanto se publique. Y
  porque era el **único** de los 29 criterios de aceptación de `REFACTOR_PHASE_2` demostrablemente
  incumplido: *«no existe terminología OKF en la API pública»*.
  Renombres aplicados (69 apariciones en 12 ficheros de producción + 16 de test):
  `conformant` → `valid` · `requireConformantResult` → `requireValidResult` · `allowNonconformant`
  → `allowInvalid` · `NONCONFORMANT_RESULT` → `INVALID_RESULT` ·
  `ErrorCode::NonconformantResult` → `ErrorCode::InvalidResult` ·
  `WorkspaceError::NonconformantResult` → `WorkspaceError::InvalidResult`. El catálogo **sigue
  teniendo 16 filas**: se sustituye una, no se añade ninguna.
  También la salida humana de `lodestar check`: `CONFORME`/`NO CONFORME` → `VÁLIDO`/`NO VÁLIDO`. No
  lo exigía la tabla de `§20.3` —que habla de la API—, pero es la aparición más visible del término
  retirado: la línea que un humano lee en cada ejecución de CI.
  **No se tocaron** los documentos históricos (`docs/history/REFACTOR.md`,
  `docs/history/REFACTOR_DISENO_PROPUESTA.md`) ni las tablas de terminología que **documentan el propio
  renombre** (`§20.3`, `REFACTOR_PHASE_2 §Terminología`): ahí `Conformant` es el término de partida
  y sustituirlo las dejaría diciendo «Valid → Valid».
