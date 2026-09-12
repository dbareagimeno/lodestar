# Workflow de desarrollo con Codex

Este documento es la referencia operativa del flujo de agentes de Lodestar. Conserva spec primero,
TDD independiente, contrato explícito y revisión fresca, pero aplica esas garantías de forma
proporcional al riesgo y las refuerza con scripts verificables.

## Principios

1. **Spec suficiente antes de comportamiento nuevo.** Una feature usa historia ratificada; un bug
   inequívoco puede usar issue o reproducción estable.
2. **Rojo independiente.** Un agente define pruebas a partir de la spec, no de una solución.
3. **Verde con tests bloqueados.** El implementador no puede cambiar los ficheros aceptados en rojo
   sin romper un hash.
4. **Entregable completo antes de juzgar.** Código, tests, contrato, docs y estado entran juntos en
   el expediente.
5. **Revisión fresca por evidencia.** Los jueces no reciben intención ni razonamiento del autor.
6. **Aislamiento por historia.** Cada historia nace desde `develop` actualizado en una rama y un
   worktree nuevos, que no se reutilizan para otra historia.
7. **CI hasta verde.** Cuando la historia tiene una ejecución remota, el SHA vigente no está Done
   hasta que terminan verdes todos los checks obligatorios; cualquier rojo se diagnostica y repara.
8. **Aprendizaje acumulativo.** Toda causa confirmada de rojo CI se añade al registro append-only
   para mejorar tests, agentes, skills y gates sin reescribir incidentes anteriores.

## Jerarquía de autoridad

1. `ARCHITECTURE.md`.
2. `docs/REFACTOR_PHASE_2.md` y `ARCHITECTURE.md §20`.
3. Historias ratificadas de `requirements/`.
4. Decisiones abiertas de `decisiones/`.
5. `contracts/mcp.yml`.
6. `IMPLEMENTATION_STATUS.md`.

El prototipo de `prototype/` solo documenta v0.2.x. No resuelve discrepancias del producto actual.

## Nivel de proceso

| Nivel | Ejemplos | Flujo |
| --- | --- | --- |
| Docs o mecánico | typo, formato, movimiento sin semántica | Cambio directo, check específico, revisión del diff. |
| Bugfix | comportamiento observado contrario a una regla inequívoca | Reproducción, rojo, fix, gates, revisión fresca. |
| Historia | nueva conducta acotada | Ratificación, rojo/verde separados, contrato/docs, gates, revisión. |
| Arquitectura | nuevo subsistema, invariante o varias historias | `$planificar`, dos ratificaciones, historias individuales. |

Una tarea sube de nivel si toca contrato MCP, dependencias, rutas, concurrencia, publicación,
recuperación o decisiones abiertas.

## Piezas

| Capa | Ubicación | Responsabilidad |
| --- | --- | --- |
| Instrucciones estables | `AGENTS.md` | Autoridad, invariantes, riesgo y acciones de integración. |
| Skills | `.agents/skills/` | Workflows públicos: planificar, especificar, ciclo, revisar y mutantes. |
| Agentes | `.codex/agents/` | Roles especializados con esfuerzo y sandbox propios. |
| Enforcement | `scripts/` | Alcance, hashes, contrato, guidance y gates. |
| CI | `.github/workflows/ci.yml` | Puertas independientes de plataforma y release. |

Los roles que escriben heredan el modelo de la sesión y usan `workspace-write`. Las tres lentes de
revisión se configuran con `read-only`. La configuración de `.codex/` requiere que el checkout esté
marcado como proyecto confiable en Codex.

## Secuencia de una historia o bug

```mermaid
flowchart LR
    S["Spec o reproducción"] --> R["Rojo independiente"]
    R --> L["Lock de tests"]
    L --> G["Implementación verde"]
    G --> D["Contrato y docs"]
    D --> Q["Gates"]
    Q --> J["Revisión fresca"]
    J -- "fallo reproducible" --> G
    J -- "decisión normativa" --> S
    J -- "integración autorizada" --> C["CI del SHA vigente"]
    C -- "rojo" --> F["Registrar causa append-only"]
    F --> G
    C -- "verde" --> Z["Historia cerrada"]
```

### Rojo

El orquestador toma un inventario previo:

