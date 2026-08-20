# Roadmap

> Dirección de producto de Lodestar a partir de **v0.6.2 (2026-08-20)**.
>
> Este documento fija **hacia dónde va el proyecto y en qué orden**. No sustituye a
> `ARCHITECTURE.md` (diseño ratificado), `decisiones/` (criterio de producto pendiente o tomado),
> `requirements/` (épicas e historias ejecutables) ni `IMPLEMENTATION_STATUS.md` (estado real).
> Cuando una fase de este roadmap vaya a ejecutarse, se diseña y descompone mediante el workflow
> normal del repo; el roadmap no congela por adelantado IDs de épicas, nombres de tools ni wire MCP.

## Principios que ninguna fase puede romper

1. **Markdown universal por defecto.** Un workspace sin configuración, tipos ni frontmatter debe
   seguir siendo válido y utilizable.
2. **El disco es la fuente de verdad.** SQLite, índices, embeddings y cualquier otra persistencia
   derivada deben poder borrarse y reconstruirse.
3. **SQLite es acelerador, no autoridad.** Cuando core y cache pudieran discrepar, gana la semántica
   canónica del core.
4. **`lodestar-core` permanece puro.** Parsers de formatos binarios, I/O, SQLite, watchers o modelos
   de embeddings no entran en core.
5. **Capacidades de dominio opt-in.** Los contratos documentales añaden garantías a quien los
   declara; nunca convierten Lodestar de nuevo en un formato documental obligatorio.
6. **Referenciar no implica interpretar.** Un PDF, DOCX, PPTX, imagen o fichero de código puede
   formar parte estructural del brain sin que Lodestar tenga que parsear su contenido.
7. **Los ghosts son intención explícita.** Un destino ausente solo es un ghost si existe una
   declaración versionada que dice que debe existir; de lo contrario es una referencia rota.
8. **Ergonomía como requisito transversal.** Cada capacidad nueva debe ser descubrible y utilizable
   por humanos y agentes sin conocer las tripas del motor.
9. **La superficie expresa intención y resultado, no lowering interno.** Las representaciones
   ejecutables que Lodestar necesita para aplicar una operación pueden permanecer privadas cuando
   exponerlas solo añade tokens, latencia o conocimiento accidental de implementación.

## Estado de partida

La v0.6.2 deja consolidado el motor headless sobre Markdown universal, el camino transaccional de
escritura, el MCP por eras y el arreglo RFC 7386 de `patch_frontmatter`. El trabajo futuro parte de
esa base; los bugs históricos ya cerrados no forman parte del roadmap.

Hay, no obstante, trabajo documentado que debe cerrarse o resolverse antes de construir las capas
nuevas:

- **E33 — banco de evidencia**: H01 (corpus reproducible) y H02 (runner asertable) están hechas;
  quedan H03–H08: centinelas, banco de rendimiento, umbrales, dogfooding, enganche a release y el
  paquete de evidencia para decidir el destino del store.
- **decisiones §20 — renombrado del proyecto**: el nombre Lodestar colisiona con otro proyecto
  conocido y el cambio acordado es total, incluido el directorio de estado `.lodestar/`.
- **decisiones §21 — comillas en el lenguaje de consulta**: decisión ya tomada para poder expresar
  claves como `frontmatter."sonar.projectKey"` sin ambigüedad.
- **issue #47 — diff de frontmatter por clave**: `semanticDiff` debe distinguir claves añadidas,
  eliminadas y modificadas, incluidas rutas anidadas. Se integra como primera mejora de la fase de
  ergonomía porque además de observabilidad es una red de seguridad frente a pérdidas silenciosas.
- **decisiones §22 — integridad referencial del frontmatter**: los campos que se declaren como
  referencias deben poder detectar destinos inexistentes.
- **decisiones §24 — equivalencia de paths por caja/Unicode**: falta decidir la política portable
  para colisiones como `Notas/A.md` frente a `notas/a.md` o NFC frente a NFD.

---

## Fase 0 — Cerrar el banco de evidencia

**Objetivo:** terminar E33-H03…H08 y obtener datos reproducibles de rendimiento y dogfooding.

