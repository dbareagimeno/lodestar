---
id: 26
titulo: "Un replace_text sin coincidencias reescribe el fichero y reserializa el frontmatter"
estado: "abierta"
prioridad: 3
etiquetas: ["escritura", "normalizacion", "dogfooding", "frontmatter"]
origen: "juez-ciego"
abierta_en: "2026-08-07"
revisada_en: "2026-08-07"
relacionadas: [23, 22]
---

# §26 — Un `replace_text` sin coincidencias reescribe el fichero y reserializa el frontmatter

> **Origen**: verificación de `§23/A-06` durante la escoba documental de `E30-H03`, y confirmado por
> su juez ciego **ejecutando el binario real** contra un documento del workspace de ejemplo. A-06
> preguntaba si el no-op estaba bien documentado; documentarlo destapó que **el no-op no es un
> no-op**.

## El defecto

Un `replace_text` cuyo patrón **no casa ninguna ocurrencia** no se queda quieto: normaliza a un
`replace_body` de **documento entero**, que reserializa el frontmatter al escribir. El efecto
observable, reproducido por el wire sobre `examples/demo/overview.md`:

- `semanticDiff.modified` contiene el documento — **pese a que no se pidió ningún cambio efectivo**.
- `bodyChanges` y `frontmatterChanges` van **vacíos**: el diff semántico no encuentra nada que
  contar, porque semánticamente no cambió nada.
- El fichero en disco **sí cambia**: `tags: [atlas, overview]` (estilo *flow*) vuelve como lista en
  estilo *bloque*.

O sea: churn de bytes sin cambio semántico, en un camino que el usuario invocó esperando que no
hiciera nada. Contamina el diff de git de quien versione su workspace, y hace que
`workspaceRevision` avance por una operación vacía.

## Por qué quedó fuera de `E30-H03`

Causa raíz distinta a la de A-06. A-06 era **documental** —declarar que el no-op existe y no es un
error— y eso se hizo (`docs/user/safe-changes.md`, con el caveat honesto). El reserializado vive en
la **normalización** de operaciones, que la épica `E30` excluía explícitamente: es la misma familia
que los «hallazgos preexistentes registrados» de
[`requirements/epica-28-defectos-destructivos-testbench.md`](../requirements/epica-28-defectos-destructivos-testbench.md).

## El test que hoy lo fija

`replace_text_sin_ocurrencias_en_forma_array_es_noop`
(`crates/lodestar-app/tests/plan.rs`) **asevera la realidad, no el deseo**: exige que el documento
de frontmatter *flow* aparezca en `modified`, con un mensaje que dice qué hacer cuando esto se
arregle («invierte la aserción y actualiza el caveat de `docs/user/safe-changes.md`»). Su documento
de frontmatter *bloque* es el anti-vacuo: demuestra que `modified` responde al churn de bytes y no a
que se liste todo siempre.

> **Nota de método, porque es el motivo de que esta ficha exista**: la primera versión de ese test
> aseveraba **lo contrario** —que el documento *no* aparecía en `modified`— y pasaba, porque el
> fixture compartido no tenía ningún frontmatter en estilo *flow*. El juez ciego lo tumbó añadiendo
> `tags: [a, b]`. Un test verde no probaba nada; la mutación sí.

## Las salidas

1. **Que el no-op sea un no-op de verdad**: si `replace_text` no casa nada, la operación no llega a
   generar escritura y el documento no entra en `modified`. Es lo que el usuario espera y lo que la
   doc tendría que poder afirmar sin caveat.
2. **Que la normalización no reserialice cuando el frontmatter no cambia**: arreglo más general —
   preservar los bytes originales del frontmatter si ninguna operación lo toca. Cubre este caso y
   cualquier otro que reescriba el cuerpo sin tocar la cabecera.
3. **Declararlo comportamiento aceptado** y dejar el caveat como está.

**Recomendación: (2), y con ella cae (1).** El problema no es de `replace_text` sino de que la
escritura reserializa lo que no se le pidió tocar; atacarlo en la normalización cierra la familia
entera en vez de un síntoma. (1) sola dejaría vivo el mismo churn para cualquier otra operación de
cuerpo. (3) es la peor: convierte en contrato un efecto que nadie pidió y que ensucia el historial
de quien versione sus `.md`.

## Criterio de aceptación cuando se ejecute

- Un `replace_text` sin coincidencias no modifica el fichero en disco (bytes idénticos, mtime
  aparte) y no aparece en `semanticDiff.modified`.
- Un documento con frontmatter en estilo *flow* al que se le cambia **solo el cuerpo** conserva el
  estilo *flow* de su frontmatter.
- El test citado arriba se **invierte** y el caveat de `docs/user/safe-changes.md` se retira.
