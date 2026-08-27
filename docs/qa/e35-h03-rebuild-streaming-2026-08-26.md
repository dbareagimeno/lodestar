---
id: E35-H03-REBUILD-EVIDENCE
historia: E35-H03
fecha: 2026-08-26
estado: evidencia-tecnica
---

# Evidencia E35-H03 — rebuild SQLite streaming

Esta nota resume la matriz opt-in Realista de E35-H03 ejecutada durante la implementación (no es
una corrida de release). Los JSON completos se conservaron como artefactos temporales de esta
verificación; como resultados generados, no se versionan. La corrida por release deberá repetirse en la máquina
baseline y retener, conforme a ARCHITECTURE.md §22, el resumen y manifiesto en Git junto con los
JSON brutos como artefacto externo estable; esta nota no inventa esa URL ni ese manifiesto. La sonda
no participa en CI ni en el gate H05.

## Método reproducible

```text
cargo build --release -p lodestar-bench --locked
target/release/lodestar-bench --extreme --profile realista --scale <1000|10000|100000> --iterations 1 --json-output <salida.json>
```

- Plataforma: `macos/aarch64`; perfil de compilación: `release`; semilla: `33`.
- Base `develop`: `2d5375c4cc657758f0dc2d3b23bf8cc436456cd8`.
- La procedencia declara `working_tree_clean=false` porque la medición incluye el diff todavía no
  committed de E35-H03.
- El informe conserva dos observaciones RSS distintas. El objetivo y la tabla usan el high-water
  mark absoluto del worker SQLite aislado, capturado inmediatamente después del rebuild y antes de
  las queries (`getrusage(RUSAGE_SELF).ru_maxrss` en macOS/Linux y
  `GetProcessMemoryInfo(PROCESS_MEMORY_COUNTERS.PeakWorkingSetSize)` en Windows). El desglose de
  Store, en cambio, muestrea working set residente actual por fase
  (`mach_task_basic_info.resident_size` en macOS, `/proc/self/statm` en Linux y
  `GetProcessMemoryInfo(PROCESS_MEMORY_COUNTERS.WorkingSetSize)` en Windows), incluidos los extremos
  de cada ventana. Se conserva el máximo de cada fase y no se confunde con `max_live_body_bytes`;
  la corrida 100k tomó respectivamente
  `579`/`7558`/`669`/`7` muestras en inventario/indexación/validación/swap.
- El inventario transporta candidatos compactos y no abre cuerpos. La segunda pasada lee cada
  candidato una vez: los UTF-8 válidos se parsean una vez y reutilizan ese `Parsed`; los inválidos
  pasan a `other_files` sin parseo/proyección. La promoción `O(log N)` reata enlaces adelantados con
  valores derivados de `LinkTarget`.
- El inventario transporta los fingerprints capturados durante discovery de la raíz —y su destino
  real si es symlink—, cada entrada y la frontera de directorios recorridos. Discovery comprueba su
  propia estabilidad y Store vuelve a compararlos antes de leer, tras el streaming y justo antes
  del swap; modificaciones, altas, bajas y renames abortan sin un tercer walker. El gate de writer
  combina mutex local con lock nativo interproceso RAII
  (`flock`/`LockFileEx`).
- La conexión `.next` instala un authorizer real de SQLite: todo `DELETE` lógico queda denegado y
  cada prepare del builder se reconcilia con los callbacks de SQLite. Solo se permite el
  mantenimiento de tablas shadow `documents_fts_*` mientras se ejecuta el INSERT FTS auditado o su
  commit; un `DELETE` directo, incluida una shadow table fuera de esa ventana, sigue denegado.
- La proyección conserva en `documents.frontmatter_text` exactamente la misma cadena que inserta en
  FTS, de modo que un upsert/remove posterior puede ejecutar el delete contentless con la tupla
  antigua exacta. La publicación sincroniza el directorio en Unix y usa
  `MoveFileExW(..., MOVEFILE_WRITE_THROUGH)` como barrera equivalente en Windows.

## Resultados

| Escala solicitada | Docs admitidos | Markdown (bytes) | Rebuild (s) | Pico RSS worker tras rebuild (bytes) | SQLite (bytes) | Filas | Prepares | Deletes |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1.000 | 1.004 | 1.339.706 | 0,121 | 12.107.776 | 3.330.048 | 10.165 | 10 | 0 |
| 10.000 | 10.004 | 13.451.914 | 1,157 | 23.052.288 | 32.505.856 | 101.496 | 10 | 0 |
| 100.000 | 100.004 | 134.783.275 | 13,584 | 136.445.952 | 326.725.632 | 1.014.685 | 10 | 0 |

Las cuatro fases de la corrida 100k fueron: inventario `0,867 s`, indexación `11,371 s`, validación
`1,004 s` y swap `0,007 s`. El informe contó `100004` lecturas de candidato,
`914681` inserciones relacionales y `100004` inserciones FTS (`1014685` filas totales). El máximo payload vivo fue `3362`
bytes, separado del RSS diagnóstico. En 1k, donde los métodos no dieron exactamente el mismo valor,
el high-water mark del worker de la tabla fue `12.107.776` bytes y el máximo de las ventanas
internas fue `12.091.392` bytes; ambos valores y sus métodos permanecen explícitos en el JSON.

La equivalencia funcional de las tres variantes fue `true` en las tres escalas. El número de
statements preparados permaneció constante y no hubo deletes lógicos durante la construcción de
`.next`; los deletes internos de compactación FTS5 no son statements del builder y quedan acotados
por el authorizer a la ejecución/commit del INSERT FTS auditado.

## Interpretación

En esta máquina y corrida, Realista/100k quedó por debajo de los objetivos de ingeniería de 60 s y
512 MiB. Ambos siguen publicados con `gate=false`: esta evidencia no ratifica una máquina de
referencia, no convierte los valores en gate y no promete rendimiento externo. SQLite continúa
fuera del camino normal de lectura mientras `decisiones §14` siga abierta.

La autenticidad del flujo se fija además con tests de integración: política canónica, una sola
lectura de cuerpo en la segunda pasada, RSS externo, carga insert-only, integridad antes del swap,
preservación del índice activo, writer gate, paridad core/store y salida pública del benchmark sin
seams de test.
