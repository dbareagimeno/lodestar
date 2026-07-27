# Propuesta: arreglos sugeridos (`Fix` / `apply_fix`)

> **Estado**: PROPUESTA DE DISEÑO, no ratificada. **Nada de este documento está implementado.**
> Se escribe en la PR #17 (`E23-H11`), que **retira** la operación `apply_fix` de la superficie, para
> que la decisión no se pierda y `/planificar` pueda consumirla en una PR posterior.
> Autoridad de diseño: `ARCHITECTURE.md §19.3` (catálogo de diagnósticos) y `§20.11` (operaciones).
> Origen: revisión de la PR #17 (2026-07-25) y decisión del usuario de 2026-07-26.

---

## 1. Qué era y qué se rompió

La idea original (`E10-H07`/`E12-H07`) era un ciclo de auto-arreglo:

1. `knowledge_check` devuelve un diagnóstico con uno o más arreglos sugeridos colgando —
   `Fix { fixId, title, safe }` (`crates/lodestar-core/src/types.rs:286-290`).
2. El agente elige uno y manda `change_plan` con la operación `apply_fix { fixId }`.
3. `normalize_apply_fix` recompone el mismo universo de diagnósticos, localiza ese `Fix` y lo
   materializa como una operación terminal normal, que pasa por el gate como cualquier otra.

`E20-H03` retiró `core::schema`, y con él `validate_relations`/`REL-TARGET`, que era **el único sitio
del árbol que llegaba a construir un `Fix`**. Se retiró el productor y se dejaron en pie los dos
consumidores. Hoy `Check::with_fixes` (`types.rs:370`) no lo llama nadie: cero llamadas en todo el
repo.

## 2. El estado que E23 encontró, en dos mitades

**Lectura — `fixes`.** El campo existe en cada diagnóstico y siempre vale `[]`. Se ve por MCP en
`knowledge_check` y por CLI en `check --json`. No falla nunca: simplemente nunca trae nada. E
`includeSuggestedFixes` es un interruptor que apaga algo ya apagado.

**Escritura — `apply_fix`.** La operación se anunciaba en el `inputSchema` como una de las 8 ops y en
`contracts/mcp.yml`, y su implementación entera
(`crates/lodestar-core/src/plan.rs:953-958`) era una línea que devolvía error sin mirar siquiera el
workspace.

La diferencia entre las dos mitades es lo que decidió el alcance de `E23-H11`: **se retiró la
escritura y se conservó la lectura**. Un array vacío se lee como «no hay sugerencias» y es verdad.
Una operación invocable que siempre falla no solo miente en el schema: el error que devolvía era
`DOCUMENT_NOT_FOUND` (porque `CoreError::FixNotFound` no tenía código propio en un catálogo
congelado de 16, ver `lodestar-app/src/lib.rs`), o sea que el motor decía «no encuentro el
documento» con el documento perfectamente en su sitio. Un agente razonable concluye que se equivocó
de ruta, reintenta, revalida el path y se atasca donde no hay nada roto.

## 3. Por qué reactivarlo no es urgente: el catálogo vivo no da para más

De los **10 `CheckCode`** vivos (`crates/lodestar-core/src/types.rs:186-247`), la pregunta que
decide es: *¿este diagnóstico tiene una reparación única, mecánica y sin criterio humano?*

| Diagnóstico | ¿Arreglo único? |
|---|---|
| `FM-UNCLOSED` | No. ¿Cerrar el bloque dónde? Un `---` suelto arriba puede ser frontmatter sin cerrar **o** una línea horizontal del cuerpo. |
| `FM-YAML-INVALID` | No. Reparar YAML roto es adivinar la intención. |
| `DOC-CONFLICT-MARKER` | No. Elegir «lo mío» o «lo suyo» es decisión humana por definición. |
| `LINK-TARGET-MISSING` | No, y **ya se intentó**: `create_stub` y `retarget` eran exactamente eso, y `E23-H05` las retiró porque se aceptaban sin ejecutarse. Crear el documento ausente y redirigir el enlace son arreglos distintos, y ninguno es deducible. |
| `LINK-ESCAPES-WORKSPACE` | No. El destino está fuera; Lodestar no puede saber qué se quería decir. |
| `DOC-NOT-UTF8` · `PATH-NOT-UTF8` | No. Reconvertir exige adivinar la codificación de origen. |
| `DOC-TOO-LARGE` | No es una edición del documento: o sube el límite de la política o se parte la nota. |
| `SYMLINK-UNSUPPORTED` | No. Materializar o borrar el enlace es decisión del proyecto. |
| **`LINK-CASE-MISMATCH`** | **Sí.** Único caso limpio del catálogo. |

