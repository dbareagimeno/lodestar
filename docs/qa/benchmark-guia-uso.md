# Benchmark de Lodestar — guía de uso

`lodestar-bench` es un crate interno (`publish = false`) para repetir baselines, detectar
regresiones y evaluar cambios de rendimiento. La [referencia de métricas
actuales](benchmark-metricas-actuales.md) explica con qué comparar una nueva corrida.

## Preparación

Ejecuta los comandos desde la raíz del repositorio y usa siempre `release` para obtener cifras
comparables:

```bash
cargo build --release --locked -p lodestar-bench
```

Guarda las salidas con nombres nuevos. No sobrescribas las evidencias canónicas salvo que se esté
actualizando formalmente la baseline y se vaya a revisar el diff.

## Elegir el modo

| Modo | Uso | Coste esperado |
|---|---|---|
| `--smoke` | Comprobar rápidamente que el banco funciona | Bajo; es el modo usado en CI |
| full, sin selector | Baseline oficial Plano/Realista × 100/1k/10k | Alto; 10 muestras por defecto |
| `--extreme` | Probar una escala positiva arbitraria y medir footprint/RSS | Potencialmente muy alto |
| `--gate` | Comparar un informe H04 con umbrales y baseline | Bajo; no genera corpus |

La sonda extrema no alimenta smoke, full, CI ni el gate H05.

## Smoke

Sobre un workspace existente, el banco crea una copia privada y no modifica el original:

```bash
cargo run --release --locked -p lodestar-bench -- \
  --smoke --seed 33 --root /ruta/al/workspace \
  --json-output /tmp/lodestar-smoke.json \
  --markdown-output /tmp/lodestar-smoke.md
```

Úsalo para validar el arnés, no para comparar absolutos entre máquinas.

## Corrida full H04

El full genera seis corpus temporales: Plano y Realista a 100, 1.000 y 10.000 documentos. Por
defecto toma 10 muestras por lectura y ciclo:

```bash
env -u LODESTAR_BENCH_TEST_ITERATIONS \
  cargo run --release --locked -p lodestar-bench -- --seed 33 \
  --json-output /tmp/lodestar-full.json \
  --markdown-output /tmp/lodestar-full.md
```

La calibración MCP/stdio puede añadirse con `--wire-calibration-input PATH`; consulta el
[runbook del testbench](testbench/README.md) para regenerar esa entrada validada.

## Sonda extrema H09

`--profile`, `--scale` y `--iterations` son obligatorios. `--scale` acepta cualquier entero positivo
representable; no existe una whitelist de 10k, 100k o 1M.

Ejemplo Realista/100k equivalente a la evidencia actual:

```bash
mkdir -p benchmark-results
cargo run --release --locked -p lodestar-bench -- --extreme \
  --profile realista \
  --scale 100000 \
  --iterations 1 \
  --json-output benchmark-results/realista-100k.json \
  --markdown-output benchmark-results/realista-100k.md
```

Para otra escala sólo cambia el número:

```bash
cargo run --release --locked -p lodestar-bench -- --extreme \
  --profile realista --scale 250000 --iterations 1 \
  --json-output benchmark-results/realista-250k.json \
  --markdown-output benchmark-results/realista-250k.md
```

Si no se entrega `--root`, el corpus vive en un directorio temporal y se elimina al terminar. Un
`--root` explícito debe no existir al comenzar, queda bajo limpieza RAII y tampoco puede contener
las salidas JSON/Markdown. Las salidas solicitadas sobreviven fuera del corpus.

### Preflight y 1M

El preflight consulta espacio de disco antes de materializar el primer documento. La heurística
vigente es `32 KiB × scale + 256 MiB`; es una protección modificable, no una estimación contractual
del footprint final.

Para `scale >= 1000000` se exige siempre confirmación explícita:

```bash
cargo run --release --locked -p lodestar-bench -- --extreme \
  --profile realista --scale 1000000 --iterations 1 --confirm-extreme \
  --json-output benchmark-results/realista-1m.json \
  --markdown-output benchmark-results/realista-1m.md
```

`--confirm-extreme` no fuerza la ejecución: si el espacio conocido es insuficiente, el preflight
falla igualmente y no deja un corpus parcial. Cuando `df` no puede verificar el espacio, la
confirmación también es obligatoria para escalas menores. El preflight no verifica RAM; usa las
[métricas actuales de 100k](benchmark-metricas-actuales.md#sonda-h09--realista100k) para planificar
capacidad y tiempo antes de aumentar la escala.

## Leer el informe

Los campos principales del JSON extremo son:

- `measurements[].tools`: muestras cronológicas, p50/p95, payload y resultado por tool;
- `measurements[].cold_open`: apertura más primera lectura;
- `measurements[].rebuild`: rebuild SQLite separado de los percentiles de lectura;
- `measurements[].rss`: baseline, pico absoluto, delta, método y unidades por worker aislado;
- `corpus` y `sqlite`: documentos y bytes persistentes al final de la medición;
- `functional_equivalence` y `equivalence_divergences`: comparación de las tres variantes;
- `preflight`: espacio disponible/requerido y estado explícito de verificación de memoria.

Con una sola iteración, p50 y p95 son la misma muestra. Para comparar latencias estables, aumenta
`--iterations` sólo después de estimar el coste total.

## Comparar dos corridas

Compara únicamente informes con el mismo perfil, escala, semilla, número de iteraciones, perfil de
compilación y máquina. Conserva al menos:

- los JSON completos;
- el resumen Markdown generado por el mismo binario;
- commit, fecha y descripción de la máquina;
- cambios de código/configuración entre ambas corridas.

RSS es un pico por proceso, no memoria retenida en estado estacionario. `rebuild` está separado y
no debe mezclarse con los percentiles de lectura. Los absolutos H05 sólo se juzgan en la máquina de
baseline designada.

## Gate H05/10k

El gate evalúa exclusivamente el camino `disk-reparseo` Realista/10k contra la baseline y los
umbrales ratificados:

```bash
target/release/lodestar-bench --gate \
  --report /ruta/temporal/e33-h04-full-2026-08-22.json \
  --thresholds docs/qa/testbench/umbrales.json \
  --baseline docs/qa/e33-h05-baseline-release-macbook-2026-08.json \
  --machine-id release-macbook-2026-08
```

Este gate no juzga SQLite ni la sonda extrema.

## Verificar el banco tras modificarlo

```bash
cargo test -p lodestar-bench --tests
scripts/agent-gates.sh full
```

Estos comandos prueban el arnés con fixtures pequeñas; no vuelven a ejecutar automáticamente 100k
ni 1M.
