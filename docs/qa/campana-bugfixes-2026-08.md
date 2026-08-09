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
| 1 | §16(g) | E29-H10 | repliegue de `create_document`/`write_document`/`merge_frontmatter`/`publish` a `pub(crate)` (compilación+suite) | ✓ | ✓ | ✓ (juez ciego) | ✅ cerrada |
| 1 | §16(b) | E29-H11 | retirada de `Envelope`/`ErrorEnvelope` (compilación+suite) | ✓ | ✓ | ✓ (juez ciego) | ✅ cerrada |
| 1 | D-01 | E29-H09 | `instructions` por perfil + rechazo de `protocolVersion` (`main.rs`) | ✓ | ✓ | ✓ (7/7) | ✅ cerrada |
| 1 | A-04 | E29-H04 | `starts_with`/`ends_with` sobre no-string → type error (`eval.rs`, `consulta.rs`, remate `681ec45`) | ✓ | ✓ | ✓ (7/7) | ✅ cerrada |
| 1 | A-07 | E29-H05 | scope `paths` exige existencia → `DOCUMENT_NOT_FOUND` (`e2e.rs`, `validacion.rs`) | ✓ | ✓ | ✓ (6/6) | ✅ cerrada |
| 2 | §16(j) + A-02/A-03 | E30-H01 (+ remate `2d32eeb`) | los 9 de `mcp.rs` (cursor malformado/ajeno/vacío, recorrido en las 4 tools) + `cursor_no_ascii_no_tumba_el_servidor` | ✓ (2026-08-07) | ✓ | ✓ (re-juez de robustez tras el panic) | ✅ cerrada |
| 2 | §16(l) mutantes | E30 — pasada scoped | `cuarentena.rs` (2), `escritura.rs` (3), `lib.rs` (cota de `pagina()`) | ✓ (6 mutantes) | ✓ | n/a (higiene) | ✅ cerrada |
| 2 | flakiness del lock | E30-H02 (`9cd129a` + remate `6b5c2b7`) | `crash_por_senal_no_deja_parciales` (20/20 limpias) + 3 de `transactions.rs` (barrido de temporales, frontera del reclamo) | ✓ (30/30 repro) | ✓ (0/30) | ✓ (4 remates saldados) | ✅ cerrada |
| 3 | D-02, A-01, A-06, A-09, A-10 (+ 3 defectos) | E30-H03 (`0ef66d2` + remate `8621e40`) | `consulta.rs` (`contains` type error), `mcp.rs` (`protocolVersion` no-string), `plan.rs` (A-06 con fixture flow/block), `cli.rs` (SARIF), `lib.rs` (`PATH-NOT-UTF8` sintético) | ✓ (2026-08-07) | ✓ | ✓ (9/11 + 3 MAYOR saldados) | ✅ cerrada |

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

**Observación — CERRADA por E30-H02 (`9cd129a`)**: `crash_por_senal_no_deja_parciales`
(`crates/lodestar-mcp/tests/crash_senal.rs`) era flaky bajo carga (~50% con `--workspace`),
preexistente, señalado por tres jueces. **No era un test frágil: era un bug real.** Causa raíz —
`acquire_lock` publicaba el lock en dos pasos no atómicos entre sí (`create_new`, que ya ganaba la
exclusión con el fichero **vacío**, y solo después el cuerpo). Un `SIGKILL` en esa ventana dejaba en
disco un lock existente y sin `pid` ni `timestamp`: un estado **terminal**, porque sin pid no hay
prueba de vida y sin timestamp no hay TTL, así que el workspace quedaba cerrado a la escritura para
siempre. Bajo carga el scheduler puede desalojar el proceso justo entre las dos llamadas y ensanchar
la ventana de microsegundos a decenas de milisegundos — la escala exacta de los retrasos escalonados
del test (40/70/100/130/170 ms), de ahí el ~50 % con la suite entera y el 0 % en aislamiento.
Arreglo: el cuerpo se escribe y se `fsync`ea en un temporal del mismo directorio y se publica con
`hard_link` (atómico y no-clobber, conserva la exclusión que daba `create_new`), más el reclamo de
los locks vacíos que la ventana ya dejó en disco. **Verificado por el juez ciego: 20/20 corridas
limpias** del test bajo carga.

