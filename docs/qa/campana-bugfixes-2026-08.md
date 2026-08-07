# Campaña de bugfixes 2026-08 — hallazgos del testbench homelab y deuda decidida

> **Goal**: cerrar todo lo accionable de `decisiones/23-hallazgos-testbench-homelab.md` y del
> orden de trabajo de `decisiones/README.md` (revisado 2026-08-06). **El goal no se da por
> cumplido hasta que cada bug de comportamiento tenga un test e2e de regresión con evidencia
> rojo→verde**, la suite completa (incl. failpoints) esté en verde, cada historia esté aprobada
> por juez ciego y los documentos de estado reflejen el cierre.
>
> Orquestación: Fable (orquestador, no implementa) + subagentes del repo — `historiador`/
> `guardian-contrato` (sonnet), `planificador`/`juez-historia` (opus), `autor-tests`/
> `implementador` (opus donde hay criterio, sonnet en lo mecánico).
>
> Criterio ratificado por el usuario (2026-08-06): A-04 → type error ruidoso; D-02 → corregir
> `ARCHITECTURE.md §20.4` y declarar merge-patch RFC 7386; A-07 → `DOCUMENT_NOT_FOUND`.

## Estado por bug

| Fase | Bug | Historia | Test e2e de regresión | Rojo | Verde | Juez | Estado |
|---|---|---|---|---|---|---|---|
| 0 | M-01 + §16(i) | E28-H01 (+H03 adenda) | `revertir_el_revert_restaura_el_estado_post_apply`, `revertir_un_revert_produce_receipt_id_distinto`, `revertir_un_revert_no_toca_recovery_ni_receipts_previos`, `revertir_tres_veces_compone_sin_perder_estado`, `apply_revert_reapply_revert_de_plan_identico_completa_con_cuatro_receipts_unicos` (`e2e_ciclo_vida.rs`) + failpoints en `escritura.rs` | ✓ (2026-08-06) | ✓ | ✓ (panel 3 + re-juez robustez) | ✅ cerrada |
| 0 | A-05 | E28-H02 (+H04 adenda) | los 6 de `mcp.rs` de H02 + los 5 intra-plan de H04 + `apply_de_plan_persistido_con_colision_intra_plan_rechaza_sin_tocar_disco` (`plan.rs`) | ✓ (2026-08-06) | ✓ | ✓ (panel 3 + re-juez robustez) | ✅ cerrada |
| 1 | §19(a) | E29-H03 | `has`/`missing` sobre frontmatter pelado (`core.rs`/`mcp.rs`) | ✓ | ✓ | ✓ (7/7) | ✅ cerrada |
| 1 | §19(b) | E29-H02 | `policy` parcial respeta el `Default` (`core.rs`/`mcp.rs`, remate `2f6ecf4`) | ✓ | ✓ | ✓ (5/5) | ✅ cerrada |
| 1 | §18 | E29-H07 | `change_apply` rechaza `canApply: false` (`plan.rs`, `e2e_ciclo_vida.rs`) | ✓ | ✓ | ✓ (bloqueante doc saldado en `f97004c`) | ✅ cerrada |
| 1 | §15 | E29-H08 | validación por unión contra la tabla de campos por operación (`mcp.rs`, `descubribilidad.rs`) | ✓ | ✓ | ✓ (11/11) | ✅ cerrada |
| 1 | §16(e) (+A-08) | E29-H01 | config estricta: claves desconocidas + familias de `validation` (`e2e.rs`, `config.rs`) | ✓ | ✓ | ✓ (8/8) | ✅ cerrada |
| 1 | §16(f) | E29-H06 | `WORKSPACE-EMPTY` (warn) sobre raíz sin documentos (`discovery.rs`, `validacion.rs`, remate `6a3a6ca`) | ✓ | ✓ | ✓ (aprobada) | ✅ cerrada |
| 1 | §16(g) | E29-H10 | repliegue de `create_document`/`write_document`/`merge_frontmatter` a `pub(crate)` (compilación+suite) | ✓ | ✓ | en verificación final | 🔶 pendiente de juez |
| 1 | §16(b) | E29-H11 | retirada de `Envelope`/`ErrorEnvelope` (compilación+suite) | ✓ | ✓ | en verificación final | 🔶 pendiente de juez |
| 1 | D-01 | E29-H09 | `instructions` por perfil + rechazo de `protocolVersion` (`main.rs`) | ✓ | ✓ | ✓ (7/7) | ✅ cerrada |
| 1 | A-04 | E29-H04 | `starts_with`/`ends_with` sobre no-string → type error (`eval.rs`, `consulta.rs`, remate `681ec45`) | ✓ | ✓ | ✓ (7/7) | ✅ cerrada |
| 1 | A-07 | E29-H05 | scope `paths` exige existencia → `DOCUMENT_NOT_FOUND` (`e2e.rs`, `validacion.rs`) | ✓ | ✓ | ✓ (6/6) | ✅ cerrada |
| 2 | §16(j) + A-02/A-03 | pendiente | — | — | — | — | pendiente |
| 2 | §16(l) mutantes | pendiente | — (tests anti-mutante) | — | — | — | pendiente |
| 3 | D-02, A-01, A-06, A-09, A-10 | historia-escoba | — (guardia donde aplique) | — | — | — | pendiente |

