---
name: ciclo
description: Pipeline completo de una historia - /historia (spec + ratificación) → /tdd (rojo-verde) → /contrato --check si toca la frontera → /juzgar (juez ciego) → docs de estado → commit en rama. El camino feliz para features; úsalo cuando el usuario pida "hacer X" de principio a fin.
argument-hint: <descripción de la feature | ID E<n>-H<nn>>
---

# /ciclo — de necesidad a commit juzgado

Encadena el flujo completo del repo. Cada etapa tiene su skill; tú orquestas y no te saltas puertas.

## Etapas (en orden, con sus puertas)

1. **Spec** — si no hay historia ratificada para el argumento, ejecuta `/historia`. **Puerta:
   ratificación explícita del usuario.** (Si ya existe historia ratificada, salta a 2. Si el
   alcance NO cabe en una historia, esto no es un ciclo: redirige a `/planificar`, que produce la
   épica cuyas historias sí se ejecutan con `/ciclo`.)
2. **Rama** — trabaja en una rama `claude/<slug-de-la-historia>` desde `main` actualizado.
3. **TDD** — ejecuta `/tdd <ID>` (rojo → verde → gates). **Puerta: gates locales en verde.**
4. **Contrato** — si el diff toca `core::types`, `src-tauri`, `lodestar-mcp` o `frontend/src/lib/ipc/`,
   ejecuta `/contrato --check`. **Puerta: sin drift BLOQUEANTE.**
5. **Juicio** — ejecuta `/juzgar <ID>` (añade `--panel` si el diff es grande, toca la frontera o
   superficies de seguridad). **Puerta: veredicto APROBADA (o CON RESERVAS aceptadas explícitamente
   por el usuario).** Si RECHAZADA: vuelve a la etapa que el veredicto señale (3 si es de
   implementación, 1 si es de spec) y re-juzga con un juez fresco — nunca negocies con el mismo juez.
6. **Estado** — actualiza `IMPLEMENTATION_STATUS.md` (y `DECISIONES.md` si la historia cerró o abrió
   algo) en el mismo cambio.
7. **Commit** — mensaje en español que nombra la historia (`E<n>-H<nn>: <qué entrega>`). El push/PR
   queda a criterio del usuario: propónlo, no lo hagas por defecto.

## Opcionales que puedes proponer al cierre

- `/mutantes --file <módulos tocados>` para medir si la suite nueva muerde.
- `/simplify` si el verde dejó complejidad evidente.

## Regla de oro

Las puertas no se negocian en caliente: si una falla, se vuelve atrás con el artefacto corregido.
El coste de re-ratificar una spec es minutos; el de un invariante roto en `main`, no.
