# Validación del workflow Codex

Validación ejecutada sobre el checkout real de Lodestar en `develop` el 15 de agosto de 2026. No
se crearon ramas, commits, pushes ni PRs.

## Resultado

El overlay es sintácticamente válido, sus controles positivos y negativos muerden como se espera
y `scripts/agent-gates.sh full` terminó con código 0.

## Configuración y skills

- Todos los scripts Python compilan con `python3 -m py_compile`.
- `scripts/agent-gates.sh` pasa `bash -n`.
- `.codex/config.toml` y los siete agentes de `.codex/agents/` parsean con `tomllib`.
- Los cinco skills pasan el validador oficial de `skill-creator`: `planificar`, `especificar`,
  `ciclo`, `revisar` y `mutantes`.
- `.github/workflows/ci.yml` parsea como YAML.
- No quedan placeholders `TODO`, `FIXME` o `REPLACE_ME` en el overlay.

## Controles ejecutables

- `python3 scripts/check-agent-guidance.py`: OK sobre la guidance Codex vigente.
- `python3 scripts/check-agent-guidance.py --include-legacy`: OK sobre 36 ficheros, incluida la
  capa de compatibilidad `.claude/`.
- `python3 scripts/check-contract-surface.py`: OK; registro, despacho y contrato contienen las
  mismas diez tools en el mismo orden, y las tres tools de cambio coinciden.
- `scripts/agent-gates.sh contract`: OK, incluidos los tests Rust de schemas y parámetros.
- `scripts/agent-gates.sh policy`: OK, incluida pureza de core, dependencias retiradas y fuente
  única de códigos de error.

### Alcance rojo y lock de tests

Se probaron ambos controles en positivo y negativo con ficheros temporales eliminados al terminar:

1. `phase-scope.py` aceptó un cambio limitado a un test de integración.
2. El mismo control rechazó un fichero creado fuera del alcance permitido.
3. `tdd-test-lock.py` aceptó los hashes sin cambios.
4. El lock rechazó una modificación posterior del test bloqueado.

## Gate completo

`scripts/agent-gates.sh full` terminó con código 0 y cubrió:

- `cargo fmt --all --check`;
- Clippy de workspace, todos los targets y features, con warnings como error;
- build de todo el workspace y todos los targets;
- `cargo test --workspace --locked`;
- `lodestar-workspace` con `test-failpoints` (66 tests de transacciones instrumentadas, todos
  verdes, además del resto del crate);
- `lodestar-app` con `test-failpoints` (26 tests de escritura instrumentada, todos verdes, además
  del resto del crate);
- `cargo doc --workspace --no-deps --locked` con warnings como error;
- política de dependencias, tipos y contrato.

El smoke de la demo se omitió localmente porque el host era macOS; el script lo ejecuta en Linux y
el job dedicado de CI sigue ejecutándolo en `ubuntu-latest`.

### Nota del entorno macOS

El sandbox local demoraba el arranque de binarios Rust recién enlazados por el atributo de
procedencia de macOS. Para completar la validación sin modificar el toolchain instalado se usó una
copia temporal aislada de `rustc`, `rustdoc` y Clippy, más un runner temporal que retiraba el
atributo solo de binarios generados en `target/`. Esa adaptación no forma parte del repositorio ni
del workflow instalado.

## Higiene del diff

- `git diff --check`: OK.
- No quedan artefactos de las pruebas positivas/negativas en el árbol de trabajo.
- El checkout conserva los cambios sin commit para revisión e integración explícitas.
