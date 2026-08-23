# E33-H08 — Paquete de evidencia para §14 (2026-08-22)

Este documento reúne la evidencia que faltaba para que [`decisiones §14`](../../decisiones/14-store-sin-consumidor.md)
quede **lista para decidir**. No escoge entre conectar, acotar o retirar el store: las tres salidas,
el estado `abierta` y la prioridad 5 siguen intactos. La decisión y su ratificación quedan fuera de
E33, como exige `ARCHITECTURE.md §21.5`/§22.8.

## 1. Trazabilidad y método

La corrida sintética completa de H04 está identificada por el
[manifiesto de evidencia de v0.6.2](corridas/v0.6.2/manifest.json), con resumen en
[`e33-h04-banco-rendimiento-2026-08-22.md`](e33-h04-banco-rendimiento-2026-08-22.md). El bruto
comprimido se conserva en la release de GitHub, fuera de Git.
Su procedencia declara `macos/aarch64`, `lodestar-bench` release, semilla `33`, commit
`0c9d0ba7980a0d1059d918844ab3864f361b661b`, perfiles `plano`/`realista` y escalas
`100`/`1000`/`10000`. La procedencia del JSON marca `working_tree_clean: false`; por ello este
paquete identifica el commit y no presenta la medición como una release limpia.

La corrida sobre el repo real de H06 está identificada en el [manifiesto de evidencia de v0.6.2](corridas/v0.6.2/manifest.json),
con resumen en [`e33-h06-repo-real-2026-08-22.md`](e33-h06-repo-real-2026-08-22.md) y diario de uso en
[`dogfooding-2026-08.md`](dogfooding-2026-08.md). Ambos JSON se verificaron con `jq` para las
claves, conteos de documentos, variantes, herramientas, muestras, p50, p95, payload y rebuild que
aparecen abajo. Las unidades son nanosegundos (`ns`) y bytes (`B`) salvo que se indique `s` o `ms`.

El enganche de H07 se trata aquí con el estado que realmente tiene: [`RELEASING.md`](../../RELEASING.md)
define el recorrido de release, la convención de [`docs/qa/corridas/v0.6.2/`](corridas/v0.6.2/)
conserva la primera corrida local, y [`testbench.yml`](../../.github/workflows/testbench.yml) deja
preparado el `workflow_dispatch` de conformidad y smoke. El YAML, el runbook y el smoke del bench
se validaron localmente; todavía no existe un run remoto enlazable porque no se ha hecho commit ni
push. H08 no cuenta esa BDD remota como ejecutada: H07 queda localmente listo, con la verificación
externa pendiente.

H05 fija sus techos y su comparación en [`umbrales.json`](testbench/umbrales.json), la baseline
anonimizada [`e33-h05-baseline-release-macbook-2026-08.json`](e33-h05-baseline-release-macbook-2026-08.json)
y la propuesta [`e33-h05-propuesta-ratificacion-2026-08-22.md`](e33-h05-propuesta-ratificacion-2026-08-22.md).
Son un gate absoluto del camino `disk-reparseo` en la máquina baseline; no juzgan SQLite-raw ni
RAM-memoizado y no convierten esas alternativas medidas en un veto.

Los hashes SHA-256, tamaños y esquemas de los cinco brutos externos que fijan la evidencia están
en el [manifiesto de v0.6.2](corridas/v0.6.2/manifest.json). Los artefactos wire, matrices y
calibraciones pequeñas permanecen versionados.

| Artefacto | SHA-256 |
|---|---|
| [`e33-h04-wire-evidencia-2026-08-22.json`](e33-h04-wire-evidencia-2026-08-22.json) | `05dbd305196168c27095577a30990f4372c81cc20cd03170ae521a49af8d9a8f` |
| [`e33-h04-wire-artifact-2026-08-22.json`](e33-h04-wire-artifact-2026-08-22.json) | `7b393f5fb8d70cf9694497a2055ef450be8d67e9424fdee8715410541395f277` |
| [`e33-h09-realista-100k-2026-08-23.md`](e33-h09-realista-100k-2026-08-23.md) | `b360cb0d92589cfce8af62fd0cd76f0652a21ffb1ca494bc52113a6608a9cda7` |