Esta fase es condición de entrada para conectar el store al camino de lectura real. En particular,
se deben medir cold-open y coste por llamada con los caminos previstos por el diseño de E33, fijar
umbrales y dejar el banco ejecutable por release.

**Termina cuando:** existe el paquete de evidencia que permite decidir `decisiones §14` —el store
SQLite/FTS5 ya construido pero todavía sin consumidor en las tools— con datos y no por intuición.

## Fase 1 — Renombrado total

**Objetivo:** resolver `decisiones §20` —el cambio de nombre del proyecto por la colisión de marca—
y ejecutar la migración completa antes de aumentar más la superficie pública.

El alcance incluye repo, crates, binarios, nombre del servidor MCP, documentación, identificadores
y directorio de estado. La migración del directorio de estado debe tratar workspaces existentes sin
pérdida ni sorpresa.

**Por qué va antes del store conectado:** si el directorio derivado cambia de nombre, es mejor
resolver su ciclo de vida una vez antes de convertirlo en parte activa de todas las lecturas.

**Termina cuando:** el nombre nuevo está verificado y la compatibilidad/migración del nombre antiguo
está resuelta según la decisión ratificada.

## Fase 2 — Hardening pequeño de la superficie actual

**Objetivo:** cerrar límites conocidos, acotados y de corrección antes de la pasada específica de
ergonomía y antes de añadir nuevas primitivas de dominio.

Incluye como mínimo:

- `decisiones §21`: comillas/escape en el lenguaje de consulta;
- `decisiones §24`: resolver y ejecutar la política de colisiones por caja/Unicode;
- cualquier ajuste pequeño descubierto por el banco de evidencia que sea un defecto o límite de
  corrección y no justifique una capacidad nueva.

La issue #47 deja de tratarse como una pieza suelta de esta escoba: forma parte explícita de la fase
3 porque su valor principal es que el plan explique mejor qué ha ocurrido.

Esta fase no debe convertirse en una épica-escoba indefinida: solo recoge deuda concreta ya
identificada.

## Fase 3 — Ergonomía de la superficie actual

**Objetivo:** reducir el coste mental, de contexto y de integración de usar las capacidades que
Lodestar **ya tiene**, antes de optimizar su camino de lectura con el store o ampliar el dominio con
contratos, ghosts o búsqueda semántica.

Esta fase no añade una segunda capa de producto ni una UI. Su sujeto es la superficie existente:
MCP, CLI, schemas, respuestas y configuración. El principio rector es:

> **Lodestar debe exponer intenciones y resultados; las representaciones internas necesarias para
> ejecutarlos no deben filtrarse a la superficie salvo que aporten valor al consumidor.**

La prioridad interna es **P0 → P1 → P2**. P0 elimina ambigüedad o coste estructural del agente; P1
reduce pasos y hace el producto más observable; P2 limpia inconsistencias que merecen esperar a una
ventana adecuada de compatibilidad o distribución.

### P0 — El agente no debe conocer el lowering interno

1. **`change_plan` compacto y orientado a intención.**
   - `replace_text`, `edit_section` u otras operaciones pueden bajar internamente a terminales como
     `ReplaceBody`, pero el wire no debe repetir cuerpos completos solo porque esa sea la
     representación ejecutable del planner.
   - El runtime conserva el plan normalizado íntegro para que `change_apply(changeSetId)` pueda
     reproducir exactamente lo validado.
   - La respuesta pública ofrece la operación solicitada, paths afectados, conteos relevantes,
     `semanticDiff`, riesgo, diagnósticos y cualquier dato necesario para decidir si aplicar.
   - Debe medirse la reducción de tamaño de respuesta en casos con varios `replace_text` sobre un
     documento grande para demostrar que la mejora reduce tokens sin quitar información útil.

2. **Selección de secciones honesta en `knowledge_get`.**
   - Un `headingPath` inexistente no puede ser indistinguible de una sección legítimamente vacía.
   - El diseño debe hacer visible qué selecciones casaron y cuáles no —por ejemplo mediante
     `matchedSections`/`missingSections` o una forma equivalente— sin obligar al agente a releer el
     documento completo para desambiguar.
   - No se debe volver silencioso un error de selección por comodidad.

