# Lodestar H04 — banco de rendimiento

- **machine**: `"macos/aarch64"`
- **binary**: `"lodestar-bench"`
- **build_profile**: `"release"`
- **commit**: `"0c9d0ba7980a0d1059d918844ab3864f361b661b"`
- **seed**: `33`
- **profiles**: `["plano","realista"]`
- **scales**: `[100,1000,10000]`

> SQLite-raw evita walk+I/O; `DocumentSet::from_store` reparsea raw. `rebuild` se registra separado.

| Perfil | Escala | Variante | Tool | Muestras | p50 (ns) | p95 (ns) | Payload (bytes) |
|---|---:|---|---|---:|---:|---:|---:|
| plano | 100 | disk-reparseo | graph_query | 10 | 1875375 | 3309250 | 6807 |
| plano | 100 | disk-reparseo | impact_analyze | 10 | 3671792 | 6341542 | 261 |
| plano | 100 | disk-reparseo | knowledge_check | 10 | 2054959 | 3242708 | 825 |
| plano | 100 | disk-reparseo | knowledge_get | 10 | 3679792 | 6717417 | 747 |
| plano | 100 | disk-reparseo | knowledge_search | 10 | 2110833 | 5594833 | 269 |
| plano | 100 | disk-reparseo | metadata_inspect | 10 | 1656958 | 3246958 | 213 |
| plano | 100 | disk-reparseo | workspace_status | 10 | 2056417 | 9092958 | 520 |
| plano | 100 | sqlite-raw | graph_query | 10 | 762583 | 780125 | 6807 |
| plano | 100 | sqlite-raw | impact_analyze | 10 | 731000 | 747917 | 261 |
| plano | 100 | sqlite-raw | knowledge_check | 10 | 953833 | 968042 | 825 |
| plano | 100 | sqlite-raw | knowledge_get | 10 | 735042 | 749667 | 747 |
| plano | 100 | sqlite-raw | knowledge_search | 10 | 990000 | 1029500 | 269 |
| plano | 100 | sqlite-raw | metadata_inspect | 10 | 564375 | 573792 | 213 |
| plano | 100 | sqlite-raw | workspace_status | 10 | 947625 | 1045125 | 520 |
| plano | 100 | sqlite-raw | rebuild (separate) | 1 | 9528375 | 9528375 | - |
| plano | 100 | ram-memoizado | graph_query | 10 | 31875 | 37167 | 6807 |
| plano | 100 | ram-memoizado | impact_analyze | 10 | 2000 | 3833 | 261 |
| plano | 100 | ram-memoizado | knowledge_check | 10 | 228458 | 234916 | 825 |
| plano | 100 | ram-memoizado | knowledge_get | 10 | 5375 | 6958 | 747 |
| plano | 100 | ram-memoizado | knowledge_search | 10 | 268708 | 271625 | 269 |
| plano | 100 | ram-memoizado | metadata_inspect | 10 | 10625 | 12625 | 213 |
| plano | 100 | ram-memoizado | workspace_status | 10 | 219792 | 416708 | 520 |
| plano | 1000 | disk-reparseo | graph_query | 10 | 17967666 | 21647084 | 6807 |
| plano | 1000 | disk-reparseo | impact_analyze | 10 | 35810916 | 37414084 | 261 |
| plano | 1000 | disk-reparseo | knowledge_check | 10 | 20147500 | 25785875 | 825 |
| plano | 1000 | disk-reparseo | knowledge_get | 10 | 35700542 | 36712167 | 747 |
| plano | 1000 | disk-reparseo | knowledge_search | 10 | 20371625 | 29547000 | 269 |
| plano | 1000 | disk-reparseo | metadata_inspect | 10 | 15569750 | 21819000 | 214 |
| plano | 1000 | disk-reparseo | workspace_status | 10 | 20283500 | 25246291 | 523 |
| plano | 1000 | sqlite-raw | graph_query | 10 | 7772791 | 7942375 | 6807 |
| plano | 1000 | sqlite-raw | impact_analyze | 10 | 7525208 | 7750834 | 261 |
| plano | 1000 | sqlite-raw | knowledge_check | 10 | 9848250 | 10057167 | 825 |
| plano | 1000 | sqlite-raw | knowledge_get | 10 | 7561208 | 7838042 | 747 |
| plano | 1000 | sqlite-raw | knowledge_search | 10 | 10172167 | 10564250 | 269 |
| plano | 1000 | sqlite-raw | metadata_inspect | 10 | 5744125 | 5948917 | 214 |
| plano | 1000 | sqlite-raw | workspace_status | 10 | 9769417 | 10446209 | 523 |
| plano | 1000 | sqlite-raw | rebuild (separate) | 1 | 262127250 | 262127250 | - |
| plano | 1000 | ram-memoizado | graph_query | 10 | 259583 | 268875 | 6807 |
| plano | 1000 | ram-memoizado | impact_analyze | 10 | 2917 | 4625 | 261 |
| plano | 1000 | ram-memoizado | knowledge_check | 10 | 2384042 | 2449167 | 825 |
| plano | 1000 | ram-memoizado | knowledge_get | 10 | 6417 | 7792 | 747 |
| plano | 1000 | ram-memoizado | knowledge_search | 10 | 2634250 | 2657083 | 269 |
| plano | 1000 | ram-memoizado | metadata_inspect | 10 | 103833 | 120292 | 214 |
| plano | 1000 | ram-memoizado | workspace_status | 10 | 2243250 | 4142333 | 523 |
| plano | 10000 | disk-reparseo | graph_query | 10 | 204340292 | 206433750 | 6807 |
| plano | 10000 | disk-reparseo | impact_analyze | 10 | 402410792 | 407018125 | 261 |
| plano | 10000 | disk-reparseo | knowledge_check | 10 | 224042791 | 225142916 | 825 |
| plano | 10000 | disk-reparseo | knowledge_get | 10 | 403041750 | 405713625 | 747 |
| plano | 10000 | disk-reparseo | knowledge_search | 10 | 228169834 | 229671542 | 269 |
| plano | 10000 | disk-reparseo | metadata_inspect | 10 | 181219458 | 182683708 | 215 |
| plano | 10000 | disk-reparseo | workspace_status | 10 | 224225000 | 229693000 | 526 |
| plano | 10000 | sqlite-raw | graph_query | 10 | 88890291 | 91052875 | 6807 |
| plano | 10000 | sqlite-raw | impact_analyze | 10 | 86511459 | 87978917 | 261 |
| plano | 10000 | sqlite-raw | knowledge_check | 10 | 109000583 | 112918416 | 825 |
| plano | 10000 | sqlite-raw | knowledge_get | 10 | 85944458 | 87259792 | 747 |
| plano | 10000 | sqlite-raw | knowledge_search | 10 | 112807958 | 114909083 | 269 |
| plano | 10000 | sqlite-raw | metadata_inspect | 10 | 66463208 | 68033125 | 215 |
| plano | 10000 | sqlite-raw | workspace_status | 10 | 108429875 | 110258625 | 526 |
| plano | 10000 | sqlite-raw | rebuild (separate) | 1 | 28957367000 | 28957367000 | - |
| plano | 10000 | ram-memoizado | graph_query | 10 | 2734083 | 2827334 | 6807 |
| plano | 10000 | ram-memoizado | impact_analyze | 10 | 6958 | 10417 | 261 |
| plano | 10000 | ram-memoizado | knowledge_check | 10 | 23468583 | 23705541 | 825 |
| plano | 10000 | ram-memoizado | knowledge_get | 10 | 11334 | 15500 | 747 |
| plano | 10000 | ram-memoizado | knowledge_search | 10 | 26481792 | 27025917 | 269 |
| plano | 10000 | ram-memoizado | metadata_inspect | 10 | 1121042 | 1594000 | 215 |
| plano | 10000 | ram-memoizado | workspace_status | 10 | 22135958 | 42464542 | 526 |
| realista | 100 | disk-reparseo | graph_query | 10 | 2229083 | 2267792 | 19962 |
| realista | 100 | disk-reparseo | impact_analyze | 10 | 4345750 | 4399459 | 261 |
| realista | 100 | disk-reparseo | knowledge_check | 10 | 2303750 | 2325625 | 6161 |
| realista | 100 | disk-reparseo | knowledge_get | 10 | 4348541 | 4472542 | 747 |
| realista | 100 | disk-reparseo | knowledge_search | 10 | 2635167 | 2844458 | 269 |
| realista | 100 | disk-reparseo | metadata_inspect | 10 | 1835792 | 1857042 | 372 |
| realista | 100 | disk-reparseo | workspace_status | 10 | 2271291 | 3354667 | 526 |
| realista | 100 | sqlite-raw | graph_query | 10 | 1081042 | 1095500 | 19962 |
| realista | 100 | sqlite-raw | impact_analyze | 10 | 1000167 | 1005083 | 261 |
| realista | 100 | sqlite-raw | knowledge_check | 10 | 1137333 | 1146125 | 6161 |
| realista | 100 | sqlite-raw | knowledge_get | 10 | 1002041 | 1008042 | 747 |
| realista | 100 | sqlite-raw | knowledge_search | 10 | 1463000 | 1500250 | 269 |
| realista | 100 | sqlite-raw | metadata_inspect | 10 | 716125 | 724042 | 372 |
| realista | 100 | sqlite-raw | workspace_status | 10 | 1120458 | 1178833 | 526 |
| realista | 100 | sqlite-raw | rebuild (separate) | 1 | 9648875 | 9648875 | - |
| realista | 100 | ram-memoizado | graph_query | 10 | 71042 | 82209 | 19962 |
| realista | 100 | ram-memoizado | impact_analyze | 10 | 1833 | 5958 | 261 |
| realista | 100 | ram-memoizado | knowledge_check | 10 | 138250 | 140916 | 6161 |
| realista | 100 | ram-memoizado | knowledge_get | 10 | 4625 | 5542 | 747 |
| realista | 100 | ram-memoizado | knowledge_search | 10 | 462333 | 474750 | 269 |
| realista | 100 | ram-memoizado | metadata_inspect | 10 | 22291 | 23958 | 372 |
| realista | 100 | ram-memoizado | workspace_status | 10 | 118292 | 429459 | 526 |
| realista | 1000 | disk-reparseo | graph_query | 10 | 20584250 | 20813750 | 9608 |
| realista | 1000 | disk-reparseo | impact_analyze | 10 | 40581250 | 41147000 | 261 |
| realista | 1000 | disk-reparseo | knowledge_check | 10 | 21484167 | 21748625 | 35785 |
| realista | 1000 | disk-reparseo | knowledge_get | 10 | 40771625 | 42056666 | 747 |
| realista | 1000 | disk-reparseo | knowledge_search | 10 | 24803208 | 26122917 | 269 |
| realista | 1000 | disk-reparseo | metadata_inspect | 10 | 16825000 | 17149333 | 379 |
| realista | 1000 | disk-reparseo | workspace_status | 10 | 21353500 | 23253000 | 532 |
| realista | 1000 | sqlite-raw | graph_query | 10 | 10636875 | 10709375 | 9608 |
| realista | 1000 | sqlite-raw | impact_analyze | 10 | 10127125 | 10188208 | 261 |
| realista | 1000 | sqlite-raw | knowledge_check | 10 | 11488250 | 11637125 | 35785 |
| realista | 1000 | sqlite-raw | knowledge_get | 10 | 10131459 | 10232542 | 747 |
| realista | 1000 | sqlite-raw | knowledge_search | 10 | 14738250 | 14901958 | 269 |
| realista | 1000 | sqlite-raw | metadata_inspect | 10 | 7012500 | 7064500 | 379 |
| realista | 1000 | sqlite-raw | workspace_status | 10 | 11335000 | 11831666 | 532 |
| realista | 1000 | sqlite-raw | rebuild (separate) | 1 | 168679333 | 168679333 | - |
| realista | 1000 | ram-memoizado | graph_query | 10 | 536334 | 555834 | 9608 |
| realista | 1000 | ram-memoizado | impact_analyze | 10 | 1875 | 3583 | 261 |
| realista | 1000 | ram-memoizado | knowledge_check | 10 | 1375334 | 1403792 | 35785 |
| realista | 1000 | ram-memoizado | knowledge_get | 10 | 4917 | 7459 | 747 |
| realista | 1000 | ram-memoizado | knowledge_search | 10 | 4605125 | 4641500 | 269 |
| realista | 1000 | ram-memoizado | metadata_inspect | 10 | 217542 | 221917 | 379 |
| realista | 1000 | ram-memoizado | workspace_status | 10 | 1162416 | 4526042 | 532 |
| realista | 10000 | disk-reparseo | graph_query | 10 | 242525667 | 243586125 | 7382 |
| realista | 10000 | disk-reparseo | impact_analyze | 10 | 477926542 | 481542291 | 261 |
| realista | 10000 | disk-reparseo | knowledge_check | 10 | 250406209 | 251015708 | 36030 |
| realista | 10000 | disk-reparseo | knowledge_get | 10 | 477197042 | 481312208 | 747 |
| realista | 10000 | disk-reparseo | knowledge_search | 10 | 283462958 | 288724209 | 269 |
| realista | 10000 | disk-reparseo | metadata_inspect | 10 | 199995667 | 201016375 | 386 |
| realista | 10000 | disk-reparseo | workspace_status | 10 | 248179916 | 265405167 | 539 |
| realista | 10000 | sqlite-raw | graph_query | 10 | 116082958 | 117066625 | 7382 |
| realista | 10000 | sqlite-raw | impact_analyze | 10 | 109943125 | 111245833 | 261 |
| realista | 10000 | sqlite-raw | knowledge_check | 10 | 123854166 | 125181041 | 36030 |
| realista | 10000 | sqlite-raw | knowledge_get | 10 | 110394916 | 111349083 | 747 |
| realista | 10000 | sqlite-raw | knowledge_search | 10 | 156375958 | 157917750 | 269 |
| realista | 10000 | sqlite-raw | metadata_inspect | 10 | 76573542 | 77454584 | 386 |
| realista | 10000 | sqlite-raw | workspace_status | 10 | 121928959 | 123002541 | 539 |
| realista | 10000 | sqlite-raw | rebuild (separate) | 1 | 14046911667 | 14046911667 | - |
| realista | 10000 | ram-memoizado | graph_query | 10 | 6215250 | 6262833 | 7382 |
| realista | 10000 | ram-memoizado | impact_analyze | 10 | 4125 | 6459 | 261 |
| realista | 10000 | ram-memoizado | knowledge_check | 10 | 13761042 | 13780791 | 36030 |
| realista | 10000 | ram-memoizado | knowledge_get | 10 | 8000 | 11417 | 747 |
| realista | 10000 | ram-memoizado | knowledge_search | 10 | 46254708 | 46493500 | 269 |
| realista | 10000 | ram-memoizado | metadata_inspect | 10 | 2310916 | 2371208 | 386 |
| realista | 10000 | ram-memoizado | workspace_status | 10 | 11845625 | 48005042 | 539 |