En las tablas compactas `p50/p95/payload` significa `p50_ns/p95_ns/payload_bytes`. El cold-open
de cada variante tiene una sola muestra en H04 y H06; cada tool tiene 10 muestras en H04 y 2 en
H06. El rebuild separado tiene una muestra. No son intervalos de confianza ni una baseline
estadística: son las observaciones versionadas de estas corridas.

## 2. Medición H04: 3 variantes × 3 escalas

La tabla completa conserva ambos perfiles del generador. `tools` está en el orden
`workspace_status`, `knowledge_search`, `knowledge_get`, `metadata_inspect`, `graph_query`,
`impact_analyze`, `knowledge_check`; cada valor es `p50/p95/payload` (`ns/ns/B`). `cold-open` es
`p50/p95/payload` y `rebuild` es `p50/p95` (`ns`, solo SQLite-raw). Las cifras son las rutas
`.runs[].measurements[]` del JSON H04, comprobadas con `jq`.

| Perfil | Escala | Variante | Docs | Cold-open | Rebuild | Tools: p50/p95/payload (`ns/ns/B`) |
|---|---:|---|---:|---:|---:|---|
| plano | 100 | disk-reparseo | 104 | 2054041/2054041/520 | — | `workspace_status=2056417/9092958/520; knowledge_search=2110833/5594833/269; knowledge_get=3679792/6717417/747; metadata_inspect=1656958/3246958/213; graph_query=1875375/3309250/6807; impact_analyze=3671792/6341542/261; knowledge_check=2054959/3242708/825` |
| plano | 100 | sqlite-raw | 104 | 1579791/1579791/520 | 9528375/9528375 | `workspace_status=947625/1045125/520; knowledge_search=990000/1029500/269; knowledge_get=735042/749667/747; metadata_inspect=564375/573792/213; graph_query=762583/780125/6807; impact_analyze=731000/747917/261; knowledge_check=953833/968042/825` |
| plano | 100 | ram-memoizado | 104 | 2132417/2132417/520 | — | `workspace_status=219792/416708/520; knowledge_search=268708/271625/269; knowledge_get=5375/6958/747; metadata_inspect=10625/12625/213; graph_query=31875/37167/6807; impact_analyze=2000/3833/261; knowledge_check=228458/234916/825` |
| plano | 1000 | disk-reparseo | 1004 | 20000625/20000625/523 | — | `workspace_status=20283500/25246291/523; knowledge_search=20371625/29547000/269; knowledge_get=35700542/36712167/747; metadata_inspect=15569750/21819000/214; graph_query=17967666/21647084/6807; impact_analyze=35810916/37414084/261; knowledge_check=20147500/25785875/825` |
| plano | 1000 | sqlite-raw | 1004 | 10879666/10879666/523 | 262127250/262127250 | `workspace_status=9769417/10446209/523; knowledge_search=10172167/10564250/269; knowledge_get=7561208/7838042/747; metadata_inspect=5744125/5948917/214; graph_query=7772791/7942375/6807; impact_analyze=7525208/7750834/261; knowledge_check=9848250/10057167/825` |
| plano | 1000 | ram-memoizado | 1004 | 21487583/21487583/523 | — | `workspace_status=2243250/4142333/523; knowledge_search=2634250/2657083/269; knowledge_get=6417/7792/747; metadata_inspect=103833/120292/214; graph_query=259583/268875/6807; impact_analyze=2917/4625/261; knowledge_check=2384042/2449167/825` |
| plano | 10000 | disk-reparseo | 10004 | 224462416/224462416/526 | — | `workspace_status=224225000/229693000/526; knowledge_search=228169834/229671542/269; knowledge_get=403041750/405713625/747; metadata_inspect=181219458/182683708/215; graph_query=204340292/206433750/6807; impact_analyze=402410792/407018125/261; knowledge_check=224042791/225142916/825` |
| plano | 10000 | sqlite-raw | 10004 | 115669416/115669416/526 | 28957367000/28957367000 | `workspace_status=108429875/110258625/526; knowledge_search=112807958/114909083/269; knowledge_get=85944458/87259792/747; metadata_inspect=66463208/68033125/215; graph_query=88890291/91052875/6807; impact_analyze=86511459/87978917/261; knowledge_check=109000583/112918416/825` |
| plano | 10000 | ram-memoizado | 10004 | 223668667/223668667/526 | — | `workspace_status=22135958/42464542/526; knowledge_search=26481792/27025917/269; knowledge_get=11334/15500/747; metadata_inspect=1121042/1594000/215; graph_query=2734083/2827334/6807; impact_analyze=6958/10417/261; knowledge_check=23468583/23705541/825` |
| realista | 100 | disk-reparseo | 104 | 2222875/2222875/526 | — | `workspace_status=2271291/3354667/526; knowledge_search=2635167/2844458/269; knowledge_get=4348541/4472542/747; metadata_inspect=1835792/1857042/372; graph_query=2229083/2267792/19962; impact_analyze=4345750/4399459/261; knowledge_check=2303750/2325625/6161` |
| realista | 100 | sqlite-raw | 104 | 1682792/1682792/526 | 9648875/9648875 | `workspace_status=1120458/1178833/526; knowledge_search=1463000/1500250/269; knowledge_get=1002041/1008042/747; metadata_inspect=716125/724042/372; graph_query=1081042/1095500/19962; impact_analyze=1000167/1005083/261; knowledge_check=1137333/1146125/6161` |
| realista | 100 | ram-memoizado | 104 | 2356459/2356459/526 | — | `workspace_status=118292/429459/526; knowledge_search=462333/474750/269; knowledge_get=4625/5542/747; metadata_inspect=22291/23958/372; graph_query=71042/82209/19962; impact_analyze=1833/5958/261; knowledge_check=138250/140916/6161` |
| realista | 1000 | disk-reparseo | 1004 | 21233583/21233583/532 | — | `workspace_status=21353500/23253000/532; knowledge_search=24803208/26122917/269; knowledge_get=40771625/42056666/747; metadata_inspect=16825000/17149333/379; graph_query=20584250/20813750/9608; impact_analyze=40581250/41147000/261; knowledge_check=21484167/21748625/35785` |
| realista | 1000 | sqlite-raw | 1004 | 12635250/12635250/532 | 168679333/168679333 | `workspace_status=11335000/11831666/532; knowledge_search=14738250/14901958/269; knowledge_get=10131459/10232542/747; metadata_inspect=7012500/7064500/379; graph_query=10636875/10709375/9608; impact_analyze=10127125/10188208/261; knowledge_check=11488250/11637125/35785` |
| realista | 1000 | ram-memoizado | 1004 | 22098916/22098916/532 | — | `workspace_status=1162416/4526042/532; knowledge_search=4605125/4641500/269; knowledge_get=4917/7459/747; metadata_inspect=217542/221917/379; graph_query=536334/555834/9608; impact_analyze=1875/3583/261; knowledge_check=1375334/1403792/35785` |
| realista | 10000 | disk-reparseo | 10004 | 248825583/248825583/539 | — | `workspace_status=248179916/265405167/539; knowledge_search=283462958/288724209/269; knowledge_get=477197042/481312208/747; metadata_inspect=199995667/201016375/386; graph_query=242525667/243586125/7382; impact_analyze=477926542/481542291/261; knowledge_check=250406209/251015708/36030` |
| realista | 10000 | sqlite-raw | 10004 | 132058416/132058416/539 | 14046911667/14046911667 | `workspace_status=121928959/123002541/539; knowledge_search=156375958/157917750/269; knowledge_get=110394916/111349083/747; metadata_inspect=76573542/77454584/386; graph_query=116082958/117066625/7382; impact_analyze=109943125/111245833/261; knowledge_check=123854166/125181041/36030` |
| realista | 10000 | ram-memoizado | 10004 | 249147083/249147083/539 | — | `workspace_status=11845625/48005042/539; knowledge_search=46254708/46493500/269; knowledge_get=8000/11417/747; metadata_inspect=2310916/2371208/386; graph_query=6215250/6262833/7382; impact_analyze=4125/6459/261; knowledge_check=13761042/13780791/36030` |

