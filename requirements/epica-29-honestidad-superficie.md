# E29 — Fase 1 de la campaña de bugfixes del testbench homelab: honestidad de superficie

> **Origen**: el orden de trabajo de [`decisiones/README.md`](../decisiones/README.md) (L110-132,
> revisado 2026-08-06), punto **1 — «Épica de honestidad de superficie»**, y las fichas que lo
> alimentan: [`§19(a)/(b)`](../decisiones/19-hallazgos-referencia-usuario.md),
> [`§18`](../decisiones/18-canapply-no-vincula-apply.md),
> [`§15`](../decisiones/15-parametros-no-declarados.md),
> [`§16(b)/(e)/(f)/(g)`](../decisiones/16-deuda-auditoria-e25-e26.md) y
> [`§23`](../decisiones/23-hallazgos-testbench-homelab.md) (D-01, A-04, A-07, y la mitad de A-08 que
> pide rechazo). Evidencia reproducible en
> [`docs/qa/informe-homelab-2026-08-06.md`](../docs/qa/informe-homelab-2026-08-06.md) (§2 D-01, §3
> A-04/A-07/A-08) y en el testbench re-ejecutable de `docs/qa/testbench/`. Es la **Fase 1** de
> [`docs/qa/campana-bugfixes-2026-08.md`](../docs/qa/campana-bugfixes-2026-08.md); la Fase 0 es
> [`E28`](epica-28-defectos-destructivos-testbench.md) (defectos destructivos), ya en curso.
>
> **Objetivo de la épica**: que todo lo que la superficie de Lodestar **afirma** —el contrato MCP, el
> `inputSchema` de cada tool, el texto `instructions`, `docs/user/`, `ARCHITECTURE.md §20.8`, el
> `config.yaml` que el usuario escribe— lo **ejecute el motor**, y que allí donde el motor no puede
> ejecutar lo prometido, lo diga en voz alta en vez de responder algo silenciosamente equivocado.
> No entra ningún defecto con riesgo de pérdida de datos (esos son E28) ni ninguna capacidad nueva:
> once puntos ya decididos, todos de la forma «la superficie promete X y el motor hace Y».
> Referencias maestras: `ARCHITECTURE.md §19.6` (superficie MCP y perfiles), `§20.8` (lenguaje de
> consulta), `§20.9` (validación y familias), `§20.5` (descubrimiento y config), `§21.5` (principio
> de honestidad de la superficie externa), `contracts/mcp.yml`, `CLAUDE.md` (invariantes).

**Principio rector**: el de `§21.5`, generalizado del exterior al wire — ***la superficie solo
promete lo que el motor ejecuta hoy***, con su corolario operativo, que es el principio rector de
E26: ***una respuesta silenciosamente equivocada es peor que un error***. Ante una duda de esta
épica —¿rechazar o tolerar?, ¿avisar o callar?, ¿código nuevo o reuso?— gana la opción que hace
**observable** la discrepancia; y cuando la discrepancia no se puede eliminar, se **retira la
promesa** en vez de dejarla escrita. Corolario de reparto que desempata el resto: si lo desconocido
llega por el wire y por disco, el criterio es **el mismo** (`§15`: «el repo no se queda con dos
criterios opuestos según si lo desconocido llega por el wire o por disco»).

**Fuera de alcance (explícito)**: esta épica **no** reabre ninguna de las fichas que ejecuta, y en
particular deja fuera:

- **M-01 y A-05** (`decisiones §23`, prioridades 5 y 4) — son **E28**, la fase 0 que va por delante.
- **§16(i)** coreografía de sellado, **§16(j)** cursor inválido, **§23/A-02/A-03** cursor ajeno y
  **§16(l)** pasada de mutantes — son el **ciclo de higiene** (fase 2 del orden de trabajo). §16(i)
  lo absorbe E28-H01 por compartir zona.
- **D-02** (`patch_frontmatter` null-vs-remove) y los nits documentales **A-01, A-06, A-09, A-10** —
  van con `§19` en la historia-escoba (fase 3), no aquí. **Excepción declarada**: la mitad
  *documental* de **A-08** (listar las familias de `validation` en una fuente de usuario) sí entra,
  porque su mitad *ejecutable* (rechazar la clave desconocida) es §16(e), y rechazar una clave sin
  publicar cuáles son las válidas sería cambiar un silencio por un muro.
- **§9** (banco de pruebas), **§14** (store sin consumidor), **§20** (renombrado), **§21**
  (comillas), **§22** (integridad referencial), **§16(h)** (escritores de runtime sin lock),
  **§16(k)** (matriz de trazabilidad) y **§10** (ghosts).
- Cualquier **capacidad nueva**: ni un parámetro nuevo, ni una tool nueva, ni sintaxis nueva de
  consulta. Las once historias solo cierran la distancia entre lo declarado y lo ejecutado.
- La **retirada de `additionalProperties: false`** de los `inputSchema` como alternativa a §15: la
  ficha ya descartó esa opción (b) y decidió la (a), ejecutar.

---

## Mapa de la épica

| ID | Origen | Título corto | Frontera | Modelo sugerido |
|---|---|---|---|---|
| E29-H01 | `§16(e)` + `§23/A-08` | Config estricta: claves desconocidas y config ilegible | no | opus |
| E29-H02 | `§19(b)` | `policy` parcial en `change_plan` respeta el `Default` | sí (nota) | sonnet |
| E29-H03 | `§19(a)` | `has(frontmatter)`/`missing(frontmatter)` responden la verdad | no | opus |
| E29-H04 | `§23/A-04` | `starts_with`/`ends_with` sobre no-string es type error | sí | opus |
| E29-H05 | `§23/A-07` | `knowledge_check` scope `paths` con path inexistente | sí | sonnet |
| E29-H06 | `§16(f)` | Workspace vacío: aviso en vez de silencio | sí | sonnet |
| E29-H07 | `§18` | `canApply: false` vincula a `change_apply` | sí | opus |
| E29-H08 | `§15` | El wire rechaza los parámetros no declarados | sí | opus |
| E29-H09 | `§23/D-01` | `instructions` por perfil y `protocolVersion` no soportada | sí | sonnet |
| E29-H10 | `§16(g)` | La API no transaccional de `Workspace` se cierra al exterior | no | sonnet |
| E29-H11 | `§16(b)` | Retirada de `Envelope`/`ErrorEnvelope` | no | sonnet |

---

## E29-H01 — Config estricta: una clave desconocida se rechaza y una config ilegible no degrada a *defaults*

- **Objetivo**: `.lodestar/config.yaml` deja de aceptar en silencio lo que no entiende. Una clave
  desconocida en cualquier sección —incluidas las **familias** de `validation`— es un error de
  arranque (`exit 3` en la CLI, fallo de `App::open` en el MCP), y un fichero de config que existe
  pero **no se puede leer** deja de caer a los valores por defecto sin decir nada. De paso, las
  familias válidas de `§20.9` dejan de existir solo en `config.rs` y se publican en la
  documentación de usuario.

- **Síntoma reproducible**: (1) `writable_roots`/`writeableRoots` (en vez de `writableRoots`) se
  ignora sin avisar y el workspace queda con la política por defecto —es decir, **más permisivo**
  que lo que el usuario cree haber configurado— (`decisiones §16(e)`); (2)
  `validation: { "LINK-TARGET-MISSING": ignore }` —usar un **código** de diagnóstico donde el motor
  espera una **familia**— se acepta y es **silenciosamente inerte**: el usuario cree haber silenciado
  un diagnóstico y sigue viéndolo (caso G1-04 del testbench, `§23/A-08`); (3) un `config.yaml`
  ilegible (permisos, directorio en su lugar, error de I/O) degrada a *defaults*, mientras
  `docs/user/ci.md` L295 afirma literalmente *«A malformed `config.yaml` is exit 3, never a silent
  fallback to defaults»*.

- **Causa raíz** (verificada en código): `crates/lodestar-workspace/src/config.rs` —
  - `WorkspaceConfig` y sus secciones llevan `#[serde(rename_all = "camelCase", default)]` pero
    **no** `deny_unknown_fields`, así que serde descarta toda clave que no reconozca;
  - `ValidationSection` es `#[serde(transparent)]` sobre un `BTreeMap<String, ValidationSeverity>`
    **abierto a propósito** (su doc-comment lo declara: *«las familias no son una lista cerrada»*),
    de modo que solo se valida la severidad, nunca el nombre de la familia;
  - `WorkspaceConfig::load` (L320-326) hace `match std::fs::read_to_string(&path) { Ok(text) => …,
    Err(_) => WorkspaceConfig::default() }`: **cualquier** error de lectura —no solo `NotFound`— cae
    al default.

- **Referencias**: `ARCHITECTURE.md §20.5` (config opcional, la raíz sale de `--root`/cwd) · `§20.9`
  (catálogo de familias de validación) · `decisiones §16(e)` (decidido: estricto) ·
  `decisiones §23/A-08` (mitad documental) · `crates/lodestar-workspace/src/config.rs`
  (`WorkspaceConfig`, `ValidationSection`, `load:320`, constantes `FAMILY_*:194-204`) ·
  `crates/lodestar-cli/src/commands.rs` (`check`, exit 3 vía `App::open`) ·
  `docs/user/ci.md` L262-300 · `docs/user/quickstart.md` L165-173 (ejemplo ya publicado del exit 3
  por config inválida) · `CLAUDE.md` invariante #1.

- **Alcance**:
  - `#[serde(deny_unknown_fields)]` en `WorkspaceConfig` y en **todas** sus secciones
    (`WorkspaceSection`, `DiscoverySection`, `GateSection`, `TransactionsSection`), compatible con el
    `default` que ya llevan. El mensaje resultante debe **nombrar la clave** rechazada (serde lo
    hace) y conservar el prefijo actual `«.lodestar/config.yaml inválido: …»`.
  - **Excepción declarada y documentada**: `workspace.root` **se sigue ignorando** sin error. Su
    doc-comment de `WorkspaceSection` ya declara que la clave se ignora a propósito (`E15-H08`,
    `§20.5`: es circular). Se mantiene como campo deserializado-y-descartado o vía `alias`/campo
    fantasma; lo que **no** se admite es que `deny_unknown_fields` lo convierta en error sin decidirlo,
    porque hay `config.yaml` reales que lo llevan.
  - **Familias de `validation` como lista cerrada**: la sección deja de aceptar cualquier `String`.
    Las válidas son las **cinco de `§20.9`**: `malformedFrontmatter`, `danglingDocumentLinks`,
    `missingWorkspaceFiles`, `caseMismatch` **e `isolatedDocuments`**. Esta última **no tiene
    productor** (el código `ORPHAN` murió en E16-H02 y su default `ignore` es un no-op, como
    documenta `family_of`), pero **se acepta igualmente**: `§20.9` la declara, y rechazar una familia
    que el contrato publica sería cambiar un silencio por una mentira nueva. La forma concreta (enum
    `ValidationFamily` con `Deserialize`, o validación explícita tras deserializar) la decide la fase
    roja; el criterio es que la lista viva en **un solo sitio** y que las constantes `FAMILY_*` no se
    dupliquen.
  - `WorkspaceConfig::load` distingue **ausente** de **ilegible**: solo `ErrorKind::NotFound` cae a
    `WorkspaceConfig::default()`; cualquier otro error de I/O es `Err(String)` con el mensaje del
    sistema y la ruta.
  - **Documentación de las familias** (mitad documental de A-08): la lista de las cinco familias, con
    su severidad por defecto y qué diagnósticos cubre cada una, se publica en `docs/user/ci.md`
    (sección «Tuning the gate», que ya menciona *«lower or raise the severity of individual
    diagnostic families»* sin decir cuáles son) — en **inglés**, por `§21.1`. Se declara explícitamente
    que las claves son **familias**, no códigos de diagnóstico, y que un código como
    `LINK-TARGET-MISSING` ahí es un error.

- **Fuera de alcance**: la **recarga** de la config en caliente (`§23/A-09`: se lee una vez por
  sesión) — es un nit documental de `§19`, no de esta historia; el contenido semántico de ninguna
  sección (no se añaden ni retiran claves válidas); `.lodestarignore`/`.gitignore`, que no son config.

