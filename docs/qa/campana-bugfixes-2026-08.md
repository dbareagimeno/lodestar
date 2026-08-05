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
| 0 | M-01 + §16(i) | E28-H01 | — (pendiente) | — | — | — | ⏳ en curso |
| 0 | A-05 | E28-H02 | — (pendiente) | — | — | — | ⏳ pendiente |
| 1 | §19(a) | épica honestidad | — | — | — | — | pendiente |
| 1 | §19(b) | épica honestidad | — | — | — | — | pendiente |
| 1 | §18 | épica honestidad | — | — | — | — | pendiente |
| 1 | §15 | épica honestidad | — | — | — | — | pendiente |
| 1 | §16(e) (+A-08) | épica honestidad | — | — | — | — | pendiente |
| 1 | §16(f) | épica honestidad | — | — | — | — | pendiente |
| 1 | §16(g) | épica honestidad | — (compilación+suite) | — | — | — | pendiente |
| 1 | §16(b) | épica honestidad | — (compilación+suite) | — | — | — | pendiente |
| 1 | D-01 | épica honestidad | — | — | — | — | pendiente |
| 1 | A-04 | épica honestidad | — | — | — | — | pendiente |
| 1 | A-07 | épica honestidad | — | — | — | — | pendiente |
| 2 | §16(j) + A-02/A-03 | pendiente | — | — | — | — | pendiente |
| 2 | §16(l) mutantes | pendiente | — (tests anti-mutante) | — | — | — | pendiente |
| 3 | D-02, A-01, A-06, A-09, A-10 | historia-escoba | — (guardia donde aplique) | — | — | — | pendiente |

**Fuera de alcance**: §9, §14, §20, §21, §22, §10, §16(k), §16(h) y los 12 esperados refutados
del informe §4 (`docs/qa/informe-homelab-2026-08-06.md`).

## Criterio de cierre por bug

1. Test e2e de regresión (patrón `Sesion` para estado encadenado, `roundtrip()` para superficie
   fría, core-tests para semántica de consulta) que **falló antes del fix** y pasa después.
2. Suite completa en verde, incluidos los dos crates con `--features test-failpoints`.
3. Veredicto de juez ciego (panel si toca `contracts/mcp.yml`) favorable.
4. `/contrato --check` limpio si se tocó la frontera.
5. Frontmatter de la decisión correspondiente actualizado y fila de esta tabla cerrada.
