---
id: 5
titulo: "i18n multi-idioma"
estado: "abierta"
prioridad: 1
etiquetas: ["i18n", "docs"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
revisada_en: "2026-07-23"
---

# §5 — i18n multi-idioma

- **Estado**: la app es **español-only** en v1 (decisión ya tomada en `CLAUDE.md`). El catálogo de
  conformidad está **keyed por `CheckCode`** (`frontend/src/lib/i18n.ts`) y el core emite `code`+
  `targets`, así que añadir un locale = añadir un objeto con las mismas claves.
- **Qué decidir**: ¿hay que soportar inglés u otro idioma en v1? Si no, esto queda cerrado.
- **Recomendación**: mantener español-only en v1; la arquitectura ya no lo impide en el futuro.
