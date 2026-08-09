---
id: 17
titulo: "Superficie externa y apertura OSS"
estado: "cerrada"
prioridad: 3
reabrible: true
etiquetas: ["distribucion", "docs", "comunidad"]
origen: "revision-externa"
abierta_en: "2026-08-01"
cerrada_en: "2026-08-01"
revisada_en: "2026-08-02"
epica: "E27"
historias: ["E27-H10"]
bloquea: ["E27-H10"]
subpuntos: ["DA", "DB", "DC", "DD"]
relacionadas: [1, 14, 20]
---

# §17 — Superficie externa y apertura OSS (E27)

- **Contexto**: una review OSS externa (2026-08-01), evaluada y verificada punto a punto contra
  `main` (v0.5.0), concluyó que el proyecto está **por delante de su adopción**: instalación sin
  binarios documentados, sin demo end-to-end, sin docs de usuario, embudo de contribución cerrado.
  La puerta 1 de la épica **E27** (`requirements/epica-27-producto-distribucion-oss.md`) cerró las
  cuatro decisiones abiertas; el diseño ratificado vive en `ARCHITECTURE.md §21`.
- **DA — crates.io: DIFERIDO** (esta es la mitad **reabrible** de la entrada). Vías de instalación
  soportadas: binarios de GitHub Releases + `cargo install --git`. Publicar en crates.io es
  permanente (solo *yank*) y los crates de dominio (`core`/`store`/`workspace`/`app`) no están
  pensados como API de librería estable. **E27-H10 queda `[BLOQUEADA por decisiones §17]`** hasta
  que esta mitad se reabra y se cierre en (a). Datos registrados para entonces:
  - la **disponibilidad de los nombres no está verificada** (`cargo search lodestar` pendiente);
  - existe una **colisión de marca**: *Lodestar* es un cliente de consenso de Ethereum muy conocido
    (ChainSafe, TypeScript). No afecta a crates.io en lo técnico pero sí a la **descubribilidad**
    del proyecto en buscadores, se publique donde se publique. No se renombra: solo queda anotado.

  > **Actualización 2026-08-02**: la colisión de marca **dejó de estar «solo anotada»** — el usuario
  > tiene intención firme de **renombrar el proyecto**, con alcance total, y eso abrió
  > [`§20`](20-renombrado-del-proyecto.md). Consecuencias para DA: sigue **diferida, y ahora por un
  > motivo mejor** —no se reservan nombres que se van a abandonar—; la verificación de
  > disponibilidad se hace **dentro de §20**, con el nombre nuevo y ampliada a GitHub, dominio y
  > buscadores; y **E27-H10 pasa a estar bloqueada por §20**, no solo por esta ficha.
- **DB — Política de contribución: issues-first + Discussions OFF.** Bugs y docs se aceptan por PR
  directo con checklist; las features requieren issue previa donde el mantenedor decide si pasan
  por el proceso de diseño. Discussions queda desactivado hasta que haya tráfico que lo justifique
  (activarlo es un toggle de settings, no requiere historia).
- **DC — `docs/REFACTOR_PHASE_2.md` NO se mueve.** Es la spec de comportamiento **vigente**, citada
  por ~51 ficheros (CLAUDE.md, ARCHITECTURE, 10 épicas, tests y código): moverla rompería o
  obligaría a tocar todos. A `docs/history/` van solo los 4 documentos genuinamente superseded
  (`REFACTOR.md`, `REFACTOR_DISENO_PROPUESTA.md`, `PROPUESTA_CLI.md`, `PROPUESTA_FIXES.md`). El
  criterio taxonómico es *vigente/superseded*, no *viejo/nuevo* (`§21.3`).
- **DD — CoC y seguridad**: **Contributor Covenant 2.1** en inglés, contacto público
  `dbareagimeno@icloud.com`; **GitHub Private Vulnerability Reporting** como canal primario de
  reporte de vulnerabilidades, con ese email como fallback en `SECURITY.md` y el CoC.
- **Regla transversal ratificada** (`§21.5`): mientras **[`§14`](14-store-sin-consumidor.md)** siga abierta, la superficie
  externa no presenta `reindex`/la cache SQLite como camino de lectura del producto ni promete
  rendimiento a escala. Principio rector de E27: *la superficie externa solo promete lo que el
  motor ejecuta hoy*.
- **Ya ejecutado antes de la épica** (2026-08-01, no forma parte de E27): retro-tag `v0.5.0`
  empujado (el tag `0.5.0` sin prefijo nunca disparó `release.yml`; la release huérfana sin assets
  se borra al verificar la nueva) y la rama `chore/higiene-docs-release` con los quick fixes de
  README/RELEASING/fixtures.
