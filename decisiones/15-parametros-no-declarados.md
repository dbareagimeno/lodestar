---
id: 15
titulo: "¿Debe el servidor rechazar los parámetros que no declara?"
estado: "abierta"
prioridad: 4
etiquetas: ["contrato", "mcp", "configuracion"]
origen: "auditoria"
abierta_en: "2026-07-25"
revisada_en: "2026-07-25"
epica: "E24"
historias: ["E24-H09"]
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
  razonamiento no es trivial: `operacion_item_schema()` declara **18 propiedades planas a
  propósito** —sin `oneOf` por operación— porque un `oneOf` mal escrito rechazaría entradas
  válidas. Activar `additionalProperties` en ejecución sin resolver eso primero rompería `create`
  con campos de otra op.
- **Qué decidir**: (a) **ejecutar** lo que el schema declara, resolviendo antes el `oneOf` por
  operación; (b) **dejar de declararlo** — quitar `additionalProperties: false` de los schemas, de
  modo que la superficie deje de afirmar lo que no cumple, a costa de que el cliente ya no valide;
  (c) **declararlo como tolerancia deliberada** y documentarlo en las `instructions` del servidor,
  para que un agente sepa que un parámetro inventado se descarta.
- **Recomendación**: **(a)**, en la misma épica que E24-H07/H08 (v0.4.0), porque las tres tocan la
  misma superficie de entrada y comparten el criterio de fondo: *lo que el motor no entiende, lo
  dice*. Hoy no es un bug de datos —nada se corrompe—, pero sí una respuesta silenciosamente
  equivocada, que es la clase de defecto que esta épica ha estado cerrando.
