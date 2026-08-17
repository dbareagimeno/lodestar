# E32 — Los seis gaps de suite que midió la pasada de mutantes de E31

> **Origen**: [`decisiones §27`](../decisiones/27-gaps-de-suite-en-model-y-plan.md), abierta por la
> pasada de `/mutantes` que cerró la épica `E31` (2026-08-08), acotada a
> `crates/lodestar-core/src/model.rs` y `crates/lodestar-core/src/plan.rs` (333 mutantes: 207
> muertos, 97 supervivientes, 25 inviables, 4 timeouts). La ficha destiló los 95 supervivientes
> preexistentes en **seis gaps reales de suite** concentrados en seis funciones, cada uno verificado
> **aplicando la mutación al árbol real y corriendo las suites** — incluidas las de los otros
> crates, porque «lo cubrirá el crate de arriba» resultó ser una hipótesis que solo se sostuvo una
> vez de ocho.
>
> **Objetivo de la épica**: escribir los seis tests que la ficha ya especifica —nombre, fichero
> destino y asserts— y con ello **cerrar `§27` entera** (salida **1** de la ficha, elegida por el
> usuario el 2026-08-09; la recomendación escrita era la (2), pero la elección de salida es del
> usuario y **no se relitiga aquí**). Trabajo **tests-only sobre `lodestar-core`**: cero código de
> producción, cero cambio de comportamiento, cero delta de contrato.
> Referencias maestras: `decisiones §27` · E31 (`requirements/epica-31-seguimientos-campana.md`,
> origen de la pasada) · `decisiones §26`/`E31-H02` (las tandas (a) y (b) son la red que falta bajo
> ese arreglo) · lecciones de E30/E31 (**la evidencia es la mutación, no que el test pase**) ·
> `CLAUDE.md` invariantes #2 (core puro) y #3 (una sola verdad computada).

**Principio rector**: *un test que nace verde no ha demostrado nada*. Cada uno de los seis tests se
ve **en rojo con su mutación aplicada al árbol** y en verde sin ella — es el criterio de aceptación
literal de la propia ficha, y la lección repetida de E23, E30 y de la pasada que la originó. La
suite en verde no es la evidencia; la mutación muerta sí.

## Decisión de alcance tomada con el usuario (2026-08-09)

| Punto | Decisión |
|---|---|
| Salida de `§27` | **Salida (1)**: las seis tandas en una historia propia; cierre entero de la ficha. |

## Historias

| ID | Cierra | Título | Frontera | Fase roja |
|---|---|---|---|---|
| E32-H01 | `§27` (entera) | Los seis tests de suite que la ficha `§27` deja especificados | no | sí — pero el rojo es **la mutación aplicada**, no un stub |

---

## E32-H01 — Los seis tests de suite que la ficha `§27` deja especificados

- **Objetivo**: la suite de `lodestar-core` gana los seis tests que `decisiones §27` especifica
  (secciones (a)–(f), «Test propuesto» de cada una), matando ~30 mutantes supervivientes en seis
  funciones de `model.rs`/`plan.rs` que hoy se pueden mutar —o invertir enteras— sin que nada se
  ponga rojo. Ninguno es un defecto: son **agujeros de la suite** sobre comportamiento correcto; lo
  que se compra es que un cambio futuro en esas funciones tenga quien lo frene.

- **Síntoma**: ninguno observable en producción. La evidencia es **medida, no leída**: cada gap se
  verificó en la pasada de E31 aplicando la mutación al árbol y viendo la suite entera en verde
  (crates de arriba incluidos, salvo el caso `plan.rs:172` que sí muere en `lodestar-app`).

