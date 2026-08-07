---
id: 18
titulo: "canApply false no vincula a change_apply"
estado: "cerrada"
prioridad: 4
etiquetas: ["contrato", "mcp", "escritura"]
origen: "hallazgo-de-implementacion"
abierta_en: "2026-08-02"
cerrada_en: "2026-08-07"
revisada_en: "2026-08-07"
epica: "E29"
historias: ["E29-H07"]
relacionadas: [13, 15]
---

# §18 — `canApply: false` no vincula a `change_apply`

**Hallazgo** (2026-08-02, al ejecutar el guion de la demo contra un workspace con un error
preexistente deliberado): `change_plan` bajo la policy por defecto (`requireValidResult: true`)
devuelve `canApply: false` cuando el resultado simulado no es válido — pero `change_apply` **no
consulta ese veredicto**: `can_apply` se computa en `App::change_plan`
(`crates/lodestar-app/src/lib.rs:1783`, con `core::plan::can_apply`) y viaja al cliente, mientras
que el camino de apply ejerce solo su propio gate de staging, que **sí aplicó** el plan
(`applied: true` con `validation.valid: false`, sin diagnósticos nuevos). Resultado: la superficie
dice «este plan no es aplicable bajo tu policy» y el motor lo aplica igual si el cliente insiste.

**Opciones**: (a) `change_apply` rechaza planes con `canApply: false` (código nuevo o
`INVALID_SCHEMA`? — tocaría el catálogo de 16 y la frontera); (b) documentar `canApply` como
**advisory** (el contrato ya no promete que apply lo ejerza; el gate real es el de staging); (c)
que el gate de staging adopte la policy del plan. Cada una toca la frontera MCP → historia propia
con delta de contrato, fuera de E27 (que tiene prohibido cambiar comportamiento del motor).

**DECIDIDO (2026-08-02): (a) vinculante** — `change_apply` rechaza los planes con
`canApply: false`. Es lo que cualquiera asume al leer el campo, y dejarlo como consejo que nadie
ejerce es exactamente la clase de superficie que E23/E26 vinieron a cerrar. Entra en la **épica de
honestidad de superficie**, con **delta de contrato**: hay que elegir si el rechazo usa un código
existente del catálogo de 16 o justifica el decimoséptimo — el catálogo solo se ha abierto una vez
(E23-H14), así que la carga de la prueba está en el código nuevo.

**Mitigación en E27** (vigente hasta que la épica cierre): el guion de la demo usa
`policy: {requireValidResult: false}` — coherente con el error deliberado del workspace — y no
muestra la incoherencia; `docs/user/safe-changes.md` describe `canApply` como veredicto del **plan**,
sin prometer que apply lo re-ejerza. Esa redacción hay que revisarla al implementar.

**Cerrada (2026-08-07) por `E29-H07`** (commits `9df617f` + `f97004c`, épica
[`epica-29-honestidad-superficie.md`](../requirements/epica-29-honestidad-superficie.md)):
`change_apply` rechaza los planes con `canApply: false` bajo su propia policy; juez ciego aprobada,
con el bloqueante de documentación (la doc que negaba el gate) saldado en el remate, que además fija
los pines de representación.