### Ciclo `change_plan` → `change_apply`

Este ciclo se mide sobre la copia privada de App/disco y se reporta una vez por perfil y escala;
no se presenta como una medición de las tres variantes de lectura. `p50/p95/payload` pertenece a
la respuesta del ciclo; `preparación` es el coste de preparar la copia. Los seis renglones se
verificaron en `.runs[].change_cycle` del JSON H04.

| Perfil | Escala | Docs antes/después | Muestras | p50/p95/payload (`ns/ns/B`) | Preparación (`ns`) | Planes/applies |
|---|---:|---:|---:|---:|---:|---:|
| plano | 100 | 104/104 | 10 | 127633375/137820000/580 | 16284417 | 10/10 |
| plano | 1000 | 1004/1004 | 10 | 353445333/385285042/580 | 135359583 | 10/10 |
| plano | 10000 | 10004/10004 | 10 | 3086557125/3150164292/580 | 1345983083 | 10/10 |
| realista | 100 | 104/104 | 10 | 123551083/149418125/580 | 19717209 | 10/10 |
| realista | 1000 | 1004/1004 | 10 | 372299541/382350708/582 | 153518208 | 10/10 |
| realista | 10000 | 10004/10004 | 10 | 3252654334/3377063500/584 | 1774696708 | 10/10 |

