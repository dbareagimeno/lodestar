---
id: 23
titulo: "Hallazgos del testbench MCP sobre el homelab"
estado: "cerrada"
prioridad: 5
etiquetas: ["escritura", "mcp", "contrato", "docs", "dogfooding", "lenguaje-consulta"]
origen: "dogfooding"
abierta_en: "2026-08-06"
revisada_en: "2026-08-07"
cerrada_en: "2026-08-07"
subpuntos: ["M-01", "D-01", "D-02", "A-01", "A-02", "A-03", "A-04", "A-05", "A-06", "A-07", "A-08", "A-09", "A-10"]
relacionadas: [3, 9, 14, 16, 18, 19, 22]
---

# §23 — Hallazgos del testbench MCP sobre el homelab

> **Origen**: dogfooding sistemático, no auditoría de código. 189 casos
> esperado-vs-real sobre el workspace real del homelab, con verificación
> adversarial de todo veredicto no-PASS; informe completo y arnés reproducible
> en [`docs/qa/`](../docs/qa/informe-homelab-2026-08-06.md). Es el primer
> resultado del banco de pruebas que [`§9`](09-transversales-diferidas.md)
> pedía y la segunda entrega del dogfooding tras
> [`§22`](22-integridad-referencial-frontmatter.md).
>
> Trece hallazgos de naturalezas incompatibles no caben bajo una sola
> prioridad — la lección de [`§16`](16-deuda-auditoria-e25-e26.md) —, así que
> esta ficha nace **ya disuelta**: cada punto lleva su dueño y su prioridad, y
> la ficha conserva el registro para que la próxima pasada no lo redescubra.
> La prioridad 5 del frontmatter es la del punto más alto vivo (M-01), no la
> del conjunto.

## Priorización (2026-08-06)

De más urgente a menos. «Dueño» = dónde se ejecuta el trabajo; los puntos que
caen en decisiones o épicas ya acordadas no crean trabajo nuevo, aportan
evidencia y criterio de aceptación.

