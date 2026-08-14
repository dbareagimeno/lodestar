---
name: mutantes
description: Ejecutar mutation testing acotado con cargo-mutants y clasificar supervivientes como gaps reales o mutantes equivalentes. Usar tras una historia de lógica, al investigar tests débiles o antes y después de un refactor; no usar como gate rutinario de CI ni sobre todo el workspace sin necesidad.
---

# Mutantes

Medir si la suite detecta cambios de comportamiento plausiblemente incorrectos.

## Ejecutar

1. Comprobar `cargo mutants --version`. Si falta, pedir permiso antes de instalar.
2. Acotar a un crate o, preferentemente, a los ficheros modificados:

   ```bash
   cargo mutants -p lodestar-core --no-times
   cargo mutants -p lodestar-core --file crates/lodestar-core/src/query.rs --no-times
   ```

3. Usar la configuración de `.cargo/mutants.toml` y un límite de tiempo proporcional. No borrar
   salidas anteriores del usuario; `mutants.out*/` ya está ignorado.
4. Leer `missed.txt`, `caught.txt`, `unviable.txt` y `timeout.txt` de la salida producida.

## Analizar

Para cada superviviente:

- reproducir qué comportamiento cambia;
- clasificarlo como gap real, mutante equivalente o código muerto;
- proponer el test mínimo que lo mataría, con fichero y assertion concreta;
- priorizar caminos de escritura, validación, contrato y negativos.

En un refactor, comparar antes y después con el mismo alcance: un mutante que antes moría y después
sobrevive indica debilitamiento. Reportar totales y evidencia. No añadir tests ni borrar código
automáticamente salvo que el usuario haya pedido corregir los gaps.
