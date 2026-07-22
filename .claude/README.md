# Agentes y skills de lodestar

> Cómo se desarrolla en este repo: **SDD** (spec primero, en `requirements/`), **TDD** (test
> primero, con separación de poderes), **BDD** (criterios Dado/Cuando/Entonces mapeados a tests) y
> **jueces ciegos** (revisión sin contexto contaminado). La frontera front↔back tiene contrato
> explícito en `contracts/`. Este fichero es la **referencia operativa**; la guía explicativa
> (el porqué del proceso y los recorridos, con diagramas) está en
> [`docs/WORKFLOWS.md`](../docs/WORKFLOWS.md).

> **El motor es headless** (`ARCHITECTURE.md §19`, giro E9–E14): `frontend/` y `src-tauri/` quedan
> **CONGELADOS** en el flujo de desarrollo de v2. `/ciclo`, `/historia` y `/ux` **no** modifican esos
> directorios; el skill `/ux` y el agente `disenador-ux` quedan marcados **no aplicables al giro
> headless** — se conservan documentados abajo (igual que git quedó dormido en `lodestar-vcs`), por
> si la UI vuelve a evolucionar en el futuro. `git` también sale de la superficie de las tools/skills:
> ninguna receta de este documento invoca ya subcomandos git de la CLI ni tools MCP de git. Las
> historias de `E10` en adelante introducen el crate **`lodestar-app`** (servicios de caso de uso
> compartidos por `lodestar-cli`/`lodestar-mcp`; ver `CLAUDE.md` y `ARCHITECTURE.md §19.2`) — no
> existe todavía; cuando llegue, sus historias siguen el mismo circuito `/historia` → `/tdd`.

## Agentes (`.claude/agents/`)

| Agente | Modelo | Rol |
|---|---|---|
| `planificador` | sesión | Convierte una spec/diseño mayor en una **épica**: fase A (propuesta de diseño → adenda a `ARCHITECTURE.md`) y fase B (descomposición en historias ordenadas + trazabilidad), cada una con puerta de ratificación. |
| `historiador` | sesión | Redacta historias `E<n>-H<nn>` con criterios BDD y delta de contrato. Propone; el usuario ratifica. |
| `autor-tests` | opus | Fase ROJA: escribe los tests de la historia y **verifica que fallan**. No toca `src/`. |
| `implementador` | opus | Fase VERDE: pone los tests en verde + gates. **No puede modificar los tests.** |
| `juez-historia` | sesión | Juez **ciego**: recibe solo spec + diff, veredicto estructurado criterio a criterio. |
| `guardian-contrato` | opus | Coherencia de las 5 superficies de la frontera (`types.rs` ↔ `types.ts` ↔ src-tauri ↔ mcp ↔ `contracts/*.yml`). |
| `disenador-ux` | sesión | **No aplicable al giro headless** (UI congelada). Experto UX: flujos `.excalidraw`, mockups HTML unitarios por pantalla/estado y auditorías contra buenas prácticas (Nielsen, ley de Jakob, estados obligatorios, accesibilidad). Solo escribe en `design/`, nunca código de producción. Documentado como registro histórico, no se invoca en v2. |

La separación de poderes es deliberada: quien especifica no implementa, quien testea no implementa,
quien implementa no toca los tests, y quien juzga no conoce las intenciones de nadie.

## Skills (`.claude/skills/`)

| Skill | Qué hace |
|---|---|
| `/planificar <spec\|§N>` | De spec/diseño mayor a épica de historias ordenadas (2 puertas: diseño y épica). Puerta de entrada de las **features grandes**. |
| `/historia <desc\|ID>` | Redacta/refina la spec y pide ratificación. Puerta de entrada del trabajo que cabe en **una historia**. |
| `/tdd <ID>` | Rojo → verde → refactor de una historia ratificada. |
| `/juzgar [ID] [--panel]` | 1 juez ciego (o panel de lentes: corrección / invariantes / paridad+tests, + fidelidad UX automática si el diff toca `frontend/` y la historia cita artefactos de `design/`). |
| `/contrato [--check]` | Verifica (o sincroniza) la frontera front↔back contra `contracts/`. |
| `/mutantes [-p crate] [--file ruta]` | cargo-mutants scoped: qué mutaciones sobreviven a la suite + tests propuestos. |
| `/ux <flujo\|mockup\|audit> <desc\|ID\|ruta>` | **No aplicable al giro headless** (UI congelada) — spec visual ratificable en `design/` (flujos, mockups unitarios) o auditoría de la UI contra heurísticas. Documentado por si la UI vuelve a evolucionar; no se invoca en v2. |
| `/ciclo <desc\|ID>` | Pipeline completo: historia → tdd → contrato → juez → docs → commit en rama. **No toca `frontend/`/`src-tauri/`** en v2 (UI congelada). |

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
- **Trabajo con UI nueva**: **no aplica en v2** — la UI está congelada (`§19.1`), así que no hay
  circuito de entrada por `/ux` mientras el motor sea headless. La descripción que sigue queda
  como registro de cómo funcionaba el circuito antes del giro, por si la UI vuelve a evolucionar:
  `/ux flujo` (y mockups si hace falta verlo) **antes** de `/historia`; los artefactos ratificados
  de `design/` se citan en las Referencias de la historia como spec visual. Patrón conocido por
  defecto (ley de Jakob) — un patrón nuevo exige justificación explícita en el artefacto.
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