3. **Schemas discriminados que describan lo que realmente se ejecuta.**
   - Una variante de operación debe aceptar únicamente los campos que pertenecen a esa variante.
   - El mismo criterio se aplica donde hoy una forma depende de un discriminante: operaciones de
     `change_plan`, variantes de `graph_query`, scopes de `knowledge_check` y superficies análogas.
   - El objetivo es que un cliente que valida contra el schema pueda confiar en que una petición
     admitida por el schema no contiene campos que Lodestar vaya a ignorar.
   - No se introduce una capa DTO paralela: los schemas siguen derivados o vinculados al contrato
     canónico de la superficie.

### P1 — Observabilidad y menos pasos para humanos y agentes

4. **Issue #47 — diff de frontmatter por clave.**
   - `semanticDiff` debe distinguir al menos claves añadidas, eliminadas y modificadas mediante
     rutas estables, incluidas rutas anidadas.
   - El diff se deriva del estado inicial y final realmente simulados, no solo de la intención de la
     operación, para que también haga visibles efectos laterales inesperados.
   - Los arrays pueden tratarse inicialmente como valores atómicos; el orden de paths debe ser
     determinista.
   - Esta es la primera historia P1 porque mejora ergonomía y a la vez actúa como red de seguridad
     ante futuras regresiones del camino de escritura.

5. **`noOpOperations` expresado en términos de intención.**
   - Si el usuario pidió `replace_text`, la superficie debe poder decir que **ese `replace_text`** no
     produjo cambios, aunque internamente se hubiera normalizado a `replace_body`.
   - Si sigue siendo útil exponer el terminal normalizado para diagnóstico, debe aparecer como dato
     secundario y explícito, no sustituir silenciosamente el nombre de la intención original.
   - El comportamiento debe mantenerse por terminal cuando varias operaciones componen sobre el
     mismo documento; se mejora la explicación, no la semántica secuencial ya corregida.

6. **CLI read-only para explorar sin montar un cliente MCP.**
   - Añadir fachadas humanas finas sobre `lodestar-app` para las capacidades de lectura más útiles:
     estado, búsqueda, lectura de documento, grafo e impacto.
   - Los nombres exactos (`status`, `search`, `get`, `graph`, `impact` o equivalentes) se fijan en la
     puerta de diseño; el roadmap congela la capacidad, no la sintaxis.
   - La CLI no reimplementa lógica y debe producir los mismos resultados semánticos que MCP.
   - El camino transaccional `change_plan → change_apply → change_revert` no necesita entrar en esta
     primera ampliación de CLI: el objetivo es poder evaluar y depurar Lodestar sin otra aplicación.

7. **Configuración observable desde CLI.**
   - Debe existir una forma humana de ver la configuración efectiva y validar el fichero de
     configuración sin arrancar un cliente MCP.
   - Capacidades mínimas: mostrar la configuración resuelta y comprobar que el fichero es válido,
     con mensajes accionables.
   - No se introduce hot reload del servidor: la configuración puede seguir fijándose al arrancar;
     esta historia mejora observabilidad, no el ciclo de vida del proceso.

### P2 — Limpieza de contrato, idioma y distribución

8. **Una sola convención de referencia por path en la próxima ventana de wire.**
   - Hoy distintas partes de la superficie usan `ref.path`, `path`, `from`/`to` y formas parecidas.
     Cuando exista una razón suficiente para aceptar un cambio incompatible, se debe escoger una
     convención coherente para identidades de documento y extremos de operaciones.
   - No se añaden aliases perpetuos únicamente para suavizar esta limpieza: una migración explícita
     es preferible a mantener dos dialectos para siempre.
   - `move` puede conservar nombres semánticos de origen/destino si el diseño demuestra que son más
     claros que forzar artificialmente una única clave; la meta es consistencia conceptual, no
     uniformidad ciega.

9. **Unificar la forma de operación individual y masiva.**
   - Una operación seleccionada por query debe reutilizar el mismo vocabulario y estructura que la
     operación individual, omitiendo únicamente la identidad que aporta la selección.
   - El agente no debería aprender dos representaciones distintas de `patch_frontmatter`,
     `replace_text` o `delete` dependiendo de si actúa sobre uno o muchos documentos.
   - Se coordina con la historia de schemas discriminados y se ejecuta solo cuando la estrategia de
     compatibilidad del wire esté clara.

