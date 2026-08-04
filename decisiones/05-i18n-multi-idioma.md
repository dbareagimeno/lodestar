---
id: 5
titulo: "i18n multi-idioma"
estado: "cerrada"
prioridad: 1
etiquetas: ["i18n", "docs"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
cerrada_en: "2026-08-02"
revisada_en: "2026-08-02"
relacionadas: [17]
---

# §5 — i18n multi-idioma

- **Estado**: la app es **español-only** en v1 (decisión ya tomada en `CLAUDE.md`). El catálogo de
  conformidad está **keyed por `CheckCode`** (`frontend/src/lib/i18n.ts`) y el core emite `code`+
  `targets`, así que añadir un locale = añadir un objeto con las mismas claves.
- **Qué decidir**: ¿hay que soportar inglés u otro idioma en v1? Si no, esto queda cerrado.
- **Recomendación**: mantener español-only en v1; la arquitectura ya no lo impide en el futuro.

## CERRADA el 2026-08-02 — la pregunta ya tiene respuesta por otra vía

La **decisión de idioma partido** del 2026-08-01 (ratificada en E27, `§17`/`§21`) contestó esto sin
pasar por la ficha: la **superficie pública** —README, `docs/user/`, CONTRIBUTING, SECURITY,
templates de issues/PR— se escribe en **inglés** para adopción OSS, y lo **interno** —ARCHITECTURE,
`decisiones/`, `requirements/`, commits, código y comentarios— sigue en **español**.

Dos matices que la cierran del todo:
- El sujeto original era la **UI de escritorio**, que se retiró de `main` a `experimental/ui-desktop`;
  `frontend/src/lib/i18n.ts` **ya no existe en este repo**.
- Lo que queda con idioma en el motor son los **mensajes de diagnóstico y error** del core/MCP, hoy
  en español. Si algún día se traducen, la vía sigue siendo la buena: están *keyed* por código y el
  core emite `code` + `targets`. Eso es **trabajo futuro con camino claro**, no una decisión abierta.
