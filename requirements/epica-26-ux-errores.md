# E26 — UX de errores de la superficie MCP: código, mensaje y cota

> **Fase**: posterior a v0.3.1, sobre la misma rama que E25. No es una fase de `§20.14` ni de
> `§19.8`: es la continuación directa del bloque C de E24 hasta el final de la superficie.
> **Objetivo de la épica**: que ningún error de la frontera MCP llegue al agente como un código
> pelado, que ninguna consulta que el motor no entiende se conteste con un resultado, que haya un
> solo dialecto de dot-paths en toda la superficie, que ninguna tool pueda devolver una respuesta sin
> cota, y que el contrato escrito vuelva a describir lo que el servidor hace.
> Referencias maestras: `contracts/mcp.yml`, `ARCHITECTURE.md §19.3` (códigos estables) ·
> `§19.6` (superficie de 10 tools) · `§20.8`/`§20.10` (lenguaje de consulta e inspección de
> metadata), `CLAUDE.md` (invariantes #3 y #4).

**Origen**: auditoría de la superficie de errores (2026-07-29), hermana de la del camino de escritura
que abre E25. Los seis defectos se localizaron sobre el código de la frontera y **cada historia
declara el escenario que hoy los reproduce** por el wire.

**Principio rector**: el de **E24-H07**, llevado hasta el final — *«una respuesta silenciosamente
equivocada es peor que un error»*. E24-H07/H08 lo aplicaron al lenguaje de consulta; aquí se aplica a
lo que queda: el mensaje que acompaña al código, el `TypeError` que hoy se traga un documento, el
dot-path que significa dos cosas distintas según la tool, la respuesta sin límite y el contrato que
describe una versión anterior del servidor. Su corolario operativo: **cuando el motor no puede
responder a lo que se le pidió, lo dice, y lo dice de forma que un agente pueda ramificar por código
y corregir por mensaje.**

**Fuera de alcance (explícito)**:

- **Rechazar parámetros NO declarados.** Sigue siendo la decisión abierta que E24-H18 registró en
  `decisiones/`; la política vigente (`contracts/mcp.yml:311-325`) es «se validan los VALORES de lo
  declarado, se ignora lo no declarado» y esta épica **la cumple**, no la revisa.
- **Añadir códigos al catálogo.** `ErrorCode` sigue teniendo **16 filas** (`§19.3`, invariante #4).
  Todas las historias resuelven con los códigos existentes; la única fila que cambia de estado en
  toda la rama es `RECOVERY_FAILED`, y la mueve **E25-H02**, no esta épica.
- **La publicación y los documentos de estado** (`CHANGELOG.md`, `IMPLEMENTATION_STATUS.md`,
  `requirements/README.md`, `requirements/trazabilidad.md`), compartidos con E25. E26-H11 **recopila**
  las consecuencias declaradas para la nota de release, pero no publica.

---

## Bloque A — El error dice qué pasó

### E26-H07 — Todo error de superficie lleva código **y** mensaje

- **Objetivo**: que un agente pueda ramificar por código y **corregir por mensaje**, en las diez
  tools y no en dos.
- **Defectos (U1 + U2) y escenarios que los reproducen hoy**:
  1. **Ocho de las diez tools devuelven el código pelado**: `crates/lodestar-mcp/src/tools.rs:327`
     (`knowledge_get`), `:340` (`metadata_inspect`), `:371` (`knowledge_check`), `:406`
     (`graph_query`), `:427` (`impact_analyze`), `:467` (`change_plan`), `:486` (`change_apply`) y
     `:505` (`change_revert`) hacen todos `.map_err(|e| e.as_str().to_string())`. El agente recibe
     literalmente `INVALID_SCHEMA`, sin una palabra sobre **qué** parámetro, **qué** valor o **qué**
     se esperaba. No es un descuido del despachador: los productores de `lodestar-app` son
     `Result<_, ErrorCode>` y **no tienen sitio donde poner el mensaje** —`metadata_inspect`
     (`crates/lodestar-app/src/lib.rs:840`, `:841`, `:847`) y `graph_query` (`:1275`) devuelven la
     variante desnuda—. La excepción es `knowledge_search` (`tools.rs:304`), que desde E24-H10 emite
     `"{código}: {mensaje}"`: existe el patrón, falta extenderlo.
  2. **`graph_query` sin `ref` responde `DOCUMENT_NOT_FOUND`** (`crates/lodestar-app/src/lib.rs:1175`,
     y lo mismo en `:1199`, `:1204`, `:1238`, `:1239`). El propio rustdoc lo admite y razona por qué
     (`:1112-1114`): no había código de «falta parámetro» y se reusó el más cercano. Con mensaje sí
     lo hay: `INVALID_SCHEMA` nombrando el parámetro que falta y la operación que lo exige. Tal como
     está, un agente que **olvida** el `ref` recibe el mismo error que si el documento **no existe**,
     y toma el camino de recuperación equivocado.
  3. **`change_plan` descarta el `ParseError`**: `build_selection_expression`
     (`crates/lodestar-app/src/lib.rs:2358`) hace `map_err(|_| ErrorCode::InvalidSchema)` en sus dos
     ramas (`:2363` para `where`, `:2367` para `filter`), tirando el diagnóstico del parser del core.
     `build_search_expression` (`:2970`), en cambio, lo **entrega entero** y saneado
     (`:2976-2983`, con `mensaje_de_filtro:2961` filtrando el interno de serde). La misma consulta
     malformada, dos calidades de respuesta según la tool — que es la asimetría que E24-H10 cerró
     para el **código** y dejó abierta para el **mensaje**.
- **Referencias**: `ARCHITECTURE.md §19.3` (catálogo de 16 códigos) · `contracts/mcp.yml:303-310`
  (`errores_ejecucion`) · `crates/lodestar-core/src/types.rs` (`ErrorCode:1273`) ·
  `crates/lodestar-app/src/lib.rs` (`metadata_inspect:827`, `graph_query:1158`,
  `build_selection_expression:2358`, `build_search_expression:2970`, `mensaje_de_filtro:2961`) ·
  `crates/lodestar-mcp/src/tools.rs` (`call:261`). Precedente: **E24-H10** (el código estable abre el
  mensaje) y **E23-H11** (aceptar y descartar en silencio es el defecto, no la tolerancia).
- **Alcance**:
  - Los productores de `lodestar-app` que hoy devuelven `ErrorCode` desnudo pasan a devolver
    **código + mensaje**. El catálogo **no se toca** (sigue en `core::types`, 16 filas, invariante #4)
    y **no se crea una jerarquía paralela de códigos**: lo que se introduce es el envoltorio de
    fachada que empareja un `ErrorCode` del core con un `String`, en **un solo** sitio de
    `lodestar-app`, reusando el patrón que ya existe con `WorkspaceError::InvalidSchema` +
    `workspace_error_code`. El grep de CI de E24-H17 sigue prohibiendo redefinir literales de código
    fuera del core, y debe seguir en verde.
  - Los ocho brazos de `tools::call` emiten `"{código}: {mensaje}"`, exactamente el formato que ya usa
    `knowledge_search` (`tools.rs:304`) y las comprobaciones locales del despachador
    (`tools.rs:283`, `:357`, `:457`).
  - `graph_query` sin `ref` (o sin `to` en `path_between`) → **`INVALID_SCHEMA`** con mensaje que
    nombra el parámetro y la operación. `DOCUMENT_NOT_FOUND` queda para lo que su nombre dice: un
    `ref` presente que **no resuelve**.
  - `change_plan` conserva el texto del `ParseError`/`FilterError`, saneado por la **misma**
    `mensaje_de_filtro` (`:2961`) — no una segunda copia (invariante #3). `build_search_expression` y
    `build_selection_expression` quedan con el mismo criterio de mensaje.
  - Los mensajes van en **español** (regla de idioma del repo), salvo los identificadores congelados
    (nombres de código, de tool, de parámetro y de operación).
- **Fuera de alcance**: cambiar qué código produce cada situación **más allá** del caso `graph_query`
  sin `ref`. Esta historia añade mensaje; no reclasifica el catálogo.
- **Criterios de aceptación**:
  - **Dado** cada una de las 8 tools que hoy devuelven el código pelado, **Cuando** se provoca su
    error más común, **Entonces** el texto abre con el código estable y **continúa** con un mensaje
    no vacío → `todas_las_tools_dan_codigo_y_mensaje` (hoy 8 de 10 devuelven solo el código).
  - **Dado** `graph_query{operation:"backlinks"}` sin `ref`, **Cuando** se llama, **Entonces** el
    código es `INVALID_SCHEMA` y el mensaje nombra `ref` y la operación →
    `graph_query_sin_ref_es_invalid_schema`.
  - **Dado** `graph_query{operation:"backlinks", ref:{path:"no-existe.md"}}`, **Cuando** se llama,
    **Entonces** sigue siendo `DOCUMENT_NOT_FOUND` → `ref_que_no_resuelve_sigue_siendo_not_found`
    (control anti-vacuo: el arreglo no puede consistir en mapear todo a `INVALID_SCHEMA`).
  - **Dado** `change_plan` con `selection.where: "status ="`, **Cuando** se llama, **Entonces** el
    mensaje contiene el diagnóstico del parser, igual que el de `knowledge_search` con la misma
    consulta → `change_plan_conserva_el_error_del_parser`.
  - **Dado** un `filter` JSON malformado en cualquiera de las dos tools, **Cuando** se llama,
    **Entonces** el mensaje **no** contiene `"untagged enum"` →
    `errores_no_filtran_internos_de_serde` (existente de E24-H10, ampliado a `change_plan`).
  - **Dado** el catálogo de `ErrorCode`, **Cuando** corre la suite, **Entonces** sigue teniendo 16
    filas → `catalogo_de_errores_tiene_dieciseis_filas` (existente de E24-H17, sigue verde).
  - **No regresión**: los tests de la superficie de error de `crates/lodestar-mcp/tests/mcp.rs` y
    `crates/lodestar-app/tests/error.rs` siguen verdes; el grep de CI del catálogo, también.
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-mcp/tests/mcp.rs` (la tabla de las 10 tools) ·
  `crates/lodestar-app/tests/error.rs` · `crates/lodestar-app/tests/grafo.rs`.
- **Frontera (mcp.yml)**: **sí** — cambia la lista `errores` de 8 tools y el código de `graph_query`
  sin `ref`.
- **Delta de contrato**:
  ```yaml
  # contracts/mcp.yml
  meta:
    errores_ejecucion: >-
      … Desde E26-H07 el texto de TODA tool tiene la forma «CODIGO: mensaje»: el código estable de
      `ErrorCode::as_str()` (nunca el `Debug` de la variante Rust) seguido de un mensaje accionable
      en español. Hasta v0.4.0 ocho de las diez tools devolvían el código PELADO, porque los
      productores de `lodestar-app` eran `Result<_, ErrorCode>` y no tenían dónde poner el mensaje.

  tools:
    - nombre: graph_query
      params:
        - { nombre: ref, …, semantica: "… requerido en backlinks/outgoing/neighborhood y como
              origen en path_between; su AUSENCIA en esas operaciones es INVALID_SCHEMA (E26-H07;
              hasta v0.4.0 era DOCUMENT_NOT_FOUND, indistinguible de un ref que no resuelve).
              Un ref PRESENTE que no resuelve sigue siendo DOCUMENT_NOT_FOUND" }
        - { nombre: to, …, semantica: "… su ausencia en path_between es INVALID_SCHEMA (idem)" }
      errores: ["INVALID_SCHEMA (falta «operation»; falta «ref»/«to» en la operación que lo exige;
                 operation desconocida)", "DOCUMENT_NOT_FOUND (ref/to presentes que NO resuelven)",
                "WorkspaceError"]
    - nombre: knowledge_get
      errores: ["INVALID_SCHEMA (falta «ref» o su forma es inválida)", "DOCUMENT_NOT_FOUND",
                "WorkspaceError"]      # el «falta el parámetro «ref»» suelto desaparece
    - nombre: metadata_inspect
      errores: ["INVALID_SCHEMA (falta «mode»; mode desconocido; mode «field» sin «field»
                 o con un dot-path inválido)", "WorkspaceError"]
    - nombre: impact_analyze
      errores: ["INVALID_SCHEMA (falta «ref» o «proposedOperation.kind»; kind fuera de
                 {move, delete})", "DOCUMENT_NOT_FOUND (ref no resuelve)", "INTERNAL_IO_ERROR",
                "WorkspaceError"]
    - nombre: change_plan
      errores: [..., "INVALID_SCHEMA (… un `where`/`filter` de `selection` malformado, CON el
                 diagnóstico del parser del core en el mensaje — E26-H07; hasta v0.4.0 el
                 ParseError se descartaba con `map_err(|_| …)`)"]
  ```

### E26-H08 — Un `TypeError` de consulta se reporta, no excluye documentos en silencio

- **Objetivo**: que una consulta cuyo tipo no casa con los datos **falle**, en vez de devolver una
  lista recortada que nadie puede distinguir de la respuesta correcta.
- **Defecto (U3) y escenario que lo reproduce hoy**: la evaluación por documento se descarta con
  `if !matches!(evaluate(...), Ok(true)) { continue; }` en los **dos** consumidores del lenguaje:
  `knowledge_search` (`crates/lodestar-app/src/lib.rs:623`) y la selección masiva de `change_plan`
  (`expand_selection`, `crates/lodestar-app/src/lib.rs:2340`). Un `Err(TypeError)` —por ejemplo
  `TypeError::OrderNotDefined` (`crates/lodestar-core/src/eval.rs:298`), que produce
  `priority >= "high"` sobre un documento con `priority: 2`— cae en el mismo `continue` que un
  `Ok(false)`: el documento **se excluye**. Efectos medidos:
  - una consulta con un error de tipo real devuelve `[]` **sin un solo aviso**, indistinguible de «no
    hay resultados»;
  - la exclusión se decide **documento a documento**, así que sobre una base heterogénea la misma
    consulta devuelve unos documentos y calla sobre otros — el resultado depende del contenido y no
    hay forma de saber cuáles faltan;
  - en `change_plan` es peor: una **selección masiva** salta documentos en silencio, y el plan afecta
    a menos ficheros de los que el agente cree haber seleccionado.
  Los dos rustdoc lo consagran hoy como comportamiento deseado (`lib.rs:615-616` y `:2303-2304`,
  *«sin propagarse a la búsqueda entera»* / *«sin abortar el plan»*): esta historia lo revisa, igual
  que E24-H07 revisó el criterio de E19-H04.
- **Referencias**: `ARCHITECTURE.md §20.8` (lenguaje tipado) · `crates/lodestar-core/src/eval.rs`
  (`eval_orden:285`, `TypeError` y sus variantes) · `crates/lodestar-app/src/lib.rs`
  (`knowledge_search`, `expand_selection:2305`) · **E24-H07** (mismo principio, aplicado al parseo) ·
  `contracts/mcp.yml:387` (la limitación declarada de fechas, que es justo el generador de
  `TypeError` más probable en una base real).
- **Alcance**:
  - Un `TypeError` durante la evaluación de `where`/`filter` **aborta la consulta** con
    `INVALID_SCHEMA` y un mensaje que nombra el campo, el operador, el tipo del campo y el tipo del
    literal — la información que `TypeError` ya lleva (`eval.rs:298-303`). Mismo tratamiento en
    `knowledge_search` y en `change_plan.selection`: la equivalencia entre las dos superficies se
    mantiene (`§20.10`).
  - **Determinismo**: el documento y el error reportados son los del **primer** documento en el orden
    total ya existente (`Analysis::documents`), no el primero que toque el planificador. Un mismo
    workspace y una misma consulta producen siempre el mismo error, palabra por palabra.
  - `Ok(false)` sigue siendo exclusión: no casar **no** es un error. Solo `Err` cambia de
    tratamiento.
- **Consecuencia declarada (material de nota de release)**: consultas que hoy «funcionan» sobre bases
  heterogéneas pasarán a **fallar**. Es el mismo cambio de veredicto que E24-H07 declaró para el
  parseo, ahora en la evaluación, y el aviso tiene que decir cómo salir: acotar la consulta con
  `has(campo)` / comparar con el tipo correcto. Va en la nota de release (recopilada por E26-H11).
- **Criterios de aceptación**:
  - **Dado** un workspace con `priority: 2` (número) y la consulta `where: "priority >= \"high\""`,
    **Cuando** se llama a `knowledge_search`, **Entonces** falla con `INVALID_SCHEMA` y el mensaje
    nombra el campo y los dos tipos → `type_error_de_orden_es_error_de_consulta` (hoy devuelve `[]`).
  - **Dado** esa misma consulta como `selection.where` de `change_plan`, **Cuando** se llama,
    **Entonces** falla con el mismo código y el mismo mensaje →
    `misma_consulta_mismo_error_en_search_y_en_plan`.
  - **Dado** un workspace donde **unos** documentos tienen `priority` numérico y otros textual,
    **Cuando** se ejecuta la consulta dos veces, **Entonces** el error es idéntico las dos veces y
    nombra el mismo documento → `el_type_error_reportado_es_determinista`.
  - **Dado** una consulta que simplemente **no casa** (`where: "status = borrador"` sobre documentos
    con `status: draft`), **Cuando** se llama, **Entonces** devuelve `[]` **sin error** →
    `no_casar_sigue_siendo_ausencia` (control anti-vacuo: el arreglo no puede convertir cualquier
    resultado vacío en un fallo).
  - **Dado** un `where` sobre un campo **ausente** en algunos documentos, **Cuando** se evalúa,
    **Entonces** esos documentos se excluyen sin error, como hasta ahora →
    `campo_ausente_no_es_type_error`.
  - **No regresión**: los tests del lenguaje de consulta
    (`crates/lodestar-core/tests/consulta.rs`) y los de selección
    (`crates/lodestar-app/tests/seleccion.rs`) siguen verdes; los que fijaban la exclusión silenciosa
    se **amplían con el criterio nuevo**, no se borran, y su comentario deja constancia de que E19
    decidió lo contrario.
- **Dependencias**: **E26-H07** (necesita el canal código+mensaje para poder decir qué tipo chocó con
  cuál).
- **Pruebas**: `crates/lodestar-core/tests/consulta.rs` (el `TypeError` en el core) ·
  `crates/lodestar-app/tests/seleccion.rs` · `crates/lodestar-mcp/tests/mcp.rs` (por el wire).
- **Frontera (mcp.yml)**: **sí** — `knowledge_search` y `change_plan` ganan un caso de error.
- **Delta de contrato**:
  ```yaml
  tools:
    - nombre: knowledge_search
      errores: ["INVALID_SCHEMA (where/filter malformado — E24-H10; una entrada de `include` que no
                 sea «frontmatter.<fieldPath>» — E23-H11; y desde E26-H08 un TYPE ERROR de evaluación:
                 comparar un campo con un literal de tipo incompatible, p. ej. `priority >= \"high\"`
                 sobre `priority: 2`)", "WorkspaceError"]
      semantica: >-
        … Un error de TIPO durante la evaluación aborta la consulta con INVALID_SCHEMA nombrando
        campo, operador y los dos tipos (E26-H08). Hasta v0.4.0 EXCLUÍA el documento en silencio, así
        que la respuesta era una lista recortada indistinguible de la correcta, decidida documento a
        documento. No casar (Ok(false)) sigue siendo ausencia, no error.
    - nombre: change_plan
      errores: [..., "INVALID_SCHEMA (… y un TYPE ERROR al evaluar `selection.where`/`selection.filter`
                 — E26-H08: una selección masiva ya no salta documentos en silencio)"]
  ```

---

## Bloque B — Un solo dialecto, y siempre una cota

### E26-H09 — `metadata_inspect` habla el mismo dialecto de dot-paths que la consulta

- **Objetivo**: que un `field path` signifique lo mismo en toda la superficie, y que lo que
  `metadata_inspect` **anuncia** se pueda consultar e inspeccionar tal cual.
- **Defecto (U4) y escenario que lo reproduce hoy**: `App::metadata_inspect` normaliza con
  `FieldPath::parse` (`crates/lodestar-app/src/lib.rs:841`,
  `crates/lodestar-core/src/types.rs:431`) en vez de con `parse::build_field_path`
  (`crates/lodestar-core/src/parse.rs:444`), que es el punto **único** por el que pasan `where`,
  `filter` y `has`/`missing` (E24-H07). Consecuencias medidas:
  - `metadata_inspect{mode:"field", field:"graph.backlinks"}` inspecciona la **clave de
    frontmatter** `graph.backlinks`, mientras `where: "graph.backlinks = 7"` consulta el **grafo**:
    el mismo texto, dos significados según la tool;
  - `field:"frontmatter.graph.backlinks"` —la sintaxis que el propio mensaje de error del parser
    recomienda (`parse.rs:484-486`)— busca una clave literal llamada `frontmatter` y devuelve
    `presentIn: 0`: **silenciosamente equivocado**, la forma exacta de defecto que E24-H08 retiró del
    lenguaje de consulta y que sobrevive aquí;
  - `metadata_inspect{mode:"catalog"}` emite los `name` que produce `ParsedFrontmatter::walk`, así que
    sobre un frontmatter con una clave `graph:` **anuncia** `graph.backlinks` — un nombre que ni
    `where` ni el propio `metadata_inspect` aceptan. La tool que existe para hacer descubrible una
    base desconocida (`§20.10`, E23-H11) devuelve nombres no direccionables.
- **Referencias**: `ARCHITECTURE.md §20.8`/`§20.10` · `crates/lodestar-core/src/parse.rs`
  (`build_field_path:444`, con el anclaje de E24-H08 en `:453` y la validación de namespace de
  E24-H07 en `:477-493`) · `crates/lodestar-core/src/types.rs` (`FieldPath:406`, `parse:431`,
  `from_segments:441`, `es_namespace_reservado:460`, `props_del_namespace:468`,
  `FRONTMATTER_ANCHOR:421`) · `crates/lodestar-core/src/metadata.rs` (`catalog:35`,
  `inspect_field:87`) · `crates/lodestar-app/src/lib.rs` (`metadata_inspect:827`) ·
  **E24-H07** y **E24-H08**, que fijaron el dialecto en `build_field_path` sin extenderlo a esta tool.
- **Alcance**:
  - `build_field_path` pasa de `pub(crate)` a **público** en `core::parse` (su rustdoc `:440-443` ya
    explica por qué debe ser el único normalizador) y `App::metadata_inspect` lo usa en lugar de
    `FieldPath::parse`. **Un solo dialecto**, por construcción y no por convención.
  - El `field` que se le pasa a `metadata_inspect` hereda entonces las tres reglas del lenguaje: la
    abreviatura `frontmatter.status` ≡ `status`, el **anclaje** `frontmatter.graph.backlinks` que
    alcanza la clave del usuario, y el **rechazo** de una propiedad desconocida bajo namespace
    reservado, con el mismo mensaje.
  - Un `field` bajo namespace reservado **válido** (`graph.backlinks`, `document.path`) no es
    inspeccionable: `metadata_inspect` describe **metadata**, y una propiedad calculada no vive en
    ningún frontmatter. Se rechaza con `INVALID_SCHEMA` y un mensaje que apunta a `graph_query`
    (para el grafo) o al anclaje `frontmatter.` (para la clave homónima del usuario). Con E26-H07 ese
    mensaje llega entero al agente.
  - **Direccionabilidad del catálogo**: el `name` que emite `mode:"catalog"` debe ser un texto que
    `metadata_inspect{mode:"field"}` y `where` acepten y resuelvan **al mismo campo**. Cuando la clave
    del usuario colisiona con un namespace reservado, el catálogo emite la forma **anclada**
    (`frontmatter.graph.backlinks`). No se toca `FieldPath::from_segments` ni `walk`: la restricción
    de E24-H07 sigue vigente —validar ahí reventaría el catálogo de cualquier documento con una clave
    `graph`— y lo que cambia es cómo se **rinde** el nombre, no cómo se construye el path.
- **Consecuencia declarada (material de nota de release)**:
  `metadata_inspect{field:"graph.backlinks"}` pasa de devolver una inspección (equivocada) a fallar;
  `field:"frontmatter.status"` pasa de `presentIn: 0` a inspeccionar `status` de verdad; y el
  `name` del catálogo cambia de texto para las claves que colisionan con un namespace.
- **Criterios de aceptación**:
  - **Dado** un documento con frontmatter `graph: {backlinks: 7}`, **Cuando** se llama a
    `metadata_inspect{mode:"field", field:"frontmatter.graph.backlinks"}`, **Entonces** devuelve
    `presentIn: 1` con el valor `7` → `anclaje_frontmatter_alcanza_la_clave_reservada` (hoy
    `presentIn: 0`).
  - **Dado** ese mismo documento, **Cuando** se llama con `field:"graph.backlinks"`, **Entonces**
    falla con `INVALID_SCHEMA` y el mensaje apunta al anclaje y a `graph_query` →
    `namespace_reservado_no_es_inspeccionable`.
  - **Dado** un documento con `status: draft`, **Cuando** se llama con `field:"frontmatter.status"` y
    con `field:"status"`, **Entonces** las dos respuestas son **idénticas** →
    `la_abreviatura_vale_tambien_en_metadata_inspect` (hoy la primera devuelve `presentIn: 0`).
  - **Dado** cualquier `name` que devuelva `mode:"catalog"`, **Cuando** se le pasa a
    `mode:"field"` y a `knowledge_search{where}`, **Entonces** las tres coinciden en el campo →
    `el_catalogo_es_direccionable` (propiedad, sobre un fixture que incluye una clave `graph:` y una
    clave con punto).
  - **Dado** un `field` con un dot-path inválido (`"a..b"`, `"service."`), **Cuando** se llama,
    **Entonces** sigue siendo `INVALID_SCHEMA` → `dot_path_invalido_sigue_rechazandose` (control
    anti-vacuo).
  - **No regresión**: `crates/lodestar-core/tests/metadata.rs` y los tests de descubribilidad
    (`crates/lodestar-mcp/tests/descubribilidad.rs`) siguen verdes; los de E24-H08 en
    `crates/lodestar-core/tests/consulta.rs`, también.
- **Dependencias**: **E26-H08** (comparten el canal de mensaje y el mismo fichero de la fachada).
- **Pruebas**: `crates/lodestar-core/tests/metadata.rs` · `crates/lodestar-mcp/tests/descubribilidad.rs`
  (la propiedad de round-trip catálogo → consulta) · `crates/lodestar-mcp/tests/mcp.rs`.
- **Frontera (mcp.yml)**: **sí** — cambia la semántica del parámetro `field` y los valores de `name`
  del catálogo.
- **Delta de contrato**:
  ```yaml
  tools:
    - nombre: metadata_inspect
      params:
        - { nombre: field, tipo: string, requerido: false, semantica: "dot-path del campo a
              inspeccionar, normalizado por el MISMO `core::parse::build_field_path` que `where`/
              `filter`/`has` (E26-H09: hasta v0.4.0 usaba `FieldPath::parse`, un segundo dialecto).
              Reglas heredadas del lenguaje: «frontmatter.status» ≡ «status» (abreviatura);
              «frontmatter.graph.backlinks» ANCLA a la clave del usuario (E24-H08); una propiedad
              desconocida bajo namespace reservado es INVALID_SCHEMA (E24-H07); y un namespace
              reservado VÁLIDO («graph.backlinks», «document.path») también lo es, porque describe
              una propiedad calculada y no metadata — el mensaje remite a graph_query o al anclaje" }
      retorno: "… `name` del catálogo es DIRECCIONABLE (E26-H09): el texto que emite se puede pasar
        tal cual a mode «field» y a `where`, y resuelve al mismo campo; una clave del usuario que
        colisiona con un namespace reservado se rinde ANCLADA («frontmatter.graph.backlinks»)"
      errores: ["INVALID_SCHEMA (falta «mode»; mode desconocido; mode «field» sin «field»; dot-path
                 inválido; namespace reservado — con mensaje que dice cómo alcanzarlo)",
                "WorkspaceError"]
  ```

### E26-H10 — Ninguna respuesta viaja sin cota

- **Objetivo**: que ninguna tool pueda devolver una respuesta de tamaño proporcional al workspace.
- **Defecto (U5) y escenario que lo reproduce hoy**:
  - **`graph_query` no tiene ni default ni máximo**: `limit` es opcional y `None => total`
    (`crates/lodestar-app/src/lib.rs:1283-1285`). Un `operation:"components"` sobre una base grande
    sirve **el grafo completo** (`:1266-1271`, `graph_model` entero) en una sola respuesta. El
    `inputSchema` declara `minimum: 1` y **ningún** `maximum` (`contracts/mcp.yml:479`), así que ni el
    cliente puede protegerse.
  - **`metadata_inspect` no tiene paginación en ningún modo**: `catalog`
    (`crates/lodestar-core/src/metadata.rs:35-54`) emite un `FieldStats` por **cada** field path que
    aparece en algún documento —incluidos los mapas intermedios, como su propio rustdoc advierte
    (`metadata.rs:22-24`)—, y `inspect_field` (`metadata.rs:87`) emite un `ValueCount` por **cada**
    valor escalar distinto: en un campo de alta cardinalidad (un `id`, una fecha, un `owner`) eso es
    **N entradas para N documentos**. Es la única de las 10 tools sin `limit` ni `cursor`.
  El contraste es interno: `knowledge_search` (20/100) y `knowledge_check` (100/1000) llevan cota y
  cursor desde E10, y E24-H09 los hizo cumplir de verdad.
- **Referencias**: `ARCHITECTURE.md §19.6` (presupuesto de payload) · `contracts/mcp.yml:390-391`
  (el patrón `limit`/`cursor` de `knowledge_search`) · `crates/lodestar-app/src/lib.rs`
  (`graph_query:1158`, paginación en `:1278-1291`) · `crates/lodestar-core/src/metadata.rs` ·
  `crates/lodestar-mcp/src/tools.rs` (`limit_validado`, usado en `:298`, `:365`, `:392`) ·
  **E24-H09** (la validación de valores que estas cotas heredan).
- **Alcance**:
  - **`graph_query`**: `limit` gana **default 100** y **máximo 1000**, declarados en el `inputSchema`
    y verificados por `limit_validado` (`tools.rs:392`, hoy invocado con `u64::MAX`). El cursor ya
    existe y no cambia de esquema.
  - **`metadata_inspect`**: gana `limit` y `cursor` en **los dos** modos, con el mismo cursor-offset
    hex autosuficiente que el resto de la superficie. Cotas: **default 100, máximo 1000** en ambos
    (`fields` del catálogo y `values` de la inspección), por analogía con `knowledge_check`, que es la
    otra tool que enumera un catálogo. Los conteos agregados (`presentIn`/`missingIn`/`inferredTypes`)
    se computan sobre **todo** el workspace y **no** se ven afectados por la página: lo que se pagina
    es la lista, no la estadística.
  - La cota se aplica en la **fachada** (`lodestar-app`), no dentro de `core::metadata`: el core sigue
    siendo puro y devolviendo la verdad completa (invariantes #2 y #3), y quien la trunca es quien
    sirve el wire — igual que hace hoy `graph_query` (`lib.rs:1278-1291`).
  - Los valores por defecto y los máximos quedan fijados **por esta historia y por el contrato**:
    cambiarlos más adelante exige actualizar el delta.
- **Consecuencia declarada (material de nota de release)**: `graph_query` sin `limit` deja de
  devolver el grafo completo; devuelve 100 nodos y un `nextCursor`. Es un cambio observable para
  cualquier cliente que asumiera la respuesta entera, y hay que decirlo con la forma de recorrerla
  (el cursor).
- **Criterios de aceptación**:
  - **Dado** un workspace de ~1.000 documentos, **Cuando** se llama a `graph_query{operation:
    "components"}` **sin** `limit`, **Entonces** devuelve como mucho 100 nodos, `truncated: true` y
    un `nextCursor` → `graph_query_tiene_default` (hoy devuelve el grafo entero).
  - **Dado** `graph_query` con `limit: 5000`, **Cuando** se llama, **Entonces** falla con
    `INVALID_SCHEMA` por exceder el máximo declarado → `graph_query_respeta_su_maximo`.
  - **Dado** un campo de alta cardinalidad (un valor distinto por documento), **Cuando** se llama a
    `metadata_inspect{mode:"field"}` sin `limit`, **Entonces** `values` trae como mucho 100 entradas y
    un `nextCursor` → `metadata_inspect_field_pagina` (hoy trae N).
  - **Dado** un workspace con muchos field paths, **Cuando** se llama a `mode:"catalog"` sin `limit`,
    **Entonces** `fields` trae como mucho 100 entradas y un `nextCursor` →
    `metadata_inspect_catalog_pagina`.
  - **Dado** un recorrido completo por cursor en cualquiera de los tres casos, **Cuando** se
    concatenan las páginas, **Entonces** el resultado es **exactamente** el que devolvía v0.4.0 sin
    paginar, sin repeticiones ni huecos → `paginar_no_pierde_ni_duplica` (control anti-vacuo: la cota
    no puede consistir en tirar datos).
  - **Dado** un cursor obtenido en un proceso y usado en otro **fresco**, **Cuando** se reanuda,
    **Entonces** continúa idéntico → `el_cursor_es_autosuficiente` (mismo criterio que
    `knowledge_search`).
  - **Dado** los conteos agregados de `metadata_inspect`, **Cuando** se pide una página, **Entonces**
    `presentIn`/`missingIn` siguen refiriéndose a **todo** el workspace →
    `la_estadistica_no_se_pagina`.
- **Dependencias**: **E26-H09** (tocan la misma tool y el mismo tipo de retorno).
- **Pruebas**: `crates/lodestar-core/tests/metadata.rs` (la verdad completa del core) ·
  `crates/lodestar-mcp/tests/mcp.rs` (las cotas por el wire) ·
  `crates/lodestar-mcp/tests/escala_wire.rs` (el caso grande, ya montado en E24-H16).
- **Frontera (mcp.yml)**: **sí** — `graph_query.limit` gana default y máximo, y `metadata_inspect`
  gana dos parámetros y `nextCursor` en su retorno (cambia el `outputSchema`).
- **Delta de contrato**:
  ```yaml
  tools:
    - nombre: graph_query
      params:
        - { nombre: limit, tipo: integer, default: 100, minimo: 1, maximo: 1000,
            semantica: "trunca el nº de nodos de la página (orden total estable por id). E26-H10:
              hasta v0.4.0 NO tenía default ni máximo (`None => total`), así que `components` sobre
              una base grande servía el grafo COMPLETO en una respuesta" }
    - nombre: metadata_inspect
      params:
        - { nombre: limit, tipo: integer, default: 100, minimo: 1, maximo: 1000,
            semantica: "E26-H10 — trunca `fields` (mode catalog) o `values` (mode field). Era la
              única de las 10 tools sin cota: el catálogo emite un field path por cada clave y mapa
              intermedio, y `values` una entrada por valor distinto (N para N documentos en un campo
              de alta cardinalidad)" }
        - { nombre: cursor, tipo: string, requerido: false,
            semantica: "cursor opaco (offset hex, mismo esquema que knowledge_search/knowledge_check/
              graph_query) devuelto en nextCursor" }
      retorno: "… ambas variantes ganan `nextCursor` (E26-H10). Los agregados
        (`presentIn`/`missingIn`/`inferredTypes`) se computan sobre TODO el workspace: se pagina la
        lista, no la estadística"
      output_schema: "sí — schemars::schema_for!(MetadataInspection) (ACTUALIZADO en E26-H10)"
  ```

---

## Bloque C — El contrato vuelve a decir la verdad

### E26-H11 — El contrato describe el servidor que hay

- **Objetivo**: que `contracts/mcp.yml` deje de describir una versión anterior del servidor, y que
  `/contrato --check` vuelva a estar limpio.
- **Defecto (U6) y drift verificado**:
  - `contracts/mcp.yml:394` sigue declarando, en los `errores` de `knowledge_search`, *«un
    `where`/`filter` malformado → `WorkspaceError::Core` genérico; el mapeo fino a `INVALID_SCHEMA`
    es E20»*. Es el comportamiento **pre-E24-H10**: hoy `build_search_expression`
    (`crates/lodestar-app/src/lib.rs:2976-2983`) produce `WorkspaceError::InvalidSchema` con mensaje
    saneado. La cabecera del contrato **sí** documenta E24-H10 (`contracts/mcp.yml:256-266`), así que
    el fichero se contradice a sí mismo.
  - **E24-H07 declaró frontera y el contrato no se tocó**: el commit `fd52e4c` (E24-H07/H08,
    v0.4.0) cambió el conjunto de errores de `knowledge_search` y `change_plan` —una propiedad
    desconocida bajo namespace reservado pasó a ser `INVALID_SCHEMA`— sin una línea en
    `contracts/mcp.yml`. La cabecera termina en el bloque de E24 para v0.3.1.
  - Errores declarados como **prosa suelta** que E26-H07 sustituye por códigos: `«falta el parámetro
    «ref»»` (`:415`), `«falta el parámetro «mode»»` (`:430`), `«falta el parámetro «operation»»`
    (`:483`), `«falta el parámetro «proposedOperation.kind»»` (`:509`).
- **Referencias**: `contracts/README.md` (reglas del fichero) · `contracts/mcp.yml` (cabecera
  `:1-266`, tools `:342-…`, `codigos_sin_emisor:651`) · `.claude/` skill `/contrato` ·
  **E23-H13** (que retiró los números de línea del contrato por envejecer en silencio: la misma
  familia de defecto, un nivel más arriba).
- **Alcance**:
  - **Bloque nuevo de cabecera** para E24-H07/H08 (el que faltó) y para E25/E26, con el mismo estilo
    narrativo del resto: qué cambió, por qué, y qué comportamiento sustituye.
  - **Aplicar los deltas** que declararon E26-H07, E26-H08, E26-H09 y E26-H10, más el de **E25-H02**
    (`RECOVERY_FAILED` sale de `codigos_sin_emisor` y entra en los `errores` de
    `change_apply`/`change_revert`).
  - **Corregir el drift**: la línea `:394` y las cuatro entradas de prosa suelta.
  - **Pasada final de coherencia**: `/contrato --check` en verde — el contrato ↔ `tools::list()`/
    `tools::call()` ↔ `core::types`, sin discrepancias.
  - **Recopilar las consecuencias declaradas** de esta rama en un solo sitio del documento, como
    material para la nota de release: `graph_query` sin `ref` cambia de código (H07); un `TypeError`
    pasa de excluir a fallar (H08); `metadata_inspect` cambia de dialecto y el catálogo de nombres
    (H09); `graph_query` sin `limit` deja de devolver el grafo entero (H10); y, desde E25, la promesa
    de convergencia pasa a estar condicionada a que las copias verifiquen (E25-H02).
- **Fuera de alcance**: escribir el `CHANGELOG.md`, subir la versión y publicar. E26-H11 deja el
  material listo; la release se planifica aparte.
- **Criterios de aceptación** (esta historia es de coherencia documental: sus criterios son
  **checklist binario**, salvo el último):
  - `/contrato --check` no reporta ninguna discrepancia.
  - `grep -n "WorkspaceError::Core genérico" contracts/mcp.yml` no devuelve nada.
  - `grep -n "falta el parámetro" contracts/mcp.yml` no devuelve ninguna línea dentro de una lista
    `errores:` (la prosa suelta se sustituyó por `INVALID_SCHEMA` + explicación).
  - La cabecera del contrato contiene un bloque para **E24-H07/H08**, uno para **E25** y uno para
    **E26**, cada uno nombrando las tools afectadas.
  - `codigos_sin_emisor` tiene **cuatro** filas (perdió `RECOVERY_FAILED` en E25-H02) y su `nota` lo
    dice.
  - Cada tool cuya lista `errores` cambió en H07–H10 la tiene actualizada, y ningún código citado en
    el contrato está fuera de las 16 filas de `ErrorCode` →
    `los_codigos_del_contrato_estan_en_el_catalogo` (test: extrae los códigos del YAML y los coteja
    con `ErrorCode`; es el único criterio no-checklist, y existe porque una lista de errores escrita a
    mano es exactamente lo que envejece en silencio).
- **Dependencias**: **E26-H07**, **E26-H08**, **E26-H09**, **E26-H10** y **E25-H02** (su delta forma
  parte de esta pasada).
- **Pruebas**: `crates/lodestar-mcp/tests/descubribilidad.rs` (el test de coteja de códigos) ·
  la ejecución de `/contrato --check`.
- **Frontera (mcp.yml)**: **sí** — es la historia que la escribe.

---

## Orden de construcción

```
H07 ─→ H08 ─→ H09 ─→ H10 ─→ H11
                              ▲
                     E25-H02 ─┘   (su delta de contrato entra en la misma pasada)
```

**H07 va primero** porque abre el canal por el que las tres siguientes hablan: sin código+mensaje,
H08 no puede decir qué tipo chocó con cuál, H09 no puede explicar cómo alcanzar una clave anclada y
H10 no puede decir qué máximo se excedió. **H08** y **H09** tocan la misma fachada y el mismo
dialecto; **H10** cierra sobre las dos tools que H09 acaba de tocar; **H11** es la pasada final y
depende de las cuatro **y** de E25-H02.

**Relación con E25**: E26 se ejecuta **después** de E25 (misma rama), pero no depende
funcionalmente de ella salvo en H11, que recoge su delta de contrato. Si por cualquier razón se
paralelizan, el único punto de encuentro es `contracts/mcp.yml`.

Ninguna historia está **[BLOQUEADA]**. La única decisión abierta que roza esta épica —rechazar
parámetros **no** declarados, `decisiones/` (E24-H18)— está explícitamente fuera de alcance y
ninguna historia la presupone en ningún sentido.

## Proceso por historia

| Ciclo | Historias | Por qué |
|---|---|---|
| **Completo** (spec → roja → verde → juez ciego con *mutation testing*) | H08 · H09 · H10 | Cambian resultados observables de llamadas que hoy se aceptan |
| **Corto** (regresión en rojo → fix → verificación) | H07 · H11 | H07 es sistemática y mecánica sobre 8 brazos; H11 es coherencia documental verificada por `/contrato --check` |

**Nota de proceso**: H08, H09 y H10 cambian **respuestas**, no solo errores. Las tres declaran su
consecuencia y las tres necesitan que el juez ciego compruebe que el control anti-vacuo muerde — en
H10 en particular, que la cota no se implementó tirando datos (el criterio
`paginar_no_pierde_ni_duplica` es el que lo demuestra).

## Criterio de salida

Las diez tools devuelven código **y** mensaje; `graph_query` distingue «falta el parámetro» de «el
documento no existe»; un `TypeError` de evaluación se reporta en vez de recortar la respuesta en
silencio, igual en `knowledge_search` que en la selección masiva de `change_plan`;
`metadata_inspect` habla el mismo dialecto de dot-paths que la consulta y su catálogo devuelve
nombres direccionables; ninguna respuesta viaja sin cota y todo recorrido paginado reconstruye el
resultado completo; y `contracts/mcp.yml` describe el servidor que hay, con `/contrato --check` en
verde.
