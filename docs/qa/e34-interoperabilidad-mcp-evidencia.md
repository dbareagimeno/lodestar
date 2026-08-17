# E34 — evidencia de ejecución

Fecha de ejecución: 2026-08-17. Base: `develop`.

Este registro acompaña a la épica y permite auditar las fases TDD sin depender de la conversación
del implementador. Los snapshots completos de alcance se generaron con `phase-scope.py snapshot`
en `target/agent-state/e34-hNN/pre-red.json`; los locks se generaron después del rojo con
`tdd-test-lock.py snapshot` y se verificaron antes y después de cada verde.

## Rojo independiente y lock

| Historia | Resultado rojo observado antes de producción | Test bloqueado (SHA-256) |
| --- | --- | --- |
| H01 | La matriz de policy/MSRV/capacidades no encontraba la política dual ni las dependencias ratificadas. | `e34_h01_policy.rs` `6ff7acfbb35a9e953f45e6ad4945f2931c2bea6589a7f0689b856ff09d91f618` |
| H02 | La suite no podía importar ni ejercer `LodestarMcpService`; no existía el seam neutral. | `e34_h02_service.rs` `321e4ea2e9a5ac6dc3172b751f3e4687ffa080999a23c4d93603fa8a91057b56` |
| H03 | Los casos de transporte detectaron el lector/framing manual y la ausencia del cliente oficial rmcp. | `e34_h03_transport.rs` `97183aa067f1104db511f654bd01adea26a57baf196bcebfc42d3784f3c3252e` |
| H04 | Discovery y metadata Modern no existían; las respuestas carecían de `resultType` y cache hints. | `e34_h04_modern.rs` `e8e2f833cb09eb9ec8e204dbea7ba85358a97ac2fbd3215638e221582e4f8e7d` |
| H05 | La reproducción cruda de issue #38 devolvía el rechazo histórico en vez de negociar Legacy. | `e34_h05_legacy.rs` `a93981f6ec9736a379d5c1dc85fcb3554baabb95413aaf1932b6810849d12e23` |
| H06 | Tres pruebas pasaban y `cancelacion_transaccional_sin_parciales` fallaba porque la request cancelada llegaba a alterar el Markdown. | `e34_h06_conformance.rs` `0c07d34aede2b97b4377ad65600acb4c728beed34d766cab429c2d726754831e` |

`verify-tests-only` pasó en cada fase roja. `tdd-test-lock.py verify` pasó antes y después de cada
verde. H06 necesitó un microciclo de formato: se restauró el fallo de cancelación, se ejecutó
`cargo fmt`, se regeneró el lock y se volvió a aplicar el verde sin modificar el test bloqueado.

Tras publicar la PR, CI #100 aportó un rojo de portabilidad independiente en Windows: el probe C3
generaba rutas `D:\...` sin escapar dentro de `Cargo.toml`. El arreglo sólo sustituyó la
interpolación textual de las tres rutas por `toml_basic_string`; no cambió el escenario, sus
requests ni sus aserciones. El lock H03 original del rojo funcional era
`ce3c6fe1cffa91a19635ac34a2228ac8c28122ff6af7d045f192268d39e99885`; después del microciclo de
portabilidad se regeneró al hash H03 mostrado en la tabla y `tdd-test-lock.py verify` volvió a
terminar con `exit 0`.

CI #102 comprobó que C3 ya pasaba en Windows y reveló el mismo defecto de escape en el manifest
auxiliar de cancelación H06. Se aplicó el mismo escape TOML exclusivamente a las rutas de
`lodestar-mcp` y `lodestar-app`; el probe, la barrera, las requests y las aserciones de cancelación
permanecieron intactos. El lock H06 original del rojo funcional era
`f790ba0df21656c4d5f620bf4ada7f3d935f55cdb922e6cdb1e644dd4bdc33ad`; tras este microciclo se
regeneró al hash H06 mostrado en la tabla y su `verify` terminó con `exit 0`.

## Verde y gates

Resultados finales observados antes de la revisión:

```text
E34-H01: 3 passed; 0 failed; 1 ignored (gate explícito de toolchains)
E34-H02: 3 passed; 0 failed
E34-H03: 5 passed; 0 failed
E34-H04: 8 passed; 0 failed
E34-H05: 7 passed; 0 failed
E34-H06: 4 passed; 0 failed
lodestar-mcp/tests/mcp.rs: 160 passed; 0 failed
scripts/agent-gates.sh contract: exit 0
scripts/agent-gates.sh policy: exit 0
scripts/agent-gates.sh full: exit 0
```

Los clientes auxiliares oficiales ejecutan Cargo offline y reciben
`E34_TOKIO_STREAM_SOURCE`. `scripts/agent-gates.sh full` la localiza; CI obtiene exactamente
`tokio-stream = 0.1.17` antes de ejecutar las suites. Un `cargo test` directo de esas suites debe
configurar la misma variable o usar el gate, para no convertir la disponibilidad de red durante
el test en parte del contrato.

La matriz CI ejecuta además el workspace sin MCP con Rust 1.80.1 y `lodestar-mcp` con Rust 1.88.0.
El lock fija `indexmap 2.11.4` (MSRV 1.63) y `clap_lex 1.0.1` (MSRV 1.74); el gate de política
impide que vuelvan a revisiones que requieren Rust 1.82/1.85 y elevarían transitivamente el MSRV
del workspace conservado. El check local con Rust 1.80.1 termina con `exit 0`; CI repite el check
con las fuentes oficiales de crates.io.

## Issue #38 y PR #39

`issue_38_repro_exacta` conserva el transcript de la reproducción: un `initialize` que ofrece una
revisión distinta responde la baseline Legacy, con stdout MCP puro y cierre por EOF. La cobertura
de negociación incluye revisiones antiguas, futuras e inventadas y demuestra que añadir otra fecha
a una lista histórica —la solución parcial de PR #39— no implementa E34.
