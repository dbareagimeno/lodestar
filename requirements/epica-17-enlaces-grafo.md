# E17 — Enlaces y grafo universal

> **Fase**: `§20.14` PRs 3 + 4 fusionados (`REFACTOR_PHASE_2 §Fase 7` y `§Fase 8`).
> **Objetivo de la épica**: que un documento de la raíz enlace a otro tres niveles por debajo, y al
> revés, y que ambos enlaces caigan en el **mismo grafo**. Al cerrar E17 el grafo contiene todos los
> `.md` descubiertos, los backlinks funcionan globalmente y los enlaces se resuelven **únicamente por
> path**, sin una sola heurística.
> Referencias maestras: `ARCHITECTURE.md §20.6`, `§20.7`, `§20.9`; `CLAUDE.md` invariantes #3/#6.

**Por qué una sola épica para dos PRs**: el grafo es la inversión de los enlaces resueltos. Separarlos
obliga a un `Analysis` intermedio que ningún consumidor usaría (`§20.14`, ajuste 1).

**Principio rector**: *determinismo sin heurística*. La misma colección de ficheros produce siempre el
mismo inventario, grafo, backlinks, diagnósticos y resultado de consulta. Prohibido: buscar por
basename, por título, añadir `.md`, resolver un directorio como `index.md`, tratar `README.md` como
fallback, interpretar aliases o desempatar por similitud. Sin sintaxis de Obsidian.

## Decisión de diseño de la épica: `pulldown-cmark` en el core

El parser actual son **dos regex** (`crates/lodestar-core/src/model.rs:16-17,257-258`) que solo ven
`[texto](href)`. La Fase 7 exige además enlaces **de referencia** (`[t][id]` con su definición
`[id]: ../p.md` en otro punto del documento), lo que obliga a una segunda pasada con resolución de
etiquetas, y a no confundir enlaces dentro de bloques de código.

Se adopta **`pulldown-cmark`** como dependencia de `lodestar-core`:

- Es **pura** — sin I/O, sin runtime, sin C: no viola el invariante #2 ni el job `core-purity` del CI
  (que prohíbe `tokio`/`rusqlite`/`git2`/`notify`/`tauri`).
- Resuelve los enlaces de referencia de forma nativa y expone `link_type` (inline / reference /
  collapsed / shortcut / autolink), que es exactamente la clasificación que pide `§20.6`.
- Su `OffsetIter` da el rango de bytes de cada enlace — necesario para el `range` de los diagnósticos
  de `§20.9` y para la reescritura quirúrgica de destinos en `move_document` (E21).
- Reconoce fences, HTML embebido y escapes, que la regex trata como texto plano.

Alternativa descartada: extender la regex. No cubre enlaces de referencia sin reimplementar buena
parte de un parser Markdown, y la reescritura de `move_document` necesita offsets fiables.

---

### E17-H01 — Extracción de enlaces del documento

- **Objetivo**: obtener, de un cuerpo Markdown, **todos** los enlaces con su href crudo y su posición,
  sin resolverlos todavía.
- **Referencias**: `ARCHITECTURE.md §20.6` · `REFACTOR_PHASE_2 §Fase 7 (Tipos admitidos / no admitidos)` ·
  `crates/lodestar-core/src/model.rs:16-17,224-269` (`LINK_RE`, `out_links`, `out_links_with_href`,
  `raw_rel_links`).
- **Alcance**:
  - Módulo nuevo `crates/lodestar-core/src/links.rs` con `extract_links(body) -> Vec<RawLink>`, donde
    `RawLink` lleva el href crudo, el texto del enlace, el rango de bytes y el tipo de enlace.
  - Cubre: inline `[t](p.md)`, con título `[t](p.md "T")`, con fragmento `[t](p.md#s)`, de referencia
    `[t][id]` + `[id]: ../p.md`, colapsados `[id][]`, cortos `[id]`, anchors `[t](#s)` y autolinks.
  - **No** cubre (y debe ignorar como texto plano): `[[doc]]`, `![[doc]]`, `[[doc#h]]`, `[[doc|alias]]`.
  - Los enlaces dentro de bloques de código (fences e indentados) y de spans de código **no** cuentan.
  - Añadir `pulldown-cmark` a `Cargo.toml` (workspace + `lodestar-core`).
  - Retirar `LINK_RE`, `out_links`, `out_links_with_href` y `raw_rel_links` de `model.rs` una vez que
    E17-H02 haya migrado a sus consumidores.
