---
name: tdd
description: Ejecuta las fases roja y verde de una historia ratificada con autor independiente, alcance verificable y tests bloqueados por hash. Es una fase interna de /ciclo.
argument-hint: <ID E<n>-H<nn> ratificado>
---

# /tdd — rojo y verde verificables

1. Localiza la spec ratificada completa.
2. Ejecuta `python3 scripts/phase-scope.py snapshot target/agent-state/pre-red.json`.
3. Lanza `autor-tests` con spec y snapshot. Exige un test por criterio, negativos y rojo por la
   razón correcta.
4. Ejecuta `python3 scripts/phase-scope.py verify-tests-only target/agent-state/pre-red.json`.
5. Bloquea tests y fixtures con `tdd-test-lock.py snapshot target/agent-state/tests.json ...`.
6. Lanza `implementador` con spec, tests rojos y lock.
7. Verifica el lock antes y después del verde.
8. Ejecuta `scripts/agent-gates.sh full` antes de cerrar.

No se permiten stubs ni tests inline en el circuito separado. Si el test está mal frente a una spec
inequívoca, decide un juez de tests fresco y repite el rojo con otro autor. La ambigüedad normativa
vuelve al usuario.
