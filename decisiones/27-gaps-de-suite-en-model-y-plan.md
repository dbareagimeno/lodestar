---
id: 27
titulo: "Seis gaps de suite en model.rs y plan.rs, medidos por mutantes"
estado: "cerrada"
prioridad: 3
etiquetas: ["tests", "mutantes", "core", "higiene"]
origen: "mutantes"
abierta_en: "2026-08-09"
revisada_en: "2026-08-10"
cerrada_en: "2026-08-10"
relacionadas: [9, 16, 26]
---

# §27 — Seis gaps de suite en `model.rs` y `plan.rs`, medidos por mutantes

> **CERRADA (2026-08-10) por `E32-H01`**
> ([épica](../requirements/epica-32-gaps-suite-mutantes.md)): el usuario eligió la **salida (1)** —
> las seis tandas en una historia propia— en vez de la (2) recomendada, y las seis se ejecutaron
> con el criterio de aceptación de abajo cumplido: **cada test se vio en ROJO con su(s)
> mutación(es) aplicada(s) al árbol y en verde sin ellas** (16 mutaciones vistas en rojo a mano,
> más una demostrada equivalente), y la
> re-pasada de `/mutantes` con el mismo alcance dejó de listar las seis funciones en `missed.txt`.
> Los detalles de ejecución que se desviaron de esta ficha, con su porqué, en el
> [cierre](#cierre-2026-08-10-lo-que-la-ejecución-corrigió-de-esta-ficha) al final.

> **Origen**: la pasada de `/mutantes` que cerró la épica `E31` (2026-08-08), acotada a los dos
> ficheros que esa épica tocó: `crates/lodestar-core/src/model.rs` y
> `crates/lodestar-core/src/plan.rs`. **333 mutantes: 207 muertos, 97 supervivientes, 25 inviables,
> 4 timeouts.** No es un hallazgo de lectura: cada gap de abajo se verificó **aplicando la mutación
> al árbol real y corriendo las suites**, incluidas las de los otros crates.

## Lo primero, porque acota el alcance: E31 no introdujo ninguno

Las funciones que la épica creó o cambió —`model::replace_body_preservando_cabecera`,
`model::build_raw`, el brazo `ReplaceBody` de `plan::apply_one`, `plan::documento_resultante_de`—
**no aparecen entre los supervivientes**: todas sus mutaciones murieron. Los dos supervivientes de
`op_variant_name` (`plan.rs:1309`) son **falsos positivos de alcance**, verificado ejecutando: mueren
en la suite de `lodestar-app` (el test `no_op_operations_lista_solo_las_operaciones_sin_efecto`
asevera `op == "replace_body"`), y la pasada solo corría la de `lodestar-core`.

Los **95 restantes son preexistentes**. De ellos, ~24 son gaps reales concentrados en seis funciones;
el resto es equivalente, trivial o no concluyente (ver el final).

> **Lección de método, que vale más que la lista**: de los ocho supervivientes que se probaron contra
> las suites de otros crates, **solo uno murió** (`plan.rs:172`, `frontmatter_changes`). O sea: «lo
> cubrirá el crate de arriba» es una hipótesis, no una explicación — hay que ejecutarla.

## Los dos que caen bajo lo que E31 acaba de construir

Estos dos justifican por sí solos la ficha: no son gaps cualesquiera, son **la red que falta debajo
del arreglo de `§26`**.

### (a) CRLF en `split_front` es territorio virgen

`grep` de `\r\n` en **todos** los tests de `lodestar-core`: **una sola línea**
(`documento.rs:125`), y es un `starts_with` de otra cosa. Sobreviven, verificado contra app incluido:

- `model.rs:133` — `nl - 1` → `nl + 1` (el recorte del `\r` de cola del delimitador).
- `model.rs:140` — `body_start += 1` → `-=`.

**Por qué importa ahora**: `replace_body_preservando_cabecera` es literalmente
`&raw[..split_front(raw).body_offset(raw)] + body`. Si `body_offset` se desvía un byte en CRLF, la
función **parte el documento en medio del `\r\n`** y su promesa entera —«la cabecera sobrevive byte a
byte»— se cae en cualquier workspace escrito en Windows. `body_offset` es correcto **hoy**; lo que no
existe es lo que avisaría si dejara de serlo. Y el precedente es reciente: `E30` cerró un defecto que
solo se veía en Windows (`f7301a8`).

**Test propuesto** — `crates/lodestar-core/tests/documento.rs`,
`split_front_corta_igual_con_crlf_que_con_lf()`: el mismo documento en las dos codificaciones,
aseverando (1) `fm_text` idéntico y **sin `\r` de cola** (mata el `nl-1`→`nl+1`), (2) el cuerpo
empieza en su primer carácter real (mata el `+=`→`-=`), y (3) el round-trip
`replace_body_preservando_cabecera(raw, body(raw)) == raw` en la variante CRLF — ese tercer assert es
el que ata las dos mutaciones a la garantía de `E31-H02`.

### (b) El no-op de `patch_frontmatter` no tiene quien lo sujete

- `model.rs:485` — el guard `edits.is_empty()`, que devuelve el documento **byte a byte** cuando el
  patch no cambia nada. Neutralizarlo hace que un patch sin efecto **reescriba el bloque**.
- `model.rs:480` — el `&&` → `||` afloja la validación que decide si el escaneo por líneas es fiable;
  en producción, editar por líneas un bloque **mal escaneado** es corromper el YAML del usuario.

Es el mismo churn que `§26` acaba de erradicar en la otra mitad del camino de escritura, en la mitad
que ya era quirúrgica desde `E16-H04` — y sin test que lo fije.

**Test propuesto** — `documento.rs`, `patch_sin_efecto_devuelve_el_documento_byte_a_byte()`:
frontmatter con formato distintivo (flow `tags: [a, b]`, comillas, un comentario), patch que escribe
el valor que ya estaba, `assert_eq!(resultado.raw, raw)` exacto y `reserialized == false`. Con un
complemento que ate el `480`: un bloque que fuerce el fallback debe salir con `reserialized == true`.

## Los otros cuatro

### (c) `relation_changes` no se asevera en ninguna parte — el más grave de los cuatro

`plan.rs:182`. Los tres tests de `semantic_diff` (`core.rs:1214-1355`) aseveran
`created`/`modified` y los diagnósticos, **jamás** `relation_changes`. Se puede invertir el predicado
entero sin que nada se ponga rojo, y `semanticDiff` viaja al wire en las **tres** tools de cambio
(`contracts/mcp.yml`). Que su hermano `frontmatter_changes` (`plan.rs:172`) **sí** muera en
`lodestar-app` mientras este no muere en ningún sitio demuestra que el hueco es específico, no
ambiental.

**Test propuesto** — `core.rs`, `diff_marca_relation_changes_solo_donde_cambian_los_enlaces()`: un
documento que añade enlace, otro que quita, y un tercero cuyo cuerpo cambia **sin tocar enlaces**;
el `!contains` del tercero es lo que mata la mutación.

### (d) `ensure_exists` puede devolver siempre `Ok`

`plan.rs:591`, punto **único** de verificación de existencia de `normalize_patch_frontmatter` y
`normalize_replace_body`. Neutralizado, planificar sobre un path inexistente deja de dar
`DOCUMENT_NOT_FOUND` y sigue adelante inventando el documento. Lo permite la suite entera, core y app.

**Test propuesto** — `core.rs`,
`normalizar_contenido_sobre_documento_inexistente_da_target_not_found()`: los dos normalizadores
sobre un path ausente, aseverando `CoreError::NormalizeTargetNotFound` **con el path dentro**.

### (e) `sort_paths_cmp` **sí** es contractual (la sospecha contraria era falsa)

17 supervivientes. Se asumió que era orden de criterio interno; **es observable en el wire**:
`diff.rs:144` lo usa para ordenar las claves de `diff_snap`, y ese orden se propaga a
`semanticDiff.created/modified/deleted`. Lo cubre **un solo** test (`core.rs:631`,
`diff_snap_ordena_numeric_aware`) con **dos** paths, que ejercita la rama numérica y nada más.

**Test propuesto** — `core.rs`, `sort_paths_cmp_es_un_orden_total_estable()`: ~12 paths mezclando
prefijo común con longitudes distintas, ceros a la izquierda, números en medio y no-ASCII;
`assert_eq!` del **vector ordenado entero** (no `contains` — es lo que mata los `<`→`<=`/`==`), más
antisimetría sobre todos los pares.

### (f) `locate_section` puede editar la sección equivocada

`model.rs:887`. El `&&` → `||` rompe el estrechamiento de rango que garantiza que el segundo segmento
de un `heading_path` case **dentro** del primero. En producción: un
`edit_section(["Security","Rotation"])` editando la `Rotation` que cuelga de **otro** heading —
escritura en el sitio equivocado, en silencio, que es la categoría de defecto de `E28`.

**Test propuesto** — `core.rs`,
`locate_section_no_cruza_a_una_seccion_hermana_con_el_mismo_titulo()`: `## Security → ### Rotation` y
`## Deploy → ### Rotation`, con contenidos distintos; los dos asserts juntos matan el `||` y el
`<`→`<=`.

## Lo que NO merece test (clasificado, para no volver a mirarlo)

- **`relative_dir_href` (17) + `redirigir_href_a_directorio` (2)** — el bloque más grande, y aun así
  segunda fila: es `E23-H11` (recálculo de hrefs a directorio al mover), código real y observable,
  pero caso de esquina de un caso de esquina, y su hermano `relative_href` sí está cubierto. **Si se
  quiere una séptima tanda, empezar aquí**: un `move` de un documento con `[volver](../)` a distinta
  profundidad mataría la mayoría de los 17 de un golpe.
- **`accion` (5) y los umbrales de `assess_risk` (2)** — texto legible de una razón y la frontera
  exacta `>=`/`>` de una **heurística** autodeclarada. El nivel de riesgo sí es contractual; el punto
  de corte no.
- **`locale_cmp` (3)** — aquí sí es criterio no contractual: su doc-comment declara la paridad ICU
  exacta como **no-goal** explícito, y no alimenta ningún orden del wire.
- **Cola de `-> ""` / `"xyzzy"`** en `fm_text`, `basename`, `normalize`, `yaml_key_text`,
  `extract_sections`, `remove_inline_links` — alcanzadas indirectamente o brazos defensivos
  inalcanzables (p. ej. claves YAML numéricas, que `FieldPath` no puede direccionar).
- **4 timeouts** (`model.rs:256/260/620`, `plan.rs:698`) — **no concluyentes**, no supervivientes:
  son `+=`→`*=` sobre contadores que arrancan en 0, o sea bucles infinitos. El test de (e) los
  cubriría si el runner llegara a ejecutarlos.

## Las salidas

1. **Los seis tests, en una historia propia** dentro del próximo ciclo de higiene. Matan ~30
   mutantes y cierran la ficha entera.
2. **Solo (a) y (b) ahora**, por ser la red que falta bajo `E31`, y el resto a la épica de evidencia
   (`§9`). Es la opción barata: dos tests.
3. **Nada, y se registra como deuda medida.** Ninguno es un defecto: son **agujeros de la suite**, no
   comportamiento roto. El coste de no hacerlo es que un cambio futuro en esas seis funciones no
   tiene quien lo frene.

**Recomendación: (2).** (a) y (b) tienen un argumento que los otros cuatro no tienen —sujetan algo
que se acaba de construir y que el repo ya sabe que se rompe distinto en Windows—, mientras que
(c)–(f) son deuda antigua y estable que encaja mejor con el banco de pruebas de `§9`, donde hay
criterio para priorizarla junto al resto de la evidencia. Partir la ficha evita además que seis tests
sin relación entre sí viajen en un mismo PR.

## Criterio de aceptación cuando se ejecute

- Cada test propuesto **se ve en rojo** con su mutación aplicada y en verde sin ella: la evidencia es
  la mutación, no que el test pase (es la lección de `E30`, y la de esta misma ficha).
- La pasada de `/mutantes` sobre las funciones cubiertas **deja de listarlas** en `missed.txt`, con el
  mismo alcance (`-p lodestar-core --file crates/lodestar-core/src/model.rs --file
  crates/lodestar-core/src/plan.rs`).
- Si al escribir un test resulta que el comportamiento mutado era **correcto** —que el supervivor era
  equivalente y no gap—, se **retira** de esta ficha con la razón escrita, en vez de forzar un test
  que consagre un detalle accidental.

## Cierre (2026-08-10): lo que la ejecución corrigió de esta ficha

Las seis tandas se ejecutaron y **ninguna se retiró**: los seis gaps eran reales. Pero la ejecución
corrigió tres detalles de esta ficha, y los tres refuerzan su propia lección («la evidencia se
ejecuta, no se lee»):

1. **El fixture propuesto en (b) para el `480` no funcionaba tal cual**: una clave `01:` no fuerza
   el fallback porque `serde_yaml` la parsea como **string** `"01"` (no como el entero `1`), así que
   el escaneo por líneas casa y el patch entra por el camino quirúrgico. El fixture real es `1.50:`
   (float `1.5`), cuyo texto sí diverge de su valor parseado. Verificado en ambas direcciones: el
   test en verde sin mutación y en rojo con el `&&`→`||` aplicado.
2. **El test propuesto en (e) mataba 14 de los 17, no los 17**: la primera re-pasada dejó vivos 3
   supervivientes de `sort_paths_cmp` que el vector propuesto no podía distinguir — el `&&`→`||`
   que abre la tira numérica **en falso** (exige un par dígito-contra-letra en el punto de
   divergencia, y todos los paths del vector divergían dígito-contra-dígito o letra-contra-letra) y
   los dos `<`→`<=` de los bucles internos (exigen un path que **termine en dígito**, y todos
   acababan en `.md`). El vector final añade `doc-7.md`/`doc-abc.md` y `v2` a secas; la segunda
   re-pasada no deja ninguno.
3. **Un mutante de la zona resultó equivalente demostrable** (verificado a mano, no forzado a
   test): el `-`→`+` del desempate final de `sort_paths_cmp` no es observable, porque llegar ahí
   exige que todas las tiras numéricas empataran también en longitud — `i == j` siempre en ese
   punto, y `len − i` ordena igual que `len + i`. Es la ilustración exacta de la cláusula de
   equivalencia de arriba, aplicada a un mutante suelto y no a una tanda.

**Resultado medido** (mismo alcance, `-p lodestar-core --file …/model.rs --file …/plan.rs`,
333 mutantes): los supervivientes bajan de **97 a 62** (242 muertos, 25 inviables, los mismos 4
timeouts no concluyentes), y **ninguno de los seis gaps de esta ficha sigue vivo**. De los 62, el
único que toca una función con nombre aquí es `plan.rs:172` (`frontmatter_changes`, dentro de
`semantic_diff` pero **fuera** de los seis gaps): es el falso positivo de alcance que esta misma
ficha documenta arriba, y se **re-verificó ejecutando** — con la mutación aplicada, la suite de
`lodestar-app` se pone en rojo; ídem los dos de `op_variant_name` (`plan.rs:1309`). El resto es la
cola ya clasificada en «lo que NO merece test» (`relative_dir_href` y compañía, `accion`, los
umbrales de `assess_risk`, `locale_cmp`, brazos defensivos, más `scan_top_level`/`split_key_line`,
que quedan con la misma consideración). La séptima tanda potencial (`relative_dir_href`, 17
supervivientes) sigue **abierta a propósito**: esta ficha la deja señalada como el punto de partida
si algún día se quiere otra tanda, y su sitio es la épica de evidencia (`§9`).
