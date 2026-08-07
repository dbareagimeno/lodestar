# Decisiones

Esta carpeta recoge las decisiones que **no se pueden tomar por inercia** desde el código o
`ARCHITECTURE.md` y que dependen de tu criterio de producto/entorno. Cada decisión es un fichero
`.md` con frontmatter YAML; el número `§N` sigue siendo el identificador estable de cada una, y lo
que antes vivía en el encabezado del documento único (estado, prioridad, revisión) vive ahora en
ese frontmatter, consultable por el propio motor. Nada aquí bloquea lo ya implementado (el motor
headless E9–E27 está completo y testeado); son decisiones para cerrar el último tramo del proyecto
y para afinar comportamiento.

## Cómo consultarlas

El frontmatter de cada fichero es consultable con el propio motor Lodestar: `knowledge_search` con
`where` (y `metadata_inspect` para el catálogo de propiedades). Algunos ejemplos:

```
prioridad >= 4 and estado = "abierta"
# lo que exige criterio hoy → devuelve 3: §9, §14, §20

estado = "abierta"
# todo lo vivo

etiquetas contains "contrato"
# por tema

revisada_en < "2026-07-01" and estado = "abierta"
# decisiones abiertas que nadie toca

reabrible = true
# cerradas con una mitad que puede volver (§17)
```

La prioridad va de **1 a 5, donde 5 es lo más importante**. El lenguaje no tiene `sort`: la
prioridad **filtra**, no ordena. El vocabulario de `estado` es cerrado: `abierta`, `tomada`,
`ratificada`, `cerrada`, `diferida`, `obsoleta`.

## Índice

| § | Decisión | Estado | Prio | Doc |
|---|---|---|---|---|
| 0 | Giro a motor headless de integridad semántica | ratificada | 2 | [`00-giro-headless.md`](00-giro-headless.md) |
| 1 | Firma/notarización de binarios (ex-fachada Tauri) | diferida | 3 | [`01-build-fachada-escritorio.md`](01-build-fachada-escritorio.md) |
| 2 | Port de la UI del prototipo | obsoleta | 1 | [`02-port-ui-prototipo.md`](02-port-ui-prototipo.md) |
| 3 | Transporte MCP: stdio propio frente a rmcp | abierta | 2 | [`03-transporte-mcp-rmcp.md`](03-transporte-mcp-rmcp.md) |
| 4 | Generación del `.d.ts` desde Rust | obsoleta | 1 | [`04-generacion-dts-ts-rs.md`](04-generacion-dts-ts-rs.md) |
| 5 | i18n multi-idioma | cerrada | 1 | [`05-i18n-multi-idioma.md`](05-i18n-multi-idioma.md) |
| 6 | Semántica de `merge` local | obsoleta | 1 | [`06-semantica-merge-local.md`](06-semantica-merge-local.md) |
| 7 | `lodestar check --range a..b` | obsoleta | 1 | [`07-check-range.md`](07-check-range.md) |
| 8 | Esquema de `lodestar.toml` | obsoleta | 1 | [`08-esquema-lodestar-toml.md`](08-esquema-lodestar-toml.md) |
| 9 | Transversales: bench · firma · threat model | abierta | **4** | [`09-transversales-diferidas.md`](09-transversales-diferidas.md) |
| 10 | Ghosts como primitiva de planificación + templates | abierta | 3 | [`10-ghosts-y-templates.md`](10-ghosts-y-templates.md) |
| 11 | `pulldown-cmark` en `lodestar-core` | tomada | 1 | [`11-pulldown-cmark-en-core.md`](11-pulldown-cmark-en-core.md) |
| 12 | Comparación de fechas en el lenguaje de consulta | cerrada | 1 | [`12-fechas-en-consultas.md`](12-fechas-en-consultas.md) |
| 13 | `Conformant → Valid` | cerrada | 1 | [`13-conformant-a-valid.md`](13-conformant-a-valid.md) |
| 14 | El store (E18) no tiene ningún consumidor | abierta | **5** | [`14-store-sin-consumidor.md`](14-store-sin-consumidor.md) |
| 15 | ¿Rechazar los parámetros no declarados? | cerrada | 4 | [`15-parametros-no-declarados.md`](15-parametros-no-declarados.md) |
| 16 | Deuda declarada por la auditoría de E25/E26 | cerrada (disuelta) | 4 | [`16-deuda-auditoria-e25-e26.md`](16-deuda-auditoria-e25-e26.md) |
| 17 | Superficie externa y apertura OSS | cerrada | 3 | [`17-superficie-externa-oss.md`](17-superficie-externa-oss.md) |
| 18 | `canApply: false` no vincula a `change_apply` | cerrada | 4 | [`18-canapply-no-vincula-apply.md`](18-canapply-no-vincula-apply.md) |
| 19 | Hallazgos de documentar la referencia de usuario | cerrada | 5 | [`19-hallazgos-referencia-usuario.md`](19-hallazgos-referencia-usuario.md) |
| 20 | Renombrado del proyecto | abierta | **5** | [`20-renombrado-del-proyecto.md`](20-renombrado-del-proyecto.md) |
| 21 | Comillas en el lenguaje de consulta | tomada | 3 | [`21-comillas-lenguaje-consulta.md`](21-comillas-lenguaje-consulta.md) |
| 22 | Integridad referencial de los valores del frontmatter | abierta | 3 | [`22-integridad-referencial-frontmatter.md`](22-integridad-referencial-frontmatter.md) |
| 23 | Hallazgos del testbench MCP sobre el homelab | abierta | **5** | [`23-hallazgos-testbench-homelab.md`](23-hallazgos-testbench-homelab.md) |
| 24 | Equivalencia de paths por caja/Unicode en el guard de colisión | abierta | 3 | [`24-equivalencia-caja-unicode.md`](24-equivalencia-caja-unicode.md) |