### Calibración wire

La calibración oficial H04 está completa en [`e33-h04-wire-artifact-2026-08-22.json`](e33-h04-wire-artifact-2026-08-22.json)
y su captura trazable en [`e33-h04-wire-evidencia-2026-08-22.json`](e33-h04-wire-evidencia-2026-08-22.json).
Es `lodestar-mcp` release, perfil `readonly`, transporte JSON-RPC/stdio, corpus `realista` a escala
`10000`, `10004` documentos, cinco muestras por tool. El payload stdout es el envelope completo;
`structuredContent` es el JSON compacto interno. El JSON H04 marca `status: complete`.

| Tool wire | Muestras | p50/p95 (`s`) | stdout (`B`) | structuredContent (`B`) | Resultado |
|---|---:|---:|---:|---:|---|
| `workspace_status` | 5 | 0.26/0.29 | 1298 | 506 | `documents=10004`, `is_error=false` |
| `knowledge_search` | 5 | 0.29/0.29 | 800 | 269 | `is_error=false`, `total_approximate=1` |

La calibración mide el proceso completo (arranque MCP, `initialize`, llamada, respuesta y cierre),
no solo el cuerpo de `App`. Por tanto no se suma a los nanosegundos de la tabla: sirve para acotar
el overhead del framing y del proceso real.

## 3. Dogfooding y número frío del repo real (H06)

El diario [`dogfooding-2026-08.md`](dogfooding-2026-08.md) registra tres sesiones reales en
`readonly`, contra el árbol del repo y usando literalmente las consultas operativas de
`decisiones/README.md`:

1. `prioridad >= 4 and estado = "abierta"` → tres fichas abiertas, sin error.
2. `etiquetas contains "contrato"` → 11 fichas, sin error.
3. `revisada_en < "2026-07-01" and estado = "abierta"` → resultado vacío, sin error.

El snapshot tenía 29 documentos Markdown bajo `decisiones/` y 38 bajo `requirements/` (67 fichas
operativas); el banco completo del snapshot descubrió 133 documentos. Las tres sesiones no
observaron fricción, reformulación, error de perfil ni mutación accidental. El diario registra
`0.05 s` de tiempo de proceso por sesión y cierra la ventana con el veredicto «a esta escala el
reparseo no molesta». Ese veredicto es percepción acotada, no una decisión sobre el store.