| # | Punto | Qué es | Dueño | Prio |
|---|---|---|---|---|
| 1 | ✅ **M-01** revert de un recibo `-revert` | **Bug de motor con pérdida de datos**: no-op silencioso que responde `reverted: true` y sobrescribe las copias del redo | **Ejecutado**: `E28-H01` (`296147b`) + adenda `E28-H03` (`8c86b6b`, `c532929`) | **5** |
| 2 | ✅ **A-05** `create`/`move` sobre path ocupado | `canApply: true` sin fricción; aplicado, pisa conocimiento. Único hueco con riesgo destructivo para un agente | **Ejecutado**: `E28-H02` (`043f233`) + adenda `E28-H04` (`8c86b6b`, `c532929`) | **4** |
| 3 | ✅ **D-01** `instructions` nombra 10 tools bajo `readonly` (+ `protocolVersion` sin validar) | La superficie afirma lo que el perfil no sirve — la definición literal de la **épica de honestidad de superficie** | **Ejecutado**: `E29-H09` (`5e7edc0`) | **4** |
| 4 | ✅ **A-02/A-03** cursor malformado → offset 0 · cursor ajeno aceptado | Ya **decidido** en §16(j) (`INVALID_SCHEMA`); el testbench aporta la repro y descubre la variante cross-tool | **Ejecutado**: `E30-H01` (`8359294`) — cursor firmado con su origen; malformado o ajeno → `INVALID_SCHEMA`, nunca cae a la página 1. Remate de robustez en `2d32eeb` (un cursor no-ASCII hacía `panic!` y tumbaba la sesión) | **3** |
| 5 | ✅ **A-04** `starts_with`/`ends_with` sobre campo no-string → `false` silencioso | El mismo modo de fallo que E26-H08 cerró para el orden; `eval.rs` reconoce el hueco sin test | **Ejecutado**: `E29-H04` (`b3b79fb`) — type error ruidoso, alineado con E26-H08 | **3** |
| 6 | ✅ **A-08** sintaxis de `validation` por familias, clave desconocida inerte | La mitad «clave desconocida» ya está decidida en §16(e) (config estricta); falta **documentar las familias** (`danglingDocumentLinks`…) que hoy solo existen en `config.rs` | **Absorbido**: mitad de rechazo por `E29-H01` (`4a52f59`, `§16(e)`) · familias en `docs/user/` para [`§19`](19-hallazgos-referencia-usuario.md) | **3** |
| 7 | ✅ **D-02** `patch_frontmatter`: §20.4 promete null-vs-remove, el wire es RFC 7386 | ARCHITECTURE se contradice internamente; el brazo `Some(Null)` del core es inalcanzable desde MCP | **Ejecutado**: `E30-H03` (`0ef66d2`) — criterio ratificado (corregir §20.4). Su nota histórica sobre el primer nivel quedó **superada por #46**: RFC 7386 se aplica recursivamente; `null` borra la clave del objeto que lo contiene y un array sigue siendo atómico. El `Some(Null)` interno continúa siendo una distinción de core inalcanzable desde el wire | **3** |
| 8 | ✅ **A-01** `sections` omite en silencio el heading sin match | Body acotado indistinguible de «todas las secciones existían»; solo lo fija un doc-comment del core | **Ejecutado**: `E30-H03` (`0ef66d2`) — omisión declarada en `mcp.yml` y `mcp-clients.md`, con la consecuencia explícita: si ningún headingPath casa, `body` es la cadena vacía, indistinguible de una sección vacía | **2** |
| 9 | ⚠️ **A-06** `replace_text` 0-ocurrencias sin aserción → plan no-op | El vacío-sin-error solo está documentado para selecciones masivas | **Documentado** en `E30-H03` (`docs/user/safe-changes.md`), pero la verificación **destapó un defecto real distinto**: el no-op **reescribe el fichero** —normaliza a `replace_body` de documento entero y reserializa el frontmatter (`tags: [a, b]` de flow a bloque), con `semanticDiff.modified` no vacío y `bodyChanges`/`frontmatterChanges` vacíos. Fuera del alcance de H03 por causa raíz distinta; promovido a ficha propia: **[`§26`](26-replace-text-noop-reserializa.md)** | **2** |
| 10 | ✅ **A-07** `knowledge_check` scope `paths` traga paths inexistentes | Un typo desaparece; la enumeración de errores excluye `paths` de `DOCUMENT_NOT_FOUND` a propósito o por omisión | **Ejecutado**: `E29-H05` (`fc5c26b`) — `DOCUMENT_NOT_FOUND`, coherente con `document`/`affected` | **2** |
| 11 | ✅ **A-09** la config se lee una vez por sesión | Un `config.yaml` escrito con el servidor vivo no se aplica; solo lo fija un comentario de `lib.rs` | **Ejecutado**: `E30-H03` (`0ef66d2`) — ciclo de vida declarado en `mcp.yml` y `mcp-clients.md`: se lee al abrir y queda fijo toda la sesión; una config ilegible impide arrancar, no degrada en silencio | **2** |
| 12 | ✅ **A-10** «path que normaliza a un directorio» (mcp.yml L149-151) | Impreciso: solo la raíz da `workspaceDirectory`; un directorio con nombre es `missing` | **Ejecutado**: `E30-H03` (`0ef66d2`) — redacción precisada y **verificada contra `core::links::clasificar`**: el criterio es que no sobreviva ningún segmento con nombre a la normalización; un directorio con nombre da `missing` | **1** |
| 13 | Registro sin acción: 12 esperados nuestros refutados | Lecturas erróneas del contrato que la verificación adversarial corrigió (sección 4 del informe) | Ninguno — material de onboarding para quien escriba contra el contrato | — |

## Cierre (2026-08-07)

**Los 12 subpuntos accionables están ejecutados**, en la campaña de bugfixes
documentada en [`docs/qa/campana-bugfixes-2026-08.md`](../docs/qa/campana-bugfixes-2026-08.md):
Fase 0 (épica `E28`, defectos destructivos M-01 y A-05), Fase 1 (épica `E29`,
honestidad de superficie, 11 historias) y Fases 2-3 (épica `E30`, higiene y
escoba, 3 historias). El punto 13 no pedía acción. Cada bug de comportamiento
tiene test de regresión con evidencia rojo→verde y veredicto de juez ciego.

Los tres puntos que pedían criterio se resolvieron **como recomendaba esta
ficha**, ratificados por el usuario el 2026-08-06: A-04 → type error ruidoso ·
D-02 → corregir §20.4 y declarar RFC 7386 · A-07 → `DOCUMENT_NOT_FOUND`.

