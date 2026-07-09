---
name: mutantes
description: Mutation testing con cargo-mutants sobre lodestar-core - detecta qué mutaciones del código sobreviven a la suite (= dónde los tests no muerden) y propone tests que las maten. A demanda, sin CI. Útil tras cerrar una historia o antes/después de un refactor.
argument-hint: "[-p <crate>] [--file <ruta desde la raíz del workspace>]"
---

# /mutantes — ¿tus tests muerden?

cargo-mutants introduce mutaciones en el código (invierte condiciones, devuelve valores por
defecto, elimina ramas…) y corre la suite: cada **mutante superviviente** es un cambio de
comportamiento que ningún test detectó — un agujero real de la suite.

## Pasos

1. **Comprueba la instalación**: `cargo mutants --version`. Si no está, indícale al usuario
   `cargo install cargo-mutants` (o `brew install cargo-mutants`) y para — no lo instales sin
   preguntar.
2. **Acota el alcance** (una corrida completa del workspace es lenta y no es el objetivo):
   - Por defecto: `cargo mutants -p lodestar-core --no-times`
   - Con `--file`: añade `--file <ruta>` con la ruta **desde la raíz del workspace** (p. ej.
     `--file crates/lodestar-core/src/query.rs`; el patrón usa semántica gitignore anclada a la
     raíz — `src/query.rs` a secas no casa nada). Es el modo recomendado tras cerrar una
     historia, apuntando a los módulos que tocó.
   - La config compartida vive en `.cargo/mutants.toml` (exclusiones y timeouts).
   - Corre con timeout generoso de Bash (10 min) y `run_in_background` si el alcance es más de un
     fichero.
3. **Analiza los resultados** (`mutants.out/`): `missed.txt` (supervivientes — lo importante),
   `caught.txt`, `unviable.txt`, `timeout.txt`. Lanza un agente (tipo `general-purpose`,
   `model: opus`) con la lista de supervivientes y los ficheros afectados para que:
   - Clasifique cada superviviente: **gap real de la suite** vs **mutante trivial/equivalente**
     (p. ej. mutaciones en código de presentación de errores o en `Display`).
   - Para cada gap real, proponga el test concreto que lo mataría (fichero destino + esbozo del
     assert), siguiendo los patrones del repo (`crates/*/tests/*.rs`, fixtures de
     `lodestar-fixtures`).
4. **Reporta al usuario**: totales (caught/missed/unviable), la lista de gaps reales priorizada, y
   los tests propuestos. **No escribas los tests sin que el usuario elija cuáles** — los gaps a
   veces revelan código muerto que conviene borrar en vez de testear.

## Contexto del repo

- `lodestar-core` es el candidato ideal: crate puro, tests rápidos. Los diferenciales se saltan
  sin node, así que la señal de mutantes ahí puede ser parcial — dilo en el informe si aplica.
- Uso en refactors: corre antes y después con el mismo alcance; si después sobreviven mutantes que
  antes morían, el refactor debilitó la suite.
- `mutants.out*/` no se commitea (está en `.gitignore`).