## Dónde está el criterio hoy

> **Repriorización conjunta del 2026-08-02.** Las prioridades anteriores se habían asignado con el
> criterio «cuánta deuda es». Tras E27 el eje cambió: el repo es público, `docs/user/` está
> publicado y `§21.5` ratificó que *la superficie externa solo promete lo que el motor ejecuta hoy*.
> Con ese criterio el orden se reordena solo. Cambios de esta pasada: **§19 sube a 5** · **§9 sube a
> 4** (el banco de pruebas pasa a ser condición de entrada de §14) · **§1 sube a 3** y queda
> congelada · **§16 se disuelve** en sus dueños reales · **§5 se cierra** (resuelta por la decisión
> de idioma partido) · **§20 y §21 son nuevas**.

> **Pasada del testbench (2026-08-06).** El banco de pruebas que §9 pedía existe y corrió: 189
> casos contra el homelab real ([informe](../docs/qa/informe-homelab-2026-08-06.md)). **§23 es
> nueva** y nace ya disuelta en dueños: un bug de motor con pérdida de datos (M-01, entra por
> delante de todo), un guard de colisión que falta (A-05), evidencia nueva para la épica de
> honestidad (D-01, A-04, A-07) y el ciclo de higiene (A-02/A-03), y material de §19 (el resto).

**Lo único que sigue exigiendo criterio tuyo** son estas fichas abiertas:

- **§20 — renombrado del proyecto** (prio 5). Falta elegir nombre y verificar disponibilidad.
  **Congela** la firma de binarios (§1/§9) y crates.io (§17-DA): no se firma ni se reserva nada bajo
  un nombre que va a cambiar.
- **§14 — el store sin consumidor** (prio 5). Sigue gobernando a las demás, pero ya **no se decide a
  ciegas**: su condición de entrada es el banco de pruebas de §9. Absorbe §16(c) y §16(l/E26-H09).
- **§9 — banco de pruebas** (prio 4). Ya tiene primera entrega (el testbench de §23,
  re-ejecutable en `docs/qa/testbench/`); falta convertirlo en banco permanente por release.
- **§23 — hallazgos del testbench** (prio 5 por M-01). Tres subpuntos piden criterio —
  A-04 (¿type error ruidoso?), D-02 (¿corregir §20.4 o ampliar el wire?), A-07
  (¿`DOCUMENT_NOT_FOUND` en scope `paths`?) — con recomendación escrita en la ficha; el resto
  ya tiene dueño.

**Ya decididas, pendientes de ejecutar** (`estado: tomada` — son trabajo, no criterio): §21. §15,
§18 y §19 —más los puntos (b), (e), (f) y (g) de §16— formaban la **épica de honestidad de
superficie**; **cerradas el 2026-08-07** por `E29` (11/11 historias, pendiente de merge).

