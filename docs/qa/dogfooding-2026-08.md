# Dogfooding acotado — 2026-08 (E33-H06)

> Evidencia de uso real del repo como workspace Lodestar. Esta entrega registra tres sesiones
> ejecutadas el 2026-08-22; no decide `decisiones §14`, no altera el motor, el contrato ni las
> fichas abiertas. La ventana queda cerrada con el número frío de H04 y un veredicto explícito.

## Alcance y snapshot

- **Fecha/hora de las sesiones:** 2026-08-22, Europe/Madrid (`+0200`). Los timestamps son los
  observados al inicio y al final de cada proceso.
- **Workspace real:** snapshot del repositorio abierto desde un worktree aislado (rama
  `agent/e33-h06`). Las rutas de máquina se normalizan como `<repo-worktree>` en esta evidencia.
- **Snapshot observado:** commit `0c9d0ba7980a0d1059d918844ab3864f361b661b` (develop v0.6.2).
- **Corpus de las consultas:** el árbol real, no un corpus generado. El recuento observado mediante
  `find` fue de **29** ficheros Markdown bajo `decisiones/` y **38** bajo `requirements/`, **67**
  fichas operativas en total. El número frío posterior mide el workspace completo y Lodestar
  descubre **133** documentos Markdown.
- **MCP:** release `lodestar-mcp v0.6.2`, compilado en este worktree con:

  ```text
  cargo build --release -p lodestar-mcp --locked
  ```

  Resultado: `Finished release profile [optimized] target(s) in 13.79s`; la compilación identificó
  explícitamente `lodestar-mcp v0.6.2`.
- **Transporte y perfil:** cada fila es un proceso independiente de
  `docs/qa/testbench/lodestar_harness.py --call`, contra el root real y con `--profile readonly`.
  El arnés aplicó la regla dura para roots reales; ninguna sesión incluyó `shell`, `spawn` ni una
  tool de cambio.
- **Redacción del bench:** `<private-root>` en el anexo JSON es la copia efímera poseída por el
  banco para la medición; no designa el worktree real ni una ruta fija de máquina.
- **Medición:** `/usr/bin/time -p` envolvió cada invocación completa. `real` incluye arranque
  del proceso MCP, `initialize`, la llamada, la lectura de la respuesta y el cierre; no es una
  medición aislada del cuerpo interno de `knowledge_search`.

- **Estado del root antes:**

  ```text
  ## agent/e33-h06
  ?? docs/qa/dogfooding-2026-08.md
  ```

- **Estado del root después de las consultas:** idéntico al anterior; las sesiones no produjeron
  cambios Git fuera del diario. El build solo actualizó el binario release ignorado bajo `target/`.

Las tres filas son procesos MCP independientes, no tres llamadas dentro de una misma sesión.
Las expresiones se copiaron de `decisiones/README.md`, sección «Cómo consultarlas».

## Sesiones ejecutadas

### S01 — prioridad y estado

- **Timestamp:** `START 2026-08-22T01:38:38+0200` —
  `END 2026-08-22T01:38:38+0200`.
- **Tool:** `knowledge_search`.
- **Pregunta:** «¿Qué decisiones abiertas tienen prioridad 4 o mayor y exigen criterio?»
- **Comando ejecutado (rutas normalizadas; argumentos literales):**

  ```text
  /usr/bin/time -p python3 docs/qa/testbench/lodestar_harness.py --root <repo-worktree> --profile readonly --binary <repo-worktree>/target/release/lodestar-mcp --call knowledge_search '{"where":"prioridad >= 4 and estado = \"abierta\""}'
  ```

- **Resultado MCP exacto:** `is_error: false`; `nextCursor: null`; `totalApproximate: 3`.
  Paths devueltos, en el orden del wire: `decisiones/09-transversales-diferidas.md`,
  `decisiones/14-store-sin-consumidor.md`, `decisiones/20-renombrado-del-proyecto.md`.
- **Latencia medida:** `real 0.05 s` (`user 0.04 s`, `sys 0.01 s`).
- **Fricción observada:** ninguna. La expresión literal fue aceptada y devolvió las tres fichas
  esperadas, sin rechazo ni paso adicional.

### S02 — etiqueta

- **Timestamp:** `START 2026-08-22T01:38:42+0200` —
  `END 2026-08-22T01:38:42+0200`.
- **Tool:** `knowledge_search`.
- **Pregunta:** «¿Qué fichas llevan la etiqueta `contrato`?»
- **Comando ejecutado (rutas normalizadas; argumentos literales):**

  ```text
  /usr/bin/time -p python3 docs/qa/testbench/lodestar_harness.py --root <repo-worktree> --profile readonly --binary <repo-worktree>/target/release/lodestar-mcp --call knowledge_search '{"where":"etiquetas contains \"contrato\""}'
  ```

