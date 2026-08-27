# Lodestar: instrucciones de proyecto

Lodestar es un motor headless de integridad semántica para workspaces Markdown. El producto activo
son los crates Rust y las fachadas CLI/MCP; la UI, git como capacidad y el prototipo ejecutable no
forman parte de la superficie vigente.

## Autoridad

Resolver contradicciones en este orden:

1. `ARCHITECTURE.md` (diseño ratificado).
2. `docs/REFACTOR_PHASE_2.md` y `ARCHITECTURE.md §20` (comportamiento de la migración).
3. Historias ratificadas de `requirements/`.
4. Decisiones abiertas de `decisiones/` (requieren criterio del usuario).
5. `contracts/mcp.yml` (superficie y semántica MCP).
6. `IMPLEMENTATION_STATUS.md` (estado, no diseño).

`prototype/index.html` es referencia histórica de v0.2.x. Nunca usarlo como oráculo ni reintroducir
sondas diferenciales.

## Reglas del repositorio

- La base de integración es `develop`; `main` contiene únicamente releases.
- Toda historia se implementa en un worktree y rama nuevos, exclusivos de esa historia y creados
  después de actualizar `develop`. No reutilizar el checkout principal ni el worktree de otra historia.
- No crear commits, pushes ni PRs salvo petición explícita. Cuando exista una ejecución remota de CI,
  vigilarla hasta que todos los checks obligatorios estén verdes; un rojo se diagnostica, registra y
  repara antes de cerrar la historia.
- Preservar los seis invariantes activos de `ARCHITECTURE.md`/`CLAUDE.md`: Markdown en disco como
  fuente de verdad, core puro, una sola verdad computada, un solo contrato de tipos, único escritor
  y `RelPath` como chokepoint.
- No decidir por cuenta propia una entrada abierta de `decisiones/`. Exponer la decisión normativa
  y pedir ratificación.
- Corregir automáticamente bugs inequívocos dentro del alcance. Si un test contradice una spec
  inequívoca, pedir arbitraje a un `juez_tests` fresco; si la spec es ambigua, volver al usuario.
- Completar contrato, documentación y estado afectados antes del juicio final.

## Proceso proporcional al riesgo

| Nivel | Flujo mínimo |
| --- | --- |
| Docs o mecánico | Cambio directo, comprobación específica y revisión del diff. |
| Bugfix | Reproducción estable, test rojo, fix, gates y revisión fresca. |
| Historia | Spec ratificada, rojo/verde separados, contrato/docs, gates y revisión. |
| Arquitectura | `$planificar`, dos ratificaciones y después historias individuales. |

Usar los skills de `.agents/skills/`: `$planificar`, `$especificar`, `$ciclo`, `$revisar` y
`$mutantes`. No delegar por defecto fuera de un skill que lo exija o una petición expresa del
usuario.

## Garantías ejecutables

- Fase roja: `python3 scripts/phase-scope.py snapshot ...` y después `verify-tests-only`.
- Bloqueo de tests: `python3 scripts/tdd-test-lock.py snapshot ...` y `verify` antes y después del
  verde.
- Contrato: `scripts/agent-gates.sh contract`.
- Política: `scripts/agent-gates.sh policy`.
- Entrega de código: `scripts/agent-gates.sh full`.
- Registro append-only de rojos CI: `python3 scripts/check-ci-failure-log.py` (incluido en `policy`
  y `full`).

Preferir tests de integración en `crates/<crate>/tests/` y fixtures de
`crates/lodestar-fixtures/`. Un test inline comparte fichero con producción y no permite bloquear
de forma verificable las dos fases.

## Revisión

Calcular la entrega contra `develop` e incluir cambios committed, staged, unstaged y ficheros
nuevos. Los jueces reciben únicamente spec, diff, autoridades y evidencia de gates; nunca el
razonamiento ni el resumen del implementador.

Un criterio incumplido, un invariante roto o un fallo reproducible bloquea por sí solo. Un riesgo
hipotético abre investigación; no se convierte en bloqueante por votación. Cada reparación termina
con gates repetidos y un juez fresco.
