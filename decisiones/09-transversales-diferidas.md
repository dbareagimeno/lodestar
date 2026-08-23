---
id: 9
titulo: "Transversales diferidas de producto"
estado: "abierta"
prioridad: 4
etiquetas: ["rendimiento", "seguridad", "distribucion"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
revisada_en: "2026-08-10"
epica: "E8"
relacionadas: [1, 14, 20]
---

# §9 — Transversales diferidas de producto (E8)

> **Repriorizada el 2026-08-02**: la ficha llevaba tres transversales con una sola prioridad (2) y
> los tres han divergido. Prioridad de la ficha = la del punto más alto (**4**, el banco de
> pruebas); cada punto lleva la suya abajo.

- **Gate de rendimiento — prioridad 4, SUBE.** Bench de cold-open ~10k y coste por llamada, con
  umbrales. Deja de ser un transversal aparcado: es la **condición de entrada de
  [`§14`](14-store-sin-consumidor.md)**, la decisión de prioridad 5 del repo, que hoy se pediría
  tomar sin un solo número. Va en la **épica de evidencia** junto al dogfooding. (El umbral
  «edit→UI < 150 ms» del enunciado original **ya no aplica**: no hay UI desde el giro headless; el
  equivalente es el coste por llamada MCP.)
  **ENTREGADO LOCALMENTE (2026-08-22, E33)**: el diseño está ratificado como `ARCHITECTURE.md §22`
  (banco en dos piezas, corpus canónico determinista, umbral-tras-medición con anclas p95 ≤ 1 s
  por tool a 10k y cold-open ≤ 5 s, medición de la cache por API pública sin conectarla, enganche
  release-first, dogfooding acotado y centinelas de `§22`/`§24`). H04–H06 dejan versionados el
  banco, los umbrales y la evidencia de uso; H07 deja preparado `RELEASING.md`, la convención de
  artefactos y el workflow manual. La corrida local y la validación de configuración/smoke están
  documentadas, pero todavía no se ha ejecutado el `workflow_dispatch` remoto ni enlazado un run
  verde (requiere integrar, commitear y publicar). Por tanto, el BDD remoto de H07 sigue pendiente
  y este punto no se presenta como un banco ya ejecutado por release/CI. El paquete de
  [`E33-H08`](../docs/qa/evidencia-14-store-2026-08.md) deja disponible el dato para `decisiones §14`.
  La ficha §9 **sigue abierta** por la verificación BDD remota pendiente, firma/notarización y
  threat model; esos estados y prioridades no cambian.
- **Firma/notarización + updater — prioridad 3, SUBE… y queda CONGELADA por
  [`§20`](20-renombrado-del-proyecto.md).** Sube porque E27 convirtió los binarios de GitHub
  Releases en el **camino de instalación recomendado**, y salen sin firmar: el aviso de Gatekeeper
  (macOS) y SmartScreen (Windows) pasó de irrelevante a ser lo primero que ve un desconocido.
  Congelada porque cablear la notarización y publicar releases firmadas con un nombre a punto de
  cambiar es gastar el ciclo dos veces. **Alternativa barata mientras tanto**: documentar el aviso
  y cómo saltarlo en `docs/user/`. Ver [`§1`](01-build-fachada-escritorio.md).
- **Threat model — prioridad 2.** Documentado (§12 seguridad); las piezas ya están (RelPath anti
  path/zip-slip, FTS5 escapado). Gana algo de peso ahora que `SECURITY.md` y el canal privado de
  reporte son públicos (`§17-DD`), pero nadie lo ha pedido.
- ~~Arnés diferencial JS-vs-Rust (E1-H18)~~ — **hecho y luego RETIRADO en `E15-H04`** (el prototipo
  dejó de ser spec con la migración a Markdown universal, `ARCHITECTURE.md §20.13`). Histórico:
  `prototype/harness/` ejecutaba las funciones
  puras del prototipo en node como oráculo y `tests/differential.rs` compara con el core (6 fixtures);
  cazó y cerró 6 divergencias de paridad.