10. **Superficie runtime pública en inglés.**
    - README y `docs/user/` ya son ingleses, mientras que mensajes de CLI/MCP y diagnósticos siguen
      en español. Para adopción OSS, la superficie que consume un usuario externo debe converger a
      inglés.
    - No se construye un sistema de i18n: códigos, identificadores y estructura siguen siendo la
      parte estable; se traducen los mensajes públicos cuando se ejecute esta historia.
    - Specs, decisiones, requirements, comentarios de implementación y material interno pueden
      seguir en español según la política actual del repo.

11. **Instalación de un comando después del renombrado.**
    - Con el nombre definitivo ya resuelto en fase 1, evaluar y ofrecer al menos una vía sencilla de
      instalación que evite descargar, descomprimir y mover manualmente dos binarios.
    - Homebrew, `cargo-binstall`, script de instalación u otra vía son opciones de diseño, no una
      decisión congelada aquí; debe elegirse por mantenimiento y seguridad, no por número de
      canales.
    - La vía manual de GitHub Releases + checksums continúa siendo una base válida y verificable.

### No objetivos de esta fase

- construir una TUI o resucitar la UI de escritorio;
- hot reload de configuración;
- aumentar el número de tools MCP para crear aliases ergonómicos de cada operación;
- introducir `sort`, fuzzy search, embeddings u otras capacidades nuevas bajo la etiqueta de UX;
- eliminar `change_plan → change_apply`: esa fricción es deliberada y forma parte de la garantía
  transaccional;
- esconder errores reales convirtiéndolos en defaults silenciosos.

**Termina cuando:** un agente puede usar la superficie sin conocer terminales internos del planner,
los schemas describen fielmente lo ejecutable, las selecciones parciales no son ambiguas, los planes
explican los cambios de metadata y los no-ops en términos de intención, y una persona puede explorar
y diagnosticar un workspace desde la CLI sin configurar MCP. Las limpiezas P2 pueden cerrarse en la
misma épica o quedar ratificadas para la siguiente ventana incompatible si ejecutarlas en ese momento
rompiera clientes sin beneficio proporcional.

## Fase 4 — Activar el store y la cache interna

**Objetivo:** resolver `decisiones §14` conectando —si la evidencia ratifica esa opción— el
`lodestar-store` existente al camino real de lectura.

No se construye una base de datos nueva. Lodestar ya dispone de SQLite + FTS5, tablas derivadas e
infraestructura de indexación. El trabajo consiste en hacerla consumible sin violar las invariantes
actuales.

La ejecución debe cubrir como mínimo:

- paridad estricta entre descubrimiento/core/store;
- `DiscoveryPolicy` única, sin walker paralelo con semántica distinta;
- invalidación incremental por contenido/hash;
- watcher solo si aporta una semántica coherente con el motor headless;
- reconstrucción completa y segura de la cache;
- comportamiento correcto cuando la cache falta, está vieja o se corrompe;
- una única convención de `field_path` entre catálogo, consultas y store.

**Termina cuando:** las lecturas que deban beneficiarse de la cache lo hacen realmente, los gates de
E33 se mantienen y borrar el índice no cambia la semántica observable.

## Fase 5 — Contratos documentales opt-in

**Objetivo:** permitir que un brain declare reglas deterministas para familias concretas de
documentos sin abandonar Markdown universal.

Un contrato documental debe poder expresar progresivamente restricciones como:

- campos de frontmatter obligatorios;
- tipos YAML;
- enumeraciones y restricciones simples;
- referencias a otras entidades/documentos;
- cardinalidad de relaciones cuando sea útil.

Ejemplo de intención: una historia de usuario puede declarar que siempre necesita `id`, `title`,
`status` y una referencia válida a su épica, mientras que un README ajeno a ese contrato continúa
siendo un Markdown perfectamente válido.