- **Criterios de aceptación**:
  - **Dado** un `.lodestar/config.yaml` con `workspace: { writeableRoots: ["notas"] }` (typo),
    **Cuando** se ejecuta `lodestar check`, **Entonces** el exit code es `3` y el mensaje nombra la
    clave desconocida → **test: `config_con_clave_desconocida_es_exit_3`**.
  - **Dado** ese mismo fichero, **Cuando** arranca `lodestar-mcp`, **Entonces** el proceso falla al
    abrir el workspace (exit 3, mensaje por stderr) en vez de servir con la política por defecto →
    **test: `mcp_no_arranca_con_config_de_clave_desconocida`**.
  - **Dado** un `config.yaml` con `validation: { "LINK-TARGET-MISSING": ignore }` (un **código** en
    lugar de una familia), **Cuando** se abre el workspace, **Entonces** es error de config y el
    mensaje nombra las familias válidas → **test:
    `familia_de_validation_desconocida_es_error_de_config`**.
  - **Dado** un `config.yaml` con `validation: { isolatedDocuments: ignore }`, **Cuando** se abre el
    workspace, **Entonces** se acepta sin error (familia declarada por `§20.9`, aunque hoy sea un
    no-op) → **test: `familia_isolated_documents_sigue_siendo_valida`** (control anti-vacuo: el
    rechazo no puede cerrarse de más).
  - **Dado** un `config.yaml` con `workspace: { root: "otra" }`, **Cuando** se abre el workspace,
    **Entonces** se acepta y se ignora, exactamente como hoy → **test:
    `workspace_root_se_sigue_ignorando_sin_error`** (control anti-vacuo de la excepción declarada).
  - **Dado** un `.lodestar/config.yaml` que existe pero no se puede leer (p. ej. sustituido por un
    directorio), **Cuando** se ejecuta `lodestar check`, **Entonces** el exit code es `3` y el
    mensaje dice que la config no se pudo leer → **test: `config_ilegible_no_degrada_a_defaults`**.
  - **Dado** un workspace **sin** `.lodestar/config.yaml`, **Cuando** se ejecuta `lodestar check`,
    **Entonces** funciona con los defaults y sale `0`/`1` según los diagnósticos, igual que hoy →
    **test: `config_ausente_sigue_cayendo_a_defaults`** (control anti-vacuo: la ausencia es legítima
    y no puede convertirse en error).
  - **Dado** `docs/user/ci.md` tras la historia, **Cuando** se busca la lista de familias,
    **Entonces** están las cinco de `§20.9` con su severidad por defecto y la advertencia
    familia≠código → checklist estructural (revisión de diff + grep del criterio).

- **Dependencias**: ninguna. Es la **primera** de la épica por decisión explícita de
  `decisiones §16(e)` («historia propia y primera de la épica de honestidad»), y porque fija el
  criterio —lo desconocido se rechaza— que E29-H08 aplicará al wire.

- **Pruebas**: tests unitarios de `crates/lodestar-workspace/src/config.rs` (el módulo `#[cfg(test)]`
  ya tiene `severidad_desconocida_es_error`, que es el precedente exacto: se le añaden hermanos para
  clave desconocida, familia desconocida y familia `isolatedDocuments`) + `crates/lodestar-cli/tests/e2e.rs`
  (exit 3 real del binario con cada fixture de config) + `crates/lodestar-mcp/tests/mcp.rs` con
  `roundtrip()` (el servidor no arranca: el vector de respuestas sale vacío, patrón ya documentado en
  `roundtrip_en`). Fixtures: workspaces temporales con `.lodestar/config.yaml` escrito ad hoc — no
  hacen falta fixtures compartidos nuevos.

- **Frontera (mcp.yml)**: no. La config no es superficie MCP; el contrato solo la menciona como
  fuente de `INTERNAL_IO_ERROR`. **Sí** toca `docs/user/ci.md` (documentación de usuario, inglés).

- **Proceso**: ciclo **completo**. Es barata en código pero cambia el arranque de las dos fachadas y
  puede romper workspaces reales (el propio repo tiene config), así que el juez ciego debe verificar
  los controles anti-vacuo, que son la mitad delicada.

---

## E29-H02 — Una `policy` parcial en `change_plan` respeta el `Default` que el contrato promete

- **Objetivo**: `{"policy": {"requireValidResult": false}}` deja de ser `INVALID_SCHEMA` y toma el
  valor por defecto del campo omitido, que es lo que el `inputSchema` y `contracts/mcp.yml` llevan
  declarando desde E12-H04.

- **Síntoma reproducible** (`decisiones §19(b)`, hallado escribiendo `docs/user/safe-changes.md`):
  `change_plan` con `policy: {requireValidResult: false}` → `INVALID_SCHEMA: … missing field
  allowWarnings`. Omitir `policy` **entera** sí funciona. El `inputSchema` de la tool declara los dos
  campos con `default` (`tools.rs` L192-194: `requireValidResult` default `true`, `allowWarnings`
  default `true`) y `contracts/mcp.yml` L791 dice `{ requireValidResult?, allowWarnings? }` — los dos
  con interrogante, o sea opcionales.

- **Causa raíz**: `crates/lodestar-core/src/plan.rs:271` — `PlanPolicy` deriva `Deserialize` con
  `#[serde(rename_all = "camelCase")]` pero **sus campos no llevan `#[serde(default)]`**, pese a que
  el `impl Default for PlanPolicy` (L279-288) ya existe y define exactamente los valores que el
  schema publica. La fachada compensa a medias: `tools.rs` L473-478 usa `PlanPolicy::default()`
  cuando `policy` **falta entera**, pero delega en serde cuando está presente.

- **Referencias**: `ARCHITECTURE.md §19.6` (change_plan) · `contracts/mcp.yml` (`change_plan`, param
  `policy`) · `crates/lodestar-core/src/plan.rs` (`PlanPolicy:271`, `Default:279`, `can_apply:293`) ·
  `crates/lodestar-mcp/src/tools.rs` (L473-478) · `docs/user/safe-changes.md` (instruye enviar ambas
  claves — la instrucción deja de hacer falta) · `decisiones §19(b)`.

