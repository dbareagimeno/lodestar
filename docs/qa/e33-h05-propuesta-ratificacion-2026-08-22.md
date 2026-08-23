# E33-H05 — constancia de ratificación de umbrales

La corrida release de H04 dejó estos p95 en `realista/10000/disk-reparseo` (ns):

| Tool | p95 H04 |
|---|---:|
| `workspace_status` | 265405167 |
| `knowledge_search` | 288724209 |
| `knowledge_get` | 481312208 |
| `metadata_inspect` | 201016375 |
| `graph_query` | 243586125 |
| `impact_analyze` | 481542291 |
| `knowledge_check` | 251015708 |
| cold-open | 248825583 |

Con esos números delante, el usuario ratificó el 2026-08-22 los máximos de `p95_ns =
1000000000` y `cold_open_ns = 5000000000`. Son techos máximos para detectar regresión del
camino de producto `disk-reparseo`, no objetivo, promesa de rendimiento ni veto o criterio para
descartar SQLite. El gate absoluto solo juzga esa variante a escala 10000 en la máquina
`release-macbook-2026-08`; en cualquier otra máquina declara modo tendencia y jamás juzga estos
absolutos.

La baseline versionada es [`e33-h05-baseline-release-macbook-2026-08.json`](e33-h05-baseline-release-macbook-2026-08.json).