- **Referencias**: `decisiones §27` (la fuente: los seis tests vienen especificados ahí y esta
  historia los recoge **fielmente, sin reinventarlos**) · E31 (la pasada de `/mutantes` que los
  midió, `requirements/epica-31-seguimientos-campana.md`) · `decisiones §26` + `E31-H02` — las
  tandas (a) y (b) sujetan `replace_body_preservando_cabecera` (`model.rs:560`) y el no-op
  quirúrgico de `patch_frontmatter`, es decir, lo que E31 acaba de construir · E30 (`f7301a8`: el
  precedente de defecto solo-Windows que motiva (a); y su lección de método: la evidencia se
  ejecuta) · `CLAUDE.md` invariantes **#2** (core puro: esta historia no añade dependencias ni
  código a `lodestar-core`, solo tests) y **#3** (una sola verdad computada: (c) y (e) protegen
  `semantic_diff`/`diff_snap`, la verdad de diff del core que viaja al wire) · ubicaciones
  verificadas en el árbol el 2026-08-09: `model::split_front` (`model.rs:104`),
  `model::patch_frontmatter` (`model.rs:433`), `model::sort_paths_cmp` (`model.rs:246` — vive en
  `model.rs`, dentro del alcance de la pasada), `model::locate_section` (`model.rs:878`),
  `plan::semantic_diff`/`relation_changes` (`plan.rs:156`/`:182`), `plan::ensure_exists`
  (`plan.rs:590`, privada; se ejercita vía `normalize_patch_frontmatter` `:612` y
  `normalize_replace_body` `:632`, públicas).

- **Alcance** — las seis tandas, tal como las especifica la ficha:

  1. **(a) CRLF en `split_front`** — `crates/lodestar-core/tests/documento.rs`,
     `split_front_corta_igual_con_crlf_que_con_lf()`: el mismo documento en LF y en CRLF,
     aseverando (1) `fm_text` idéntico y **sin `\r` de cola**, (2) el cuerpo empieza en su primer
     carácter real, y (3) el round-trip
     `replace_body_preservando_cabecera(raw, split_front(raw).body(raw)) == raw` en la variante
     CRLF — el tercer assert es el que ata las mutaciones a la garantía de `E31-H02`. Hoy el
     universo CRLF de los tests de `lodestar-core` es **una línea** (`documento.rs:125`, y es un
     `starts_with` de otra cosa).
  2. **(b) No-op de `patch_frontmatter`** — `documento.rs`,
     `patch_sin_efecto_devuelve_el_documento_byte_a_byte()`: frontmatter con formato distintivo
     (flow `tags: [a, b]`, comillas, un comentario), patch que escribe el valor que ya estaba,
     `assert_eq!(resultado.raw, raw)` **exacto** y `reserialized == false`. Más el complemento que
     ata la validación del escaneo: un bloque que fuerce el fallback debe salir con
     `reserialized == true` (nombre propuesto:
     `patch_con_bloque_mal_escaneado_cae_al_fallback_y_lo_declara()`).
  3. **(c) `relation_changes` de `semantic_diff`** — `crates/lodestar-core/tests/core.rs`,
     `diff_marca_relation_changes_solo_donde_cambian_los_enlaces()`: un documento que añade enlace,
     otro que quita, y un tercero cuyo cuerpo cambia **sin tocar enlaces**; el `!contains` del
     tercero es lo que mata la mutación. Los tres tests actuales de `semantic_diff`
     (`core.rs:1214-1355`) jamás aseveran `relation_changes`, y ese campo viaja al wire en las tres
     tools de cambio.
  4. **(d) `ensure_exists`** — `core.rs`,
     `normalizar_contenido_sobre_documento_inexistente_da_target_not_found()`: los dos
     normalizadores públicos (`normalize_patch_frontmatter` y `normalize_replace_body`) sobre un
     path ausente, aseverando `CoreError::NormalizeTargetNotFound` **con el path dentro**.
  5. **(e) `sort_paths_cmp` es contractual** — `core.rs`,
     `sort_paths_cmp_es_un_orden_total_estable()`: ~12 paths mezclando prefijo común con longitudes
     distintas, ceros a la izquierda, números en medio y no-ASCII; `assert_eq!` del **vector
     ordenado entero** (no `contains`) más **antisimetría sobre todos los pares**. El único test
     actual (`core.rs:631`, `diff_snap_ordena_numeric_aware`) usa dos paths y solo ejercita la rama
     numérica; el orden se propaga al wire vía `diff.rs:144` →
     `semanticDiff.created/modified/deleted`.
  6. **(f) `locate_section` no cruza a una hermana** — `core.rs`,
     `locate_section_no_cruza_a_una_seccion_hermana_con_el_mismo_titulo()`: cuerpo con
     `## Security → ### Rotation` y `## Deploy → ### Rotation`, contenidos distintos; se asevera
     que `["Security","Rotation"]` y `["Deploy","Rotation"]` devuelven **cada uno su** rango — los
     dos asserts juntos matan las dos mutaciones del estrechamiento de rango.

  Además:
  - **Verificación de cierre**: re-pasada de `/mutantes` con el **mismo alcance de la ficha**
    (`-p lodestar-core --file crates/lodestar-core/src/model.rs --file
    crates/lodestar-core/src/plan.rs`) y comprobación de que las seis funciones **dejan de
    aparecer** en `missed.txt`. Beneficio colateral esperado (no criterio): el test (e) puede
    convertir en muertos los 4 timeouts (`model.rs:256/260/620`, `plan.rs:698`) si el runner llega
    a ejecutarlos.
  - **Consecuencia documental en el mismo PR** (la spec la enuncia como resultado esperado, no la
    ejecuta): `decisiones/27-gaps-de-suite-en-model-y-plan.md` → `estado: cerrada` (con
    `cerrada_en`/`revisada_en`), su fila en `decisiones/README.md`, y la fila de E32 en
    `IMPLEMENTATION_STATUS.md`. Si alguna tanda se retira por equivalencia (ver abajo), la ficha se
    edita **antes** de cerrarla, con la razón escrita.

