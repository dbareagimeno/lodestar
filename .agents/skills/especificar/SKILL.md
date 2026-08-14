---
name: especificar
description: Redactar o refinar una historia de Lodestar con criterios BDD, pruebas, alcance y delta de contrato antes de implementar. Usar para features de comportamiento que caben en una historia o para corregir una spec defectuosa; no usar para un bug inequívoco, docs mecánicos ni arquitectura multi-historia.
---

# Especificar

Producir una historia ratificable que permita a un autor de tests independiente definir el rojo sin
conocer una solución concreta.

## Flujo

1. Leer `AGENTS.md`, `requirements/README.md`, la épica destino y las autoridades relevantes.
2. Si llega un ID, localizar la historia real y comprobar su estado. Si llega una descripción,
   determinar el siguiente ID sin renumerar historias existentes.
3. Delegar a un `historiador` fresco el borrador. Exigir:
   - objetivo y resultado observable;
   - referencias vigentes y decisiones afectadas;
   - alcance y fuera de alcance;
   - criterios Dado/Cuando/Entonces verificables;
   - mapeo criterio-test, negativos y guardas anti-vacuidad;
   - dependencias y delta de contrato/documentación;
   - ambigüedades normativas explícitas.
4. Revisar que ningún criterio dependa del prototipo histórico ni prescriba una implementación sin
   que la arquitectura la congele.
5. Presentar el texto al usuario para ratificación. No iniciar la fase roja antes de recibirla.
6. Aplicar la historia ratificada en `requirements/` y dejar el trabajo listo para `$ciclo`.

Si una pregunta cambia producto, compatibilidad, wire o invariantes, detener la ratificación y
devolverla al usuario. Resolver detalles mecánicos que no cambien esas decisiones sin añadir otra
puerta.