**Fase 1 cerrada a falta del juez de §16(g)/§16(b) (2026-08-07).** Épica E29 (11/11 historias) en
`feat/e29-honestidad-superficie`: H01 (`4a52f59`), H02 (`46c1492`+`2f6ecf4`), H03 (`99900d3`), H04
(`b3b79fb`+`681ec45`), H05 (`fc5c26b`), H06 (`88e99b2`+`6a3a6ca`), H07 (`9df617f`+`f97004c`), H08
(`f7dc5fd`+`f720ba8`), H09 (`5e7edc0`), H10+H11 (`7f519d2`). Suite completa en verde, incluidos los
dos crates con `--features test-failpoints`; clippy `-D warnings`, `fmt --check` y `cargo doc`
limpios; pureza del core intacta. Jueces ciegos: H01 8/8, H02 5/5, H03 7/7, H04 7/7, H05 6/6, H06
aprobada, H07 aprobada (bloqueante doc saldado en el remate), H08 11/11, H09 7/7 — todas las
reservas MAYORES/bloqueantes saldadas en los commits de remate. **H10/H11 con juez en curso**: en
verificación final, sin veredicto todavía. Pendiente además el merge de la rama.

**Fases 2-3 cerradas (2026-08-07).** Épica E30 (3/3 historias) en `fix/f2-ciclo-higiene`: H01
(`8359294` + `2d32eeb`), H02 (`9cd129a` + `6b5c2b7`), H03 (`0ef66d2` + `8621e40`), más la pasada de
mutantes de `§16(l)` (`9d09c62`). Suite completa en verde (**726 tests**), incluidos los dos crates
con `--features test-failpoints`; clippy `-D warnings`, `fmt --check` y `cargo doc` limpios.
Guardián de contrato: **sin drift**, con A-01/A-09/A-10 verificados **ejecutando el binario real**
(A-09 editando `config.yaml` con el servidor vivo; A-10 con los cuatro tipos de enlace).

**La lección de estas fases — los tests también mienten, y los jueces ciegos son quien lo descubre.**
Ninguno de estos cuatro fallos lo detectó la suite, que estaba en verde en todos los casos:

1. Un test que **aseveraba la negación** del defecto documentado en su propio commit (A-06), y que
   solo pasaba por un accidente del fixture.
2. Un arreglo (SARIF) **sin un solo test**: neutralizar el guard dejaba los 51 tests del crate en
   verde.
3. Una **costura extraída para un test que nunca se escribió**, con un rustdoc de 22 líneas
   explicando para qué servía (`PATH-NOT-UTF8`).
4. Tres tests de lock que **cubrían el camino pero no la frontera**: dejaban pasar la mutación
   `&&`→`||` que reclama locks de dueños vivos.

Es la misma lección que E23 dejó escrita en `CLAUDE.md` —*cuando dudes de si algo funciona,
ejecútalo*— aplicada un nivel más arriba: **cuando dudes de si un test muerde, mútalo**.

**Seguimientos nuevos registrados al cerrar la Fase 1**, sin numerar como punto nuevo de
`decisiones §23`:

- **Familia `contains`-literal**: ~~candidata a Fase 2/escoba~~ — **CERRADA por E30-H03**
  (`0ef66d2`). `contains` sobre un campo STRING con literal no-string devolvía `Ok(false)`: la misma
  mentira silenciosa que E29-H04 cerró para `starts_with`/`ends_with`. Pasa a type error ruidoso
  (criterio ratificado por el usuario). Sobre campo LISTA no cambia: sigue siendo pertenencia y
  admite literales de cualquier tipo — asimetría deliberada, ahora documentada.
- **Divergencia `workspace_status.counts`**: ~~causa registrada: «diagnósticos sin target»~~ —
  **la hipótesis registrada era FALSA**, refutada ejecutando en E30-H03. `SYMLINK-UNSUPPORTED` y
  `DOC-NOT-UTF8` **sí** llevan targets y divergen igual. El criterio real es que el fichero **nunca
  entra al inventario**, así que `DocumentSet::analyze()` (lo que usa `workspace_status`) no puede
  verlo, mientras `full_analysis()` (lo que usa `knowledge_check`) sí fusiona los diagnósticos de
  descubrimiento. Documentado como limitación conocida, con `knowledge_check` declarado autoritativo.
- **SARIF `.lodestar`**: ~~a revisar~~ — **CONFIRMADO y CERRADO por E30-H03**. El URI fantasma era
  real: los hallazgos sin target emitían `artifactLocation: ".lodestar"`, un path que normalmente
  **no existe en disco**. Pasan a emitirse sin `locations`, que es lo que SARIF 2.1.0 prescribe para
  un hallazgo que no pertenece a ningún artefacto. El arreglo llegó **sin test** y el juez ciego lo
  demostró neutralizando el guard: la suite entera de `lodestar-cli` (51 tests) pasó igual. Cubierto
  en el remate.