- **Fuera de alcance**:
  - **Cualquier línea de código de producción.** Si al escribir un test se descubre que el
    comportamiento actual es **defectuoso** (no un gap de suite: la ficha afirma que los seis son
    comportamiento correcto sin red), se **PARA y se vuelve al usuario** — esta historia no
    autoriza arreglos, ni siquiera de una línea.
  - **La séptima tanda potencial** (`relative_dir_href` + `redirigir_href_a_directorio`, 19
    supervivientes): la ficha la deja clasificada como «si se quiere, empezar aquí», no la pide.
  - El resto de supervivientes clasificados como «no merece test» (`accion`, umbrales de
    `assess_risk`, `locale_cmp`, colas defensivas `-> ""`/`"xyzzy"`): la ficha los descarta con
    motivo; no se les escribe test.
  - Tocar tests existentes (salvo que una tanda lo exija y se justifique en el PR), fixtures de
    `lodestar-fixtures`, u otros crates.
  - Las decisiones abiertas `§9` (banco de pruebas), `§14`, `§20`: no se tocan ni se reabren. El
    cierre de `§27` por la salida (1) ya está decidido por el usuario.

- **Criterios de aceptación**:
  - **[BDD-a] Dado** un documento con frontmatter escrito con finales CRLF y su gemelo en LF,
    **Cuando** `split_front` los corta, **Entonces** `fm_text` es idéntico en ambos y sin `\r` de
    cola, el cuerpo empieza en su primer carácter real, y
    `replace_body_preservando_cabecera(raw, body(raw)) == raw` en la variante CRLF
    → test: `split_front_corta_igual_con_crlf_que_con_lf` (`documento.rs`).
  - **[BDD-b1] Dado** un documento con frontmatter de formato distintivo (flow, comillas,
    comentario), **Cuando** se aplica un patch que escribe el valor que ya estaba, **Entonces** el
    resultado es **byte a byte** el documento original y `reserialized == false`
    → test: `patch_sin_efecto_devuelve_el_documento_byte_a_byte` (`documento.rs`).
  - **[BDD-b2] Dado** un bloque de frontmatter cuyo escaneo por líneas no es fiable, **Cuando** se
    aplica un patch con ediciones, **Entonces** el resultado declara `reserialized == true`
    → test: `patch_con_bloque_mal_escaneado_cae_al_fallback_y_lo_declara` (`documento.rs`).
  - **[BDD-c] Dado** tres documentos —uno que añade un enlace, otro que quita uno, y un tercero
    cuyo cuerpo cambia sin tocar enlaces—, **Cuando** se computa `semantic_diff`, **Entonces**
    `relation_changes` contiene exactamente los dos primeros y **no** el tercero
    → test: `diff_marca_relation_changes_solo_donde_cambian_los_enlaces` (`core.rs`).
  - **[BDD-d] Dado** un path sin fichero en el workspace, **Cuando** se normaliza un
    `patch_frontmatter` y un `replace_body` sobre él, **Entonces** ambos devuelven
    `CoreError::NormalizeTargetNotFound` con el path en el error
    → test: `normalizar_contenido_sobre_documento_inexistente_da_target_not_found` (`core.rs`).
  - **[BDD-e] Dado** ~12 paths que mezclan prefijos comunes, longitudes distintas, ceros a la
    izquierda, números en medio y no-ASCII, **Cuando** se ordenan con `sort_paths_cmp`,
    **Entonces** el vector ordenado entero es el esperado (`assert_eq!`, no `contains`) y el orden
    es antisimétrico sobre todos los pares
    → test: `sort_paths_cmp_es_un_orden_total_estable` (`core.rs`).
  - **[BDD-f] Dado** un cuerpo con `## Security → ### Rotation` y `## Deploy → ### Rotation` de
    contenidos distintos, **Cuando** `locate_section` resuelve cada `heading_path`, **Entonces**
    cada uno devuelve el rango de **su** subsección, sin cruzar a la hermana homónima
    → test: `locate_section_no_cruza_a_una_seccion_hermana_con_el_mismo_titulo` (`core.rs`).
  - **[EVIDENCIA — el criterio central] Dado** cada test nuevo, **Cuando** se aplica al árbol su(s)
    mutación(es) de la tabla de abajo y se corre la suite de `lodestar-core`, **Entonces** el test
    está en **ROJO**; **y Cuando** se revierte la mutación, **Entonces** está en **VERDE**. El PR
    deja constancia de la verificación por tanda (qué mutación, qué test la mató). Las líneas son
    las de la ficha (2026-08-08) y **pueden haber derivado: se verifican contra el árbol actual**,
    lo que manda es el operador sobre la expresión, no el número de línea.

    | Tanda | Mutación(es) según la ficha | Test que debe ponerse rojo |
    |---|---|---|
    | (a) | `model.rs:133` `nl - 1` → `nl + 1` · `model.rs:140` `body_start += 1` → `-=` | `split_front_corta_igual_con_crlf_que_con_lf` |
    | (b) | `model.rs:485` guard `edits.is_empty()` neutralizado · `model.rs:480` `&&` → `\|\|` | `patch_sin_efecto_devuelve_el_documento_byte_a_byte` (485) · el complemento b2 (480) |
    | (c) | `plan.rs:182` predicado de `relation_changes` invertido/neutralizado | `diff_marca_relation_changes_solo_donde_cambian_los_enlaces` |
    | (d) | `plan.rs:591` `ensure_exists` devolviendo siempre `Ok` | `normalizar_contenido_sobre_documento_inexistente_da_target_not_found` |
    | (e) | los supervivientes de `sort_paths_cmp` (`model.rs:246-287`): `<` → `<=`/`==` y afines (17) | `sort_paths_cmp_es_un_orden_total_estable` |
    | (f) | `model.rs:887` `&&` → `\|\|` y `<` → `<=` | `locate_section_no_cruza_a_una_seccion_hermana_con_el_mismo_titulo` |

  - **[EQUIVALENCIA — cláusula de la ficha] Dado** que al escribir un test resulte que el
    comportamiento mutado era **correcto** (superviviente equivalente, no gap), **Entonces** esa
    tanda se **retira de la ficha con la razón escrita** en vez de forzar un test que consagre un
    detalle accidental — y el cierre de `§27` documenta la retirada.
  - **[PARADA] Dado** que un test destape que el comportamiento actual es **defectuoso**,
    **Entonces** se detiene la historia en esa tanda y se vuelve al usuario con el hallazgo: la
    historia no autoriza tocar código de producción.
  - **[Estructural] El diff del PR solo toca** `crates/lodestar-core/tests/*.rs` (más los
    documentos de estado del cierre): cero cambios en `src/`, cero dependencias nuevas, cero delta
    en `contracts/mcp.yml`. `cargo test --workspace --locked`, fmt, clippy `-D warnings` y doc en
    verde.
  - **[Cierre] Dado** la re-pasada de `/mutantes` con el alcance de la ficha
    (`-p lodestar-core --file crates/lodestar-core/src/model.rs --file
    crates/lodestar-core/src/plan.rs`), **Cuando** termina, **Entonces** `missed.txt` **no lista
    ninguna de las seis funciones** (ni las que queden tras una eventual retirada por
    equivalencia).

