---
name: ux
description: UX con buenas prácticas - genera flujos de usuario (.excalidraw en design/flujos/), mockups HTML unitarios por pantalla/estado (design/mockups/) o audita la UI contra heurísticas (Nielsen, ley de Jakob, estados obligatorios, accesibilidad). Úsalo ANTES de /historia cuando el trabajo introduce UI nueva, o a demanda para auditar la existente.
argument-hint: "<flujo|mockup|audit> <descripción | ID E<n>-H<nn> | ruta>"
---

# /ux — flujos, mockups y auditoría con buenas prácticas

Da forma visual al trabajo de UI **antes** de especificarlo (los artefactos ratificados se citan
después en la historia) o audita la UI existente. **Este skill no escribe código de producción**:
sus entregables viven en `design/`.

## Modos

| Modo | Entrada | Entregable |
|---|---|---|
| `flujo` | descripción o ID de historia | `design/flujos/<slug>.excalidraw` (caminos felices + error/cancelación) |
| `mockup` | descripción o ID, opcionalmente estados concretos | `design/mockups/<slug>--<estado>.html`, uno por pantalla/estado |
| `audit` | ruta de `frontend/`, `--diff`, o nada (app entera) | informe de hallazgos por severidad |

Sin modo explícito, dedúcelo: "cómo debería funcionar X" → `flujo`; "cómo se vería X" → `mockup`;
"revisa/valora X" → `audit`.

## Pasos

1. **Prepara el contexto**: si el argumento es un ID `E<n>-H<nn>`, localiza la historia en
   `requirements/epica-*.md` y pásala como base. Para `audit --diff`, computa
   `git diff main...HEAD` (o working tree) y filtra a `frontend/`.
2. **Lanza el agente `disenador-ux`** con: la petición tal cual, el modo, y el recordatorio de sus
   reglas (patrón conocido por defecto con excepción justificada, tokens del prototipo, estados
   obligatorios vacío/cargando/error/éxito, artefactos solo en `design/`, un HTML por
   pantalla/estado — nunca una réplica de la app).
3. **Verifica su salida**:
   - `flujo`: el `.excalidraw` es JSON válido y cubre error/cancelación, no solo el camino feliz.
   - `mockup`: un fichero por estado, autocontenido, con las variables CSS del prototipo (no
     valores inventados) y datos de ejemplo del dominio.
   - `audit`: cada hallazgo cita heurística + ubicación + recomendación; lo opinable va como NOTA.
   - En todos: si propone un patrón nuevo, la justificación (qué patrón conocido descarta y por
     qué) está presente — sin ella, se devuelve al agente.
4. **Presenta al usuario**: artefactos generados (rutas), patrones usados, decisiones abiertas, y
   pide **ratificación explícita**. Los artefactos ratificados se citan en las Referencias de la
   historia (`/historia`) para que autor-tests, implementador y juez los tengan como spec visual.
   En `audit`, reporta el informe tal cual y **no apliques arreglos sin que el usuario lo pida**.

## Reglas

- Los artefactos de `design/` son especificación visual ratificable, no código: `frontend/` nunca
  los importa, y no se mantienen sincronizados con la app — se archivan cuando su historia cierra.
- UI nueva sin flujo/mockup ratificado es como código sin historia: si el trabajo introduce
  pantallas o interacciones que el prototipo no tiene, pasa por aquí antes de `/historia`.
- Si un flujo depende de una decisión abierta de `DECISIONES.md`, se presenta como borrador con
  las opciones — decide el usuario, nunca el agente.
