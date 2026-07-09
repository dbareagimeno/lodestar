---
name: tdd
description: Ciclo TDD rojo-verde-refactor de una historia ratificada - autor-tests escribe tests que fallan, implementador los pone en verde sin poder tocarlos, gates locales al final. Úsalo tras ratificar una historia con /historia.
argument-hint: <ID E<n>-H<nn> de una historia ratificada>
---

# /tdd — rojo → verde → refactor

Implementa una historia **ya ratificada** con separación de poderes: quien escribe los tests no
implementa, y quien implementa no puede tocar los tests.

## Pasos

1. **Prepara el encargo**: localiza la historia en `requirements/epica-*.md` y copia su texto
   completo (criterios + Pruebas). Si no existe o no está ratificada, para y redirige a `/historia`.
2. **ROJO** — lanza el agente **autor-tests** (tipo `autor-tests`) con el texto de la historia.
   Exige en su salida la evidencia de que los tests nuevos fallan por la razón correcta. Si algún
   test nuevo pasa sin implementación, devuélveselo: es vacuo.
3. **VERDE** — lanza el agente **implementador** (tipo `implementador`) con el texto de la historia
   y la lista exacta de tests en rojo (fichero + nombres). Recuérdale la regla: prohibido modificar
   los tests; si cree que un test está mal, debe parar y reportarlo.
   - Si reporta un test defectuoso: **no arbitres tú el código** — vuelve al autor-tests con el
     razonamiento del implementador para que lo corrija (o confirme), y reanuda.
4. **Gates** — verifica que el implementador aportó evidencia de:
   `cargo test --workspace --locked` · `cargo fmt --all --check` ·
   `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` ·
   `cargo doc` con `RUSTDOCFLAGS="-D warnings"` · y si tocó `frontend/`: `npm run check && npm run build`.
   Si falta alguno, córrelo tú antes de dar el ciclo por cerrado.
5. **REFACTOR (opcional)** — si el verde dejó duplicación o complejidad evidente, propone al usuario
   correr `/simplify` sobre el diff. No refactorices por inercia.
6. **Cierre** — resume al usuario: tests añadidos, qué se implementó, estado de los gates, y
   recuerda los siguientes pasos del flujo: `/contrato --check` si tocó la frontera, y `/juzgar` antes
   de commitear.

## Reglas

- El orquestador (tú) **no escribe tests ni implementación**: supervisa, pasa artefactos entre
  agentes y verifica evidencias. Así el juez posterior tampoco hereda sesgo tuyo.
- Todo en español; identificadores técnicos en inglés (formato del repo).
