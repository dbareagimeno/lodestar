# Compatibilidad del workflow de Claude

La referencia operativa común del repositorio está en
[`docs/CODEX_WORKFLOW.md`](../docs/CODEX_WORKFLOW.md). Esta carpeta conserva comandos equivalentes
para clientes Claude durante la transición; no define una autoridad distinta.

## Principios compartidos

- Autoridad: `ARCHITECTURE.md` → spec de migración → historias ratificadas → decisiones abiertas →
  contrato MCP → estado.
- `prototype/` es referencia histórica, no oráculo.
- Todo desarrollo se basa en `develop`; ramas y commits no son gates y requieren una petición
  explícita.
- Rojo y verde se separan mediante `phase-scope.py` y `tdd-test-lock.py`.
- Código, tests, contrato y docs están completos antes del juicio.
- Los seis invariantes activos son obligatorios; el séptimo, git, está retirado.

## Comandos legacy

| Comando | Equivalente del flujo común |
| --- | --- |
| `/planificar` | `$planificar`: diseño y épica con dos ratificaciones. |
| `/historia` | `$especificar`: historia BDD ratificable. |
| `/tdd` | Fases roja y verde internas de `$ciclo`, con alcance y lock. |
| `/contrato --check` | `scripts/agent-gates.sh contract` + revisión semántica. |
| `/juzgar` | `$revisar`: síntesis por evidencia con jueces frescos. |
| `/mutantes` | `$mutantes`: mutation testing acotado a demanda. |
| `/ciclo` | `$ciclo`: entrega completa según el riesgo. |

## Proporcionalidad

- Docs o mecánico: cambio directo, comprobación específica y revisión.
- Bugfix: reproducción, rojo, fix, gates y juez fresco.
- Historia: spec ratificada, rojo/verde, contrato/docs, gates y revisión.
- Arquitectura: `/planificar` antes de historias individuales.

Los gates ejecutables son `scripts/agent-gates.sh contract`, `policy` y `full`. El último incluye
las dos suites `test-failpoints` que `cargo test --workspace` no activa.