El bruto H06 (`schema_version: e33-h04-v2`, commit `0c9d0ba`, semilla `33`) se verificó con `jq`
antes de publicarlo; su URL, hash y tamaño constan en el manifiesto.
Tiene 133 documentos en las tres variantes, dos muestras por tool, una de cold-open y una de
rebuild. La tabla conserva el mismo formato `p50/p95/payload` (`ns/ns/B`) y las siete tools.

| Escala observada | Variante | Cold-open (`p50/p95/payload`) | Rebuild (`p50/p95 ns`) | Tools: p50/p95/payload (`ns/ns/B`) |
|---|---|---:|---:|---|
| repo-real (133) | disk-reparseo | 11872000/11872000/466 | — | `workspace_status=32098458/32098458/466; knowledge_search=11673250/11673250/53; knowledge_get=11171208/11171208/203; metadata_inspect=4191958/4191958/254; graph_query=11968417/11968417/29214; impact_analyze=11224542/11224542/203; knowledge_check=11981792/11981792/3244` |
| repo-real (133) | sqlite-raw | 10299083/10299083/466 | 92859458/92859458 | `workspace_status=9835375/9835375/466; knowledge_search=9126875/9126875/53; knowledge_get=8718791/8718791/203; metadata_inspect=1670208/1670208/254; graph_query=9535750/9535750/29214; impact_analyze=8724875/8724875/203; knowledge_check=9605875/9605875/3244` |
| repo-real (133) | ram-memoizado | 12316709/12316709/466 | — | `workspace_status=7941292/7941292/466; knowledge_search=398375/398375/53; knowledge_get=542/542/203; metadata_inspect=15375/15375/254; graph_query=847416/847416/29214; impact_analyze=375/375/203; knowledge_check=856833/856833/3244` |

El ciclo App/disco de H06 fue de 2 muestras: `p50=207521833 ns`, `p95=207521833 ns`, payload
`580 B`, preparación `65687333 ns`, 2 planes y 2 applies, con 134 documentos antes y después.
Las tres variantes devolvieron el mismo resultado normalizado tool por tool. El mayor payload de
lectura fue `graph_query`, `29214 B`, en las tres variantes.

La calibración wire de H06 queda deliberadamente `pending` en su JSON: no se mezclan sus sesiones
de uso con la calibración formal H04. El dato wire formal es el bloque H04 anterior.

## 4. Inventario actual del coste de conexión

La inspección se hizo contra el árbol de esta revisión, con `nl -ba` y búsquedas de llamadores; no
se infiere de la memoria de E18. Las líneas son las que deben volver a comprobarse si cambia el
árbol antes de decidir.

