---
id: 21
titulo: "Comillas en el lenguaje de consulta"
estado: "tomada"
prioridad: 3
etiquetas: ["lenguaje-consulta", "contrato"]
origen: "juez-ciego"
abierta_en: "2026-07-29"
revisada_en: "2026-08-02"
relacionadas: [16, 19]
---

# §21 — Comillas en el lenguaje de consulta

> Era el punto **(a)** de [`§16`](16-deuda-auditoria-e25-e26.md), disuelta el 2026-08-02. Se
> extrae a ficha propia porque **es diseño de lenguaje con puerta propia**, no un apéndice de una
> épica de endurecimiento — y porque la decisión ya está tomada en dirección contraria a la
> recomendación original.

- **Origen**: `E26-H09` (un solo dialecto de dot-paths). Al unificar `metadata_inspect` con
  `build_field_path` quedaron a la vista tres casos que el dialecto único **no puede expresar**:
  1. una clave de frontmatter que **contiene un punto literal** (`sonar.projectKey`): direccionable
     con `FieldPath::from_segments` desde Rust, pero no desde la sintaxis textual, que siempre parte
     por puntos;
  2. una clave del usuario llamada literalmente **`frontmatter`**: el prefijo se interpreta como
     anclaje (`E24-H08`), así que la clave homónima queda tapada;
  3. la **fusión de nombres**: `a.b` como clave literal y `a` → `b` anidado producen el mismo
     `FieldPath`. **Precisión de [`§19(c)`](19-hallazgos-referencia-usuario.md)**: en documentos
     **distintos** producen **dos filas** del catálogo con el mismo `name` (no una), y `mode:"field"`
     resuelve solo la anidada; la fusión en **una** entrada ocurre cuando ambas formas coinciden en
     un mismo documento, que es lo que dice el contrato. La redacción original de §16(a) era
     imprecisa en este punto.
- **Decidido (2026-08-02)**: **añadir comillas al lenguaje** (`frontmatter."sonar.projectKey"`),
  descartando la recomendación previa de declarar los tres casos fuera de alcance por escrito. El
  motivo de la recomendación anterior sigue siendo válido —sintaxis nueva sin demanda es superficie
  que hay que mantener para siempre— pero el criterio del usuario es que un lenguaje que no puede
  nombrar un metadato existente tiene un agujero, no un límite.
- **Cómo se aborda**: abre `§20.8` (diseño del lenguaje de consulta), así que **puerta de diseño
  propia** vía `/planificar`. Toca el tokenizador, el catálogo de `metadata_inspect`, el contrato y
  `docs/user/query-language.md`.
- **Orden**: **antes que [`§10`](10-ghosts-y-templates.md)** de las dos piezas de capacidad nueva.
  Es pequeña, cerrada y elimina un límite del lenguaje que hoy está documentado como tal en la
  referencia de usuario.