- **Dependencias**: ninguna. E31 está completa; las seis tandas son independientes entre sí y el
  orden propuesto es el de la ficha, (a) → (f), con (a)/(b) primero por ser la red bajo `E31-H02`.

- **Pruebas** (esta historia **es** sus pruebas):
  - `crates/lodestar-core/tests/documento.rs` — tandas (a) y (b): tres tests
    (`split_front_corta_igual_con_crlf_que_con_lf`,
    `patch_sin_efecto_devuelve_el_documento_byte_a_byte`,
    `patch_con_bloque_mal_escaneado_cae_al_fallback_y_lo_declara`), con fixtures **inline** (raw
    strings), el estilo del fichero.
  - `crates/lodestar-core/tests/core.rs` — tandas (c)–(f): cuatro tests, con los helpers ya
    existentes del fichero (`fm(&[...])`, `rp(...)`, `DocumentSet::from_files`). **No hacen falta
    fixtures de `lodestar-fixtures`**: todo es contenido inline, como el resto de ambos ficheros.
  - La **fase roja** no es un stub: es la mutación aplicada al árbol (criterio [EVIDENCIA]). No hay
    implementador al que separar del autor de tests, así que el `/tdd` clásico no aplica; el
    protocolo es escribir el test → aplicarle su mutación → verlo rojo → revertir → verde.

