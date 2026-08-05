---
id: 23
titulo: "Hallazgos del testbench MCP sobre el homelab"
estado: "abierta"
prioridad: 5
etiquetas: ["escritura", "mcp", "contrato", "docs", "dogfooding", "lenguaje-consulta"]
origen: "dogfooding"
abierta_en: "2026-08-06"
revisada_en: "2026-08-06"
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
| 1 | **M-01** revert de un recibo `-revert` | **Bug de motor con pérdida de datos**: no-op silencioso que responde `reverted: true` y sobrescribe las copias del redo | **Historia propia, inmediata** — misma zona que §16(i) (coreografía de sellado): arreglarlos juntos | **5** |
| 2 | **A-05** `create`/`move` sobre path ocupado | `canApply: true` sin fricción; aplicado, pisa conocimiento. Único hueco con riesgo destructivo para un agente | **Historia propia** — guard de colisión en la normalización (`INVALID_SCHEMA` o código nuevo) + declararlo en el contrato | **4** |
| 3 | **D-01** `instructions` nombra 10 tools bajo `readonly` (+ `protocolVersion` sin validar) | La superficie afirma lo que el perfil no sirve — la definición literal de la **épica de honestidad de superficie** | Épica de honestidad (junto a §19(a/b), §18, §15, §16(e/f/g/b)) | **4** |
| 4 | **A-02/A-03** cursor malformado → offset 0 · cursor ajeno aceptado | Ya **decidido** en §16(j) (`INVALID_SCHEMA`); el testbench aporta la repro y descubre la variante cross-tool | **Ciclo de higiene** §16(j), ampliando su alcance: un cursor válido-en-forma pero de otro espacio también reinicia en silencio | **3** |
| 5 | **A-04** `starts_with`/`ends_with` sobre campo no-string → `false` silencioso | El mismo modo de fallo que E26-H08 cerró para el orden; `eval.rs` reconoce el hueco sin test | **Decisión de producto aquí**: alinear con E26-H08 (type error ruidoso, recomendado por coherencia) o fijar el `false` en `query-language.md`. Ejecuta: épica de honestidad | **3** |
| 6 | **A-08** sintaxis de `validation` por familias, clave desconocida inerte | La mitad «clave desconocida» ya está decidida en §16(e) (config estricta); falta **documentar las familias** (`danglingDocumentLinks`…) que hoy solo existen en `config.rs` | §16(e) para el rechazo · [`§19`](19-hallazgos-referencia-usuario.md) para las familias en `docs/user/` | **3** |
| 7 | **D-02** `patch_frontmatter`: §20.4 promete null-vs-remove, el wire es RFC 7386 | ARCHITECTURE se contradice internamente; el brazo `Some(Null)` del core es inalcanzable desde MCP | **Decisión de producto aquí**: corregir §20.4 (RFC 7386, recomendado: nadie ha pedido asignar null) o añadir sintaxis de remove al wire. Ejecuta: §19 | **3** |
| 8 | **A-01** `sections` omite en silencio el heading sin match | Body acotado indistinguible de «todas las secciones existían»; solo lo fija un doc-comment del core | §19 — declarar la omisión en `mcp.yml` y `mcp-clients.md` (o decidir ruido, pero la omisión es defendible) | **2** |
| 9 | **A-06** `replace_text` 0-ocurrencias sin aserción → plan no-op | El vacío-sin-error solo está documentado para selecciones masivas | §19 — fijar el no-op en el contrato; la aserción `expectedOccurrences` ya es la vía ruidosa opt-in | **2** |
| 10 | **A-07** `knowledge_check` scope `paths` traga paths inexistentes | Un typo desaparece; la enumeración de errores excluye `paths` de `DOCUMENT_NOT_FOUND` a propósito o por omisión | **Decisión de producto aquí**: `DOCUMENT_NOT_FOUND` (recomendado, coherente con `document`/`affected`) o declarar la tolerancia. Ejecuta: épica de honestidad | **2** |
| 11 | **A-09** la config se lee una vez por sesión | Un `config.yaml` escrito con el servidor vivo no se aplica; solo lo fija un comentario de `lib.rs` | §19 — declarar el ciclo de vida de la config en `docs/user/`; el comportamiento en sí es razonable | **2** |
| 12 | **A-10** «path que normaliza a un directorio» (mcp.yml L149-151) | Impreciso: solo la raíz da `workspaceDirectory`; un directorio con nombre es `missing` | §19 — nit de redacción | **1** |
| 13 | Registro sin acción: 12 esperados nuestros refutados | Lecturas erróneas del contrato que la verificación adversarial corrigió (sección 4 del informe) | Ninguno — material de onboarding para quien escriba contra el contrato | — |

## Lo que exige criterio (lo demás es trabajo)

Tres puntos piden decisión antes de ejecutarse; el resto ya tiene destino:

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