| Punto | Evidencia en el árbol actual | Coste o riesgo si se conecta |
|---|---|---|
| Walker del store sin `DiscoveryPolicy` | `crates/lodestar-store/src/lib.rs:240-304` define `walk_disk()` con su propio `ignore::WalkBuilder` (`:256-263`), solo activa `.hidden(false)`, `.git_ignore(true)` y excluye `.lodestar`/`.git`; no recibe ni aplica `DiscoveryPolicy`. El walker del producto es distinto: `crates/lodestar-workspace/src/discovery.rs:164-188` recibe la política, aplica `exclude`/`include`, `follow_symlinks`, `respect_gitignore`, `parents(false)`, `git_global(false)`, `git_exclude(false)` y orden determinista. | Conectar el store sin alinear ambos inventarios puede indexar documentos que el core excluye (o ignorar documentos que el core incluye), rompiendo la paridad cuando la cache pase a ser camino de lectura. También quedan sin trasladar `max_document_bytes` y `.lodestarignore` (`discovery.rs:298-325`, `:186-188`). |
| `field_path` store vs core | El store recorre `ParsedFrontmatter::walk()` y persiste `field_path.to_string()` en `crates/lodestar-store/src/index.rs:172-198` (en concreto `:185` y `:195-197`). El core publica el catálogo con anclaje para namespaces reservados en `crates/lodestar-core/src/metadata.rs:65-105` (`:77-79`); la forma `frontmatter.<campo>` está definida por `FieldPath::anclado()` en `crates/lodestar-core/src/types.rs:492-510`. | Para una clave de usuario que colisione con `graph`/`document`, el catálogo core rinde `frontmatter.graph.backlinks` mientras la fila SQL conserva el nombre crudo. Hoy no hay discrepancia observable porque ninguna tool lee esa columna SQL; al conectar hay que decidir y probar una sola representación, no crear un dialecto paralelo. |
| Watcher y su papel | `crates/lodestar-store/src/watch.rs:20-61` arranca `notify-debouncer-full` y llama `reconcile_all()` con gate de hash; `:31-48` filtra ecos de `.lodestar` y errores. `crates/lodestar-workspace/src/lib.rs:239-267` solo lo pone vivo desde `open_live()`/`enable_cache()`, después de `rebuild()`. | El watcher es la pieza de reconciliación de la cache, no del core: al conectar tendría que invalidar/reconstruir la lectura y conservar el único escritor. Al acotar puede quedar explícitamente como capacidad de `reindex`/consumidores externos; al retirar desaparece junto con la cache. Ninguna salida puede dejar un watcher «vivo» sin consumidor definido. |
| Apertura y callers de cache | `crates/lodestar-app/src/lib.rs:428-435` hace `App::open → Workspace::open`; `Workspace::open` declara la cache inactiva en `crates/lodestar-workspace/src/lib.rs:85-118`. El único caller de producción encontrado para `enable_cache()` es `crates/lodestar-cli/src/commands.rs:155-166` (`lodestar reindex`). `open_live()`/`enable_cache()` aparecen además en tests del crate workspace, no en la fachada App/MCP. | MCP/App no leen SQLite hoy. Las lecturas de App vuelven a `Workspace::document_set()` en `crates/lodestar-app/src/lib.rs:523-525` (workspace_status), `:666-677` (knowledge_search), `:838-846` (knowledge_get), `:958-965` (metadata_inspect), `:1098-1107` (knowledge_check), `:1383-1394` (graph_query), `:1586-1603` (impact_analyze), y en `:1696-1715`, `:1961-1982`, `:2218-2250` (plan/apply/revert). `Workspace::document_set_with_discovery()` llama a `discovery::discover` en `:329-332`. Conectar cambia este camino y toca las diez tools; acotar deja el camino actual documentado. |
| Rebuild y `from_store` | `crates/lodestar-store/src/lib.rs:94-115` hace `rebuild()` recorriendo y parseando para poblar SQLite; `:368-377` expone `document_set()` mediante `DocumentSet::from_store`. La advertencia de `ARCHITECTURE.md §22.5` se conserva: `from_store` vuelve a parsear `raw`; SQLite-raw ahorra walk+I/O, no el parseo. | La cifra de rebuild es parte del precio recurrente de construir la cache: H04 llega a `28957367000 ns` (plano/10k) y `14046911667 ns` (realista/10k); H06 observa `92859458 ns` para 133 documentos. La decisión debe decir cuándo se paga y qué invalida la cache. |

El hallazgo de callers también explica por qué el watcher «no corre en el motor» en el sentido de
la ficha: existe y está testeado como API de workspace, pero `App::open`/MCP no llaman a
`enable_cache`; solo el camino explícito `reindex` lo activa. No se confunde la capacidad construida
con un consumidor real.

## 5. Análisis neutral de las tres salidas

Los datos separan tres cosas que la ficha debía dejar juntas: el coste del camino actual, el precio
de materializar SQLite y la cota de una memoria ya construida.

### (a) Conectar el store

La medición aporta una mejora observada y dos costes concretos, sin convertirlos en un veredicto:

- En realista/10k, el mayor p95 de las siete lecturas es `481542291 ns` en disk-reparseo,
  `157917750 ns` en SQLite-raw y `48005042 ns` en RAM-memoizado. SQLite queda entre el reparseo y
  la memoria; el salto SQLite no equivale al salto de un `DocumentSet` retenido.
