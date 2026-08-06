---
id: 24
titulo: "Equivalencia de paths por caja/Unicode en el guard de colisión"
estado: "abierta"
prioridad: 3
etiquetas: ["escritura", "contrato", "dogfooding", "filesystem"]
origen: "dogfooding"
abierta_en: "2026-08-06"
revisada_en: "2026-08-06"
relacionadas: [22, 23]
---

# §24 — Equivalencia de paths por caja/Unicode en el guard de colisión

> **Origen**: hallazgo de los jueces ciegos que verificaron `E28-H04` (adenda correctiva de
> `decisiones/23-hallazgos-testbench-homelab.md`, fila A-05), reproducido ejecutando el binario
> real, no deducido leyendo código. Explícitamente fuera de alcance de `E28-H04` — el usuario
> decidió el 2026-08-06 registrarlo y decidirlo con calma, sin bloquear la corrección de los dos
> bloqueantes de `E28` (`E28-H03`/`E28-H04`).

## El problema

El guard de colisión que `E28-H02` introdujo para `create`/`move` (`DOCUMENT_ALREADY_EXISTS`,
`crates/lodestar-core/src/plan.rs`, `normalize_create`/`normalize_move`) compara claves de
`DocumentSet` **byte a byte**. En un sistema de ficheros **case-insensitive** (el default de
macOS y de Windows), un `create` o `move` hacia `Notas/Existente.md` no colisiona con la clave
`notas/existente.md` que ya existe en el `DocumentSet` —son bytes distintos—, así que el guard no
lo detecta, el plan aplica, y en disco el sistema de ficheros **fusiona** ambos paths en un solo
inodo: el contenido de `notas/existente.md` queda destruido, sustituido por el del `create`/
`move`, sin ningún diagnóstico. Reproducido; el mismo modo de fallo aparece con formas de
normalización Unicode distintas (NFC vs NFD) del mismo nombre visualmente idéntico.

En Linux (filesystems case-sensitive habituales: ext4, btrfs, xfs) esos dos pares —
`Notas/Existente.md` y `notas/existente.md`; la forma NFC y la forma NFD del mismo nombre— **no**
colisionan: son ficheros distintos y legítimamente coexisten. El comportamiento correcto del
guard depende, por tanto, del filesystem subyacente, no solo del contenido del `DocumentSet`.

## Por qué encaja con el criterio del proyecto

Es la misma familia de razonamiento que ya resolvió `§22` (integridad referencial de valores del
frontmatter) y que ya está aplicado en otro punto del motor: `LINK-CASE-MISMATCH`
(`ARCHITECTURE.md §20.9`, cableado por `E20-H04`) ya detecta y diagnostica discrepancias de
capitalización en los **enlaces** entre documentos. El guard de colisión de `E28-H02` es la misma
clase de comparación de paths —¿estas dos claves nombran "el mismo" documento?— aplicada a
`create`/`move` en vez de a la resolución de un enlace, y hoy usa un criterio (byte a byte) más
estricto que el que el motor ya sabe aplicar en el otro sitio.

## Lo que esto NO es

**No es una corrección acotada de `E28-H04`.** El guard de colisión intra-plan que esa historia
arregla opera correctamente byte a byte: el defecto que corrige es sobre **qué estado** consulta
(el `DocumentSet` inicial vs el acumulado), no sobre **cómo** compara claves. Ampliar la
comparación a equivalencia de caja/Unicode es una capacidad nueva, con una pregunta de producto
detrás (¿el motor debe modelar el filesystem del usuario, o mantenerse agnóstico y dejar que el
propio filesystem sea el árbitro?) que no tiene una respuesta obligada por el bug en sí.

## Opciones

1. **Colisión siempre, con clave normalizada** — el guard compara paths tras un `to_lowercase()` +
   normalización Unicode (NFC) consistente, en **todo** filesystem, independientemente de si el
   filesystem subyacente es case-sensitive o no. Más estricto que necesario en Linux (rechaza
   pares que ahí coexistirían de forma legítima), pero **portable**: un workspace creado en Linux
   y luego abierto en macOS no puede colisionar por sorpresa la primera vez que alguien lo abre en
   otro sistema.
2. **Warn no vinculante** — detectar la casi-colisión y emitir un diagnóstico de severidad `warn`
   (nueva entrada de `CheckCode`, o ampliar el catálogo de `knowledge_check`) sin bloquear el
   plan. Preserva el idioma "case-sensitive en Linux" sin fricción, pero no evita la pérdida de
   datos en macOS/Windows si el agente ignora el aviso (o si el aviso no llega a tiempo, antes del
   `change_apply`).
3. **Statu quo documentado** — no tocar el guard; documentar explícitamente en `contracts/mcp.yml`
   y en `docs/user/` que la detección de colisión es byte a byte y que un workspace sobre un
   filesystem case-insensitive puede perder datos por esta vía. Traslada el riesgo al usuario con
   conocimiento, sin cambiar código.

## Orden

No bloquea nada de `E28` (`E28-H03`/`E28-H04` no dependen de esta decisión) ni tiene nada
esperándola. Candidata natural a la **épica de evidencia** (`§9`, dogfooding) o al ciclo de
higiene de `§16(j)`, según qué opción se tome — la opción 1/2 son historia de motor; la opción 3
es documental y podría resolverse mucho antes. Prioridad 3: real, con caso reproducido, sin nada
parado.