- **Fuera de alcance**: resolver el href a un path (E17-H02); las imágenes (`![alt](img.png)`) — no son
  enlaces de navegación y `§20.6` no las lista.
- **Criterios de aceptación**:
  - **Dado** un cuerpo con un enlace inline, uno con fragmento y uno con título, **Cuando** se
    extraen, **Entonces** aparecen los 3 con su href crudo exacto → `extrae_inline`.
  - **Dado** `Consulta [la spec][spec].` y, 40 líneas después, `[spec]: ../../reference.md`,
    **Cuando** se extraen, **Entonces** aparece un enlace con href `../../reference.md`
    → `extrae_referencia`.
  - **Dado** una definición de referencia que **no** existe, **Cuando** se extrae, **Entonces** no se
    inventa ningún enlace y no se produce error → `referencia_sin_definicion`.
  - **Dado** un cuerpo con `[[wikilink]]` y `![[embed]]`, **Cuando** se extrae, **Entonces** no
    aparece ningún enlace → `wikilinks_ignorados`.
  - **Dado** un enlace dentro de un bloque ` ``` `, **Cuando** se extrae, **Entonces** no aparece
    → `enlace_en_fence_ignorado`.
  - **Dado** un enlace cualquiera, **Cuando** se extrae, **Entonces** su rango de bytes acota
    exactamente el destino dentro del cuerpo → `rango_del_destino`.
- **Dependencias**: E16 completa.
- **Pruebas**: `crates/lodestar-core/tests/`: los 6 nombres.
- **Frontera (mcp.yml)**: no.

### E17-H02 — Resolución y clasificación de destinos

- **Objetivo**: convertir cada href en un `LinkTarget` clasificado, resuelto **solo por path**.
- **Referencias**: `ARCHITECTURE.md §20.6` · `REFACTOR_PHASE_2 §Fase 7 (Algoritmo / Modelo /
  Prohibiciones)` · `crates/lodestar-core/src/model.rs:186-221` (`resolve_link` actual, que **añade
  `index.md`** a los destinos terminados en `/` — prohibido por `§20.6`).
- **Alcance**:
  - `resolve(raw: &RawLink, from: &RelPath, inventory: &FileMap) -> ResolvedLink`, siguiendo los 10
    pasos de `§20.6`: separar path/query/fragment → detectar URI externa → detectar self-anchor →
    resolver relativo al **directorio del documento origen** → normalizar `.`/`..` → verificar
    contención → resolver contra el inventario → clasificar → registrar href original **y** destino
    normalizado.
  - `LinkTarget` con las 6 variantes de `§20.6`: `Document`, `WorkspaceFile`, `ExternalUri`,
    `SelfAnchor`, `Missing`, `EscapesWorkspace`.
  - **Percent-decoding** del path (`docs/mi%20nota.md` → `docs/mi nota.md`) antes de resolver.
  - `WorkspaceFile`: un enlace a `../../src/auth/token_service.rs` que **existe en el proyecto** se
    clasifica como tal — Lodestar indica que el fichero existe, pero **no lo incorpora como nodo**
    del grafo. Requiere consultar el árbol de ficheros, no solo el inventario de `.md`.
  - Retirar del algoritmo: la conversión `foo/` → `foo/index.md`, el requisito de que el destino
    termine en `.md` para considerarse interno, y el filtro de "href relativo" de `raw_rel_links`.
- **Fuera de alcance**: los diagnósticos que se derivan de la clasificación (E17-H03).
- **Criterios de aceptación**:
  - **Dado** `README.md` en la raíz con `[x](packages/api/docs/endpoints.md)`, **Cuando** se resuelve,
    **Entonces** `Document("packages/api/docs/endpoints.md")` → `raiz_hacia_tres_niveles`.
  - **Dado** `three/levels/deep/third.md` con `[x](../../../README.md)`, **Cuando** se resuelve,
    **Entonces** `Document("README.md")` → `tres_niveles_hacia_raiz`.
  - **Dado** `one/a.md` con `[x](../two/levels/b.md)`, **Cuando** se resuelve, **Entonces** apunta al
    hermano en otro árbol → `hermanos_en_arboles_distintos`.
  - **Dado** `[x](./doc.md)` y `[x](doc.md)`, **Cuando** se resuelven, **Entonces** dan el mismo
    destino → `punto_barra_equivale`.
  - **Dado** `[x](docs/mi%20nota.md)` con `docs/mi nota.md` en el inventario, **Cuando** se resuelve,
    **Entonces** `Document("docs/mi nota.md")` → `percent_encoding`.
  - **Dado** `[x](otro.md#seccion)`, **Cuando** se resuelve, **Entonces** el destino es `otro.md` y el
    fragmento `seccion` se conserva aparte → `fragmento_separado`.
  - **Dado** `[x](#instalacion)`, **Cuando** se resuelve, **Entonces** `SelfAnchor("instalacion")`
    → `anchor_propio`.
  - **Dado** `[x](https://example.com)` y `[x](mailto:a@b.c)`, **Cuando** se resuelven, **Entonces**
    `ExternalUri` → `uri_externa`.
  - **Dado** `[x](../../src/auth/token_service.rs)` con ese fichero existiendo, **Cuando** se
    resuelve, **Entonces** `WorkspaceFile` — y **no** es nodo del grafo → `enlace_a_codigo`.
  - **Dado** `[x](no-existe.md)`, **Cuando** se resuelve, **Entonces** `Missing` → `destino_inexistente`.
  - **Dado** `[x](../../../../../../etc/passwd)`, **Cuando** se resuelve, **Entonces**
    `EscapesWorkspace` y jamás se toca el disco → `escape_del_workspace`.
  - **Dado** un directorio `guias/` con un `guias/index.md`, **Cuando** se resuelve `[x](guias/)`,
    **Entonces** NO se resuelve a `guias/index.md` → `directorio_no_es_index`.
  - **Dado** dos documentos con el mismo basename en árboles distintos, **Cuando** se resuelve un
    enlace a uno, **Entonces** apunta inequívocamente al del path indicado, sin ambigüedad
    → `mismo_basename_inequivoco`.
- **Dependencias**: E17-H01.
- **Pruebas**: `crates/lodestar-core/tests/`: los 13 nombres.
- **Frontera (mcp.yml)**: **sí** (`LinkTarget` viaja en `knowledge_get.outgoingLinks`).

### E17-H03 — Diagnósticos de enlaces

- **Objetivo**: los tres códigos de enlace de `§20.9`, derivados de la clasificación de E17-H02.
- **Referencias**: `ARCHITECTURE.md §20.9` · `REFACTOR_PHASE_2 §Fase 7 (Capitalización)` ·
  `crates/lodestar-core/src/conform.rs` (los `LINK-STUB`/`LINK-REL` que sustituyen).
- **Alcance**:
  - `LINK-TARGET-MISSING` (de `Missing`), `LINK-ESCAPES-WORKSPACE` (de `EscapesWorkspace`) y
    `LINK-CASE-MISMATCH`.
  - **Case mismatch**: un enlace a `Docs/Auth.md` cuando el fichero real es `docs/auth.md` debe
    diagnosticarse **aunque el sistema de ficheros sea case-insensitive** y el enlace "funcione"
    localmente — es un problema de portabilidad. Se detecta comparando contra el inventario, no
    contra el disco (así el test es determinista en macOS y en Linux).
  - Un `WorkspaceFile` que **no existe** produce el mismo `LINK-TARGET-MISSING`, con severidad
    warning por defecto (`missingWorkspaceFiles: warning` en `§20.9`), frente al error de un
    documento Markdown inexistente (`danglingDocumentLinks: error`).
  - Cada diagnóstico lleva el `range` del destino dentro del documento origen (posible gracias a
    E17-H01) y el path relacionado en `related`.
  - Borrar `LINK-STUB` y `LINK-REL` del catálogo.
- **Fuera de alcance**: hacer configurable la severidad por familia (E20, `validation:` de `§20.9`);
  aquí valen los defaults del documento.
- **Criterios de aceptación**:
  - **Dado** un enlace a un `.md` inexistente, **Cuando** se valida, **Entonces**
    `LINK-TARGET-MISSING` con severidad error, con el rango del destino → `link_missing_con_rango`.
  - **Dado** un enlace a `../../fuera.md` que escapa de la raíz, **Cuando** se valida, **Entonces**
    `LINK-ESCAPES-WORKSPACE` → `link_escapa`.
  - **Dado** `docs/auth.md` en el inventario y un enlace a `Docs/Auth.md`, **Cuando** se valida,
    **Entonces** `LINK-CASE-MISMATCH` con severidad warning, **en cualquier sistema de ficheros**
    → `link_case_mismatch`.
  - **Dado** un enlace a un `.rs` inexistente, **Cuando** se valida, **Entonces**
    `LINK-TARGET-MISSING` con severidad **warning**, no error → `workspace_file_ausente_es_warning`.
  - **Dado** un enlace externo y un anchor propio, **Cuando** se validan, **Entonces** no producen
    diagnóstico → `externos_y_anchors_no_diagnostican`.
- **Dependencias**: E17-H02, E16-H05.
- **Pruebas**: `crates/lodestar-core/tests/`: los 5 nombres.
- **Frontera (mcp.yml)**: **sí** (`CheckCode`).

### E17-H04 — El grafo universal: `Analysis` nueva

- **Objetivo**: nodos = todos los documentos descubiertos; aristas = enlaces resueltos entre ellos.
- **Referencias**: `ARCHITECTURE.md §20.7` · `REFACTOR_PHASE_2 §Fase 8` ·
  `crates/lodestar-core/src/bundle.rs:50-137` (`compute_analysis`), `types.rs:422-441` (`Analysis`).
- **Alcance**:
  - `Analysis` pasa a la forma de `§20.7`: `documents` · `outgoing: BTreeMap<RelPath,
    Vec<ResolvedLink>>` · `incoming: BTreeMap<RelPath, Vec<LinkReference>>` · `isolated` ·
    `dangling: Vec<DanglingLink>` · `diagnostics`.
  - `outgoing` deja de ser adyacencia de strings (`Vec<RelPath>`) y pasa a llevar el enlace resuelto
    completo (href crudo, destino, fragmento, clasificación): es lo que necesitan `knowledge_get`,
    `move_document` y el store v2.
  - `incoming` es la inversa, con la referencia del origen.
  - `isolated`: sin enlaces internos entrantes **ni** salientes (`§20.7`). No es diagnóstico.
  - `dangling`: los `Missing`, con su origen y href crudo — hoy es un `Vec<RelPath>` de destinos
    perdidos (`bundle.rs:123`), que no permite decir **quién** enlazaba mal.
  - Retirar `hard_fail`/`warn_count` en favor de un recuento derivado de `diagnostics`, o
    conservarlos si el gate de CI los usa — decidir en la fase roja y dejarlo fijado por test.
- **Fuera de alcance**: el DDL que materializa todo esto (E18).
- **Criterios de aceptación**:
  - **Dado** el fixture `arbitrary()` (raíz + 3 niveles con enlaces cruzados), **Cuando** se analiza,
    **Entonces** `documents` tiene los 4 y hay aristas en ambos sentidos entre raíz y profundo
    → `grafo_cubre_todas_las_profundidades`.
  - **Dado** un documento enlazado desde 3 orígenes distintos, **Cuando** se analiza, **Entonces**
    `incoming` lista los 3 con su href crudo → `backlinks_globales`.
  - **Dado** un enlace roto, **Cuando** se analiza, **Entonces** `dangling` identifica origen, href
    crudo y destino pretendido → `dangling_identifica_origen`.
  - **Dado** un documento sin enlaces de ningún tipo, **Cuando** se analiza, **Entonces** está en
    `isolated` y no genera diagnóstico → `isolated_sin_diagnostico`.
  - **Dado** un enlace a un fichero de código, **Cuando** se analiza, **Entonces** ese fichero **no**
    es nodo del grafo, aunque el enlace se registre → `codigo_no_es_nodo`.
  - **Dado** el mismo conjunto de ficheros analizado dos veces, **Cuando** se comparan los resultados,
    **Entonces** son idénticos (inventario, grafo, backlinks, diagnósticos) → `analisis_determinista`.
- **Dependencias**: E17-H02, E17-H03.
- **Pruebas**: `crates/lodestar-core/tests/`: los 6 nombres.
- **Frontera (mcp.yml)**: **sí** (`Analysis` es contrato).

### E17-H05 — Superficie de grafo sobre el modelo nuevo

- **Objetivo**: `graph_query`, `knowledge_get` y `impact_analyze` hablan del grafo universal.
- **Referencias**: `ARCHITECTURE.md §20.7`, `§20.10` · `crates/lodestar-core/src/graph.rs`,
  `crates/lodestar-core/src/bundle.rs:180-251` (`backlinks`), `crates/lodestar-app/src/lib.rs:945`
  (`graph_query`), `:1107` (`impact_analyze`).
- **Alcance**:
  - `Backlinks` pierde `index_refs` (ya retirado en E16-H02) y sus `inbound`/`out` pasan a los tipos
    nuevos.
  - `GraphNode` (`types.rs:460`) pierde `type`/`status` (campos OKF) y gana el **título derivado** de
    E16-H03; conserva `ghost` para los destinos `Missing`.
  - `graph_query` mantiene sus 8 operaciones (`backlinks`, `outgoing`, `neighborhood`, `orphans`,
    `dangling`, `path_between`, `cycles`, `components`), renombrando `orphans` → `isolated` en el
    wire. El BFS, `path_between`, `cycles` y `components` de `graph.rs` **no cambian de semántica**:
    operan sobre la adyacencia nueva.
  - `impact_analyze` deja de depender de tipos OKF y de relaciones tipadas: su impacto se calcula
    sobre backlinks, salientes, movimiento de paths, eliminación y documentos afectados por una
    selección de metadata (`§20.10`). Los `BlockingReference` derivados de relaciones obligatorias
    desaparecen (mueren del todo en E20 con `core::schema`).
- **Fuera de alcance**: `metadata_inspect` (E20); la selección por consulta que alimenta
  `impact_analyze` (E19/E21).
- **Criterios de aceptación**:
  - **Dado** un workspace con enlaces a 3 niveles, **Cuando** se pide `graph_query(backlinks)` sobre
    el documento raíz, **Entonces** devuelve el documento profundo que lo enlaza
    → `graph_backlinks_globales`.
  - **Dado** un workspace, **Cuando** se pide `graph_query(isolated)`, **Entonces** devuelve los
    documentos sin enlaces en ningún sentido → `graph_isolated`.
  - **Dado** un documento cualquiera, **Cuando** se pide `knowledge_get` con `outgoingLinks` y
    `backlinks`, **Entonces** ambos reflejan el grafo universal, con hrefs crudos
    → `knowledge_get_enlaces`.
  - **Dado** un `impact_analyze` sobre un documento con 5 backlinks, **Cuando** se calcula,
    **Entonces** reporta los 5 afectados sin mencionar tipos ni relaciones
    → `impacto_sin_tipos_okf`.
- **Dependencias**: E17-H04.
- **Pruebas**: `crates/lodestar-mcp/tests/mcp.rs` y `crates/lodestar-app/tests/`: los 4 nombres.
- **Frontera (mcp.yml)**: **sí**.

---

## Orden de construcción

```
H01 (extracción) ─→ H02 (resolución) ─→ H03 (diagnósticos) ─→ H04 (Analysis) ─→ H05 (superficie)
```

Estrictamente secuencial: cada historia consume el tipo que produce la anterior.

## Criterio de salida de la épica

Sobre un proyecto real con documentación repartida en `docs/`, `packages/*/docs/` y la raíz: los
enlaces relativos funcionan entre cualquier profundidad, los backlinks son globales, los enlaces a
código se clasifican sin entrar en el grafo, los escapes se rechazan y el análisis es idéntico en dos
ejecuciones consecutivas. Es el criterio de aceptación central de `REFACTOR_PHASE_2 §Resultado
esperado`.