- **Frontera (`mcp.yml`)**: **no — sin delta de contrato, explícitamente**. La historia no toca
  ninguna tool de `lodestar-mcp` ni `contracts/mcp.yml`: es tests-only sobre `lodestar-core`.
  (Las tandas (c) y (e) *protegen* campos que ya viajan en el wire —`semanticDiff.relation_changes`
  y el orden de `created/modified/deleted`—, pero el contrato no cambia ni un byte.)

- **Proceso**: ciclo **acotado** (sin `/contrato`, sin frontera). **Juez ciego** al final con
  encargo específico: verificar **ejecutando** al menos dos mutaciones de la tabla (una de
  `model.rs`, una de `plan.rs`) que el test correspondiente se pone rojo — la lección de E23/E30/E31
  es que leer no basta. La re-pasada de `/mutantes` del criterio [Cierre] es parte del Done.

## Cierre de la épica

- `decisiones/27-gaps-de-suite-en-model-y-plan.md` → `estado: cerrada`, con `cerrada_en` y
  `revisada_en` (editada antes si alguna tanda se retiró por equivalencia).
- Fila de `§27` en `decisiones/README.md`.
- `IMPLEMENTATION_STATUS.md`: épica E32 y la constancia de la verificación por mutación.
- Sección E32 de [`trazabilidad.md`](trazabilidad.md).
- Sin entrada de `CHANGELOG.md` de cara al usuario: nada observable cambia (a criterio del
  implementador anotarlo como interno).
