---
id: 2
titulo: "Port de la UI del prototipo"
estado: "obsoleta"
prioridad: 1
etiquetas: ["ui"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
cerrada_en: "2026-07-22"
revisada_en: "2026-07-22"
epica: "E6"
superada_por: "ARCHITECTURE.md §19.1"
relacionadas: [1]
---

# §2 — Port de la UI del prototipo (E6)

- **Estado**: el frontend Svelte 5 es una app funcional completa sobre el `BundleSnapshot`:
  layout de **tres columnas** (páginas · centro · enlaces) con paneles colapsables, **árbol** filtrable
  con estados (orphan/invalid), **tabs** editor · grafo · cambios, **editor multi-escritor** que guarda
  por el único escritor con validación y diagnósticos localizados, **panel de enlaces** (entrantes/
  salientes/índice), **isla imperativa del grafo** (`createStarMap`: posee el SVG + loop rAF, recibe
  nodos/aristas por `$effect`, nunca `{#each}`), y **modo «Cambios»** (diff semántico `OkfDiff` + commit
  con mensaje sugerido). Aspecto con las variables CSS portadas del prototipo. `npm run check`/`build`
  en verde.
- **Qué queda (pulido, no bloquea)**: rails **redimensionables por arrastre** (hoy son colapsables),
  overlay de grafo a pantalla completa, resaltado de query en el grafo con la **semántica del core**
  (hoy es subcadena sobre el id), y detalles de micro-interacción del prototipo.
- **Recomendación**: iterar el pulido visual según uso real; la funcionalidad completa ya está.
