---
name: mutantes
description: Ejecuta cargo-mutants de forma acotada para detectar gaps reales de tests. Úsalo tras lógica nueva o antes/después de un refactor; no es un gate rutinario de CI.
argument-hint: "[-p <crate>] [--file <ruta>]"
---

# /mutantes — comprobar que la suite muerde

1. Comprueba `cargo mutants --version`; pide permiso antes de instalar si falta.
2. Acota por crate y, preferentemente, por fichero:

   ```bash
   cargo mutants -p lodestar-core --file crates/lodestar-core/src/query.rs --no-times
   ```

3. Lee `missed.txt`, `caught.txt`, `unviable.txt` y `timeout.txt` de la salida nueva.
4. Para cada superviviente, demuestra el cambio de comportamiento y clasifica: gap real, mutante
   equivalente o código muerto.
5. Propón el test mínimo para cada gap real, con fichero y assertion. No escribas tests ni borres
   código salvo petición explícita.

En refactors, usa exactamente el mismo alcance antes y después. Un mutante que antes moría y ahora
sobrevive indica una suite debilitada.
