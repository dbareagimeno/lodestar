---
name: ciclo
description: Orquesta una historia ratificada o un bugfix completo con rojo y verde separados, contrato/docs, gates reales y revisión fresca. Para arquitectura usa /planificar; para docs mecánicas aplica un check específico.
argument-hint: <descripción de bug | ID E<n>-H<nn>>
---

# /ciclo — entrega completa

1. **Clasifica**: bug inequívoco (issue/reproducción), historia ratificada, docs/mecánico o
   arquitectura. Redirige arquitectura a `/planificar` y specs sin ratificar a `/historia`.
2. **Base**: trabaja en el checkout actual basado en `develop`. No crees rama ni commit salvo
   petición explícita.
3. **Rojo**: toma `target/agent-state/pre-red.json`, lanza `autor-tests` y ejecuta
   `phase-scope.py verify-tests-only`.
4. **Lock**: bloquea los tests y fixtures exactos con `tdd-test-lock.py snapshot`.
5. **Verde**: lanza `implementador`; verifica el lock antes y después. Si un test contradice una
   spec inequívoca, arbitra con un juez fresco; si la spec es ambigua, vuelve al usuario.
6. **Entrega**: completa contrato, docs y estado afectados.
7. **Gates**: usa `agent-gates.sh full`; añade `contract` cuando toque MCP.
8. **Juicio**: ejecuta `/juzgar` sobre el entregable completo. Tras reparar un fallo reproducible,
   repite gates y usa jueces nuevos.

No pares para pedir permiso ante un bug inequívoco dentro del alcance. Sí para una decisión
normativa, una spec ambigua o una ampliación material. Entrega diff y evidencia; no hagas commit,
push ni PR por defecto.
