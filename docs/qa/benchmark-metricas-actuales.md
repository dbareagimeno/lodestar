# Benchmark de Lodestar — métricas actuales

Referencia vigente a **2026-08-23**. Resume las corridas y remite al manifiesto; los JSON completos
son artefactos externos comprimidos y contienen todas las muestras y resultados.

El [manifiesto](corridas/v0.6.2/manifest.json) separa la identidad del asset descargable de la
carga útil: `artifact.sha256` y `artifact.size_bytes` corresponden al `.json.gz`, mientras
`raw.sha256`, `raw.size_bytes` y `raw.schema_version` corresponden al JSON descomprimido.

## Entorno de referencia

- Commit: `0c9d0ba7980a0d1059d918844ab3864f361b661b`.
- Máquina: `macos/aarch64`.
- Perfil de compilación: `release`.
- Semilla: `33`.
- Variantes: `disk-reparseo`, `sqlite-raw` y `ram-memoizado`.
- Lecturas: las siete tools de lectura de Lodestar, con equivalencia funcional exacta entre
  variantes.

Las cifras son internas y dependientes de máquina. No son una promesa pública ni deciden el destino
de SQLite; `decisiones §14` continúa abierta.

## Baseline H04 — Realista/10k

Fuente: [manifiesto de evidencia v0.6.2](corridas/v0.6.2/manifest.json) y
[resumen H04](e33-h04-banco-rendimiento-2026-08-22.md). El JSON full está publicado como
artefacto comprimido externo; son 10 muestras por tool y variante.

| Variante | Peor lectura p95 | Cold-open p95 | Rebuild SQLite p95 |
|---|---:|---:|---:|
| `disk-reparseo` | `481.542.291 ns` (`impact_analyze`, 481,54 ms) | `248.825.583 ns` (248,83 ms) | — |
| `sqlite-raw` | `157.917.750 ns` (`knowledge_search`, 157,92 ms) | `132.058.416 ns` (132,06 ms) | `14.046.911.667 ns` (14,05 s) |
| `ram-memoizado` | `48.005.042 ns` (`workspace_status`, 48,01 ms) | `249.147.083 ns` (249,15 ms) | — |

El ciclo real `change_plan` → `change_apply` por App/disco obtuvo `3.377.063.500 ns` p95, con
`1.774.696.708 ns` de preparación fuera de la muestra.

Los techos ratificados de H05 son **1 s por lectura** y **5 s de cold-open**, únicamente para
`disk-reparseo` Realista/10k en la máquina de baseline. Son máximos de regresión, no objetivos para
SQLite ni motivo para descartarlo.

## Sonda H09 — Realista/100k

Fuente: [manifiesto de evidencia v0.6.2](corridas/v0.6.2/manifest.json) y
[resumen H09/100k](e33-h09-realista-100k-2026-08-23.md). El JSON está publicado como artefacto
comprimido externo. Se solicitó `--scale 100000`; el corpus
contiene 100.000 documentos generados y cuatro controles, 100.004 en total. Se tomó una sola
muestra, por lo que p50 y p95 coinciden y no deben interpretarse como percentiles estadísticos.

| Variante | Lectura más lenta | Cold-open | Rebuild | RSS absoluto | Delta RSS |
|---|---:|---:|---:|---:|---:|
| `disk-reparseo` | `18.873.182.917 ns` (`knowledge_get`, 18,87 s) | `9.496.607.584 ns` (9,50 s) | — | `972.439.552 B` (927,39 MiB) | `969.801.728 B` (924,88 MiB) |
| `sqlite-raw` | `2.428.974.792 ns` (`workspace_status`, 2,43 s) | `1.371.578.333 ns` (1,37 s) | `1.868.411.002.750 ns` (31,14 min) | `985.530.368 B` (939,88 MiB) | `982.892.544 B` (937,36 MiB) |
| `ram-memoizado` | `523.447.750 ns` (`workspace_status`, 0,52 s) | `10.327.592.750 ns` (10,33 s) | — | `839.974.912 B` (801,06 MiB) | `837.337.088 B` (798,55 MiB) |

Footprint persistente al cerrar la medición:

- Markdown: `134.783.275 B` (128,54 MiB).
- SQLite principal: `659.578.880 B` (629,02 MiB); WAL, SHM y auxiliares: `0 B`.
- Equivalencia funcional: `true`, sin divergencias entre las tres variantes y las siete lecturas.

El preflight observó `8.729.272.320 B` disponibles frente a `3.545.235.456 B` requeridos. No
verifica memoria anticipadamente: RSS se mide después en cada worker aislado, con un baseline de
`2.637.824 B` en esta corrida.

## Qué se puede concluir

- H09 demuestra que el banco puede medir 100k de forma reproducible y que el footprint de RAM
  merece atención antes de convertir 1M en una corrida habitual.
- La variante RAM fue la más rápida en las lecturas medidas, pero no en cold-open; SQLite redujo
  tiempos de lectura frente al reparsing de disco, a cambio de un rebuild muy costoso en esta
  implementación.
- Una sola muestra de 100k sirve para dimensionamiento y comparación gruesa, no para afirmar una
  distribución de latencias.
- No se ejecutó 1M. La sonda lo admite con preflight y `--confirm-extreme`, pero la heurística actual
  estima `33.036.435.456 B` (~30,77 GiB) de espacio. Esa fórmula es modificable y no forma parte del
  contrato de H09.

Para repetir o ampliar estas mediciones, sigue la [guía de uso](benchmark-guia-uso.md).
