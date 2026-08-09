---
id: 25
titulo: "Superficie pública muerta: Workspace::revert_transaction"
estado: "cerrada"
prioridad: 3
etiquetas: ["escritura", "api", "higiene", "mutantes"]
origen: "mutantes"
abierta_en: "2026-08-07"
revisada_en: "2026-08-08"
cerrada_en: "2026-08-08"
relacionadas: [16, 23]
---

> **CERRADA el 2026-08-08 por `E31-H01`** — pero **no por la salida (1) que esta ficha recomendaba**.
> Se ejecutó la **(2), retirada completa**, y el motivo lo decidió el compilador: al replegar a
> `pub(crate)`, `clippy` marcó la función como `dead_code` —no la usaba nadie **tampoco dentro del
> crate**—, y como el CI corre con `-D warnings`, el repliegue era **literalmente incompilable**. Con
> `pub` el aviso no salía porque una función pública puede tener consumidores externos; replegarla
> fue justo lo que lo destapó. Las únicas alternativas eran `#[allow(dead_code)]` —conservar el
> código muerto *y* silenciar al detector que lo encontró— o la retirada que esta ficha ya
> contemplaba.
>
> **Y el argumento con el que esta ficha desaconsejaba retirarla era falso**: dice que la función
> «tiene un papel interno (es el cuerpo que `revert_transaction_con_recibo` envuelve)». Es al revés
> — era un **wrapper de tres líneas** sobre ella, así que no había nada que reorganizar. Lo único
> que costó fue la documentación: su doc-comment era el único sitio donde vivía la descripción de la
> mecánica de reversión, y se trasladó a `revert_transaction_con_recibo`, que es donde esa mecánica
> ocurre de verdad.
>
> Verificado por juez ciego, que además comprobó desde un consumidor externo que la función ya no es
> alcanzable, y que la suite no ganó ni un test para justificarla (el criterio anti-vacuo).

# §25 — Superficie pública muerta: `Workspace::revert_transaction`

> **Origen**: la pasada de mutantes de [`§16(l)`](16-deuda-auditoria-e25-e26.md), ejecutada al
> cerrar la campaña de bugfixes del testbench homelab (épica `E30`, 2026-08-07). No es un hallazgo
> de lectura: se descubrió **mutando** — sustituyendo el cuerpo entero de la función por
> `unreachable!()` y observando que nada se rompía.

## El hecho, medido

`Workspace::revert_transaction` (`crates/lodestar-workspace/src/recovery.rs:1083`) es **superficie
pública sin un solo llamador**:

- Sustituir su cuerpo entero por `unreachable!()` deja **los 52 binarios de test del workspace en
  verde**.
- `grep` en todo el repo no encuentra ninguna llamada: la fachada usa
  `revert_transaction_con_recibo`, y las menciones restantes son comentarios y rustdoc.

Es decir: es `pub`, cualquiera fuera del crate puede llamarla, y **el repo entero puede funcionar
como si no existiera**.

## Por qué no se resolvió en E30

Porque no es higiene de suite, que era el alcance de `§16(l)`. Escribirle un test **consagraría**
una superficie que quizá deba desaparecer: sería fijar por contrato algo que nadie usa. La decisión
correcta es la misma que el repo ya tomó dos veces para el mismo modo de fallo, y en ambas fue
**retirar**, no cubrir:

- [`§16(b)`](16-deuda-auditoria-e25-e26.md) — `Envelope`/`ErrorEnvelope` sin llamantes → retirados
  en `E29-H11`.
- [`§16(g)`](16-deuda-auditoria-e25-e26.md) — API pública no transaccional de `Workspace` →
  replegada a `pub(crate)` en `E29-H10`.

## Las tres salidas

1. **Repliegue a `pub(crate)`** (o `#[cfg(test)]` si solo la quieren los tests). Es lo que hizo
   `§16(g)` con sus cuatro hermanas —`create_document`, `write_document`, `merge_frontmatter`,
   `publish`— y encaja con el invariante de que **el único camino de escritura es transaccional**:
   una reversión que no registra recibo durable no debería ofrecerse al exterior.
2. **Retirada completa**, si ni siquiera los tests la necesitan. Es lo que hizo `§16(b)`.
3. **Conservarla con test**, si se le ve consumidor futuro — pero entonces hay que nombrarlo: hoy
   no existe, y `revert_transaction_con_recibo` cubre el caso real.

**Recomendación: (1), repliegue a `pub(crate)`.** Es la opción reversible: no borra capacidad, solo
deja de ofrecerla fuera del crate, y si aparece el consumidor se vuelve a abrir sin arqueología. La
diferencia con `§16(b)` es que allí el tipo no lo usaba **nadie**, ni dentro ni fuera; aquí la
función sí tiene un papel interno (es el cuerpo que `revert_transaction_con_recibo` envuelve), así
que retirarla del todo obligaría a reorganizar código que funciona.

## Criterio de aceptación cuando se ejecute

- La función deja de ser alcanzable desde fuera del crate (verificado con un consumidor externo,
  como se hizo en `E29-H10`).
- La suite sigue en verde sin añadir tests nuevos que la ejerzan: si hiciera falta uno, la premisa
  de esta ficha era falsa y hay que reabrir el análisis.
- `IMPLEMENTATION_STATUS.md` y la fila de `§16` reflejan el cierre.