## Calibración wire

Estado: **complete**; calibración real JSON-RPC/stdio sobre MCP.

- **wire profile**: `"realista"`
- **wire corpus_profile**: `"realista"`
- **wire runtime_profile**: `"readonly"`
- **wire scale**: `10000`
- **wire harness**: `"docs/qa/testbench/lodestar_harness.py"`
- **wire transport**: `"JSON-RPC/stdio"`
- **wire binary**: `"lodestar-mcp"`
- **wire build_profile**: `"release"`
- **wire corpus_documents**: `10004`

Las observaciones versionadas enlazan cada stdout y transcript mediante SHA-256. `stdout envelope bytes`
es el tamaño exacto del JSON de salida guardado; `structuredContent JSON bytes` es el tamaño del
JSON compacto de `structured` dentro de ese envelope. Son unidades distintas y ambas se derivan del
raw, mientras que p50/p95 conservan los tiempos formales de `/usr/bin/time -p`.

| Wire tool | Muestras | p50 (s) | p95 (s) | Payload stdout (bytes) | Payload structuredContent (bytes) |
|---|---:|---:|---:|---:|---:|
| "workspace_status" | 5 | 0.26 | 0.29 | 1298 | 506 |
| "knowledge_search" | 5 | 0.29 | 0.29 | 800 | 269 |

## Ciclo de cambio App/disco

| Perfil | Escala | Documentos antes | Documentos después | Muestras | p50 (ns) | p95 (ns) | Preparación (ns) | Planes | Applies |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| plano | 100 | 104 | 104 | 10 | 127633375 | 137820000 | 16284417 | 10 | 10 |
| plano | 1000 | 1004 | 1004 | 10 | 353445333 | 385285042 | 135359583 | 10 | 10 |
| plano | 10000 | 10004 | 10004 | 10 | 3086557125 | 3150164292 | 1345983083 | 10 | 10 |
| realista | 100 | 104 | 104 | 10 | 123551083 | 149418125 | 19717209 | 10 | 10 |
| realista | 1000 | 1004 | 1004 | 10 | 372299541 | 382350708 | 153518208 | 10 | 10 |
| realista | 10000 | 10004 | 10004 | 10 | 3252654334 | 3377063500 | 1774696708 | 10 | 10 |
