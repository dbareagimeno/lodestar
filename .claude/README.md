# Agentes y skills de lodestar

> Cómo se desarrolla en este repo: **SDD** (spec primero, en `requirements/`), **TDD** (test
> primero, con separación de poderes), **BDD** (criterios Dado/Cuando/Entonces mapeados a tests) y
> **jueces ciegos** (revisión sin contexto contaminado). La frontera front↔back tiene contrato
> explícito en `contracts/`. Este fichero es la **referencia operativa**; la guía explicativa
> (el porqué del proceso y los recorridos, con diagramas) está en
> [`docs/WORKFLOWS.md`](../docs/WORKFLOWS.md).

> **El motor es headless** (`ARCHITECTURE.md §19`, giro E9–E14): la **UI de escritorio se retiró de
> `main`** (`frontend/` Svelte + `src-tauri/`) y vive íntegra en la rama `experimental/ui-desktop`.
> Con ella se retiraron el skill `/ux` y el agente `disenador-ux`: **el circuito UX ya no existe en
> `main`** — si la UI vuelve a evolucionar, se hace en esa rama. `git` también está fuera de la
> superficie de las tools/skills: ninguna receta de este documento invoca ya subcomandos git de la
> CLI ni tools MCP de git. El crate **`lodestar-app`** (servicios de caso de uso compartidos por
> `lodestar-cli`/`lodestar-mcp`; ver `CLAUDE.md` y `ARCHITECTURE.md §19.2`) ya existe y es consumido
> por ambas fachadas.

## Agentes (`.claude/agents/`)

| Agente | Modelo | Rol |
|---|---|---|
| `planificador` | sesión | Convierte una spec/diseño mayor en una **épica**: fase A (propuesta de diseño → adenda a `ARCHITECTURE.md`) y fase B (descomposición en historias ordenadas + trazabilidad), cada una con puerta de ratificación. |
| `historiador` | sesión | Redacta historias `E<n>-H<nn>` con criterios BDD y delta de contrato. Propone; el usuario ratifica. |
| `autor-tests` | opus | Fase ROJA: escribe los tests de la historia y **verifica que fallan**. No toca `src/`. |
| `implementador` | opus | Fase VERDE: pone los tests en verde + gates. **No puede modificar los tests.** |
| `juez-historia` | sesión | Juez **ciego**: recibe solo spec + diff, veredicto estructurado criterio a criterio. |
| `guardian-contrato` | opus | Coherencia de la frontera MCP: `core::types` (fuente de tipos) ↔ tools de `lodestar-mcp` ↔ `contracts/mcp.yml`. |

La separación de poderes es deliberada: quien especifica no implementa, quien testea no implementa,
quien implementa no toca los tests, y quien juzga no conoce las intenciones de nadie.

## Skills (`.claude/skills/`)

| Skill | Qué hace |
|---|---|
| `/planificar <spec\|§N>` | De spec/diseño mayor a épica de historias ordenadas (2 puertas: diseño y épica). Puerta de entrada de las **features grandes**. |
| `/historia <desc\|ID>` | Redacta/refina la spec y pide ratificación. Puerta de entrada del trabajo que cabe en **una historia**. |
| `/tdd <ID>` | Rojo → verde → refactor de una historia ratificada. |
| `/juzgar [ID] [--panel]` | 1 juez ciego (o panel de lentes: corrección / invariantes / paridad+tests). |
| `/contrato [--check]` | Verifica (o sincroniza) la frontera MCP↔`core::types` contra `contracts/mcp.yml`. |
| `/mutantes [-p crate] [--file ruta]` | cargo-mutants scoped: qué mutaciones sobreviven a la suite + tests propuestos. |
| `/ciclo <desc\|ID>` | Pipeline completo: historia → tdd → contrato → juez → docs → commit en rama. |

## Workflows recomendados

La pirámide del flujo, de arriba abajo — cada nivel tiene su puerta:

```
/planificar  (spec/feature grande → diseño ratificado + épica de historias)
   └─ /ciclo E<n>-H<nn>   (una historia de principio a fin, en orden de construcción)
        └─ /historia · /tdd · /contrato · /juzgar   (etapas del ciclo)
```

- **Feature grande** (p. ej. ghosts+templates, `DECISIONES.md §10`): `/planificar` primero —
  cierra el diseño y produce la épica — y después `/ciclo` historia a historia. Las puertas
  (ratificación de diseño, de épica, gates, sin drift, veredicto) no se negocian: si una falla,
  se vuelve atrás.
- **Feature que cabe en una historia**: `/ciclo` directo (que empieza por `/historia`).
- **Trabajo con UI nueva**: **no aplica en `main`** — la UI de escritorio se retiró a la rama
  `experimental/ui-desktop` y con ella el circuito `/ux`; el motor es headless. Si la UI vuelve a
  evolucionar, se hace en esa rama, no en el flujo de este repo.
- **Bugfix**: no hace falta historia completa — test de regresión primero (rojo, reproduce el bug),
  fix (verde), `/juzgar` simple con el issue como spec. Si el bug es de paridad con el prototipo,
  el test va además al arnés diferencial o a la sección «Regresiones de paridad con el
  prototipo» de `core.rs`.
- **Refactor**: los tests existentes son la red. `/mutantes` con el mismo alcance **antes y
  después**: si tras el refactor sobreviven mutantes que antes morían, la suite se debilitó.
  `/simplify` para el pulido final.
- **Cambio en la frontera MCP** (`core::types`, tools de `lodestar-mcp`, `contracts/mcp.yml`):
  siempre `/contrato --check` antes del PR, y `/juzgar --panel`.
- **Release**: runbook de `RELEASING.md` (sin skill; ya está resuelto).

## Los jueces son ciegos: qué significa y por qué

Un juez se lanza **siempre como agente fresco** y recibe únicamente: la historia, el diff y las
rutas de los docs de autoridad. Nunca recibe resúmenes de la conversación, justificaciones ni el
razonamiento del implementador — si conociera la intención, heredaría sus sesgos y aprobaría por
empatía. Corolarios: un veredicto RECHAZADA se re-juzga con un juez **nuevo** (no se negocia con el
mismo), y el orquestador tampoco escribe código en `/tdd` para no contaminar lo que luego describe.

## Decisiones de proceso ya tomadas (no relitigar sin motivo)

- **BDD sin Gherkin ejecutable**: los escenarios Dado/Cuando/Entonces viven en los criterios de las
  historias y mapean a tests Rust nombrados. No se introduce cucumber-rs: el arnés diferencial
  JS-vs-Rust ya es el oráculo de comportamiento vivo, y un runner Gherkin duplicaría maquinaria.
- **Mutation testing a demanda, sin CI**: `/mutantes` scoped a lo que tocó cada historia. Se
  integrará en CI solo si demuestra valor sostenido.
- **Contrato YAML = superficie y semántica; los tipos viven en `core::types`** (invariante #4).
  Los tipos los consumen directamente `lodestar-cli`/`lodestar-mcp`; **ya no hay espejo `.ts`** (se
  retiró con la UI a `experimental/ui-desktop`), así que la generación con ts-rs de `DECISIONES.md §4`
  queda obsoleta para el espejo TS.
