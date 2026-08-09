---
id: 4
titulo: "Generación del .d.ts desde Rust con ts-rs o specta"
estado: "obsoleta"
prioridad: 1
etiquetas: ["contrato", "ui", "dependencias"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
cerrada_en: "2026-07-22"
revisada_en: "2026-07-22"
epica: "E6"
historias: ["E0-H04", "E6-H03"]
superada_por: "ARCHITECTURE.md §19.1"
relacionadas: [2]
---

# §4 — Generación del `.d.ts` desde Rust (ts-rs/specta) — E0-H04/E6-H03

> **Obsoleta para el espejo TS** desde el giro headless: `frontend/src/lib/ipc/types.ts` desapareció
> al retirar la UI de escritorio de `main` a la rama `experimental/ui-desktop`. Los tipos de
> `core::types` los consumen ya directamente `lodestar-cli`/`lodestar-mcp` (Rust), sin espejo TS que
> generar. Se conserva el registro histórico abajo; si la UI vuelve a evolucionar en esa rama, la
> decisión de ts-rs se retomaría allí.

- **Estado**: `frontend/src/lib/ipc/types.ts` era un **espejo a mano** del contrato de `core::types`,
  marcado como «a generar». Los nombres/orden coincidían con Rust.
- **Decidido (2026-07-10)**: **sí a ts-rs** — el `.d.ts` se generará desde Rust. Además, la
  frontera front↔back queda descrita por **contratos YAML de superficie** (`contracts/ipc.yml`,
  `contracts/mcp.yml` + `contracts/README.md`): el YAML documenta comandos/eventos/tools y su
  semántica; los **tipos** siguen viviendo solo en `core::types` (invariante #4). El drift se
  vigila con el skill `/contrato --check` (agente `guardian-contrato`).
- **Pendiente**: la implementación de ts-rs (deps + paso de build + marcar `types.ts` como
  generado/«NO EDITAR»). Acordado ejecutarla como **primera historia del nuevo flujo `/ciclo`**
  (dogfooding de `.claude/README.md`). Esta sección se cierra en ese PR.
