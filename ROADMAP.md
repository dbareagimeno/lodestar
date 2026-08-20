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
  eliminadas y modificadas, incluidas rutas anidadas.
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

**Objetivo:** cerrar límites conocidos y acotados antes de añadir nuevas primitivas de dominio.

Incluye como mínimo:

- `decisiones §21`: comillas/escape en el lenguaje de consulta;
- issue #47: `semanticDiff` de frontmatter por clave;
- `decisiones §24`: resolver y ejecutar la política de colisiones por caja/Unicode;
- cualquier ajuste pequeño descubierto por el banco de evidencia que no justifique una capacidad
  nueva.

Esta fase no debe convertirse en una épica-escoba indefinida: solo recoge deuda concreta ya
identificada.

## Fase 3 — Activar el store y la cache interna

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

## Fase 4 — Contratos documentales opt-in

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

## Fase 5 — Referencias tipadas a recursos

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

## Fase 6 — Ghosts first-class + templates

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

## Fase 7 — Búsqueda híbrida

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

## Fase 8 — Extractores de contenido opcionales

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

La ergonomía no es una fase separada. Cada diseño nuevo debe responder, además de a su semántica, a
estas preguntas:

- ¿cómo descubre un humano o agente que la capacidad existe?;
- ¿cómo inspecciona el estado actual sin leer SQLite ni código?;
- ¿cómo entiende por qué una validación o materialización falla?;
- ¿cómo puede previsualizar una operación antes de escribir?;
- ¿qué comportamiento seguro existe cuando no hay configuración?

Los nombres concretos de comandos o tools (`doctor`, `materialize`, `ghosts`, etc.) se decidirán en
cada puerta de diseño y no quedan congelados por este roadmap.

## Dependencias principales

```text
E33: evidencia
      │
      └──────────────→ store/cache ───────────────┐
                                                  │
renombrado ────────→ state-dir definitivo        │
                                                  ▼
hardening → contratos → referencias tipadas → templates + ghosts
                    │                              │
                    └──────────────┬───────────────┘
                                   ▼
                            búsqueda híbrida
                                   │
                                   ▼
                         extractores opcionales
```

## Fuera del camino crítico

Siguen existiendo transversales útiles que no deben bloquear las capas anteriores salvo que cambie
la evidencia:

- firma/notarización y updater, actualmente condicionados por el renombrado;
- threat model incremental;
- mejoras adicionales de distribución;
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