`LINK-CASE-MISMATCH` sí es mecánico: el inventario tiene la ruta real *salvo capitalización* o forma
Unicode, hay exactamente un candidato y la reparación es reescribir el enlace con la grafía buena.

**Y ese diagnóstico ya entrega todo lo necesario para arreglarlo sin `apply_fix`**
(`crates/lodestar-core/src/links.rs:549-559`): el mensaje lleva la grafía mala y la buena, y
`related` lleva la ruta real como dato estructurado. Un agente lo resuelve hoy con un `replace_text`.
No le falta información: le falta un atajo.

## 4. El trabajo real, si se retoma

Producir el `Fix` es lo barato — una decena de líneas en `links.rs`. Lo que hay que diseñar es lo de
detrás:

**4.1 El `fixId` tiene que ser direccionable en el tiempo.** El agente obtiene el `fixId` de un
`knowledge_check` en la revisión A y manda el `apply_fix` en la revisión B. El identificador tiene
que codificar suficiente para reconstruir la operación y, sobre todo, para **detectar que ya no
aplica** en vez de aplicar algo distinto. Es el mismo problema que `planHash` + `WriteConflict` ya
resuelven para los planes (`E12`), así que hay precedente que copiar — pero es diseño, no cableado.

**4.2 Necesita un código de error propio.** Para que el ciclo funcione, el agente debe distinguir
«ese `fixId` nunca existió» de «existía pero el documento avanzó». El catálogo de 16 códigos se abrió
**una única vez** en la PR #17 (`E23-H14`, `DECISIONES §13`), como decisión deliberada y aprovechando
que v0.3 ya era incompatible con v0.2. Volver a abrirlo tiene coste de wire.

**4.3 El gate ya está.** Un arreglo que corrige un diagnóstico no puede introducir otro, y eso lo
sostiene la validación de staging (`E14-H04`). No es trabajo nuevo, pero sí tests.

**4.4 Reintroducir la op es aditivo.** Volver a meter una variante en el enum de operaciones no rompe
el wire ni a ningún cliente. Lo difícil de deshacer era lo contrario: dejar una op rota publicada y
cambiarle la semántica después. Por eso retirarla ahora no cierra ninguna puerta.

## 5. Condición de entrada

> **No se implementa hasta que existan productores de `Fix` que justifiquen la maquinaria.**

Con un solo diagnóstico reparable —y encima ya resoluble con `replace_text`— el retorno es un
round-trip ahorrado. El cálculo cambia de golpe en cuanto aparezca una familia de diagnósticos
mecánicos: validación configurable por el usuario, normalización de frontmatter, o lo que salga de
`DECISIONES §14`. Ahí el auto-arreglo pasa a ser interesante **para muchos diagnósticos a la vez**,
que es cuando merece su propia épica.

## 6. Lo que este documento NO propone

- **No propone retirar `Fix`, `Check.fixes` ni `includeSuggestedFixes`.** Se conservan: son la mitad
  de lectura, no engañan, y `fixes` vive en `core::types::Check`, un tipo del contrato cuyo test
  `check_extension_retrocompat` fija que no se omita del wire.
- **No propone un diseño concreto del `fixId`.** Solo señala que es el problema real y dónde está el
  precedente.
- **No propone reactivar `create_stub`/`retarget`**, retiradas en `E23-H05` por el mismo motivo por
  el que `LINK-TARGET-MISSING` no está en la columna de arreglos únicos.