`decisiones §22` —referencias rotas en valores de frontmatter— debe absorberse en este modelo: una
referencia se valida porque el contrato la declara como referencia, no porque Lodestar adivine por
el nombre o por el valor.

### Restricciones de diseño

- aplicar contratos es siempre explícito/opt-in;
- los contratos son fuente de verdad versionable, nunca estado exclusivo de SQLite;
- no reutilizar automáticamente `contracts/` como ubicación: esa carpeta ya significa contratos de
  frontera del propio repo (`contracts/mcp.yml`); el namespace físico de los contratos documentales
  se decide en su puerta de diseño;
- el motor debe seguir funcionando sin contratos.

**Termina cuando:** un workspace puede mezclar documentos libres y documentos contratados y
`knowledge_check`/las superficies pertinentes explican de forma determinista qué regla incumple un
documento.

## Fase 6 — Referencias tipadas a recursos

**Objetivo:** hacer explícita la naturaleza de las relaciones del brain con documentos y recursos
sin convertir todos los formatos en documentos nativos de Lodestar.

El modelo debe distinguir, como mínimo, entre:

- referencia a documento Markdown;
- referencia a otro fichero del workspace;
- referencia a URI externa;
- destino inexistente;
- y, cuando un contrato lo necesite, restricciones sobre la clase de recurso o media type.

Esto permite expresar relaciones como evidencia → PDF, diseño → PPTX o especificación → DOCX y
validar que el destino existe, está contenido cuando corresponde y satisface el tipo esperado.

### No objetivo de esta fase

**No parsear PDF/DOCX/PPTX para convertirlos en documentos del grafo.** Un recurso puede formar parte
estructural del brain sin que Lodestar conozca su contenido. Los extractores pertenecen a una fase
posterior y deben vivir fuera de `lodestar-core`.

**Termina cuando:** el grafo y los contratos pueden razonar determinísticamente sobre relaciones a
recursos sin heurísticas y sin necesidad de extraer su contenido.

## Fase 7 — Ghosts first-class + templates

**Objetivo:** convertir la planificación de conocimiento futuro en una primitiva explícita,
componible con contratos y templates.

### Ghosts

Un ghost es una **entidad planificada declarada explícitamente** cuyo target todavía puede no
existir. La ausencia de un fichero por sí sola nunca crea un ghost.

La clasificación conceptual es:

```text
referencia
├── target existe                         → entidad materializada
├── target no existe + ghost declarado    → ghost pendiente
└── target no existe + sin declaración    → referencia rota
```

La declaración del ghost es fuente de verdad versionable; su estado se deriva del workspace:

```text
ghost declarado + target ausente   = pendiente
ghost declarado + target existente = materializado
```

SQLite puede indexarlo, pero no ser su único lugar de persistencia.

Inicialmente, **ghost significa futuro documento/artefacto gestionable por el workflow de
Lodestar**, no cualquier PDF remoto que todavía no existe. La generalización a otros artefactos solo
se hará si aparece un caso real que la justifique.

Esta dirección sustituye la ambigüedad de `decisiones §10`, donde un ghost se derivaba simplemente
de un enlace `.md` inexistente. Al planificar esta fase se debe actualizar esa decisión para que la
autoridad documental y el roadmap vuelvan a coincidir.

### Templates

Un template define **cómo crear** una instancia; un contrato define **qué es válido**; un ghost
define **qué sabemos que debe existir**.

```text
Contract  → valida
Template  → materializa
Ghost     → representa intención pendiente
```

Los templates deben poder cubrir tanto documentos individuales como estructuras. Un template de
estructura puede declarar/crear ghosts para las piezas futuras sin tener que materializar stubs
vacíos.

Ejemplo conceptual para SDD:

```text
feature/
├── spec.md          ✓
├── acceptance.yaml  👻
└── test-plan.md     👻
```

Al materializar un ghost, Lodestar puede elegir el template correspondiente y validar el resultado
contra su contrato.

**Termina cuando:** un agente puede descubrir backlog planificado, distinguirlo inequívocamente de
links rotos y materializar entidades siguiendo una estructura validable.

## Fase 8 — Búsqueda híbrida