**Tres correcciones que la ejecución impuso sobre lo que esta ficha suponía** —
se registran porque el valor del dogfooding está justamente en esto:

1. **A-06 era la punta de otro defecto.** El no-op documentado es real, pero
   verificarlo destapó que un `replace_text` sin coincidencias **reescribe el
   fichero** y reserializa el frontmatter. Causa raíz distinta, historia propia.
2. **La divergencia de `workspace_status.counts`** (seguimiento nacido en E29-H06)
   se registró con una hipótesis **falsa** —«diagnósticos sin target»—, refutada
   ejecutando: el criterio real es que el fichero nunca entra al inventario.
3. **A-02/A-03 dejó una regresión de robustez** al firmar los cursores: uno
   no-ASCII hacía `panic!` y se llevaba la sesión JSON-RPC entera. Lo cazó el
   juez ciego de robustez, no la suite.
4. **El test de guardia de A-06 aseveraba la negación del defecto que su propio
   commit documentaba**, y solo pasaba por un accidente del fixture (`alfa.md`
   no tenía frontmatter en estilo flow). El juez lo demostró añadiendo
   `tags: [a, b]` al fixture: el test falla. Un test que finge que un defecto
   conocido no existe es peor que no tenerlo, porque congela lo accidental.

Seguimientos abiertos que **sobreviven a este cierre**, cada uno con dueño:
[`§25`](25-superficie-muerta-revert-transaction.md) (`Workspace::revert_transaction`
es superficie pública sin llamadores), [`§26`](26-replace-text-noop-reserializa.md)
(el `replace_text` no-op que reserializa el frontmatter, nacido de verificar A-06),
[`§24`](24-equivalencia-caja-unicode.md) (equivalencia de paths por caja/Unicode), y
las dos familias preexistentes de normalización registradas en la épica E28. El testbench queda como activo de [`§9`](09-transversales-diferidas.md):
re-ejecutable contra cada release con `docs/qa/testbench/`.

## Lo que exigía criterio (lo demás era trabajo)

Tres puntos pidieron decisión antes de ejecutarse; el resto ya tenía destino:

- **A-04**: ¿type error ruidoso u `Ok(false)` documentado? Recomendación:
  **ruidoso** — es exactamente la clase de lista-recortada-indistinguible que
  E26-H08 declaró cerrada, y la coherencia del lenguaje vale más que la
  compatibilidad con un comportamiento que ningún test fijaba.
- **D-02**: ¿corregir §20.4 o ampliar el wire? Recomendación: **corregir
  §20.4** y declarar RFC 7386; ampliar el wire solo si aparece un caso real de
  asignar `null` explícito (no lo hubo en 189 casos).
- **A-07**: ¿`DOCUMENT_NOT_FOUND` en `paths` o tolerancia declarada?
  Recomendación: **error**, por simetría con `document`/`affected` y por el
  principio anti-typo de §22.

## Efecto sobre el orden de trabajo del README

M-01 entra **por delante** de la épica de honestidad (bug con pérdida de
datos, arreglo acotado junto a §16(i)). A-05 entra como segunda historia. Los
puntos D-01, A-04, A-07 engordan la épica de honestidad ya acordada; A-02/A-03
engordan el ciclo de higiene; el resto es §19. El testbench en sí queda como
activo de §9 (banco de pruebas): re-ejecutable contra cada release con
`docs/qa/testbench/`.

> **Nota cruzada (2026-08-06, re-jueces ciegos de E28-H04)**: la verificación de la adenda de E28
> localizó dos familias de defecto **preexistentes** (resurrección de paths liberados por
> operaciones de contenido tras `delete`/`move`, y move-chains por ocupación del origen) que
> comparten causa raíz con A-05 pero quedan fuera de su arreglo. Registradas con detalle en la
> sección «Hallazgos preexistentes registrados» de
> [`requirements/epica-28-defectos-destructivos-testbench.md`](../requirements/epica-28-defectos-destructivos-testbench.md);
> no se numeran como punto nuevo de esta tabla, quedan como candidato a priorizar junto al resto de
> hallazgos pendientes cuando se retome esta ficha.
