---
id: 22
titulo: "Integridad referencial de los valores del frontmatter"
estado: "abierta"
prioridad: 3
etiquetas: ["lenguaje-consulta", "grafo", "contrato", "dogfooding"]
origen: "dogfooding"
abierta_en: "2026-08-04"
revisada_en: "2026-08-22"
relacionadas: [9, 19, 21]
---

# §22 — Integridad referencial de los valores del frontmatter

> **Origen**: dogfooding real, no auditoría. Al montar un espacio de
> conocimiento propio con el motor —33 documentos, un homelab— apareció un modo
> de fallo que ninguna de las comprobaciones actuales cubre. Es material de la
> **épica de evidencia** ([`§9`](09-transversales-diferidas.md)), que es
> justamente donde el dogfooding tenía que producir hallazgos.

## El problema

Un enlace roto en el **cuerpo** se detecta: `LINK-TARGET-MISSING` en `check`,
y `graph.dangling_links` en el lenguaje. Un valor de **frontmatter que nombra a
otro documento** no se detecta en absoluto.

En el espacio de prueba, `affects: [bastion]` y `runs_on: [bastion]` nombran
documentos. Si un día alguien escribe `bastión` con tilde, esa nota **desaparece
de las consultas sin que nada avise**: `check` da `VÁLIDO`, el grafo no cambia,
Obsidian tampoco se queja. La consulta devuelve menos resultados y quien la
lanza no tiene forma de saber que falta algo.

No es un caso hipotético ni exclusivo de ese espacio: **este mismo repositorio
lo tiene**. Cada ficha de `decisiones/` lleva `relacionadas: [16, 19]`, que son
referencias a otras fichas por identificador. Un `19` mal tecleado como `9`
apunta a otra decisión existente y nadie se entera nunca.

## Por qué encaja con el criterio del proyecto

Es exactamente el mismo razonamiento que ya se aplicó a los namespaces
reservados, documentado en `docs/user/query-language.md`:

> *«Under a reserved namespace an unknown property is an error, not an empty
> result — a typo in `graph.backlinks` used to look exactly like "nothing
> matched"».*

Ahí se decidió que **una errata y un resultado vacío legítimo no pueden
parecerse**. En los valores de referencia del frontmatter siguen pareciéndose.

## Lo que esto NO es

La distinción importa, porque de ella depende que la idea esté en carácter con
el producto o lo contradiga.

**No es validación de esquema.** El quickstart promete «no `init`, no config
file, no mandatory frontmatter», y eso no debería cambiar: nada de campos
obligatorios, nada de declarar tipos, nada de rechazar un documento por no
ajustarse a una forma.

**Es integridad referencial**, que es otra cosa y que el motor **ya hace** para
los enlaces del cuerpo. La propuesta no añade una capacidad nueva de naturaleza
ajena: extiende una que ya existe a un sitio donde hoy no llega.

## Opciones

1. **Por convención** — tratar como referencia todo valor que coincida con la
   ruta o el nombre de un documento. **Descartable**: no hay forma de distinguir
   una etiqueta que se llama igual que un documento de una referencia real, y
   los falsos positivos en un espacio grande serían constantes.

2. **Declarado y opcional** — en `.lodestar/config.yaml`, qué campos son
   referencias y contra qué se resuelven (ruta, nombre de fichero sin extensión,
   o el valor de otro campo). Explícito, sin magia, y **ausente por defecto**:
   un workspace sin config sigue comportándose exactamente igual que hoy.

3. **Solo diagnóstico, sin lenguaje** — un código nuevo en `check`
   (`FRONTMATTER-REF-MISSING`), severidad `warn` para no romper workspaces
   existentes, sin tocar el lenguaje de consulta.

4. **Diagnóstico + propiedad computada** — lo anterior más `graph.dangling_refs`,
   espejo de `graph.dangling_links`.

## Recomendación

**Opción 2 como mecanismo, con el alcance de la 4**: declaración opcional, un
diagnóstico nuevo y una propiedad computada que refleje la que ya existe para
enlaces.

El motivo de reflejar la maquinaria de enlaces en vez de inventar otra es que
el modelo mental siga siendo **uno**: hay referencias, unas viven en el cuerpo y
otras en el frontmatter, y las dos se rompen igual y se diagnostican igual. Dos
mecanismos distintos para el mismo concepto sería peor que no tener el segundo.

## El 80 % que ya se puede hacer hoy

Conviene decirlo porque acota la urgencia. Hoy el control existe, pero es
manual:

```json
metadata_inspect {"mode": "field", "field": "affects"}
```

Devuelve todos los valores con su recuento, y un valor huérfano con `(1)` entre
otros de recuento alto es casi siempre una errata. Funciona, pero **no falla en
CI**, que es justo lo que se le pediría.

## Centinela del banco (E33-H03)

El banco mantiene el statu quo en `docs/qa/testbench/batches/sentinela_s22.json`:
`S22-01` fija el silencio de `knowledge_check` ante `relacionadas: [99]` y
`affects: [typo-inexistente]`, y `S22-02` deja ambos valores huérfanos
inspeccionables mediante `metadata_inspect`. Esta anotación no cierra la ficha;
su estado sigue siendo `abierta`.

## Orden

**Después de la épica de honestidad de superficie.** Es capacidad nueva, y el
criterio ratificado en `§21.5` —*la superficie externa solo promete lo que el
motor ejecuta hoy*— pide cerrar antes lo que ya se promete.

Encaja de forma natural **dentro de la épica de evidencia**
([`§9`](09-transversales-diferidas.md)): es un hallazgo de dogfooding, y el
dogfooding es la otra mitad de esa épica junto al banco de pruebas.

No bloquea nada ni la bloquea nadie. Prioridad 3: real y con caso de uso
demostrado, pero sin nada parado esperándola.