- **`PATH-NOT-UTF8` sin red vía `full_analysis`**: hueco real, cerrado en el **remate** de E30-H03.
  H03 extrajo la costura (`fusiona_diagnosticos_de_descubrimiento`) pero **no escribió el test que la
  spec nombraba** — el juez ciego lo cazó: era «una costura para un test que no se escribió». El
  remate clava las tres reglas del anclaje (familia en `ignore` se descarta · con `targets` ancla en
  el primero · **sin** `targets` ancla en `anchor_workspace` y nunca se descarta). El diagnóstico en
  sí no se puede provocar en un test portable (en APFS no hay forma de crear un nombre de fichero
  que no sea UTF-8 válido), de ahí el `Check` sintético.
- **`protocolVersion` no-string**: ~~candidato a la escoba~~ — **CERRADO por E30-H03**. Se colaba
  por `as_str()` y degradaba al valor por defecto; ahora el `match` escruta el `Value` y devuelve
  `-32602` nombrando el tipo recibido.
- **Mensaje duplicado de `INVALID_RESULT` del gate de staging**: ~~a saldar en la escoba~~ —
  **CERRADO por E30-H03**. El `format!` de staging repetía la frase que la plantilla de `thiserror`
  ya antepone. Corregido en el emisor, no en la plantilla (que E20-H04/E29-H07 aseveran).
- **`Workspace::revert_transaction` es superficie pública muerta** — **CERRADO por `E31-H01`** (2026-08-08): se **RETIRÓ**, no se replegó — al hacer el `pub(crate)` que §25 recomendaba, clippy la marcó como `dead_code` (no la usaba nadie ni dentro del crate) y con el CI en `-D warnings` el repliegue era incompilable. Lo destapó
  la pasada de mutantes de `§16(l)`: sustituir su cuerpo entero por `unreachable!()` deja **los 52
  binarios de test del workspace en verde**, y no tiene un solo llamador (la fachada usa
  `revert_transaction_con_recibo`). No se actuó a propósito: es la categoría de `§16(b)`/`§16(g)`,
  que se resolvieron **retirando o replegando por decisión**, no añadiendo tests. **Ficha propia**:
  [`decisiones §25`](../../decisiones/25-superficie-muerta-revert-transaction.md).
- **`replace_text` no-op que reserializa el frontmatter** — **CERRADO por `E31-H02`** (2026-08-08): resultaron ser TRES defectos —el frontmatter reserializado, un separador que inyectaba una línea en blanco y el frontmatter ILEGIBLE que se borraba entero (pérdida de datos)—, los tres cerrados por el mismo patch quirúrgico. El plan gana además `noOpOperations`. Un `replace_text`
  que no casa NADA reescribe el fichero igualmente: normaliza a un `replace_body` de documento
  entero que reserializa el frontmatter, convirtiendo `tags: [a, b]` de estilo flow a bloque. El
  `semanticDiff` lo reporta como `modified` con `bodyChanges` y `frontmatterChanges` **vacíos**.
  Detectado al documentar A-06 en E30-H03 y dejado fuera de su alcance por causa raíz distinta;
  documentado como caveat en `docs/user/safe-changes.md`. **Ficha propia**:
  [`decisiones §26`](../../decisiones/26-replace-text-noop-reserializa.md).
- **Flakiness recurrente de `crash_por_senal_no_deja_parciales`**: ~~sigue viva tras la Fase 1~~ —
  **CERRADA por E30-H02 (`9cd129a`)**, la historia de higiene de la Fase 2 que se derivó de aquí. No
  era fragilidad del test sino la ventana no atómica de publicación del lock; ver la observación de
  arriba para causa raíz, arreglo y las 20/20 corridas limpias del juez.

## Criterio de cierre por bug

1. Test e2e de regresión (patrón `Sesion` para estado encadenado, `roundtrip()` para superficie
   fría, core-tests para semántica de consulta) que **falló antes del fix** y pasa después.
2. Suite completa en verde, incluidos los dos crates con `--features test-failpoints`.
3. Veredicto de juez ciego (panel si toca `contracts/mcp.yml`) favorable.
4. `/contrato --check` limpio si se tocó la frontera.
5. Frontmatter de la decisión correspondiente actualizado y fila de esta tabla cerrada.
