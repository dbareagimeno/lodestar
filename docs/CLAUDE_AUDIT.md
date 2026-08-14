# Auditoría del workflow `.claude`

Auditoría realizada sobre `develop` antes de aplicar el overlay Codex. El objetivo no es eliminar
las ideas institucionales del flujo, sino separar las garantías valiosas de la implementación que
había acumulado drift.

## Resultado

La secuencia correcta era y sigue siendo:

> spec o reproducción → rojo independiente → implementación → contrato/docs → gates → revisión fresca

Los cuatro pilares se conservan: SDD, BDD, TDD con separación de poderes y jueces sin contexto del
implementador. Los problemas estaban en la autoridad contradictoria, gates incompletos y reglas que
solo existían como prohibiciones en prompts.

## Hallazgos observados

| Hallazgo | Evidencia previa | Riesgo | Resolución aplicada |
| --- | --- | --- | --- |
| Base de trabajo incorrecta | `.claude/skills/ciclo`, `docs/WORKFLOWS.md` | Construir y revisar desde una release antigua. | Base y diff fijados en `develop`; git deja de ser gate. |
| Prototipo con dos autoridades | `.claude/agents/implementador.md` | Implementar semántica retirada pese a una spec vigente. | Jerarquía única en `AGENTS.md`; prototipo histórico. |
| Gates incompletos | `implementador`, `/tdd` | Declarar verde sin ejecutar crash-recovery. | `scripts/agent-gates.sh full` replica todos los gates críticos. |
| Separación solo textual | `autor-tests`, `implementador` | Tests o producción modificados por la fase equivocada. | Inventario de alcance y lock SHA-256. |
| Guidance retirada | agentes/skills antiguos | Órdenes sobre VCS, UI, ZIP o skills inexistentes. | Linter ejecutable y compatibilidad legacy corregida. |
| Panel por peor voto | `/juzgar` | Una premisa errónea obtiene veto automático. | Síntesis por evidencia con tres responsabilidades. |
| Intervención humana excesiva | `/tdd`, `/juzgar` | Latencia ante bugs inequívocos. | Reparación automática; usuario solo para norma/ambigüedad. |
| Contrato autocorrector | `/contrato`, guardián | Promover drift del código a norma. | Delta ratificado manda; divergencia sin delta bloquea. |
| Proceso único para todo | `CLAUDE.md`, workflows | Coste desproporcionado para bugs/docs. | Cuatro niveles de riesgo. |
| Juicio antes de docs finales | `/ciclo` | Aprobar un entregable todavía documentalmente falso. | Docs y estado se completan antes del expediente. |

## Detalle

### Rama base

El flujo operativo creaba una rama de agente desde `main`, mientras `CONTRIBUTING.md`, CI y el
runbook de release establecen `develop` como integración. El overlay no crea ramas: presupone un
checkout basado en `develop` y calcula el expediente contra esa base.

### Autoridad del prototipo

El implementador afirmaba simultáneamente que el prototipo decidía el comportamiento y que había
dejado de ser spec. La autoridad vigente queda centralizada en `AGENTS.md`; `prototype/` no participa
en resolución de conflictos ni en pruebas diferenciales.

### Gates reales

Los prompts omitían los dos comandos con `test-failpoints`, el build de todos los targets, algunas
políticas de dependencias y el smoke de la demo. `agent-gates.sh` ofrece una única lista ejecutable
y separa `contract`, `policy` y `full`.

### Separación rojo/verde

`autor-tests` podía escribir en cualquier lugar, incluidos módulos inline de `src/`, y el
implementador solo recibía una prohibición. `phase-scope.py` permite exclusivamente tests de
integración y fixtures; `tdd-test-lock.py` bloquea los ficheros exactos tras aceptar el rojo.

### Guidance obsoleta

Se observaron referencias operativas a un fichero de tests VCS retirado, una frontera de UI
inexistente, una herramienta de pulido no instalada, siete invariantes aunque uno estaba tachado y
modelos fijados por nombre. `check-agent-guidance.py --include-legacy` convierte esas contradicciones
en un error localizable.

### Revisión

La lente antigua de paridad podía rechazar por no reproducir un oráculo ya retirado y el agregador
elegía el peor voto sin distinguir evidencia de sospecha. Los agentes nuevos separan corrección,
arquitectura y tests; cada hallazgo bloquea por su evidencia, no por su procedencia o cantidad.

### Contrato

El repositorio ya tenía tests Rust estructurales valiosos. El problema era mezclar comprobación,
sincronización y autoridad en un solo agente. El script nuevo cubre nombres, orden, despacho,
perfiles y schemas; los tests existentes cubren detalles del wire; la semántica queda en revisión
de arquitectura. El YAML no se regenera desde un cambio accidental.

### Coste

La regla absoluta de historia para cualquier cambio contradecía la política externa de
contribución. Ahora docs/mecánico, bugfix, historia y arquitectura tienen recorridos distintos sin
renunciar a una prueba roja cuando cambia comportamiento.

## Estado tras la migración

- `AGENTS.md` es la entrada de Codex y contiene solo acuerdos estables.
- `.agents/skills/` aloja cinco workflows públicos.
- `.codex/agents/` contiene cuatro roles de producción/especificación y tres jueces de lectura.
- `scripts/` hace observables alcance, locks, contrato, guidance y gates.
- `.claude/` se conserva como compatibilidad, alineado con la misma autoridad mientras dure la
  transición.
- `docs/CODEX_WORKFLOW.md` sustituye a prompts dispersos como referencia operativa común.

Los documentos históricos pueden narrar capacidades retiradas en su contexto. El linter se centra
en instrucciones operativas capaces de dirigir una sesión actual.