**§3, §10, §22, §24** son decisiones vivas de baja urgencia: tienen recomendación estable (o, en
el caso de §24, opciones abiertas sin urgencia) y esperan un caso real que las fuerce (§3 absorbió
además el problema de *timeout*/cancelación de §16(d)). §22 y §24 son hallazgos salidos del
dogfooding, así que su sitio natural es dentro de la épica de evidencia (§9); §24 nació de la
verificación por jueces ciegos de la adenda correctiva de E28 (`E28-H04`) y no bloquea esa épica.

**§2, §4, §6, §7, §8** son archivo: la pregunta ya no tiene sujeto porque el código que la motivaba
se borró (la UI de escritorio, el espejo TS, el merge local, `--range`, `lodestar.toml`). Se
conservan por trazabilidad, no por vigencia. **§5** se les une por otra vía: la contestó la decisión
de idioma partido, no la ficha.

## Orden de trabajo acordado (2026-08-02, revisado 2026-08-06 tras el testbench)

0. ✅ **§23/M-01 — revert del recibo `-revert`** (bug de motor con pérdida de datos): historia
   inmediata y acotada, junta con §16(i) porque es la misma coreografía de sellado. Le sigue
   **§23/A-05** (guard de colisión de `create`/`move`), el único hueco destructivo restante.
   **Adenda (2026-08-06)**: los jueces ciegos que verificaron esas dos historias (`E28-H01`/`H02`)
   encontraron un bloqueante en cada una — `E28-H03` cierra la identidad de `txnId` que H01 dejó
   abierta en el `apply`; `E28-H04` cierra la normalización contra estado acumulado que H02 dejó
   pendiente. `E28-H04` abre además `§24` (equivalencia de caja/Unicode), fuera de su alcance.
   **Ejecutado (2026-08-06)**: `E28` completa — H01+H02 (`043f233`/`296147b`), adenda H03+H04
   (`8c86b6b`), cierre de reservas de los re-jueces (`c532929`); detalle en
   [`docs/qa/campana-bugfixes-2026-08.md`](../docs/qa/campana-bugfixes-2026-08.md) e
   [`IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md). **Siguiente**: la épica de honestidad
   `E29` (ya ratificada).
1. ✅ **Épica de honestidad de superficie** — todo lo que la superficie documentada afirma y el motor
   no ejecuta: §19(a), §19(b), §18 vinculante, §16(f) aviso de workspace vacío, §16(e) config
   estricta (absorbe §23/A-08-rechazo), §15 wire estricto (con la tabla de campos por operación
   como primer criterio de aceptación), §16(g) cerrar la API no transaccional, §16(b) retirar el
   `Envelope`, y del testbench: §23/D-01 (`instructions` por perfil + `protocolVersion`),
   §23/A-04 y §23/A-07 (si se decide ruido).
   **Ejecutado (2026-08-07)**: `E29` completa — 11/11 historias en `feat/e29-honestidad-superficie`,
   todas con juez ciego favorable y remates saldados (H10/H11 en verificación final); detalle en
   [`IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md) y
   [`docs/qa/campana-bugfixes-2026-08.md`](../docs/qa/campana-bugfixes-2026-08.md). **Siguiente**: el
   ciclo de higiene (punto 2, §16(j) ampliado con §23/A-02/A-03) y la historia-escoba de
   `docs/qa/campana-bugfixes-2026-08.md` (Fase 3: D-02, A-01, A-06, A-09, A-10 y los seguimientos
   nuevos registrados al cerrar la Fase 1).
2. **Ciclo de higiene** — §16(i) coreografía única de sellado (ver punto 0), §16(j) cursor
   inválido **ampliado con §23/A-02/A-03** (cursor ajeno), §16(l) pasada acotada de `/mutantes`.
3. **Épica de evidencia** — banco de pruebas (§9) + dogfooding; §23 aporta la primera corrida
   completa y el arnés re-ejecutable. **Cierra §14 con datos.** Los nits documentales de §23
   (D-02, A-01, A-06, A-09, A-10) van con §19 según toque cada fichero.
4. **§20 renombrado del proyecto** (alcance total, incluido `.lodestar/`, con migración). Descongela
   §1/§9-firma y §17-DA.
5. **§21 comillas en el lenguaje de consulta** (puerta de diseño propia).
6. **§10 ghosts + templates** (puerta de diseño, ya con el dogfooding hecho).

Sueltas y sin épica asignada: **§16(k)** (matriz de trazabilidad, historia propia, delegable) y
**§16(h)** (registrada sin acción).