**Fuera de alcance**: §9, §14, §20, §21, §22, §10, §16(k), §16(h) y los 12 esperados refutados
del informe §4 (`docs/qa/informe-homelab-2026-08-06.md`).

**Fase 0 cerrada (2026-08-06).** Épica E28 completa: H01+H02 en `043f233`/`296147b`, adenda
correctiva H03+H04 (bloqueantes de los jueces ciegos de H01/H02, ejecutando el binario real) en
`8c86b6b`, cierre de reservas de los re-jueces en `c532929`. Suite completa en verde, incluidos los
dos crates con `--features test-failpoints`; jueces ciegos: H01 y H02 aprobadas con panel de 3
lentes cada una, cuyos bloqueantes se corrigieron en H03/H04 y fueron re-verificados por re-jueces
de robustez con veredicto APROBADA CON RESERVAS ya saldadas en `c532929`; guardián de contrato
COHERENTE en dos pasadas. Seguimientos registrados fuera de la campaña, sin numerar como punto
nuevo de `decisiones §23`:

- **`decisiones §24`** (equivalencia de paths por caja/Unicode en el guard de colisión) — abierta,
  nacida de la verificación de la adenda de H04.
- **Familia preexistente de normalización con contenido acumulado** (resurrección de paths
  liberados por operaciones de contenido tras `delete`/`move`, y move-chains por ocupación del
  origen) — registrada en la épica E28, sección «Hallazgos preexistentes registrados»; comparte
  causa raíz con A-05 pero queda fuera de su arreglo.

**Observación**: `crash_por_senal_no_deja_parciales` (`crates/lodestar-mcp/tests/crash_senal.rs`)
es flaky bajo carga (~50% con `--workspace`), preexistente, señalado por tres jueces — candidato a
historia de higiene.

**Fase 1 cerrada a falta del juez de §16(g)/§16(b) (2026-08-07).** Épica E29 (11/11 historias) en
`feat/e29-honestidad-superficie`: H01 (`4a52f59`), H02 (`46c1492`+`2f6ecf4`), H03 (`99900d3`), H04
(`b3b79fb`+`681ec45`), H05 (`fc5c26b`), H06 (`88e99b2`+`6a3a6ca`), H07 (`9df617f`+`f97004c`), H08
(`f7dc5fd`+`f720ba8`), H09 (`5e7edc0`), H10+H11 (`7f519d2`). Suite completa en verde, incluidos los
dos crates con `--features test-failpoints`; clippy `-D warnings`, `fmt --check` y `cargo doc`
limpios; pureza del core intacta. Jueces ciegos: H01 8/8, H02 5/5, H03 7/7, H04 7/7, H05 6/6, H06
aprobada, H07 aprobada (bloqueante doc saldado en el remate), H08 11/11, H09 7/7 — todas las
reservas MAYORES/bloqueantes saldadas en los commits de remate. **H10/H11 con juez en curso**: en
verificación final, sin veredicto todavía. Pendiente además el merge de la rama.

**Seguimientos nuevos registrados al cerrar la Fase 1**, sin numerar como punto nuevo de
`decisiones §23`:

- **Familia `contains`-literal**: variantes de la familia de consulta por substring/literal
  detectadas durante la implementación de E29-H03/H04, fuera del alcance de esas historias —
  candidata a Fase 2/escoba.
- **Divergencia `workspace_status.counts`**: el recuento que sirve `workspace_status` puede divergir
  del que computan `knowledge_check`/`graph_query` bajo ciertas condiciones de descubrimiento
  descubiertas al implementar E29-H06 — registrada para verificación en la Fase 2.
- **SARIF `.lodestar`**: la salida SARIF de `check` puede listar rutas bajo `.lodestar/` en
  condiciones de borde tocadas por E29-H01/H06 — a revisar en la historia-escoba.
- **`PATH-NOT-UTF8` sin red vía `full_analysis`**: el diagnóstico de path no-UTF8 puede alcanzar
  `full_analysis` sin cobertura de test dedicada — hueco detectado al endurecer el scope `paths` de
  E29-H05.
- **`protocolVersion` no-string → escoba**: E29-H09 fijó el rechazo de `protocolVersion` no
  soportada, pero el caso de un `protocolVersion` que no es siquiera un string (tipo incorrecto en
  el wire) queda sin cubrir — candidato a la historia-escoba.
- **Mensaje duplicado de `INVALID_RESULT` del gate de staging**: el gate de staging que E14-H04
  introdujo puede emitir el mismo mensaje de `INVALID_RESULT` por dos caminos distintos, detectado
  al tocar la validación estricta de E29-H01/H08 — a saldar en la escoba.
- **Flakiness recurrente de `crash_por_senal_no_deja_parciales`**: sigue viva tras la Fase 1 (ver
  observación arriba) — **candidata a historia de higiene de la Fase 2**.

## Criterio de cierre por bug

1. Test e2e de regresión (patrón `Sesion` para estado encadenado, `roundtrip()` para superficie
   fría, core-tests para semántica de consulta) que **falló antes del fix** y pasa después.
2. Suite completa en verde, incluidos los dos crates con `--features test-failpoints`.
3. Veredicto de juez ciego (panel si toca `contracts/mcp.yml`) favorable.
4. `/contrato --check` limpio si se tocó la frontera.
5. Frontmatter de la decisión correspondiente actualizado y fila de esta tabla cerrada.