```bash
mkdir -p target/agent-state
python3 scripts/phase-scope.py snapshot target/agent-state/pre-red.json
```

`autor_tests` solo puede escribir tests de integración y fixtures. Después:

```bash
python3 scripts/phase-scope.py verify-tests-only target/agent-state/pre-red.json
```

La comprobación compara contenido, creación, borrado y modo de fichero. Cualquier cambio de
producción rompe la fase. Esto excluye stubs y tests inline del circuito separado; un test inline
comparte fichero con la implementación y no se puede bloquear sin bloquear también el verde.

El rojo es válido solo si el test falla por el comportamiento buscado. Un error de preparación, un
fixture ausente o una assertion que nunca se alcanza no sirven como evidencia.

### Lock

Tras aceptar el rojo, bloquear exactamente las pruebas y fixtures usados:

```bash
python3 scripts/tdd-test-lock.py snapshot \
  target/agent-state/tests.json \
  crates/lodestar-mcp/tests/mi_historia.rs
```

El implementador y el orquestador verifican antes y después:

```bash
python3 scripts/tdd-test-lock.py verify target/agent-state/tests.json
```

Cambiar una assertion, un helper o un fixture bloqueado obliga a volver a un autor fresco,
demostrar otro rojo y regenerar el lock.

### Verde y reparaciones

`implementador` recibe spec, nombres exactos de tests rojos y ruta del lock. Implementa la mínima
conducta suficiente, actualiza contrato/docs afectados y comunica los gates sin cambiar tests.

- Bug inequívoco dentro del alcance: reparar y repetir automáticamente.
- Test contrario a una spec inequívoca: arbitrar con `juez_tests` fresco.
- Spec ambigua o decisión abierta: pedir criterio al usuario.
- Reparación aceptada: repetir lock, gates y juicio con agentes nuevos.

## Contrato MCP

La comprobación se divide en dos capas.

### Estructura mecanizable

`scripts/check-contract-surface.py` compara, en orden:

- las tools registradas por `tools::list()`;
- los brazos de `tools::call()`;
- las entradas de `contracts/mcp.yml`;
- `CHANGE_TOOLS` frente a los perfiles de escritura;
- presencia de `inputSchema` y `outputSchema` en las diez tools.

`scripts/agent-gates.sh contract` añade los tests Rust existentes que parsean el YAML y ejercitan
parámetros, schemas, valores de wire y guardas anti-vacuidad.

### Semántica

`juez_arquitectura` revisa errores, invariantes, efectos de escritura, compatibilidad y conducta.

- Con delta ratificado, manda la spec.
- Sin delta ratificado, una divergencia entre código y contrato bloquea.
- El código no puede convertir por sí solo un accidente en norma actualizando el YAML.

## Gates

| Comando | Contenido |
| --- | --- |
| `scripts/agent-gates.sh contract` | Superficie estática + tests estructurales MCP. |
| `scripts/agent-gates.sh policy` | Guidance vigente, registro CI append-only, contrato estático, pureza y dependencias retiradas, fuente única de errores. |
| `scripts/agent-gates.sh full` | fmt, clippy estricto, build de targets, workspace tests, dos suites `test-failpoints`, doc, policy y demo smoke en Linux. |

El gate completo refleja las puertas de CI que `cargo test --workspace` no cubre por sí solo. No
omitir `lodestar-workspace` ni `lodestar-app` con la feature `test-failpoints`.

## Revisión

`$revisar` construye un expediente con la spec, los cuatro estados del diff (committed contra
`develop`, staged, unstaged y nuevos), autoridades y evidencia cruda.

| Juez | Pregunta |
| --- | --- |
| `juez_correccion` | ¿Se cumple cada criterio con comportamiento y documentación demostrables? |
| `juez_arquitectura` | ¿Se preservan invariantes, dependencias, transacciones, rutas y contrato? |
| `juez_tests` | ¿El rojo fue auténtico y la suite detectaría una implementación incorrecta? |

La síntesis no usa votación pesimista:

- criterio incumplido, invariante roto o fallo reproducible: bloquea;
- riesgo plausible sin evidencia suficiente: exige investigación;
- sospecha especulativa: no bloquea por mayoría;
- hallazgos duplicados: se fusionan conservando sus fuentes.