- **Alcance**:
  - `#[serde(default)]` en los dos campos de `PlanPolicy` (o `#[serde(default)]` a nivel de struct,
    que es equivalente aquí y menos ruidoso), de forma que el `Default` ya existente sea la única
    fuente de los valores omitidos — sin duplicar los literales `true`/`true` en un tercer sitio
    (invariante #4: una sola verdad de los defaults).
  - Retirar de `docs/user/safe-changes.md` la instrucción de enviar **siempre** ambas claves, que
    era la mitigación de este defecto.
  - Nota de contrato: `contracts/mcp.yml` ya declara los campos opcionales — el delta es de
    **redacción**, no de forma (ver abajo).

- **Fuera de alcance**: cualquier cambio en la **semántica** de `can_apply` (eso es E29-H07); añadir
  campos nuevos a `PlanPolicy`; el comportamiento de `policy` en la forma de selección masiva, que
  usa el mismo tipo y por tanto hereda el arreglo sin lógica propia.

- **Delta de contrato propuesto** (`contracts/mcp.yml`): ninguno en la forma (el param `policy` ya
  se declara `requerido: false` con los dos campos opcionales). **Sí** una línea en la cabecera de
  historial del fichero, junto a las de E24-H09/E26-H07, declarando que desde E29 una `policy`
  **parcial** se acepta y completa con los defaults declarados — porque hasta hoy el contrato decía
  la verdad y el motor no, y el registro de por qué cambió el comportamiento es lo que evita que la
  próxima auditoría lo redescubra.

- **Criterios de aceptación**:
  - **Dado** un workspace válido, **Cuando** se llama a `change_plan` con
    `policy: {"requireValidResult": false}` y sin `allowWarnings`, **Entonces** el plan se computa
    (sin `INVALID_SCHEMA`) y `canApply` se evalúa con `allowWarnings = true` → **test:
    `policy_parcial_toma_el_default_del_campo_omitido`**.
  - **Dado** el caso simétrico, **Cuando** se llama con `policy: {"allowWarnings": false}` sobre un
    workspace cuyo resultado simulado tiene warnings, **Entonces** `canApply` es `false` (el campo
    enviado se respeta) y `requireValidResult` vale `true` por defecto → **test:
    `policy_parcial_respeta_el_campo_enviado`**.
  - **Dado** `policy: {}` (objeto vacío), **Cuando** se planifica, **Entonces** equivale a omitir
    `policy` entera → **test: `policy_vacia_equivale_a_omitirla`**.
  - **Dado** `policy` con una clave desconocida, **Cuando** se planifica, **Entonces** el
    comportamiento es el que fije **E29-H08** (rechazo estricto) si esa historia ya está integrada;
    esta historia **no** lo decide → declarado como no-criterio, para que las dos no se pisen.
  - **Dado** `docs/user/safe-changes.md`, **Cuando** se busca la instrucción de enviar ambas claves,
    **Entonces** ya no está → checklist estructural (grep del criterio).

- **Dependencias**: ninguna. Es independiente de E29-H01 y de E29-H07 (que **usa** `canApply` pero no
  lo computa).

- **Pruebas**: `roundtrip()` de `crates/lodestar-mcp/tests/mcp.rs` (una llamada aislada por caso:
  no hay estado de sesión que mantener) + tests unitarios de `crates/lodestar-core/tests/core.rs`
  para el `Deserialize` de `PlanPolicy` (los tres casos: parcial en cada campo, vacío, completo).
  Fixtures: `lodestar-fixtures`, workspace mínimo con al menos un documento y —para el criterio de
  `allowWarnings`— un diagnóstico `warn` reproducible (un enlace a un fichero de proyecto
  inexistente, que `§20.9` clasifica como `missingWorkspaceFiles: warning`).

- **Frontera (mcp.yml)**: **sí, de redacción** (línea de historial; la forma declarada no cambia).

- **Proceso**: ciclo **acotado** (spec → roja → verde → juez ciego, sin panel): el arreglo es una
  anotación de serde sobre un `Default` que ya existe, y el riesgo está en los tests, no en el
  diseño.

---

## E29-H03 — `has(frontmatter)` y `missing(frontmatter)` responden la verdad

- **Objetivo**: el anclaje pelado deja de ser el único argumento con el que `has()`/`missing()`
  mienten. `has(frontmatter)` casa los documentos que **tienen** bloque de frontmatter y
  `missing(frontmatter)` los que no, exactamente como `document.has_frontmatter = true` ya responde
  hoy y como `ARCHITECTURE.md §20.8` promete por escrito.

- **Síntoma reproducible** (`decisiones §19(a)`, hallado escribiendo `docs/user/query-language.md`):
  sobre `examples/demo/` (10 documentos, 7 con frontmatter), `has(frontmatter)` devuelve **0 de 10**
  y `missing(frontmatter)` **10 de 10**, mientras `document.has_frontmatter = true` devuelve 7. Es
  decir: la función devuelve el valor **contrario** al correcto para todos los documentos, sin
  ningún error.

- **Causa raíz** (verificada en código): `crates/lodestar-core/src/eval.rs` —
  `propiedad_presente` (L~249-255) resuelve el argumento con `resolver_campo`, y `resolver_campo`
  (L144-151) detecta el anclaje y llama a `field.sin_anclaje()?`. `FieldPath::sin_anclaje`
  (`crates/lodestar-core/src/types.rs:507-512`) devuelve `None` para el anclaje **pelado**
  (`frontmatter` a secas), porque `FieldPath::from_segments` sobre cero segmentos es inválido — su
  doc-comment lo declara: *«o si el anclaje viene solo (`frontmatter` a secas, que no direcciona
  ninguna clave y por tanto no se puede desanclar)»*. El `?` propaga ese `None` a `resolver_campo`, y
  `propiedad_presente` lo lee como «propiedad ausente». `has` → `false` siempre; `missing` → `true`
  siempre.

- **Referencias**: `ARCHITECTURE.md §20.8` (L1247-1249: *«existencia `has(x)` `missing(x)` (incluido
  `has(frontmatter)`)»* — la promesa literal) · `crates/lodestar-core/src/eval.rs`
  (`resolver_campo:135`, `propiedad_presente:~240`, `eval_funcion:~223`) ·
  `crates/lodestar-core/src/types.rs` (`FieldPath::sin_anclaje:507`, `anclado:488`,
  `FRONTMATTER_ANCHOR`) · `docs/user/query-language.md` (documenta hoy el límite observado y remite a
  `document.has_frontmatter`) · `decisiones §19(a)` (decidido: arreglarlo, no retirarlo del contrato)
  · `CLAUDE.md` invariante #3 (una sola verdad computada).

- **Alcance**:
  - `has(frontmatter)` / `missing(frontmatter)` responden sobre la **presencia del bloque de
    frontmatter** del documento, con la **misma verdad** que `document.has_frontmatter` (invariante
    #3: no se computa dos veces; el camino corto debe derivar del mismo dato).
  - **Dónde se arregla**: en el camino de existencia (`propiedad_presente`/`resolver_campo`), **no**
    ampliando `FieldPath::sin_anclaje` para que devuelva un path vacío. Un `FieldPath` sin segmentos
    es inválido por construcción (`from_segments` lo rechaza) y ese invariante sostiene el resto del
    dialecto de dot-paths (E26-H09); relajarlo para este caso abriría una clase de path vacío en todo
    el motor. El caso «el argumento **es** el anclaje pelado» se reconoce antes de intentar
    desanclarlo.
  - **`frontmatter.x` sigue igual**: un anclaje con sufijo se resuelve contra la clave del usuario
    como hoy, incluido el caso en que la clave se llama literalmente `frontmatter`
    (`frontmatter.frontmatter`), que es material de `§21` y **no** se toca.
  - `docs/user/query-language.md` retira la nota del límite observado y la remisión a
    `document.has_frontmatter` como sustituto obligado (puede seguir mencionándose como equivalente).

- **Fuera de alcance**: el comportamiento de `has()`/`missing()` sobre **namespaces calculados**
  (`has(graph.backlinks)` es trivialmente cierto para todo documento, y así lo documenta
  `propiedad_presente`) — no cambia; las tres limitaciones de *quoting* de `§21` (clave con punto
  literal, clave llamada `frontmatter`, fusión de nombres), que necesitan sintaxis nueva y tienen
  puerta de diseño propia; cualquier cambio en `metadata_inspect`.

- **Criterios de aceptación**:
  - **Dado** un workspace con 3 documentos, 2 con bloque de frontmatter y 1 sin él, **Cuando** se
    consulta `has(frontmatter)`, **Entonces** devuelve exactamente los 2 con frontmatter → **test:
    `has_frontmatter_pelado_casa_los_documentos_con_frontmatter`**.
  - **Dado** ese mismo workspace, **Cuando** se consulta `missing(frontmatter)`, **Entonces** devuelve
    exactamente el 1 sin frontmatter → **test: `missing_frontmatter_pelado_casa_los_que_no_tienen`**.
  - **Dado** ese mismo workspace, **Cuando** se comparan los resultados de `has(frontmatter)` y de
    `document.has_frontmatter = true`, **Entonces** son **el mismo conjunto** → **test:
    `has_frontmatter_coincide_con_document_has_frontmatter`** (es el criterio de invariante #3, y el
    que hace la aserción robusta a cambios futuros en la definición de «tiene frontmatter»).
  - **Dado** un documento con un frontmatter **vacío** (`---\n---`), **Cuando** se consulta
    `has(frontmatter)`, **Entonces** el veredicto coincide con el de `document.has_frontmatter`
    sobre ese mismo documento, sea cual sea → **test: `has_frontmatter_vacio_coincide_con_el_camino_largo`**
    (caso límite: el bloque existe pero no tiene claves; la historia **no** inventa una respuesta
    propia, se ata a la única verdad existente).
  - **Dado** una consulta `has(frontmatter.status)` (anclaje **con** sufijo), **Cuando** se evalúa,
    **Entonces** responde igual que antes del arreglo → **test:
    `has_con_anclaje_y_sufijo_no_cambia`** (control anti-vacuo).
  - **Dado** `has(status)` y `has(graph.backlinks)`, **Cuando** se evalúan, **Entonces** responden
    igual que antes del arreglo → **test: `has_de_clave_y_de_namespace_no_cambia`** (control
    anti-vacuo del resto del operador).
  - **Dado** `docs/user/query-language.md`, **Cuando** se busca la nota del límite de
    `has(frontmatter)`, **Entonces** ya no describe el comportamiento roto → checklist estructural.

- **Dependencias**: ninguna. Toca `core::eval`, zona que ninguna otra historia de la épica modifica
  salvo E29-H04 (mismo fichero, funciones distintas): **paralelizables, con conflicto de merge
  probable pero no lógico** — si se ejecutan a la vez, conviene el mismo autor o secuenciarlas.

- **Pruebas**: `crates/lodestar-core/tests/core.rs` (semántica de consulta pura: es donde vive la red
  de seguridad del lenguaje desde E15-H04) + un caso e2e con `roundtrip()` de
  `crates/lodestar-mcp/tests/mcp.rs` sobre `knowledge_search` con `where: "has(frontmatter)"`, para
  que la evidencia rojo→verde sea observable **por el wire**, que es como lo vio el hallazgo.
  Fixtures: `lodestar-fixtures`, workspace con documentos con y sin frontmatter (y uno con bloque
  vacío para el caso límite).

- **Frontera (mcp.yml)**: no. El contrato ya promete `has(frontmatter)`; el cambio es que empiece a
  cumplirse. (Si `contracts/mcp.yml` describe el límite en algún punto de su cabecera, se retira esa
  nota en la misma pasada.)

- **Proceso**: ciclo **completo**. Toca el evaluador del lenguaje de consulta —la pieza con más
  superficie semántica del core— y su corrección es exactamente del tipo que un test mal escrito
  puede dar por bueno al revés (el defecto actual **es** la respuesta invertida).

---

## E29-H04 — `starts_with`/`ends_with` sobre un campo no-string es un type error ruidoso

- **Objetivo**: los dos operadores de afijo dejan de responder `false` en silencio cuando el campo no
  es string, y pasan a producir el mismo **type error visible** que E26-H08 estableció para las
  comparaciones de orden. Un `priority starts_with "3"` sobre un campo numérico deja de parecer «no
  hay ninguno» y pasa a decir que la comparación no está definida.

- **Síntoma reproducible** (caso G1-20 del testbench, `§23/A-04`): en el homelab, 7 documentos tienen
  `priority: 3` (número). `priority starts_with "3"` devuelve **0 resultados sin error**: una lista
  recortada indistinguible de una lista legítimamente vacía — exactamente la clase de fallo que
  E26-H08 declaró cerrada para el orden y que el principio rector de E26 llama *«una respuesta
  silenciosamente equivocada es peor que un error»*.

- **Causa raíz** (reconocida en el propio código): `crates/lodestar-core/src/eval.rs` L340-342, el
  doc-comment de `eval_afijo`: *«Con un campo no-string o un literal no-string no hay prefijo/sufijo
  que comprobar → `false` (no hay una variante de `TypeError` «no es string», y H01 no la introduce;
  ningún test lo ejercita)»*. La función devuelve `bool`, no `Result`, así que ni siquiera tiene por
  dónde señalar el error.

- **Referencias**: `ARCHITECTURE.md §20.8` (*«sin coerción implícita»*; el lenguaje es tipado) ·
  `crates/lodestar-core/src/eval.rs` (`eval_afijo:343`, `eval_orden:283` como patrón a imitar,
  `eval_contains:317`) · `crates/lodestar-core/src/types.rs` (`TypeError:1864` con sus dos variantes
  `OrderNotDefined`/`NotAList`, `ValueType`, `ComparisonOperator`) · E26-H08 (el precedente: el type
  error del orden aborta la selección masiva en vez de saltarse el documento) · `decisiones §23/A-04`
  (**criterio ya ratificado por el usuario el 2026-08-06: type error ruidoso**) ·
  `docs/user/query-language.md`.

- **Alcance**:
  - **Tercera variante de `TypeError`** en `lodestar-core::types` para «el operador exige un string y
    el campo (o el literal) no lo es». Nombre propuesto: `NotAString { field, operator, found }`,
    con la **misma forma** que `NotAList` —los dos son «el campo no es del tipo que el operador
    exige»— para que el wire de los type errors sea uniforme. La forma exacta y si el literal
    no-string merece distinción propia lo fija la fase roja; el criterio es la simetría con las dos
    variantes existentes.
  - `eval_afijo` pasa a devolver `Result<bool, TypeError>`, como `eval_orden`/`eval_contains`, y su
    llamante en `eval_comparacion` propaga.
  - **Campo ausente sigue siendo `false`**, no error: la ausencia se cortocircuita antes, en
    `eval_comparacion` (L108-110), y ese contrato no cambia — es el mismo que rige para `NotAList`
    (*«un campo inexistente no llega aquí»*).
  - **Literal no-string**: `status starts_with 3` (literal numérico) también es type error. Hoy el
    parser puede o no permitirlo; si el parser ya lo rechaza antes, se declara y se fija con un test,
    no se duplica la validación.
  - Propagación coherente por las dos superficies que evalúan consultas: `knowledge_search`
    (`INVALID_SCHEMA` con el diagnóstico entero, camino de E26-H07) y la **selección masiva** de
    `change_plan`, donde un type error **aborta el plan** (E26-H08) en vez de reducir el conjunto en
    silencio — no hay lógica nueva, pero sí un criterio de aceptación que lo fija.
  - `docs/user/query-language.md` documenta el nuevo type error junto a los ya descritos.

- **Fuera de alcance**: cambiar el comportamiento de `contains` sobre string (subcadena) o de
  `=`/`!=` con cruce de tipos (que es `false` por contrato explícito, no error); introducir coerción
  de ningún tipo (`§20.8` la prohíbe); tocar `eval_orden` o `eval_contains_lista`.

- **Delta de contrato propuesto** (`contracts/mcp.yml`):
  - La sección que enumera los **type errors** del lenguaje de consulta gana el tercer caso: un
    operador de texto (`starts_with`/`ends_with`) sobre un campo o literal no-string es type error,
    surface como `INVALID_SCHEMA` con el diagnóstico del core, igual que los otros dos.
  - En la `semantica` de `change_plan`, la frase de E26-H08 sobre el type error que **aborta** el
    plan aplica ya a los tres casos; se ajusta la redacción si nombra solo el orden.
  - No hay código de error nuevo: `INVALID_SCHEMA` ya es el envoltorio de todo type error de
    consulta (E24-H10/E26-H07). El catálogo de `ErrorCode` **no** se abre (sigue en 17 tras E28-H02).

- **Criterios de aceptación**:
  - **Dado** un workspace con documentos cuyo `priority` es un **número**, **Cuando** se busca con
    `where: "priority starts_with \"3\""`, **Entonces** la respuesta es un error `INVALID_SCHEMA`
    cuyo mensaje nombra el campo, el operador y el tipo encontrado → **test:
    `starts_with_sobre_numero_es_type_error`**.
  - **Dado** ese mismo workspace, **Cuando** se busca con `ends_with` sobre el mismo campo,
    **Entonces** el mismo error → **test: `ends_with_sobre_numero_es_type_error`**.
  - **Dado** un documento cuyo campo `tags` es una **lista**, **Cuando** se evalúa
    `tags starts_with "x"`, **Entonces** es type error (no `false`) → **test:
    `starts_with_sobre_lista_es_type_error`**.
  - **Dado** un workspace donde **ningún** documento tiene el campo `inexistente`, **Cuando** se
    evalúa `inexistente starts_with "x"`, **Entonces** devuelve 0 resultados **sin error** (la
    ausencia no es type error) → **test: `starts_with_sobre_campo_ausente_sigue_siendo_false`**
    (control anti-vacuo y frontera semántica).
  - **Dado** un workspace con campos string, **Cuando** se evalúa `status starts_with "act"`,
    **Entonces** casa como siempre → **test: `starts_with_sobre_string_sigue_funcionando`** (control
    anti-vacuo).
  - **Dado** un `change_plan` con `selection.where` que produce el type error, **Cuando** se
    planifica, **Entonces** el plan **aborta** con `INVALID_SCHEMA` y no se expande a ninguna
    operación → **test: `selection_con_type_error_de_afijo_aborta_el_plan`** (coherencia con
    E26-H08).
  - **Dado** el catálogo de `TypeError`, **Cuando** se cuenta tras el arreglo, **Entonces** tiene 3
    variantes y el test que lo fije está actualizado a conciencia → revisión de diff.

- **Dependencias**: ninguna técnica. **Comparte fichero** con E29-H03 (`core/src/eval.rs`, funciones
  distintas) — ver la nota de paralelización de aquella.

- **Pruebas**: `crates/lodestar-core/tests/core.rs` (evaluación pura, cada caso de tipo) +
  `roundtrip()` de `crates/lodestar-mcp/tests/mcp.rs` (el error por el wire, con código y mensaje:
  es como se observó el hallazgo) + un caso de `change_plan` con selección masiva en el mismo arnés.
  Fixtures: `lodestar-fixtures` con documentos de frontmatter heterogéneo (número, lista, string en
  el mismo nombre de campo), que es justo el escenario del homelab.

- **Frontera (mcp.yml)**: **sí** (enumeración de type errors + redacción de `change_plan`). Sin
  cambio en el catálogo de `ErrorCode`.

- **Proceso**: ciclo **completo** con **panel** de jueces (toca `contracts/mcp.yml` y cambia el
  resultado de consultas que hoy devuelven `false`: es un cambio de comportamiento observable para
  cualquier cliente existente).

---

## E29-H05 — `knowledge_check` con scope `paths` y un path inexistente responde `DOCUMENT_NOT_FOUND`

- **Objetivo**: el scope `paths` deja de tragarse los paths que no existen. Un typo en la lista deja
  de desaparecer en silencio y produce el mismo `DOCUMENT_NOT_FOUND` que ya producen los scopes
  `document` y `affected`.

- **Síntoma reproducible** (caso G1-23 del testbench, `§23/A-07`):
  `knowledge_check(scope: {kind: "paths", paths: ["no-existe.md"]})` devuelve **0 diagnósticos y
  ningún error** — indistinguible de «ese documento está impecable». Con una lista mixta
  (`["real.md", "typo.md"]`) el resultado es el de `real.md` a secas: el agente cree haber auditado
  dos documentos y auditó uno.

- **Causa raíz** (verificada en código): `crates/lodestar-app/src/lib.rs`, `App::scope_paths`
  (L1232-1257). Los brazos `Document` y `Affected` resuelven cada `DocumentRef` con
  `self.resolve_ref(…)?`, que es quien produce `DOCUMENT_NOT_FOUND`; el brazo `Paths` (L1244) es
  literalmente `Ok(paths.iter().cloned().collect())` — mete los `RelPath` en el conjunto sin
  comprobar que existan. Después, el bucle de `knowledge_check` itera `analysis.documents` y filtra
  por `allowed`, así que un path que no está en el inventario simplemente no aporta nada.

- **Referencias**: `ARCHITECTURE.md §19.6` (`knowledge_check` y sus scopes) · `contracts/mcp.yml`
  (`knowledge_check`, param `scope` y su lista `errores:`) · `crates/lodestar-app/src/lib.rs`
  (`scope_paths:1232`, `resolve_ref`, `knowledge_check:1110`) · `decisiones §23/A-07` (**criterio ya
  ratificado por el usuario: `DOCUMENT_NOT_FOUND`**) · `decisiones §22` (principio anti-typo, la
  razón de fondo) · `CLAUDE.md` invariante #1.

- **Alcance**:
  - El brazo `CheckScope::Paths` valida cada path contra el inventario (`analysis.documents` /
    `doc_set.files()`) y devuelve `AppError` con `ErrorCode::DocumentNotFound` en el **primero** que
    no resuelva, nombrándolo — mismo patrón, mismo código y mismo estilo de mensaje que
    `resolve_ref`. Si es posible sin duplicar lógica, se reusa `resolve_ref` (invariante #3); si su
    firma exige un `DocumentRef`, se extrae el predicado de existencia, no se copia.
  - **Orden determinista**: con varios paths inexistentes se reporta el **primero según el orden de
    la lista recibida**, no el orden del `BTreeSet` — para que el mensaje sea reproducible y apunte a
    lo que el agente escribió primero.
  - Un path que existe en disco pero **no está en el inventario** (excluido por `.gitignore`/
    `.lodestarignore`/`discovery`) cuenta como **no encontrado**: el inventario es la verdad de qué
    documentos hay (`§20.5`/`§20.6`), y es el mismo criterio que ya aplica `resolve_ref`. Se fija con
    test para que no quede a interpretación.

- **Fuera de alcance**: los demás scopes (`workspace`/`document`/`affected`), que ya se comportan
  así; la tolerancia de `paths` **vacío** (`paths: []`), que sigue siendo un scope legítimamente
  vacío y no un error; cualquier otra tool que acepte listas de paths.

- **Delta de contrato propuesto** (`contracts/mcp.yml`):
  - En la fila `knowledge_check`, la lista `errores:` gana —o extiende— la entrada de
    `DOCUMENT_NOT_FOUND` para declarar que **también** la produce el scope `paths` con un path que no
    resuelve, no solo `document`/`affected`.
  - En la `semantica` de `scope`, se declara que `paths` exige que **todos** los paths existan en el
    inventario, y que un path excluido por el descubrimiento cuenta como inexistente.
  - Sin código nuevo: `DOCUMENT_NOT_FOUND` ya está en el catálogo.

- **Criterios de aceptación**:
  - **Dado** un workspace con `notas/alfa.md`, **Cuando** se llama a `knowledge_check` con
    `scope: {kind: "paths", paths: ["notas/no-existe.md"]}`, **Entonces** la respuesta es un error
    `DOCUMENT_NOT_FOUND` que nombra el path → **test: `check_scope_paths_con_path_inexistente_falla`**.
  - **Dado** ese mismo workspace, **Cuando** el scope mezcla un path real y uno inexistente,
    **Entonces** también falla (no devuelve el informe parcial del real) → **test:
    `check_scope_paths_falla_aunque_haya_paths_validos`**.
  - **Dado** un scope con **dos** paths inexistentes, **Cuando** se llama, **Entonces** el mensaje
    nombra el **primero de la lista recibida** → **test:
    `check_scope_paths_reporta_el_primer_path_inexistente`**.
  - **Dado** un documento excluido por `.lodestarignore`, **Cuando** se pide en `scope.paths`,
    **Entonces** es `DOCUMENT_NOT_FOUND` (no está en el inventario) → **test:
    `check_scope_paths_trata_lo_excluido_como_inexistente`**.
  - **Dado** un scope con paths que **todos** existen, **Cuando** se llama, **Entonces** devuelve los
    diagnósticos de esos documentos exactamente como hoy → **test:
    `check_scope_paths_valido_sigue_funcionando`** (control anti-vacuo).
  - **Dado** `scope: {kind: "paths", paths: []}`, **Cuando** se llama, **Entonces** devuelve un
    informe vacío sin error, como hoy → **test: `check_scope_paths_vacio_no_es_error`** (control
    anti-vacuo del borde).

- **Dependencias**: ninguna.

- **Pruebas**: `roundtrip()` de `crates/lodestar-mcp/tests/mcp.rs` (llamadas aisladas; el hallazgo se
  observó así) + tests de `crates/lodestar-app/tests/` para `scope_paths` si el arnés lo permite sin
  levantar proceso. Fixtures: `lodestar-fixtures` con al menos un documento válido y un
  `.lodestarignore` que excluya otro, para el criterio del excluido.

- **Frontera (mcp.yml)**: **sí** (lista `errores:` y semántica de `scope` en `knowledge_check`).

- **Proceso**: ciclo **acotado**. El criterio ya está ratificado y el cambio es un brazo de `match`;
  la parte delicada son los dos controles anti-vacuo, que el juez ciego debe verificar.

---

## E29-H06 — Un workspace vacío se distingue de un directorio equivocado

- **Objetivo**: `cd` a un directorio que no era el que se creía deja de responder «todo en orden».
  Un workspace con **0 documentos descubiertos** produce un diagnóstico de nivel `warn` que lo dice,
  visible por las dos fachadas (`lodestar check` y `knowledge_check`), **sin cambiar ningún exit
  code**: un repo legítimamente vacío sigue pasando la puerta de CI.

- **Síntoma reproducible** (`decisiones §16(f)`): un directorio sin `.md` —o donde la
  `DiscoveryPolicy` excluye todo— da `workspace_status` con `counts.documents: 0` y `lodestar check`
  con **exit 0 · VÁLIDO**. La respuesta es literalmente correcta (no hay nada mal) y prácticamente
  engañosa: es el «respondió que sí a algo que no entendió» del principio rector de E26, en la
  **puerta de entrada** del producto — el primer minuto de cualquier usuario nuevo.

- **Causa raíz**: no hay defecto de implementación, hay una **ausencia**: ningún productor emite
  diagnóstico por inventario vacío. `App::full_analysis`
  (`crates/lodestar-app/src/lib.rs:1286-1316`) compone el análisis del core + la política de
  severidad + los diagnósticos de descubrimiento, y ninguno de los tres puede observar «el conjunto
  quedó vacío». `commands::check` (`crates/lodestar-cli/src/commands.rs:26`) computa `valid` como
  «ningún `Err`» sobre un mapa vacío → `true` → exit 0 → «VÁLIDO».

- **Referencias**: `ARCHITECTURE.md §20.1` (arranque sin ceremonia: cualquier directorio es un
  workspace válido — la decisión que **no** se relitiga) · `§20.5` (descubrimiento y su suelo duro) ·
  `§20.9` (catálogo de diagnósticos y familias) · `crates/lodestar-app/src/lib.rs`
  (`full_analysis:1286`, `knowledge_check:1110`, `workspace_status:550`) ·
  `crates/lodestar-workspace/src/discovery.rs` (productores de los diagnósticos de descubrimiento,
  patrón a imitar) · `crates/lodestar-core/src/types.rs` (`CheckCode:191`) ·
  `decisiones §16(f)` (decidido: **avisar sin cambiar el exit code**).

- **Alcance**:
  - **`CheckCode` nuevo** para el inventario vacío. Nombre propuesto: `WORKSPACE-EMPTY`, en la
    familia de **descubrimiento** de `§20.9` (junto a `DOC-NOT-UTF8`/`SYMLINK-UNSUPPORTED`/…), que es
    donde encaja: describe lo que Lodestar **no pudo incorporar al inventario**, aquí por no haber
    nada. Severidad `Warn`, intrínseca (no configurable por familia; `family_of` devuelve `None`,
    como para `DOC-NOT-UTF8` y compañía). El mensaje debe ser accionable: nombrar la **raíz** sobre
    la que se descubrió y apuntar a la causa más probable (directorio equivocado o `discovery`
    demasiado restrictivo).
  - **Un solo productor**: se emite donde se emiten los demás diagnósticos de descubrimiento —el
    camino de `document_set_with_discovery`— para que **las dos fachadas lo vean por el mismo canal**
    (invariante #3). No se sintetiza por separado en la CLI y en el MCP.
  - **Visible en `knowledge_check` scope `workspace`** (que es donde entran los de descubrimiento) y
    en la salida de `lodestar check` (`--json`, `--sarif` y humano, vía `full_analysis`).
  - **Sin `target`**: como `PATH-NOT-UTF8`, no describe un fichero. Eso tiene una consecuencia
    conocida y hay que decidirla con test: `full_analysis` indexa los diagnósticos de descubrimiento
    **por su primer `target`** y **descarta los que no lo tienen** (`lib.rs` L1309-1312, límite ya
    documentado). Si el diagnóstico se emite sin target, `lodestar check` no lo mostraría — que es
    justo la fachada donde más importa. **Resolución propuesta**: anclarlo a la **raíz** como target
    si existe un `RelPath` válido para ella, o extender el indexado de `full_analysis` para los
    diagnósticos sin target; la fase roja elige, y el criterio de aceptación exige que **se vea en
    las dos fachadas**.
  - **Exit codes intactos**: `check` sigue saliendo `0` sobre un workspace vacío sin otros
    diagnósticos. Solo cambia lo que **imprime**.

- **Fuera de alcance**: cualquier cambio en la política de descubrimiento; cualquier gate nuevo al
  abrir (`§20.1` es explícito: cualquier directorio es workspace válido — **no** se reintroduce el
  gate que E15-H06 retiró); `workspace_status`, que ya publica `counts.documents: 0` de forma
  honesta y no necesita campo nuevo; hacer el aviso configurable por `validation`.

- **Delta de contrato propuesto** (`contracts/mcp.yml`):
  - El **catálogo de `Check.code`** de la cabecera del contrato gana la fila `WORKSPACE-EMPTY`
    (severidad `warn`, productor: descubrimiento, significado: el inventario quedó vacío bajo esta
    raíz).
  - En la `semantica` de `knowledge_check`, la enumeración de diagnósticos de descubrimiento que
    añade el scope `workspace` incluye el nuevo código.
  - Sin cambio en `ErrorCode` (no es un error de operación) ni en ningún `outputSchema` (el `Check`
    ya viaja con su `code`).

- **Criterios de aceptación**:
  - **Dado** un directorio temporal **sin ningún `.md`**, **Cuando** se ejecuta `lodestar check`,
    **Entonces** el exit code sigue siendo `0` y la salida incluye un aviso `WORKSPACE-EMPTY` que
    nombra la raíz → **test: `check_en_workspace_vacio_avisa_con_exit_0`**.
  - **Dado** ese mismo directorio, **Cuando** se llama a `knowledge_check` scope `workspace`,
    **Entonces** el informe incluye el diagnóstico `WORKSPACE-EMPTY` con severidad `warn` y
    `valid` sigue siendo `true` → **test: `knowledge_check_en_workspace_vacio_avisa`**.
  - **Dado** un directorio **con** `.md` pero cuya `discovery.include` los excluye todos, **Cuando**
    se audita, **Entonces** también avisa (el caso engañoso no es solo «no hay ficheros», es «no hay
    inventario») → **test: `workspace_con_todo_excluido_tambien_avisa`**.
  - **Dado** un workspace con **al menos un** documento, **Cuando** se audita, **Entonces** no
    aparece `WORKSPACE-EMPTY` por ninguna de las dos fachadas → **test:
    `workspace_con_documentos_no_avisa`** (control anti-vacuo).
  - **Dado** el mismo workspace vacío, **Cuando** se compara la salida de `lodestar check --json`
    con la de `knowledge_check` scope `workspace`, **Entonces** ambas contienen el diagnóstico (una
    sola verdad computada) → **test: `el_aviso_de_vacio_lo_ven_las_dos_fachadas`**.
  - **Dado** un workspace vacío con `gate.blockWarnings: true`, **Cuando** se ejecuta `check`,
    **Entonces** el exit code es `1` **por la política del usuario**, no por el aviso en sí →
    **test: `el_aviso_de_vacio_respeta_block_warnings`** (declara la interacción, que si no quedaría
    a la interpretación de quien la descubra).

- **Dependencias**: ninguna. **Nota de coordinación**: E29-H01 también toca la config y E29-H09 la
  superficie de arranque, pero en ficheros distintos.

- **Pruebas**: `crates/lodestar-cli/tests/e2e.rs` (exit code + salida humana y `--json` del binario
  real sobre un `tempdir` vacío) + `roundtrip()` de `crates/lodestar-mcp/tests/mcp.rs`
  (`knowledge_check` sobre el mismo directorio) + `crates/lodestar-core/tests/core.rs` si el código
  nuevo exige fijar el catálogo de `CheckCode`. Fixtures: ninguno compartido — un `tempdir` vacío y
  otro con `discovery.include` restrictivo.

- **Frontera (mcp.yml)**: **sí** (catálogo de `Check.code` + semántica de `knowledge_check`).

- **Proceso**: ciclo **acotado**, con una **puerta de decisión explícita en la fase roja**: dónde se
  ancla el diagnóstico sin `target` para que `lodestar check` lo muestre. Si esa decisión obliga a
  tocar el indexado de `full_analysis`, el juez ciego debe verificar que ningún otro diagnóstico
  cambia de sitio.

---

## E29-H07 — `canApply: false` vincula a `change_apply`

- **Objetivo**: el veredicto que `change_plan` publica deja de ser un consejo que nadie ejerce.
  `change_apply` de un plan cuyo `canApply` era `false` bajo su propia policy se **rechaza**, con un
  código del catálogo y un mensaje que dice por qué, y **sin escribir nada**.

- **Síntoma reproducible** (`decisiones §18`, hallado ejecutando el guion de la demo contra un
  workspace con un error preexistente deliberado): `change_plan` bajo la policy por defecto
  (`requireValidResult: true`) devuelve `canApply: false` cuando el resultado simulado no es válido;
  `change_apply` del mismo `changeSetId` responde `applied: true` con `validation.valid: false` y
  **sin diagnósticos nuevos**. La superficie dice «este plan no es aplicable bajo tu policy» y el
  motor lo aplica igual si el cliente insiste.

- **Causa raíz** (verificada en código): `can_apply` se computa en `App::change_plan`
  (`crates/lodestar-app/src/lib.rs:1783`, con `core::plan::can_apply`) y **viaja al cliente**, pero
  el camino de apply no lo consulta: `App::change_apply` recupera el plan persistido, re-verifica
  `planHash`/revisión y delega en `Workspace::apply_transaction`, cuyo único filtro de validez es el
  **gate de staging** (`rejectNewErrors`/`allowExistingErrors` de `transactions`, E20-H04) — una
  política **distinta** de la `PlanPolicy` con la que se computó `canApply`. Dos políticas, una
  publicada y otra ejercida.

- **Referencias**: `ARCHITECTURE.md §19.4/§19.5` (modelo transaccional) · `contracts/mcp.yml`
  (`change_plan`: *«`canApply` = can_apply(report, policy)»*; `change_apply`: lista `errores:` y su
  larga `semantica`) · `crates/lodestar-app/src/lib.rs` (`change_plan:1683`, `can_apply` en L1783,
  `change_apply`) · `crates/lodestar-core/src/plan.rs` (`PlanPolicy:271`, `can_apply:293`) ·
  `docs/user/safe-changes.md` (describe hoy `canApply` como veredicto del **plan**, sin prometer que
  apply lo re-ejerza: **esa redacción se revisa aquí**) · `decisiones §18` (**decidido: (a)
  vinculante**, con delta de contrato y la carga de la prueba puesta en el código nuevo).

- **Alcance**:
  - El **plan persistido** debe conservar lo necesario para re-ejercer el veredicto en el apply. La
    fase roja decide entre (i) persistir el `canApply` computado, o (ii) persistir la `PlanPolicy` y
    **recomputar** `can_apply` sobre la validación del plan. **Recomendación: (ii)**, porque el apply
    ya re-verifica todo lo demás (el `planHash` sobre la base actual, la revisión bajo el lock) y un
    booleano congelado sería el único veredicto del apply que no se re-computa; además deja el
    camino abierto a que el rechazo diga **qué** cláusula de la policy lo bloquea. Lo que **no** se
    admite es reimplementar el predicado: se llama a `core::plan::can_apply` (invariante #3).
  - El rechazo ocurre **antes** de tomar el lock y de tocar staging: es un veredicto sobre el plan,
    no sobre el disco. Es decir, antes del punto donde `WRITE_CONFLICT`/`PERMISSION_DENIED` pueden
    aparecer.
  - **Código de error**: `decisiones §18` deja abierto si usar uno existente o abrir el
    decimoctavo. **Recomendación: reusar `INVALID_RESULT`**, que el contrato ya define como *«el
    resultado del plan no es aceptable»* (hoy: gate diferencial de E20-H04) — es el mismo predicado,
    «el resultado simulado no supera la política vigente», solo que evaluado con la `PlanPolicy` del
    plan en vez de con la de `transactions`. Un código nuevo obligaría al agente a distinguir dos
    formas de «tu plan produce un resultado que no acepto», que no tienen recuperación distinta (en
    ambos casos: replanificar, o relajar la policy). **Carga de la prueba** (`§18`: el catálogo solo
    se ha abierto una vez): si la fase roja concluye que el mensaje de `INVALID_RESULT` no puede
    distinguir los dos orígenes con claridad, se abre `PLAN_NOT_APPLICABLE` como decimoctavo y se
    declara en el mismo delta — pero la opción por defecto es no abrirlo, y el mensaje debe **nombrar
    la cláusula** (`requireValidResult`/`allowWarnings`) que bloqueó.
  - `docs/user/safe-changes.md` deja de describir `canApply` como advisory y declara que apply lo
    ejerce.
  - **Mitigación de E27 revisada**: el guion de la demo usa hoy `policy: {requireValidResult: false}`
    precisamente porque el workspace de la demo tiene un error deliberado. Con `canApply` vinculante
    esa policy explícita **sigue siendo correcta y necesaria**; hay que verificar que el smoke de la
    demo (`scripts/demo-smoke.sh`, job `demo-smoke` del CI) sigue en verde y, si no, ajustar el
    guion, no el motor.

- **Fuera de alcance**: la política de staging (`rejectNewErrors`/`allowExistingErrors`), que se
  conserva tal cual como segunda línea; el valor por defecto de `PlanPolicy` (E29-H02 lo toca desde
  la deserialización, no desde la semántica); `change_revert`, que no tiene policy.

- **Delta de contrato propuesto** (`contracts/mcp.yml`):
  - `change_apply`, lista `errores:`: se declara que un plan cuyo `canApply` era `false` bajo su
    propia `policy` se **rechaza** —con `INVALID_RESULT` (recomendado) o con el código nuevo que la
    fase roja justifique—, **antes del lock** y sin escribir nada.
  - `change_apply`, `semantica`: el orden de pasos gana el nuevo gate al principio (tras cargar el
    plan y antes de `expectedWorkspaceRevision`/re-verificación del hash, o donde la implementación
    lo sitúe, pero declarado explícitamente).
  - `change_plan`, `semantica`: la frase de `canApply` deja de ser descriptiva del plan y declara que
    **vincula** al apply. Es el cambio que cierra `§18`.
  - Si se abre código nuevo: fila en el catálogo de `ErrorCode` (17→18) y actualización del test
    `catalogo_de_errores_tiene_diecisiete_filas` de `crates/lodestar-core/tests/core.rs`.

- **Criterios de aceptación**:
  - **Dado** un workspace con un error preexistente y un `change_plan` con la policy por defecto que
    devuelve `canApply: false`, **Cuando** se llama a `change_apply` con ese `changeSetId`,
    **Entonces** la llamada falla con el código declarado, el mensaje nombra la cláusula de la policy
    que bloqueó, y **el disco queda byte-idéntico** → **test:
    `apply_de_plan_no_aplicable_se_rechaza_sin_escribir`**.
  - **Dado** ese mismo workspace, **Cuando** se planifica con `policy: {requireValidResult: false}` y
    se aplica, **Entonces** el apply **funciona** como hoy → **test:
    `apply_con_policy_permisiva_sigue_aplicando`** (control anti-vacuo: el gate solo muerde donde el
    plan dijo que mordería).
  - **Dado** un plan con `allowWarnings: false` sobre un resultado con warnings (`canApply: false`
    por la **otra** cláusula), **Cuando** se aplica, **Entonces** también se rechaza y el mensaje
    nombra `allowWarnings` → **test: `apply_rechaza_tambien_por_allow_warnings`**.
  - **Dado** un plan con `canApply: true`, **Cuando** se aplica, **Entonces** se comporta exactamente
    como antes de la historia, incluidos sus gates existentes → **test:
    `apply_de_plan_aplicable_no_cambia`** (control anti-vacuo).
  - **Dado** un plan rechazado por este gate, **Cuando** se inspecciona `.lodestar/runtime/`,
    **Entonces** no hay journal, ni staging, ni recibo, ni copias de recuperación de esa transacción
    (el rechazo ocurre antes del lock) → **test: `el_rechazo_por_can_apply_no_deja_rastro_transaccional`**.
  - **Dado** el smoke de `examples/demo/`, **Cuando** corre el job `demo-smoke`, **Entonces** sigue
    en verde → checklist estructural (ejecución del script).

- **Dependencias**: ninguna **técnica**, pero **se recomienda después de E29-H02**: H02 arregla la
  deserialización de `PlanPolicy` y H07 la convierte en vinculante; con H02 dentro, los tests de H07
  pueden expresar policies parciales con naturalidad en vez de escribir siempre las dos claves. No es
  un bloqueo: si H07 va primero, sus fixtures escriben ambas claves.

- **Pruebas**: arnés de **sesión viva** `Sesion` (patrón de `crates/lodestar-mcp/tests/crash_senal.rs`
  y `descubribilidad.rs`), porque el caso exige encadenar `change_plan` → `change_apply` con el
  `changeSetId` de la respuesta anterior en el **mismo proceso**; más `crates/lodestar-app/tests/`
  para el gate en la capa de servicio; más inspección directa de `.lodestar/runtime/` para el criterio
  de «no deja rastro». Fixtures: `lodestar-fixtures` con un workspace que tenga un **error
  preexistente** (un enlace roto a un `.md`, que `§20.9` clasifica `danglingDocumentLinks: error`) —
  el mismo montaje que usa la demo.

- **Frontera (mcp.yml)**: **sí** (`change_apply.errores` + `semantica`, `change_plan.semantica`, y
  posiblemente el catálogo de `ErrorCode`).

- **Proceso**: ciclo **completo** con **panel** de jueces. Cambia el comportamiento del camino de
  escritura, toca el contrato en tres puntos y tiene una decisión de código de error con carga de la
  prueba explícita.

---

## E29-H08 — El wire rechaza los parámetros que no declara

- **Objetivo**: `additionalProperties: false` deja de ser una afirmación que el servidor no cumple.
  Un parámetro que una tool no declara —un `sort` retirado, un `offset` que no existe, un `wheres`
  con typo— se **rechaza** con `INVALID_SCHEMA` nombrándolo, en vez de descartarse en silencio y
  devolver la respuesta por defecto.

- **Síntoma reproducible** (`decisiones §15`, medido en la revisión de la v0.3.0, sonda 4): **15
  casos aceptados en silencio**, entre ellos el `sort` que E23-H11 retiró, un `offset` inexistente y
  typos como `wheres`/`filters`. Los 10 `inputSchema` declaran `additionalProperties: false`
  (`crates/lodestar-mcp/src/tools.rs`, en `list()` y en cada objeto anidado `ref`/`to`/
  `proposedOperation`) y el servidor no lo ejecuta: `tools::call` lee **campo a campo** con
  `params.get("…")` y nunca mira las claves sobrantes. Un agente que se equivoca de nombre de
  parámetro no se entera.

- **Causa raíz**: no es un bug puntual, es una **política**: la «regla de la casa» de
  `contracts/mcp.yml` (`validacion_de_argumentos`) dice *«el servidor valida los VALORES de los
  parámetros que declara, e IGNORA lo que no declara»*, y está escrita en tres sitios (el contrato, la
  cabecera de `crates/lodestar-mcp/tests/descubribilidad.rs`, y la justificación del schema plano en
  `tools.rs:57-60`). `decisiones §15` la revisó y decidió **ejecutar** lo que el schema declara. El
  riesgo técnico está identificado y no es teórico: `operacion_item_schema()` declara **17
  propiedades planas a propósito**, sin `oneOf` por operación, porque un `oneOf` mal escrito
  rechazaría entradas válidas — `path`/`ref` son intercambiables salvo en `create`, y `body`
  pertenece a **dos** ops (`create` y `replace_body`).

- **Referencias**: `ARCHITECTURE.md §19.6` (superficie MCP) · `contracts/mcp.yml`
  (`validacion_de_argumentos`, y los `inputSchema` de las 10 tools) ·
  `crates/lodestar-mcp/src/tools.rs` (`list():88`, `operacion_item_schema():61`, `call():263` y los
  helpers `limit_validado`/`entero_validado`/`bool_validado`/`str_validado`/`falta`/`forma_invalida`)
  · `crates/lodestar-app/src/lib.rs` (`normalize_raw_op:2588` — **la fuente de los nombres de campo
  por operación**; `expand_selection:2769`; `change_plan:1683`) ·
  `crates/lodestar-mcp/tests/descubribilidad.rs` (cabecera que enuncia la política vigente: **se
  reescribe aquí**) · `decisiones §15` (decidido: **(a) ejecutar**, en historia separada de §16(e)) ·
  `decisiones §16(e)` (el criterio gemelo para el disco, que E29-H01 ya habrá aplicado).

- **Alcance**:
  - **Primer criterio de aceptación y condición de entrada** (`§15` lo fija como tal): **fijar por
    tests la tabla de campos legales por operación _antes_ de activar ningún rechazo**. La tabla es
    la de `decisiones §15`, cuya fuente es `normalize_raw_op`:

    | Campo | Ops | |
    |---|---|---|
    | `op` | todas | discriminador, obligatorio |
    | `path` | todas | obligatoria en `create`, alternativa corta a `ref.path` en el resto |
    | `ref` | todas menos `create` | forma larga de `path` |
    | `expectedRevision` | todas | control de concurrencia optimista |
    | `frontmatter` | `create` | |
    | `body` | `create`, `replace_body` | **compartido entre dos ops** |
    | `patch` | `patch_frontmatter` | |
    | `find`, `replace`, `expectedOccurrences` | `replace_text` | |
    | `headingPath`, `mode`, `content` | `edit_section` | |
    | `from`, `to`, `rewriteInboundLinks` | `move` | |
    | `inboundLinksPolicy` | `delete` | |

    Esa tabla se materializa en tests **verdes antes de tocar el rechazo** (una op de cada tipo con
    todos sus campos legales, que hoy ya funciona y debe seguir funcionando).
  - **Dos niveles de rechazo, con criterio distinto**:
    1. **Nivel tool** (`tools/call`, `arguments`): las claves del objeto de argumentos se validan
       contra la lista declarada por el `inputSchema` de esa tool. Una clave desconocida →
       `INVALID_SCHEMA` nombrándola. Aquí la partición **es limpia** y el rechazo es directo.
    2. **Nivel operación** (`operations[]` de `change_plan`, y el objeto `operation` de la selección
       masiva): la validación es **por unión, no por partición** — se rechaza lo que no esté en la
       **unión** de los 17 campos legales, y **no** se rechaza un campo legal para otra op. Es decir:
       un `body` en un `patch_frontmatter` **se sigue ignorando**, y un `bodyy` se rechaza. Razón: es
       exactamente el riesgo que `§15` señala (un agente que reutiliza la misma plantilla de objeto
       para varias operaciones de un lote —perfectamente válido— empezaría a recibir rechazos si la
       partición se escribe como si fuera limpia), y el `oneOf` por operación sigue sin existir.
       Cerrar la partición por op es una **decisión posterior**, no de esta historia, y se declara
       así en el contrato.
  - **`change_plan` y el objeto entero como `raw_ops`**: hoy, cuando hay `selection`,
    `tools.rs:468-472` pasa `params.clone()` **entero** a `App::change_plan` — o sea, el objeto de
    argumentos de la tool y el de la selección masiva son el mismo. El rechazo de nivel tool debe
    escribirse sabiéndolo: las claves legales de `change_plan` son
    `expectedWorkspaceRevision`/`operations`/`selection`/`operation`/`policy`, y `selection`/
    `operation` tienen su propia forma interna (`where`/`filter`; una sola clave de op).
  - **Un solo sitio**: la lista de campos legales por tool vive en **una** estructura consultada
    tanto por el rechazo como —idealmente— por la generación del `inputSchema`, para que no puedan
    divergir. Si derivar el schema de la lista resulta invasivo, el mínimo aceptable es un test que
    verifique que **para cada tool, las claves declaradas en su `inputSchema` == las claves aceptadas
    por el despachador** (esa es la guarda que impide que esta historia envejezca).
  - Se reescribe la política en sus **tres sedes**: `contracts/mcp.yml`
    (`validacion_de_argumentos`), la cabecera de `crates/lodestar-mcp/tests/descubribilidad.rs`, y el
    doc-comment de `operacion_item_schema` en `tools.rs`. Ninguna puede quedar afirmando la política
    vieja: sería reintroducir el defecto en su forma documental.
  - `docs/user/mcp-clients.md` declara el nuevo comportamiento (un parámetro inventado ahora es un
    error), en inglés.

- **Fuera de alcance**: introducir `oneOf` por operación en `operacion_item_schema` (la partición
  estricta por op es explícitamente una decisión posterior); validar los argumentos contra el
  `inputSchema` con una librería de JSON Schema (el schema sigue siendo **declarativo**; lo que se
  ejecuta es la lista de campos, no el schema entero); cambiar la validación de **valores**, que ya
  se hace desde E24-H09; los métodos JSON-RPC distintos de `tools/call` (`initialize` es E29-H09).

- **Delta de contrato propuesto** (`contracts/mcp.yml`):
  - **`validacion_de_argumentos` se reescribe**: el servidor valida los valores que declara **y
    rechaza las claves que no declara**, en el nivel tool. Se declara la excepción del nivel
    operación (validación por unión de los 17 campos, no por partición por op) y **por qué**: `path`/
    `ref` intercambiables y `body` compartido entre dos ops hacen que una partición estricta rechace
    lotes válidos. Se declara que cerrar esa partición es trabajo futuro, no un olvido.
  - En la cabecera de historial, la línea de E24-H09 que dice *«La política sobre los NO declarados
    no cambia»* se **actualiza** apuntando a esta historia: sin eso, el contrato se contradice a sí
    mismo, que es el defecto que la épica cierra.
  - Cada fila de `tools:` cuya lista `errores:` no incluya ya `INVALID_SCHEMA` lo gana, con la
    semántica «parámetro no declarado».
  - Sin código nuevo (`INVALID_SCHEMA` ya existe) y sin cambio en ningún `inputSchema` (ya declaran
    `additionalProperties: false`: el cambio es que ahora se cumple).

- **Criterios de aceptación**:
  - **Dado** el estado previo al rechazo, **Cuando** se ejecuta un `change_plan` con **una operación
    de cada uno de los 7 tipos**, cada una con **todos** sus campos legales de la tabla de arriba,
    **Entonces** las 7 se normalizan sin error → **test:
    `los_campos_legales_de_cada_operacion_se_aceptan`** (**condición de entrada**: verde antes de
    activar el rechazo, y verde después).
  - **Dado** un `knowledge_search` con `sort: "title"` (parámetro retirado en E23-H11), **Cuando** se
    llama, **Entonces** la respuesta es `INVALID_SCHEMA` nombrando `sort`, no la lista por defecto →
    **test: `parametro_retirado_se_rechaza_nombrandolo`**.
  - **Dado** un `knowledge_search` con `wheres` (typo de `where`), **Cuando** se llama, **Entonces**
    `INVALID_SCHEMA` nombrando `wheres` → **test: `typo_de_parametro_se_rechaza`**.
  - **Dado** un `knowledge_get` con `ref: {path: "a.md", depth: 2}` (clave desconocida en el objeto
    **anidado**), **Cuando** se llama, **Entonces** también se rechaza → **test:
    `clave_desconocida_en_objeto_anidado_se_rechaza`**.
  - **Dado** un `change_plan` con una operación `patch_frontmatter` que además lleva `body`
    (campo legal de **otra** op), **Cuando** se planifica, **Entonces** se acepta y `body` se ignora,
    como hoy → **test: `campo_legal_de_otra_operacion_se_sigue_ignorando`** (la excepción declarada:
    es el criterio que impide romper lotes válidos).
  - **Dado** un `change_plan` con una operación que lleva `bodyy` (typo, no está en la unión de los
    17), **Cuando** se planifica, **Entonces** `INVALID_SCHEMA` nombrando `bodyy` → **test:
    `campo_inexistente_en_una_operacion_se_rechaza`**.
  - **Dado** un `change_plan` en forma de **selección masiva**, **Cuando** lleva
    `selection`+`operation`+`policy` (todo legal), **Entonces** se planifica como hoy → **test:
    `la_seleccion_masiva_sigue_funcionando`** (control anti-vacuo del caso donde `params` viaja
    entero).
  - **Dado** cada una de las 10 tools, **Cuando** se comparan las claves de su `inputSchema` con las
    que su despachador acepta, **Entonces** coinciden exactamente → **test:
    `el_schema_declarado_coincide_con_lo_aceptado`** (la guarda anti-envejecimiento).
  - **Dado** un `workspace_status` con cualquier argumento (su schema es el objeto vacío), **Cuando**
    se llama con `{"foo": 1}`, **Entonces** se rechaza → **test:
    `tool_sin_parametros_rechaza_cualquier_argumento`**.
  - **Dado** el benchmark funcional de `§17` y el smoke de `examples/demo/`, **Cuando** se ejecutan
    tras el cambio, **Entonces** siguen en verde → checklist estructural (ninguna llamada existente
    del repo manda parámetros no declarados; si alguna lo hace, se corrige la llamada).
  - **Dado** las tres sedes de la política vieja, **Cuando** se revisan tras la historia,
    **Entonces** ninguna afirma que lo no declarado se ignora → checklist estructural (grep del
    criterio sobre `contracts/mcp.yml`, `tests/descubribilidad.rs`, `tools.rs`).

- **Dependencias**: **E29-H01** (por criterio, no por código: fija primero que lo desconocido se
  rechaza cuando llega por disco, y esta historia aplica el mismo criterio al wire — `§15` es
  explícito en que van separadas para que el cierre de la barata no dependa de la cara).
  **Recomendada después de E29-H02** (la deserialización de `policy` ya arreglada evita mezclar dos
  causas de rechazo en el mismo objeto) y **después de E29-H09** si ambas tocan el despachador; ver
  el orden de construcción.

- **Pruebas**: `roundtrip()` de `crates/lodestar-mcp/tests/mcp.rs` (cada caso es una llamada aislada)
  + `crates/lodestar-mcp/tests/descubribilidad.rs` (donde vive hoy la guarda de la política y el test
  de coherencia schema↔despachador) + `crates/lodestar-app/tests/plan.rs` para la tabla de campos por
  operación. Fixtures: `lodestar-fixtures`, workspace con documentos suficientes para ejercer las 7
  ops (uno existente para `patch`/`replace`/`move`/`delete`, un path libre para `create`).

- **Frontera (mcp.yml)**: **sí**, y es la más profunda de la épica: cambia la **regla de la casa** de
  la validación de argumentos.

- **Proceso**: ciclo **completo** con **panel** de jueces y **pasada de `/mutantes`** sobre el módulo
  de validación nuevo. Es la historia mayor de la épica (`§15`: *«esta es la mayor de la épica y no
  debe arrastrar»* a las demás): si se complica, se para ella sola y las otras diez cierran igual.

---

## E29-H09 — `instructions` describe lo que el perfil sirve, y una `protocolVersion` no soportada deja de aceptarse en silencio

- **Objetivo**: bajo `--profile readonly`, el texto `instructions` que el agente lee en `initialize`
  nombra **exactamente** las 7 tools que `tools/list` sirve, no las 10 del perfil `standard`. Y un
  `initialize` con una `protocolVersion` que el servidor no soporta deja de responderse como si se
  hubiera negociado algo.

- **Síntoma reproducible** (caso G1-24 del testbench, `§23/D-01`): con `--profile readonly`,
  `tools/list` sirve 7 tools pero `instructions` describe el flujo completo de 10 —incluidas
  `change_plan`/`change_apply`/`change_revert`— con una nota final de que en readonly «no están
  disponibles». Un agente que siga las instrucciones al pie de la letra acaba en `-32602`. El
  contrato garantiza lo contrario: `mcp.yml` (`meta.protocolo.instructions`) exige que el texto
  *«nombre EXACTAMENTE las tools que sirve `tools/list`»*, y el test interno lo endurece («ni una de
  menos… ni una de más: una tool que no existe manda al agente a un `-32602`»). **Anexo**:
  `initialize` acepta cualquier `protocolVersion`.

- **Causa raíz** (verificada en código): `crates/lodestar-mcp/src/main.rs` —
  - `SERVER_INSTRUCTIONS` (L31-61) es una **constante única**, sin variante por perfil; el brazo
    `"initialize"` (L198-211) la sirve tal cual y **ni siquiera recibe el `profile`**, aunque `handle`
    lo tiene en su firma (L180).
  - El test que blinda la garantía, `instructions_sin_vocabulario_retirado`
    (`crates/lodestar-mcp/tests/mcp.rs:177`), usa `roundtrip()` — o sea, **solo el perfil
    `standard`**. La garantía nunca se comprobó bajo `readonly`: por eso el drift pasó desapercibido.
  - `protocolVersion`: el brazo hace
    `params.get("protocolVersion").and_then(as_str).filter(|v| matches!(…)).unwrap_or("2024-11-05")`
    (L200-204). **Matiz frente al informe**, que hay que medir en la fase roja: no «ecoa cualquier
    versión», sino que **descarta la desconocida y responde `2024-11-05` como si nada**. El efecto
    práctico es el mismo —un cliente que pidió `"1990-01-01"` recibe una respuesta de éxito— pero el
    criterio de aceptación debe fijarse sobre el comportamiento **real**, reproducido, no sobre la
    frase del informe.

- **Referencias**: `ARCHITECTURE.md §19.6` (superficie MCP y perfiles: readonly **oculta y rechaza**
  las 3 tools de cambio) · `contracts/mcp.yml` (`meta.protocolo.instructions`,
  `meta.protocolo.protocol_versions_aceptadas: ["2024-11-05","2025-03-26","2025-06-18"]`,
  `meta.perfiles.readonly`) · `crates/lodestar-mcp/src/main.rs`
  (`SERVER_INSTRUCTIONS:31`, `handle:180`, brazo `initialize:198`) ·
  `crates/lodestar-mcp/src/tools.rs` (`available_tools:235`, `available:255`, `is_change_tool`) ·
  `crates/lodestar-mcp/tests/mcp.rs` (`instructions_sin_vocabulario_retirado:177`,
  `roundtrip_profile:321` — el helper que ya existe y que este test nunca usó) ·
  `decisiones §23/D-01`.

- **Alcance**:
  - **`instructions` por perfil**: bajo `readonly`, el texto describe el flujo de lectura/verificación
    y **no nombra** `change_plan`/`change_apply`/`change_revert`. La forma concreta —dos constantes,
    una constante con secciones que se filtran, o composición desde la lista de tools— la decide la
    fase roja; el criterio es que **el conjunto de tools nombradas se derive del mismo predicado**
    que `available_tools` (`Profile::writes_enabled` + `is_change_tool`), no de una segunda lista
    escrita a mano (invariante #3: si un día entra o sale una tool, no puede haber dos sitios que
    actualizar).
  - **El test se generaliza a los dos perfiles**: `instructions_sin_vocabulario_retirado` pasa a
    ejercitar `standard` **y** `readonly` con `roundtrip_profile`, que ya existe. Esto es tan
    importante como el arreglo: el defecto existía porque la guarda solo miraba un perfil.
  - **`protocolVersion` no soportada**: se **rechaza** en vez de normalizarse. Forma propuesta: error
    JSON-RPC `-32602` (*Invalid params*) con un mensaje que **liste las versiones aceptadas** — es el
    mismo código que el servidor ya usa para «tool no disponible bajo este perfil» y para tool
    desconocida, y mantiene el error dentro del protocolo (un `initialize` fallido es un handshake
    fallido, no un error de dominio: **no** lleva código del catálogo de `ErrorCode`, que es para
    errores de tool).
  - **`protocolVersion` ausente**: sigue siendo válida y se responde con la versión por defecto del
    servidor (`2024-11-05`), como hoy — omitir no es lo mismo que pedir algo imposible. Se fija con
    test para que el rechazo no se cierre de más.
  - `docs/user/mcp-clients.md` declara ambas cosas: que las instrucciones dependen del perfil y que
    una versión de protocolo no soportada es un handshake rechazado.

- **Fuera de alcance**: cambiar el conjunto de versiones aceptadas (las tres siguen siendo las de
  `mcp.yml`); negociación de versión más elaborada (proponer la más alta común, etc.); cualquier
  cambio en qué tools sirve cada perfil; el resto de campos de la respuesta `initialize`
  (`serverInfo`, `capabilities`).

- **Delta de contrato propuesto** (`contracts/mcp.yml`):
  - `meta.protocolo.instructions`: la garantía se declara **por perfil** — el texto nombra exactamente
    las tools que sirve `tools/list` **bajo el perfil activo**.
  - `meta.protocolo`: se declara que una `protocolVersion` **no soportada** produce `-32602` con la
    lista de versiones aceptadas, y que su **ausencia** es válida (se responde la versión por
    defecto del servidor). Hoy `protocol_versions_aceptadas` es una lista sin consecuencia declarada.
  - `meta.perfiles.readonly`: se añade que también acota el `instructions`.

- **Criterios de aceptación**:
  - **Dado** el servidor con `--profile readonly`, **Cuando** se hace `initialize` + `tools/list`,
    **Entonces** el conjunto de tools nombradas en `instructions` es **exactamente** el servido por
    `tools/list` (7) → **test: `instructions_readonly_nombra_solo_las_tools_servidas`**.
  - **Dado** el servidor con `--profile standard`, **Cuando** se hace lo mismo, **Entonces** la
    igualdad se mantiene con las 10 → **test: `instructions_standard_sigue_coincidiendo`** (control
    anti-vacuo: la generalización del test no puede romper el caso que ya funcionaba).
  - **Dado** el servidor en `readonly`, **Cuando** se busca `change_apply` en el texto de
    `instructions`, **Entonces** no aparece → **test:
    `instructions_readonly_no_nombra_tools_de_cambio`** (aserción directa del síntoma).
  - **Dado** un `initialize` con `protocolVersion: "1990-01-01"`, **Cuando** se llama, **Entonces**
    la respuesta es un error `-32602` cuyo mensaje lista las tres versiones aceptadas → **test:
    `protocol_version_no_soportada_se_rechaza`**.
  - **Dado** un `initialize` con `protocolVersion: "2025-03-26"`, **Cuando** se llama, **Entonces**
    se ecoa esa versión, como hoy → **test: `initialize_ecoa_version_soportada`** (el test existente,
    que debe seguir verde).
  - **Dado** un `initialize` **sin** `protocolVersion`, **Cuando** se llama, **Entonces** responde
    `2024-11-05` sin error → **test: `initialize_sin_version_sigue_funcionando`** (control
    anti-vacuo).
  - **Dado** el código tras la historia, **Cuando** se revisa de dónde sale la lista de tools que
    nombra `instructions`, **Entonces** deriva del mismo predicado que `available_tools` y no de una
    segunda lista literal → revisión de diff (criterio estructural).

- **Dependencias**: ninguna. **Nota de coordinación**: toca `crates/lodestar-mcp/src/main.rs`
  (`handle`) mientras E29-H08 toca `tools.rs` (`call`); si se paralelizan, el punto de encuentro es
  la firma de `handle`/el paso del `profile`.

- **Pruebas**: `crates/lodestar-mcp/tests/mcp.rs` con `roundtrip()` **y** `roundtrip_profile()` (los
  dos helpers ya existen; la historia consiste en buena parte en usar el segundo donde nunca se usó).
  Fixtures: `workspace_min()` del propio fichero de tests — no hacen falta fixtures nuevos.

- **Frontera (mcp.yml)**: **sí** (`meta.protocolo` e `meta.perfiles`).

- **Proceso**: ciclo **acotado**. El arreglo es contenido y su riesgo está en el test (que la
  generalización a dos perfiles no se quede en una aserción trivialmente cierta); el juez ciego debe
  verificar que el test **falla** con el `instructions` actual bajo `readonly`.

---

## E29-H10 — La API pública no transaccional de `Workspace` se cierra al exterior

- **Objetivo**: `Workspace::create_document`, `write_document` y `merge_frontmatter` dejan de ser
  superficie pública del crate. Hoy escriben el canónico **sin lock, sin journal y sin copias de
  recuperación** —esquivando las seis garantías que E25 reforzó— y son inofensivas solo porque nadie
  de producción las llama. Es una trampa con fecha de caducidad: funciona hasta que alguien la llama.

- **Síntoma**: no hay síntoma observable — es el **defecto S8** de la auditoría del camino de
  escritura, confirmado por los jueces ciegos de E25. La evidencia es estructural: las tres funciones
  (`crates/lodestar-workspace/src/lib.rs:393`, `:414`, `:444`) toman `guard_recovery()` y luego
  escriben con `io::write_atomic` directamente, sin pasar por `apply_transaction`. **Verificado en
  este repo**: sus únicos llamantes están en `crates/lodestar-workspace/tests/workspace.rs` y
  `tests/transactions.rs`, es decir, **fuera del crate** (los tests de integración son crates
  aparte) — ese es justo el punto que hace la historia menos trivial de lo que parece.

- **Referencias**: `decisiones §16(g)` (decidido: **(a) replegar a `pub(crate)` / marcar como
  primitivas de test**) · `ARCHITECTURE.md §19.5` (modelo transaccional y sus garantías) ·
  `crates/lodestar-workspace/src/lib.rs` (`create_document:393`, `write_document:414`,
  `merge_frontmatter:444`, `guard_recovery:381`) · `crates/lodestar-workspace/src/publish.rs`
  (`publish:94`, `publish_result:143` — ya `pub(crate)`) ·
  `crates/lodestar-workspace/src/transaction.rs` (`apply_transaction_con_recibo`, el camino legítimo)
  · E23-H12 (`materialize_staging`, el precedente exacto de esta misma operación) · `CLAUDE.md`
  invariante #5 (único escritor).

- **Alcance**:
  - Las **tres** funciones de `Workspace` (`create_document`, `write_document`, `merge_frontmatter`)
    dejan de ser `pub`.
  - **`Workspace::publish` es un caso distinto y hay que decidirlo explícitamente**: a diferencia de
    las otras tres, es una pieza **real** del camino transaccional (su núcleo `publish_result` ya es
    `pub(crate)` y lo llama `transaction.rs:255`), y su llamante externo es
    `tests/transactions.rs:724,802`. `decisiones §16(g)` la nombra junto a las otras tres. La
    **recomendación** es replegarla también —quien deba publicar lo hace por `apply_transaction`—
    tratándola por el mismo mecanismo que las demás; si la fase roja encuentra que un test de
    integración legítimo no tiene sustituto, se declara y se conserva el acceso **solo** bajo el
    mecanismo de test elegido, nunca como `pub` a secas.
  - **Mecanismo para los tests de integración** (la decisión de diseño de esta historia, porque
    `pub(crate)` los rompe): opciones, en orden de preferencia recomendado —
    1. **Reescribir los tests afectados** para que monten su estado por el camino legítimo
       (`apply_transaction`/`change_plan`+`change_apply`) o escribiendo los `.md` directamente en el
       `tempdir` antes de abrir el workspace. Es la opción más honesta: si el único uso de una API
       es preparar fixtures, el fixture se prepara con `std::fs`. Coste: muchos call-sites
       (~15 en `transactions.rs`, ~6 en `workspace.rs`).
    2. **Feature `test-support`** que reexporte las funciones como `pub` solo bajo esa feature
       (patrón ya usado en el repo con `test-failpoints`), con los tests de integración
       activándola.
    3. `#[doc(hidden)] pub` con un nombre que declare el peligro. **Descartada**: sigue siendo
       superficie pública, o sea el defecto sin arreglar.

    La elección la hace la fase roja con el conteo real de call-sites a la vista; el criterio
    innegociable es que tras la historia **ningún consumidor externo al crate pueda escribir el
    canónico sin pasar por la transacción** en una build normal (sin features de test).
  - Los **doc-comments** de las funciones que sobrevivan como `pub(crate)` declaran que **no** son
    transaccionales y por qué no se exponen.

- **Fuera de alcance**: cambiar el **comportamiento** de ninguna de las funciones (siguen haciendo
  exactamente lo mismo para quien las llame desde dentro del crate); unificarlas con el camino
  transaccional o borrarlas (`§16(g)` decidió cerrarlas, no retirarlas); `Workspace::read_document`
  y demás lectura, que no escribe nada; los homónimos de `lodestar_core::DocumentSet`
  (`create_document`, `write_document_raw`, `merge_frontmatter`), que son **puros**, no escriben y
  deben seguir siendo `pub` — el core es una librería de lógica sin I/O.

- **Criterios de aceptación**:
  - **Dado** el crate `lodestar-workspace` tras la historia, **Cuando** se compila el workspace
    entero **sin** features de test, **Entonces** `Workspace::create_document`,
    `write_document` y `merge_frontmatter` (y `publish`, según lo que decida la fase roja) **no**
    forman parte de su API pública → verificación estructural: revisión de diff + `cargo doc` (no
    aparecen en la documentación pública del crate).
  - **Dado** ese mismo estado, **Cuando** se compilan `lodestar-app`, `lodestar-cli` y
    `lodestar-mcp`, **Entonces** compilan sin cambios (no las usaban) → compilación del workspace.
  - **Dado** la suite completa, **Cuando** se ejecuta (`cargo test --workspace` + los dos crates con
    `--features test-failpoints`), **Entonces** está en verde y **ningún test se ha borrado** para
    conseguirlo: los reescritos cubren lo mismo → revisión de diff (criterio explícito: el juez debe
    comprobar que las aserciones sobrevivieron a la reescritura, no solo que la suite pasa).
  - **Dado** los tres doc-comments de las funciones replegadas, **Cuando** se leen, **Entonces**
    declaran que no son transaccionales y por qué no se exponen → revisión de diff.
  - **Dado** `lodestar_core::DocumentSet`, **Cuando** se revisa su API, **Entonces** sus funciones
    homónimas siguen siendo públicas → revisión de diff (control anti-vacuo: la historia no puede
    llevarse por delante el core puro por coincidencia de nombres).

- **Dependencias**: ninguna. **Nota de coordinación**: E28-H01 está reescribiendo
  `transaction.rs`/`recovery.rs` en la fase 0, y varios de los tests a reescribir viven en
  `tests/transactions.rs`. **Esta historia debe ir después de que E28-H01 esté integrada**, para no
  resolver conflictos sobre el mismo fichero de tests.

- **Pruebas**: **la evidencia de esta historia es compilación + suite + revisión de diff**, no un
  test e2e rojo→verde: no hay comportamiento observable que cambie (es una retirada de superficie).
  Se declara así explícitamente, en línea con el criterio de cierre de la campaña
  (`docs/qa/campana-bugfixes-2026-08.md`, que ya marca §16(g) como «compilación+suite»). El trabajo
  de test es de **conservación**: `crates/lodestar-workspace/tests/workspace.rs` y
  `tests/transactions.rs` se reescriben conservando cada aserción.

- **Frontera (mcp.yml)**: no. La API de `Workspace` no es superficie de wire.

- **Proceso**: ciclo **acotado**, sin fase roja de comportamiento (no hay rojo que producir). Sí
  **juez ciego**, con el encargo específico de verificar que la reescritura de tests no perdió
  cobertura — que es el único riesgo real de la historia.

---

## E29-H11 — Retirada de `Envelope`/`ErrorEnvelope`

- **Objetivo**: retirar del repo el envelope de `lodestar-app`, capacidad construida y testeada que
  **ninguna fachada usa**, para que no queden dos formas de respuesta —una en uso y otra por si
  acaso— que es la duplicación que el invariante #4 existe para evitar.

- **Síntoma**: no hay síntoma observable, es deuda estructural. `Envelope<T>` y `ErrorEnvelope`
  existen en `crates/lodestar-app/src/lib.rs` (L35-…, L252-…) desde E10-H01 (decisión **D3** de
  `§0`), compilan y tienen tests propios (`crates/lodestar-app/tests/envelope.rs`), pero **el MCP
  devuelve `structuredContent` + texto con el código y la CLI sus exit codes**: ninguna de las dos
  construye un envelope. El propio doc-comment de `ErrorEnvelope` lo admite: *«Esta historia
  (E10-H02) solo fija la forma — nadie la construye todavía en un flujo real»*. Tras E26-H07 el wire
  ya es honesto **sin** envelope.

- **Referencias**: `decisiones §16(b)` (decidido: **retirarlo**, como se retiró `lodestar-vcs` en
  E15-H01) · `ARCHITECTURE.md §19.6` (superficie real de las tools: `structuredContent`, sin
  envelope) · `crates/lodestar-app/src/lib.rs` (`Envelope:47`, `ErrorEnvelope:261`, `ResourceLink`,
  y la mención del envelope en el doc del **módulo**, L3-7) ·
  `crates/lodestar-app/tests/envelope.rs` · `docs/history/REFACTOR_DISENO_PROPUESTA.md` (D3, que se
  conserva como historia) · `CLAUDE.md` invariante #4.

- **Alcance**:
  - Borrar `Envelope<T>`, `ErrorEnvelope` y los tipos auxiliares que **solo** ellos usan (p. ej.
    `ResourceLink`, si no tiene otro consumidor: hay que **verificarlo**, no asumirlo).
  - Borrar `crates/lodestar-app/tests/envelope.rs`.
  - Actualizar el doc-comment del **módulo** de `lodestar-app` (L3-7), que hoy presenta el crate como
    *«compone el `Envelope<T>` de protocolo … y la fachada `App`»* — tras la retirada, el crate es la
    fachada `App` y sus tipos de servicio.
  - Barrer las **referencias en documentación interna** que lo presenten como parte de la superficie
    viva: `ARCHITECTURE.md` donde describa el envelope como forma de respuesta, `contracts/mcp.yml`
    si lo menciona (las filas de tools ya dicen *«json directo, sin envelope»*, así que ese texto
    probablemente **se conserva** por ser una aclaración útil — se decide en la historia, con el
    criterio de que ningún documento vivo lo presente como algo que exista), e
    `IMPLEMENTATION_STATUS.md`.
  - **Lo que NO se toca**: la mención histórica en `docs/history/` (D3 es historia del proyecto) y
    los registros de las épicas E10 en `requirements/`, que describen lo que se construyó entonces.

- **Fuera de alcance**: cambiar la forma de respuesta de ninguna tool (el wire actual **es** el que
  se queda); `AppError`/`ErrorCode`, que son la superficie de error viva y no tienen nada que ver;
  cualquier otro tipo de `lodestar-app` con consumidor real.

- **Criterios de aceptación**:
  - **Dado** el repo tras la historia, **Cuando** se busca `Envelope` en `crates/`, **Entonces** no
    hay ninguna definición ni uso (salvo, si acaso, la frase «sin envelope» del contrato, que
    describe la ausencia) → checklist estructural (grep del criterio, del mismo tipo que el que el CI
    ya aplica a `git2`/`lodestar-vcs`/`zip` tras E15).
  - **Dado** el workspace, **Cuando** se compila y se ejecuta la suite completa (incluidos los dos
    crates con `--features test-failpoints`), **Entonces** todo verde → compilación + suite.
  - **Dado** cada tipo auxiliar del envelope, **Cuando** se comprueba antes de borrarlo,
    **Entonces** se ha verificado que **no** tiene otros consumidores → revisión de diff (criterio
    explícito: nada se borra por asociación de nombre).
  - **Dado** el doc del módulo de `lodestar-app` y los documentos de estado, **Cuando** se leen tras
    la historia, **Entonces** ninguno presenta el envelope como superficie viva → revisión de diff.
  - **Dado** la superficie MCP, **Cuando** se ejecuta cualquier tool antes y después, **Entonces** la
    respuesta es **byte-idéntica** → **test: la suite e2e existente de `mcp.rs` sigue verde sin
    tocarse** (control anti-vacuo: una retirada no puede cambiar el wire).

- **Dependencias**: ninguna. Es la historia más aislada de la épica y **la mejor candidata a
  paralelizarse** con cualquier otra.

- **Pruebas**: **la evidencia es compilación + suite + revisión de diff** (retirada de código sin
  comportamiento observable), igual que E29-H10 y como ya declara
  `docs/qa/campana-bugfixes-2026-08.md` para §16(b). El control anti-vacuo es que los e2e existentes
  de la superficie MCP sigan verdes **sin modificarse**.

- **Frontera (mcp.yml)**: no (posible ajuste de redacción si alguna frase presenta el envelope como
  algo vivo).

- **Proceso**: ciclo **acotado**, sin fase roja. Juez ciego breve, centrado en que ningún tipo se
  haya borrado por asociación de nombre y en que el wire no haya cambiado.

---

## Orden de construcción

```
  ── Bloque A · el criterio (va primero; §16(e) es «la primera de la épica») ──
  H01  config estricta (§16e + A-08)              ← fija «lo desconocido se rechaza»
        │
        │  (criterio, no código)
        ▼
  ── Bloque B · lenguaje de consulta (core::eval, paralelizable entre sí con cuidado) ──
  H03  has(frontmatter) / missing(frontmatter)    ┐ mismo fichero (core/src/eval.rs),
  H04  starts_with/ends_with type error           ┘ funciones distintas: conflicto de
                                                    merge posible, lógico no

  ── Bloque C · rechazos de superficie, baratos y aislados (paralelizables) ──
  H02  policy parcial                             ← recomendada antes de H07 y H08
  H05  knowledge_check scope paths
  H06  workspace vacío → warn
  H09  instructions por perfil + protocolVersion  ← toca main.rs (handle)

  ── Bloque D · los dos grandes (secuenciales respecto de C) ──
  H07  canApply vinculante        (después de H02)
  H08  wire estricto              (después de H01 por criterio; después de H02 y H09)

  ── Bloque E · retiradas (sin comportamiento; al final o en paralelo) ──
  H10  pub(crate) de la API no transaccional      ← DESPUÉS de que E28-H01 esté integrada
  H11  retirada del Envelope                      ← aislada, paralelizable con todo
```

**Orden lineal recomendado** (un solo implementador, o para secuenciar la rama):

`H01 → H02 → H03 → H04 → H05 → H06 → H09 → H07 → H08 → H11 → H10`

Razones del orden, punto por punto:

- **H01 primero** por decisión explícita de `decisiones §16(e)` («historia propia y primera de la
  épica de honestidad») y porque establece el criterio —lo desconocido se rechaza— que **H08** aplica
  al wire; `§15` exige que los dos criterios coincidan y que las historias vayan **separadas**.
- **H02 antes de H07 y H08**: arreglar la deserialización de `PlanPolicy` antes de hacerla vinculante
  (H07) y antes de endurecer el objeto de argumentos (H08) evita que dos causas de rechazo se mezclen
  en el mismo objeto durante el desarrollo y los tests.
- **H03 y H04 juntas** por compartir `core/src/eval.rs`: son independientes en lógica, pero
  paralelizarlas produce conflictos de merge sin ganancia.
- **H09 antes de H08** si se secuencia: las dos tocan el despachador MCP (`main.rs` vs `tools.rs`);
  H09 es pequeña y H08 es la mayor de la épica, así que conviene no tener la grande abierta mientras
  entra la pequeña.
- **H08 la última de las de comportamiento**: `decisiones §15` la declara «la mayor de la épica» y
  pide explícitamente que su complicación no arrastre a las demás. Si se atasca, las otras diez ya
  están cerradas.
- **H10 después de E28-H01**: aquella épica está reescribiendo `transaction.rs`/`recovery.rs` y esta
  reescribe `tests/transactions.rs`. Es la única dependencia **cruzada entre épicas** de E29.
- **H11 donde quepa**: no depende de nada ni nada depende de ella.

**Paralelizable de verdad** (equipos/agentes distintos, sin conflicto de ficheros): `H11` con
cualquiera; `{H05, H06}` con `{H03, H04}`; `H09` con `{H03, H04, H05}`. **No paralelizar**: `H07`
con `H08` (los dos tocan el camino de `change_plan`/`change_apply` y el contrato), ni `H03` con `H04`.

## Criterio de salida

Las once afirmaciones que la superficie hacía sin respaldo quedan honradas o retiradas: una config
con un typo ya no afloja una salvaguarda en silencio (y las familias válidas están publicadas), una
`policy` parcial se acepta como el contrato prometía, `has(frontmatter)` responde la verdad,
`starts_with` sobre un número grita en vez de esconder siete documentos, un path inexistente en
`knowledge_check` deja de desaparecer, un directorio equivocado deja de responder «VÁLIDO» a secas,
`canApply: false` impide el apply, un parámetro inventado deja de ignorarse, el `instructions` de
`readonly` describe lo que `readonly` sirve, y las dos piezas de superficie sin consumidor —la API no
transaccional de `Workspace` y el `Envelope`— dejan de estar disponibles para el primero que las
encuentre.

Cada una de las **nueve** historias de comportamiento (H01–H09) cierra con **evidencia e2e
rojo→verde** —el criterio del goal de la campaña— y veredicto de juez ciego; las **dos** de retirada
(H10, H11) cierran con compilación, suite completa (incluidos los dos crates con
`--features test-failpoints`) y revisión de diff, según se declara en cada una. `/contrato --check`
queda limpio tras las siete que tocan `contracts/mcp.yml`. Los frontmatter de `decisiones §15`,
`§18`, `§19` y `§23` se actualizan al cerrar, y `§16` gana la anotación de que sus puntos (b), (e),
(f) y (g) quedaron ejecutados aquí.
