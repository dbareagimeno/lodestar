---
id: 15
titulo: "¿Debe el servidor rechazar los parámetros que no declara?"
estado: "cerrada"
prioridad: 4
etiquetas: ["contrato", "mcp", "configuracion"]
origen: "auditoria"
abierta_en: "2026-07-25"
cerrada_en: "2026-08-07"
revisada_en: "2026-08-07"
epica: "E29"
historias: ["E24-H09", "E29-H08"]
relacionadas: [16, 19]
---

# §15 — ¿Debe el servidor RECHAZAR los parámetros que no declara?

- **Contexto**: `contracts/mcp.yml` enuncia la **regla de la casa** de la superficie MCP: *«el
  servidor valida los VALORES de los parámetros que declara, e IGNORA lo que no declara»*. La
  primera mitad **ya se cumple** desde `E24-H09`, que era donde estaba el defecto real (`limit: 0`
  devolvía 0 resultados en silencio pese a que el schema declara `minimum: 1`).
- **Lo que queda abierto es la segunda mitad**, y es una **deuda que el propio contrato declara**:
  todos los `inputSchema` anuncian `additionalProperties: false`, y **el servidor no lo ejecuta**.
  O sea, la superficie afirma algo que no cumple — exactamente el defecto que E23 vino a saldar,
  aquí en su forma más pequeña.
- **Medido** (revisión de la v0.3.0, sonda 4): 15 casos aceptados en silencio, entre ellos un
  `sort` retirado en E23-H11, un `offset` que no existe y typos como `wheres`/`filters`. Un agente
  que se equivoca de nombre de parámetro no se entera: recibe la respuesta por defecto.
- **Por qué NO se cerró en E24**: no es un bugfix, es **revisar un criterio ratificado**. La
  política vigente está escrita en tres sitios (`contracts/mcp.yml` `validacion_de_argumentos`, la
  cabecera de `tests/descubribilidad.rs`, y la justificación del schema plano en `tools.rs`), y su
  razonamiento no es trivial: `operacion_item_schema()` declara **17 propiedades planas a
  propósito** —sin `oneOf` por operación— porque un `oneOf` mal escrito rechazaría entradas
  válidas. Activar `additionalProperties` en ejecución sin resolver eso primero rompería `create`
  con campos de otra op. (La ficha decía **18**: era el recuento previo a `E23-H11`, que retiró
  `fixId` junto con su op. Verificado contra `tools.rs:61-84` el 2026-08-02.)
- **Qué decidir**: (a) **ejecutar** lo que el schema declara, resolviendo antes el `oneOf` por
  operación; (b) **dejar de declararlo** — quitar `additionalProperties: false` de los schemas, de
  modo que la superficie deje de afirmar lo que no cumple, a costa de que el cliente ya no valide;
  (c) **declararlo como tolerancia deliberada** y documentarlo en las `instructions` del servidor,
  para que un agente sepa que un parámetro inventado se descarta.
- **DECIDIDO (2026-08-02): (a) ejecutar**, con el mismo criterio estricto que
  [`§16(e)`](16-deuda-auditoria-e25-e26.md) aplica al fichero de config — el repo no se queda con
  dos criterios opuestos según si lo desconocido llega por el wire o por disco. Entra en la **épica
  de honestidad de superficie**, pero en **historia separada** de §16(e): esa es barata y cierra
  una salvaguarda silenciosa, esta es la mayor de la épica y no debe arrastrarla si se complica.
- **Primer criterio de aceptación, y condición de entrada**: fijar por tests la **tabla de campos
  legales por operación**, antes de activar ningún rechazo. Las 7 ops y sus campos (fuente:
  `normalize_raw_op` en `lodestar-app`, que es de donde salen los nombres):

  | Campo | Ops | |
  |---|---|---|
  | `op` | todas | discriminador, obligatorio |
  | `path` | todas | ruta relativa; obligatoria en `create`, alternativa corta a `ref.path` en el resto |
  | `ref` | todas menos `create` | forma larga de `path` |
  | `expectedRevision` | todas | control de concurrencia optimista |
  | `frontmatter` | `create` | |
  | `body` | `create`, `replace_body` | **compartido entre dos ops** |
  | `patch` | `patch_frontmatter` | |
  | `find`, `replace`, `expectedOccurrences` | `replace_text` | |
  | `headingPath`, `mode`, `content` | `edit_section` | |
  | `from`, `to`, `rewriteInboundLinks` | `move` | |
  | `inboundLinksPolicy` | `delete` | |

  El riesgo no es teórico y está en esa tabla: `path`/`ref` son intercambiables salvo en `create`, y
  `body` pertenece a dos ops. Un agente que hoy reutiliza la misma plantilla de objeto para varias
  operaciones de un lote —perfectamente válido— empezaría a recibir rechazos si la partición se
  escribe como si fuera limpia.

**Cerrada (2026-08-07) por `E29-H08`** (commits `f7dc5fd` + `f720ba8`, épica
[`epica-29-honestidad-superficie.md`](../requirements/epica-29-honestidad-superficie.md)): el wire
rechaza de verdad los parámetros que no declara, validando por unión contra la tabla de campos
legales por operación fijada arriba; juez ciego APROBADA (11/11) tras el remate que saldó la deuda
señalada y la cascada a los sub-objetos de operación.