Un bugfix pequeño usa corrección y tests. Una historia o cambio de superficie protegida usa las
tres lentes. Docs mecánicas pueden revisarse directamente.

## Git e integración

Antes de empezar una historia:

1. actualizar `develop` mediante fast-forward desde `origin/develop`;
2. crear una rama nueva para la historia desde ese SHA;
3. crear un worktree nuevo y exclusivo asociado a esa rama;
4. ejecutar allí spec, rojo, verde, gates y reparaciones; nunca implementar la historia en el
   checkout principal ni reutilizar el worktree de otra.

El resultado local previo a integración es un diff verde y revisado. Commit, push y PR siguen
siendo acciones explícitas: solo se realizan cuando el usuario las solicita. Si todavía no existe
una ejecución remota autorizada, el handoff debe decir **CI pendiente** y no presentar la historia
como integrada o cerrada.

### Cierre remoto: observar, registrar, reparar

Cuando ya existe un push o PR, seguir la ejecución correspondiente al **SHA vigente** hasta estado
terminal; `gh pr checks --watch` o `gh run watch` son formas válidas de observarla. No basta con que
un subconjunto de jobs pase ni con que una ejecución anterior estuviera verde.

Si cualquier check sale rojo:

1. conservar el enlace al run y el log del job que falló;
2. determinar la causa raíz antes de relanzar a ciegas, incluida la clasificación
   `product`, `test`, `portability`, `dependency`, `policy`, `documentation`, `infrastructure` o
   `flaky`;
3. añadir una entrada a `docs/qa/ci-failures.jsonl` con síntoma, causa, reparación, prevención y
   la mejora propuesta para agentes o skills;
4. reparar en el mismo worktree con el proceso proporcional al riesgo, repetir gates locales,
   publicar el nuevo SHA autorizado y volver a observar todos los checks;
5. repetir el bucle hasta verde. Un rerun por infraestructura o flakiness también se registra: el
   tiempo perdido es evidencia útil para endurecer el proceso.

El registro es JSON Lines. La primera línea declara el schema; cada línea posterior representa una
causa confirmada. Las entradas históricas son inmutables: una corrección se añade con el campo
opcional `supersedes`, nunca se edita ni borra la línea anterior. Este ejemplo muestra la forma
canónica (se presenta partido solo para lectura; en el fichero real cada objeto ocupa una línea):

```json
{
  "id": "2026-08-26-run-123-rust-windows",
  "occurred_at": "2026-08-26T12:34:56Z",
  "run_url": "https://github.com/org/repo/actions/runs/123",
  "commit": "0123456789abcdef0123456789abcdef01234567",
  "branch": "feat/e35-h03",
  "job": "Rust · fmt · clippy · build · test",
  "platform": "windows-latest",
  "classification": "portability",
  "symptom": "el test de publicación falla al reemplazar index.db",
  "root_cause": "el handle activo no compartía FILE_SHARE_DELETE",
  "repair": "cerrar el handle antes del reemplazo y añadir regresión Windows",
  "prevention": "ejecutar el test conductual en la matriz Windows",
  "process_improvement": {
    "agents": ["implementador", "juez_arquitectura"],
    "skills": ["ciclo", "revisar"],
    "action": "añadir al checklist la semántica de sharing de handles Windows"
  }
}
```

`python3 scripts/check-ci-failure-log.py` valida schema, campos e IDs, y compara el fichero con el
SHA base del PR o con el commit anterior al push; en local usa `develop` como fallback. El contenido
histórico debe seguir siendo su prefijo byte a byte. El check se ejecuta dentro de
`scripts/agent-gates.sh policy` y `full`, por lo que CI rechaza modificaciones, borrados o
reordenaciones de entradas previas y solo acepta bytes añadidos al final.

`main` queda reservado al runbook de release de `RELEASING.md`.

## Referencias de Codex

- [AGENTS.md](https://developers.openai.com/codex/agent-configuration/agents-md)
- [Subagentes y agentes personalizados](https://developers.openai.com/codex/agent-configuration/subagents)
- [Skills](https://developers.openai.com/codex/build-skills)
- [Configuración de proyecto](https://developers.openai.com/codex/config-basic)
- [Sandbox y aprobaciones](https://developers.openai.com/codex/agent-approvals-security)
