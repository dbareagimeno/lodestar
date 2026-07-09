# Agentes y skills de lodestar

> Cómo se desarrolla en este repo: **SDD** (spec primero, en `requirements/`), **TDD** (test
> primero, con separación de poderes), **BDD** (criterios Dado/Cuando/Entonces mapeados a tests) y
> **jueces ciegos** (revisión sin contexto contaminado). La frontera front↔back tiene contrato
> explícito en `contracts/`.

## Agentes (`.claude/agents/`)

| Agente | Modelo | Rol |
|---|---|---|
| `historiador` | sesión | Redacta historias `E<n>-H<nn>` con criterios BDD y delta de contrato. Propone; el usuario ratifica. |
| `autor-tests` | opus | Fase ROJA: escribe los tests de la historia y **verifica que fallan**. No toca `src/`. |
| `implementador` | opus | Fase VERDE: pone los tests en verde + gates. **No puede modificar los tests.** |
| `juez-historia` | sesión | Juez **ciego**: recibe solo spec + diff, veredicto estructurado criterio a criterio. |
| `guardian-contrato` | opus | Coherencia de las 5 superficies de la frontera (`types.rs` ↔ `types.ts` ↔ src-tauri ↔ mcp ↔ `contracts/*.yml`). |

La separación de poderes es deliberada: quien especifica no implementa, quien testea no implementa,
quien implementa no toca los tests, y quien juzga no conoce las intenciones de nadie.

## Skills (`.claude/skills/`)

| Skill | Qué hace |
|---|---|
| `/historia <desc\|ID>` | Redacta/refina la spec y pide ratificación. Puerta de entrada de todo trabajo. |
| `/tdd <ID>` | Rojo → verde → refactor de una historia ratificada. |
| `/juzgar [ID] [--panel]` | 1 juez ciego (o panel de 3 lentes: corrección / invariantes / paridad+tests). |
| `/contrato [--check]` | Verifica (o sincroniza) la frontera front↔back contra `contracts/`. |
| `/mutantes [-p crate] [--file ruta]` | cargo-mutants scoped: qué mutaciones sobreviven a la suite + tests propuestos. |
| `/ciclo <desc\|ID>` | Pipeline completo: historia → tdd → contrato → juez → docs → commit en rama. |

## Workflows recomendados

- **Feature nueva** (p. ej. ghosts+templates, `DECISIONES.md §10`): `/ciclo` completo. Las puertas
  (ratificación, gates, sin drift, veredicto) no se negocian: si una falla, se vuelve atrás.
- **Bugfix**: no hace falta historia completa — test de regresión primero (rojo, reproduce el bug),
  fix (verde), `/juzgar` simple con el issue como spec. Si el bug es de paridad con el prototipo,
  el test va además al arnés diferencial o a la sección «Regresiones de paridad con el
  prototipo» de `core.rs`.
- **Refactor**: los tests existentes son la red. `/mutantes` con el mismo alcance **antes y
  después**: si tras el refactor sobreviven mutantes que antes morían, la suite se debilitó.
  `/simplify` para el pulido final.
- **Cambio en la frontera front↔back** (`core::types`, `src-tauri`, `lodestar-mcp`,
  `frontend/src/lib/ipc/`): siempre `/contrato --check` antes del PR, y `/juzgar --panel`.
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
  Generación del espejo `.ts` con ts-rs: dirección ratificada, pendiente de implementar como
  primera historia del nuevo flujo (E0-H04/E6-H03, `DECISIONES.md §4`).
