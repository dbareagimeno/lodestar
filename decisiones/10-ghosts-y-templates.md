---
id: 10
titulo: "Ghosts como primitiva de planificación y templates"
estado: "abierta"
prioridad: 3
etiquetas: ["grafo", "mcp", "ui"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
revisada_en: "2026-08-02"
relacionadas: [2, 14, 21]
---

# §10 — Ghosts como primitiva de planificación + templates

> **Parcialmente superada por §20 (2026-07-23)**: la primitiva **sobrevive y de hecho mejora** — un
> ghost es un enlace a un `.md` inexistente, que en el modelo nuevo es un `LinkTarget::Missing`
> (`§20.6`) con su `dangling` identificando origen y href crudo (`§20.7`), más informativo que el
> `LINK-STUB` de antes. Lo que **muere** son las piezas OKF de la propuesta: el gesto de UI (la UI se
> retiró de `main`) y los *templates por `type`* con `.lodestar/templates/` (`core::schema` se borra
> en E20; `§20` no tiene tipos documentales). Si se retoma, el backlog de ghosts se lee hoy con
> `graph_query(dangling)`.

- **Contexto**: los *ghosts* («por escribir») ya existen y están portados: nodo con `ghost: bool` en
  `GraphModel` (`core/graph.rs`) derivado de enlaces a `.md` inexistentes, check `LINK-STUB` con
  severidad **info** (no rompe `check`). Dan un modelo de estados gratis y no falseable:
  ghost = planificado · existe-pero-no-conforme = en curso · conforme = hecho. Todo derivado de los
  `.md` en disco (invariante #1), sin campo `status:` que mantener.
- **Qué se quiere** (acordado como dirección, pendiente de diseño):
  1. **Crear ghosts desde la UI**: gesto de «esto habrá que crearlo». Para no introducir estado
     nuevo, «crear un ghost» debe materializarse como **insertar un enlace** en una página existente
     (la actual, o una página-plan por convención) — el ghost sigue siendo 100% derivado.
  2. **Tool MCP para leer ghosts** (`list_ghosts` o similar): ghosts con sus backlinks e in-degree
     (cuántas páginas lo reclaman = prioridad), para que un agente consuma el backlog y vaya creando
     páginas conformes siguiendo el plan. El contexto/spec de cada ghost es la prosa alrededor de
     los enlaces que le apuntan.
  3. **Templates**: plantillas tanto de **archivos sueltos** (esqueleto de frontmatter/cuerpo por
     `type`) como de **directorios** (estructura de páginas planificadas — posiblemente expresable
     como una página-plan que genera los ghosts de toda la estructura).
- **Qué decidir cuando se aborde**: UX del gesto en la UI (¿desde el grafo?, ¿desde autocompletado
  de enlaces?), dónde viven los templates (¿`.lodestar/templates/`?, ¿páginas especiales?), si el
  template de directorio crea ghosts (solo plan) o stubs (archivos reales), y la firma exacta de la
  tool MCP.
- **Recomendación**: mantener el principio «ghost = derivado de enlaces»; cualquier variante que
  requiera una lista de ghosts persistida aparte contradice el invariante #1.

## Orden fijado el 2026-08-02

Es **la más grande de las dos piezas de capacidad nueva** sobre la mesa y va **la última**:
detrás de [`§21`](21-comillas-lenguaje-consulta.md) (comillas en el lenguaje, pequeña y cerrada) y
detrás de la **épica de evidencia** (banco de pruebas + dogfooding). El motivo no es de coste sino
de diseño: §10 propone una primitiva de **planificación**, y su puerta de diseño se decide mucho
mejor con el dogfooding hecho —usar el motor de verdad sobre `decisiones/` y `requirements/`
enseña qué backlog se quiere consumir realmente— que en abstracto. Sigue siendo **prioridad 3** y
sigue siendo la dirección de producto acordada.
