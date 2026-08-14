---
name: ciclo
description: Ejecutar un bugfix o una historia ratificada de Lodestar con rojo y verde separados, alcance y tests bloqueados por hash, contrato/docs, gates reales y revisión fresca. Usar para implementar cambios de comportamiento; no usar para docs puras, cambios mecánicos ni diseño arquitectónico aún no ratificado.
---

# Ciclo

Entregar un cambio completo y revisado sin depender de prohibiciones textuales. Mantener el estado
efímero de la fase en `target/agent-state/`.

## 1. Clasificar y fijar la spec

- Bug inequívoco: usar el issue o una reproducción estable como spec; no exigir historia completa.
- Historia: localizar el texto ratificado en `requirements/`. Si falta ratificación, usar
  `$especificar` y detener el ciclo hasta obtenerla.
- Arquitectura o varias historias: redirigir a `$planificar`.
- Docs o mecánico: aplicar el cambio directamente y ejecutar solo comprobaciones específicas.

Registrar criterios, ficheros protegidos, contrato afectado y gates esperados. Trabajar sobre un
checkout basado en `develop`; no crear rama ni commit.

## 2. Producir y verificar el rojo

1. Crear `target/agent-state/`.
2. Tomar el snapshot previo:

   ```bash
   python3 scripts/phase-scope.py snapshot target/agent-state/pre-red.json
   ```

3. Delegar a un agente fresco `autor_tests` la spec completa, la ruta del snapshot y nada de la
   solución. Exigir tests de integración o fixtures, mapeo criterio-test y evidencia del fallo.
4. Verificar el alcance de forma independiente:

   ```bash
   python3 scripts/phase-scope.py verify-tests-only target/agent-state/pre-red.json
   ```

5. Repetir los tests nuevos y confirmar que fallan por la razón correcta. Un test que pasa, no
   ejecuta el camino o falla antes del comportamiento buscado no constituye rojo.
6. Bloquear por hash todos los tests y fixtures creados o modificados:

   ```bash
   python3 scripts/tdd-test-lock.py snapshot \
     target/agent-state/tests.json <test.rs> [<fixture> ...]
   ```

## 3. Producir y verificar el verde

1. Delegar a un `implementador` fresco la spec, lista exacta de tests rojos y ruta del lock.
2. Verificar el lock antes y después de cualquier reparación:

   ```bash
   python3 scripts/tdd-test-lock.py verify target/agent-state/tests.json
   ```

3. Si el implementador identifica un bug inequívoco dentro del alcance, corregirlo y repetir el
   verde automáticamente.
4. Si afirma que un test contradice una spec inequívoca, convocar un `juez_tests` fresco con solo
   spec, test y evidencia roja. Si confirma el defecto, volver a un `autor_tests` fresco, regenerar
   el lock y demostrar de nuevo el rojo. Si la spec es ambigua, pedir decisión al usuario.

## 4. Completar la entrega

Antes del juicio final, incluir:

1. implementación;
2. contrato MCP si la frontera cambió;
3. documentación pública e interna afectada;
4. `IMPLEMENTATION_STATUS.md` o `decisiones/` solo si su estado cambió realmente.

Un delta MCP ratificado manda. Si código y contrato divergen sin delta ratificado, bloquear; no
promover automáticamente el código a nueva norma.

## 5. Ejecutar gates

- Si toca la frontera: `scripts/agent-gates.sh contract`.
- Para bugfix o historia con código: `scripts/agent-gates.sh full`.
- Para docs/mecánico: comprobación específica más `scripts/agent-gates.sh policy` si cambia la
  guidance o una política mecanizada.

No declarar verde con gates incompletos. Los tests con `test-failpoints` forman parte del gate
completo.

## 6. Revisar y reparar

Invocar `$revisar` sobre el entregable completo. Reparar automáticamente fallos reproducibles que
no abran una decisión normativa, repetir lock, gates y convocar jueces frescos. Volver al usuario
solo ante una spec ambigua, una decisión abierta o una ampliación material de alcance.

Entregar diff, criterios cubiertos, evidencia de rojo/verde, gates y veredicto. No hacer commit,
push ni PR salvo petición explícita.