- SQLite-raw hace `rebuild` en `14046911667 ns` en realista/10k y `28957367000 ns` en plano/10k.
  Es un coste de preparación que no aparece en cada llamada, pero sí en arranques/reconstrucciones.
- El wire real a 10k añade el proceso MCP completo: `workspace_status` p95 `0.29 s` y
  `knowledge_search` p95 `0.29 s`, con envelopes de `1298 B` y `800 B`. No es lícito sumar ese
  tiempo al p95 de App; es una calibración separada.

Conectar tendría que especificar qué se conecta. Hay al menos tres interpretaciones distintas:

1. **SQLite como fuente de `DocumentSet` por llamada**: reutiliza el índice y su watcher, pero
   conserva el reparseo de `raw` documentado por `from_store`.
2. **Memoización RAM del `DocumentSet`**: evita walk, I/O y parseo entre llamadas, pero necesita
   invalidación y un lugar de vida de la memoria; no es lo mismo que leer SQLite.
3. **Ambas**: SQLite/watcher para persistencia e invalidación y un `DocumentSet` RAM para servir la
   sesión; añade dos estados derivados y exige demostrar que el core sigue ganando ante drift.

La opción (a) también debe pagar el alineamiento de `DiscoveryPolicy`, la representación anclada
     de `field_path`, la invalidación por hash y el papel exacto del watcher. Este paquete deja
     esas preguntas visibles; no elige una implementación.

### (b) Acotar el store

Acotar es compatible con los hechos observados si se declara explícitamente que el motor sigue
leyendo del disco y que SQLite/watcher solo sirven a `lodestar reindex` y a consumidores externos.
La ventaja es conservar la capacidad derivada sin cambiar las diez tools ni el invariante #3. El
coste es aceptar por escrito el reparseo por llamada y no contar con los p95 de SQLite/RAM como
rendimiento del producto. También habría que mantener documentados los dos riesgos latentes
(`DiscoveryPolicy` y `field_path`) para que un futuro consumidor no conecte una cache que ya nace
con inventario o nombres divergentes.

El dogfooding no mostró fricción en 133 documentos: tres consultas reales en `readonly`, sin
reformulación, con el veredicto del diario de que el reparseo no molesta a esa escala. Eso no
extrapola a 10k, pero sí evita tratar el número sintético como única señal de uso.

### (c) Retirar el store

Retirar elimina DDL, rebuild, FTS, watcher, bus y el coste de mantener la paridad store↔core. El
camino App/disco seguiría siendo la implementación completa y el core conservaría la verdad. La
evidencia que debe quedar delante de esta salida es que RAM-memoizado es mucho menor en varias
tools a 10k, pero no es una prueba de que el camino disco sea suficiente ni de que la memoria pueda
vivir con la semántica de invalidación del producto. Retirar también sacrifica la opción de que un
consumidor externo use la API pública de `lodestar-store`; ese impacto de superficie y migración no
se midió aquí.

La evidencia actualiza la recomendación histórica de (a) solo en un sentido documental: ya no se
apoya en ausencia de números, pero tampoco se convierte en una elección. La recomendación sigue
escrita en §14 como hipótesis pendiente de ratificación; H08 no la adopta ni la revoca porque no
mide todavía el alineamiento de política, la invalidación ni el ciclo de vida del watcher que
exigiría una conexión real. Las tres salidas quedan para la ratificación que el usuario haga con
estos datos y el inventario delante.

## 6. Umbrales internos y honestidad externa

Los anclajes/umbrales de H05 son **máximos internos** para conversar y gatear en la máquina
baseline: **p95 ≤ 1 s por tool de lectura a 10k** y **cold-open ≤ 5 s**. No son un veto a SQLite:
H05 excluye deliberadamente umbrales sobre SQLite-raw/RAM-memoizado porque son alternativas de
medición, no el producto mientras §14 siga abierta. Los absolutos solo se juzgan en la máquina de
baseline; CI compartido ejecuta smoke sin absolutos.