- **Resultado MCP exacto:** `is_error: false`; `nextCursor: null`; `totalApproximate: 11`.
  Paths devueltos, en el orden del wire: `decisiones/03-transporte-mcp-rmcp.md`,
  `decisiones/04-generacion-dts-ts-rs.md`, `decisiones/12-fechas-en-consultas.md`,
  `decisiones/13-conformant-a-valid.md`, `decisiones/15-parametros-no-declarados.md`,
  `decisiones/18-canapply-no-vincula-apply.md`, `decisiones/19-hallazgos-referencia-usuario.md`,
  `decisiones/21-comillas-lenguaje-consulta.md`,
  `decisiones/22-integridad-referencial-frontmatter.md`,
  `decisiones/23-hallazgos-testbench-homelab.md`,
  `decisiones/24-equivalencia-caja-unicode.md`.
- **Latencia medida:** `real 0.05 s` (`user 0.04 s`, `sys 0.01 s`).
- **Fricción observada:** ninguna. La consulta con `contains` se aceptó tal cual y la respuesta
  llegó completa; no hubo necesidad de reformularla.

### S03 — fecha de revisión y estado

- **Timestamp:** `START 2026-08-22T01:38:47+0200` —
  `END 2026-08-22T01:38:47+0200`.
- **Tool:** `knowledge_search`.
- **Pregunta:** «¿Qué decisiones abiertas no se han revisado desde el 2026-07-01?»
- **Comando ejecutado (rutas normalizadas; argumentos literales):**

  ```text
  /usr/bin/time -p python3 docs/qa/testbench/lodestar_harness.py --root <repo-worktree> --profile readonly --binary <repo-worktree>/target/release/lodestar-mcp --call knowledge_search '{"where":"revisada_en < \"2026-07-01\" and estado = \"abierta\""}'
  ```

- **Resultado MCP exacto:** `is_error: false`; `nextCursor: null`; `totalApproximate: 0`;
  `results: []`.
  El resultado vacío es un dato del snapshot, no un error de ejecución.
- **Latencia medida:** `real 0.05 s` (`user 0.04 s`, `sys 0.01 s`).
- **Fricción observada:** ninguna. La comparación de fecha se aceptó y distinguió correctamente
  una respuesta vacía de un error.

## Fricciones y candidatas

No apareció fricción reproducible de entidad en estas tres sesiones: no hubo consulta que exigiera
una forma no documentada, resultado ambiguo, fallo del perfil `readonly` ni mutación accidental.
Por tanto, esta ventana no añade una candidata nueva a `decisiones/` y no se aplicó ninguna
reparación ni decisión por inercia.

## Número frío de H04 sobre el repo real

La corrida release está trazada en el [manifiesto de evidencia de v0.6.2](corridas/v0.6.2/manifest.json)
y su
[`resumen Markdown`](e33-h06-repo-real-2026-08-22.md). Se ejecutó con el binario probado de H04,
semilla 33, sobre una copia privada del snapshot `0c9d0ba` y descubrió **133 documentos**. El smoke
usa dos muestras por tool; por tanto estos p95 son evidencia acotada del repo real, no una baseline
estadística de release.

| Variante | cold-open p95 | Peor lectura p95 | `rebuild` separado |
| --- | ---: | ---: | ---: |
| disco-reparseo | 11,87 ms | 32,10 ms (`workspace_status`) | — |
| SQLite-raw | 10,30 ms | 9,84 ms (`workspace_status`) | 92,86 ms |
| RAM-memoizado | 12,32 ms | 7,94 ms (`workspace_status`) | — |

Las tres variantes devolvieron el mismo resultado normalizado tool por tool. El artefacto conserva
los p50/p95 y payloads de las siete lecturas; el payload mayor fue `graph_query`, **29.214 bytes**
en las tres. SQLite-raw queda rotulado correctamente: ahorra walk+I/O, pero
`DocumentSet::from_store` reparsea los `raw`. El ciclo App/disco, medido aparte, obtuvo p95
**207,52 ms** más **65,69 ms** de preparación de su copia privada.

La calibración wire de este artefacto figura `pending` deliberadamente: las tres sesiones reales de
arriba ya aportan el dato MCP/stdio de uso (**0,05 s** por proceso completo), mientras que la
calibración formal Realista/10k vive en la corrida oficial H04. No se fabrica ni duplica aquí.
La plantilla preservada en el JSON usa placeholders portables: `<CORPUS_10K>` designa el root
Realista/10k, `<PATH>` el directorio del binario MCP, `<WIRE_JSON_STATUS>` y
`<WIRE_JSON_SEARCH>` sus capturas stdout, y `<WIRE_CALIBRATION_JSON>` el bloque agregado de entrada.

## Veredicto final

**A esta escala el reparseo no molesta.** Las tres consultas operativas completas tardaron 0,05 s
cada una, y en el número frío del workspace completo la peor lectura disco-reparseo quedó en
32,10 ms p95. SQLite-raw y RAM reducen parte del coste interno, pero la diferencia absoluta a 133
documentos no produjo fricción perceptible en ninguna sesión. Esto no decide el destino del store:
es solo la mitad de dogfooding que `decisiones §14` pedía, junto a la evidencia de escala de H04.

La ventana de dogfooding queda cerrada con este paquete. Cualquier fricción posterior alimentará
una ficha nueva y no reabrirá este dato.