**Objetivo:** evolucionar `knowledge_search` hacia recuperación híbrida aprovechando toda la
estructura anterior, en lugar de añadir únicamente embeddings sobre chunks Markdown.

El ranking podrá combinar progresivamente:

- búsqueda léxica/FTS;
- metadata y contratos;
- filtros del lenguaje de consulta;
- proximidad y relaciones del grafo;
- tipos de referencia/recurso;
- similitud vectorial.

SQLite/store debe actuar como índice derivado de estas señales. Los embeddings son otra cache
reconstruible, nunca fuente de verdad.

**Termina cuando:** una consulta conceptual puede recuperar resultados por texto y semántica sin
perder los filtros deterministas ni la estructura explícita del brain.

## Fase 9 — Extractores de contenido opcionales

**Objetivo:** solo si la búsqueda híbrida demuestra que aporta valor, permitir indexar el contenido
de recursos como PDF, DOCX o PPTX.

La arquitectura debe separar claramente:

```text
recurso fuente
    ↓
extractor/adaptador
    ↓
texto/chunks derivados
    ↓
índice léxico/vectorial
```

Los extractores no convierten el fichero binario en la fuente de verdad de Lodestar ni introducen
parsers pesados en `lodestar-core`. El texto extraído, chunks y embeddings se invalidan por hash del
recurso y pueden reconstruirse.

Esta fase es opcional: si referencias tipadas + Markdown + búsqueda híbrida cubren el uso real, no
hay obligación de implementarla.

---

## Ergonomía transversal

La fase 3 salda la deuda ergonómica **acumulada por la superficie actual**. A partir de ahí, la
ergonomía vuelve a ser un requisito transversal: cada capacidad nueva debe diseñarse desde el inicio
para que no vuelva a necesitar otra limpieza de este tamaño.

Cada diseño nuevo debe responder, además de a su semántica, a estas preguntas:

- ¿cómo descubre un humano o agente que la capacidad existe?;
- ¿cómo inspecciona el estado actual sin leer SQLite ni código?;
- ¿cómo entiende por qué una validación o materialización falla?;
- ¿cómo puede previsualizar una operación antes de escribir?;
- ¿el wire expresa la intención del consumidor o filtra detalles del lowering interno?;
- ¿cuánto contexto/token añade la respuesta y cuánto de él es realmente necesario para decidir?;
- ¿qué comportamiento seguro existe cuando no hay configuración?

Los nombres concretos de comandos o tools (`doctor`, `materialize`, `ghosts`, etc.) se decidirán en
cada puerta de diseño y no quedan congelados por este roadmap.

## Dependencias principales

```text
E33: evidencia ───────────────────────────────┐
                                             │
renombrado → hardening → ergonomía → store/cache → contratos → referencias tipadas
                                                               │
                                                               ▼
                                                     templates + ghosts
                                                               │
                                                               ▼
                                                        búsqueda híbrida
                                                               │
                                                               ▼
                                                     extractores opcionales
```

E33 alimenta con evidencia la decisión del store; el renombrado fija antes el nombre y el directorio
de estado definitivos. El hardening cierra corrección conocida y la fase de ergonomía estabiliza la
experiencia externa antes de hacer que todas las lecturas dependan de un camino optimizado o de
ensanchar el dominio.

## Fuera del camino crítico

Siguen existiendo transversales útiles que no deben bloquear las capas anteriores salvo que cambie
la evidencia:

- firma/notarización y updater, actualmente condicionados por el renombrado;
- threat model incremental;
- mejoras adicionales de distribución que excedan la instalación sencilla incluida en ergonomía;
- interfaces de usuario por encima del motor headless.

## Relación con los demás documentos

La autoridad debe seguir leyéndose así:

```text
ARCHITECTURE.md
    diseño e invariantes ratificados

ROADMAP.md
    dirección y orden del trabajo futuro

decisiones/
    preguntas de producto y diseño abiertas/tomadas

requirements/
    épicas e historias ejecutables de la fase activa

IMPLEMENTATION_STATUS.md
    lo que existe realmente hoy
```

Si una fase del roadmap contradice una autoridad vigente, **no se implementa por inercia**: su
puerta de diseño debe actualizar/ratificar primero la decisión o arquitectura correspondiente.