La corrida H04 queda por debajo de esos anclajes en el camino disk-reparseo observado (por ejemplo,
el máximo de lectura realista/10k es `481542291 ns` y el cold-open es `248825583 ns`), pero ese
hecho no convierte los anclajes en promesa de superficie ni sustituye su ratificación/gate. La
comparación wire es un proceso completo y tiene su propia escala y muestra.

## 5a. Extensión E33-H09: Realista/100k

La sonda extrema queda implementada de forma permanente en `crates/lodestar-bench` y acepta
`--scale N` positivo sin whitelist. La corrida está identificada en el manifiesto, con
[resumen H09](e33-h09-realista-100k-2026-08-23.md) versionado y el bruto externo publicado,
Markdown usan una iteración, 100.000 documentos solicitados más cuatro controles (100.004 reales),
134.783.275 bytes Markdown y 659.578.880 bytes SQLite (`main_bytes`; WAL 0, SHM 0, auxiliares 0
en el artefacto cerrado). Las tres variantes × siete lecturas devuelven resultados funcionalmente
equivalentes. El rebuild SQLite fue `1_868_411_002_750 ns` (~31,14 min); los workers aislados (`worker_isolated=true`) registraron
pico RSS absoluto de 972.439.552 bytes (disco), 985.530.368 bytes (SQLite) y 839.974.912 bytes
(RAM), medido con `getrusage(RUSAGE_SELF).ru_maxrss`; el baseline fue 2.637.824 bytes por worker y
los deltas fueron 969.801.728, 982.892.544 y 837.337.088 bytes respectivamente. Son valores
reconciliables (`baseline + delta = absolute`).
Estas cifras son evidencia
interna para futuras mejoras, no una promesa ni un nuevo umbral de producto.

El preflight de H09 comprueba espacio de disco antes de escribir y declara que no verifica memoria;
su resultado estructurado es `checked` cuando `df` aporta disponible/requerido y `unverified` cuando
no puede comprobarlos. En la corrida R4 fue `checked`, `confirmed=false`, con `available_bytes=8.729.272.320`
y `required_bytes=3.545.235.456`; `memory_verification` quedó estructurado como `unverified` con
motivo explícito. El espacio no verificable exige confirmación a cualquier escala; para 1M
o más la confirmación es siempre obligatoria después del preflight, sin ignorar insuficiencia
comprobada. El presupuesto vigente (`32 KiB × scale + 256 MiB`) es una heurística conservadora y
modificable, no un contrato ni un criterio para descartar la base de datos. Wire y escritura quedan
fuera de la sonda. §14 permanece abierta y espera la
ratificación normativa del usuario.

`ARCHITECTURE.md §21.5` sigue vigente: mientras §14 esté abierta, la documentación externa no
presenta `reindex`/SQLite como camino de lectura ni promete rendimiento a escala. Este documento es
paquete interno de decisión, no una garantía de producto.

## 7. Lista de pronunciamientos pendientes

Para cerrar §14, la decisión tendrá que pronunciar explícitamente:

1. **Salida principal**: ¿se elige conectar, acotar o retirar el store?
2. **Si se conecta**: ¿el consumidor de lectura será SQLite-raw, una memoización RAM de
   `DocumentSet`, o ambas capas? ¿Cuál es la invalidación por hash y qué ocurre ante drift donde
   gana el core?
3. **Inventario**: ¿se alinea el walker del store con la `DiscoveryPolicy` de §20.5 —incluidos
   `include`/`exclude`, `.lodestarignore`, `maxDocumentBytes` y determinismo— antes de exponerlo?
4. **Nombres**: ¿se adopta la representación anclada de `field_path` del core para que catálogo,
   consultas y store describan el mismo campo?
5. **Watcher**: ¿qué papel normativo tiene en la salida elegida (único escritor/reconciliador,
   capacidad solo para `reindex`/externos, o retirada), y cuál es su ciclo de vida?
6. **Superficie y estado**: ¿qué contrato, documentación, tests de paridad y estado de
   `IMPLEMENTATION_STATUS.md` se actualizan junto con la salida, manteniendo `§21.5` hasta que la
   decisión esté ratificada?

No hay una salida elegida en este paquete.

lista para decidir
