# Workflows de desarrollo

La referencia operativa común está en [`CODEX_WORKFLOW.md`](CODEX_WORKFLOW.md). `AGENTS.md` contiene
las reglas estables del proyecto; los detalles ejecutables viven en `scripts/`.

## Recorrido por riesgo

| Cambio | Recorrido |
| --- | --- |
| Docs o mecánico | Cambio directo → check específico → revisión del diff. |
| Bugfix | Reproducción → rojo independiente → fix → gates → revisión fresca. |
| Historia | Spec ratificada → rojo/verde separados → contrato/docs → gates → revisión. |
| Arquitectura | Diseño ratificado → épica ratificada → historias individuales. |

El flujo conserva SDD/BDD para comportamiento nuevo, pero no obliga a redactar una historia para un
bug inequívoco o una corrección documental.

## Codex

| Skill | Uso |
| --- | --- |
| `$planificar` | Diseño grande y descomposición con dos ratificaciones. |
| `$especificar` | Historia BDD ratificable. |
| `$ciclo` | Bug o historia completos con rojo/verde, gates y revisión. |
| `$revisar` | Jueces frescos de corrección, arquitectura y tests. |
| `$mutantes` | Mutation testing acotado a demanda. |

Los roles están en `.codex/agents/`. Los jueces usan sandbox de solo lectura; el autor de tests y el
implementador están separados por controles de alcance y hashes, no solo por instrucciones.

## Compatibilidad con Claude

La configuración de `.claude/` se mantiene temporalmente para clientes existentes, alineada con
estas mismas reglas. Sus comandos antiguos corresponden a las fases internas del ciclo; la fuente
de verdad del proceso es este documento y `docs/CODEX_WORKFLOW.md`.

## Puertas locales

```bash
scripts/agent-gates.sh contract
scripts/agent-gates.sh policy
scripts/agent-gates.sh full
```

`full` incluye las dos suites con `test-failpoints`. En Linux ejecuta también el smoke de la demo,
igual que CI.

## Integración

Todo desarrollo se basa en `develop`. `main` recibe únicamente releases según `RELEASING.md`.
Rama, commit, push y PR se realizan solo cuando el usuario los solicita; la garantía principal es
un diff completo, verde y revisado.
