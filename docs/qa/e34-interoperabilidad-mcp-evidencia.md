# E34 — evidencia de ejecución

Fecha de ejecución: 2026-08-17. Base: `develop`.

Este registro acompaña a la épica y permite auditar las fases TDD sin depender de la conversación
del implementador. Los snapshots completos de alcance se generaron con `phase-scope.py snapshot`
en `target/agent-state/e34-hNN/pre-red.json`; los locks se generaron después del rojo con
`tdd-test-lock.py snapshot` y se verificaron antes y después de cada verde.

## Rojo independiente y lock

| Historia | Resultado rojo observado antes de producción | Test bloqueado (SHA-256) |
| --- | --- | --- |
| H01 | La matriz de policy/MSRV/capacidades no encontraba la política dual ni las dependencias ratificadas. | `e34_h01_policy.rs` `3bcd552df0bf10fffedd3da9931684adc46b927dfd970c81505764bf4b4ff5fd` |
| H02 | La suite no podía importar ni ejercer `LodestarMcpService`; no existía el seam neutral. | `e34_h02_service.rs` `321e4ea2e9a5ac6dc3172b751f3e4687ffa080999a23c4d93603fa8a91057b56` |
| H03 | Los casos de transporte detectaron el lector/framing manual y la ausencia del cliente oficial rmcp. | `e34_h03_transport.rs` `97183aa067f1104db511f654bd01adea26a57baf196bcebfc42d3784f3c3252e` |
| H04 | Discovery y metadata Modern no existían; las respuestas carecían de `resultType` y cache hints. | `e34_h04_modern.rs` `45eb826a37d9f983cca2ada4f1b6ef1b0e62295b2373ea7ff69a4b9b8f2cb85a` |
| H05 | La reproducción cruda de issue #38 devolvía el rechazo histórico en vez de negociar Legacy; el transcript también fija que metadata Modern no reabre `server/discover` dentro de una sesión Legacy. | `e34_h05_legacy.rs` `0a2e5c912d18adca23c37fc3a42c66629f7459a5f125adaa7050ec6d3b8e9e9d` |
| H06 | Tres pruebas pasaban y `cancelacion_transaccional_sin_parciales` fallaba porque la request cancelada llegaba a alterar el Markdown. | `e34_h06_conformance.rs` `550d54e7adfb5f84afdba99f7124cf121b9186d0e30f137a6c20fc4a23e8f835` |

`verify-tests-only` pasó en cada fase roja. `tdd-test-lock.py verify` pasó antes y después de cada
verde. La reparación de cobertura de H04 añade una llamada equivalente antes y después de una
publicación real; la de H06 añade un gancho gateado justo después del primer rename y un segundo
transcript tardío, además de correlación explícita de IDs, bytes finales exactos y una barrera
determinista también para el perfil readonly. El lock final de ambos tests es el hash de esta tabla.

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

## Verde y gates de la implementación E34

Resultados finales observados antes de la revisión:

```text
E34-H01: 3 passed; 0 failed; 1 ignored (gate explícito de toolchains)
E34-H02: 3 passed; 0 failed
E34-H03: 5 passed; 0 failed
E34-H04: 9 passed; 0 failed
E34-H05: 7 passed; 0 failed
E34-H06: 5 passed; 0 failed
lodestar-mcp/tests/mcp.rs: 160 passed; 0 failed
scripts/agent-gates.sh contract: exit 0
scripts/agent-gates.sh policy: exit 0
scripts/agent-gates.sh full: exit 0
```

La repetición local de `cargo metadata --locked --format-version 1` queda pendiente de una fuente
descargable de `clap_lex 1.0.1`: el índice y el DNS de crates.io no están disponibles en este
entorno. El lock, el pin y el chequeo estructurado permanecen verificados; CI debe repetir el
comando con las fuentes oficiales antes de publicar la etiqueta.

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

## Reejecución de la preparación de release v0.6.1

Después de la revisión fresca de la entrega se repitieron los gates sobre el checkout actual:

```text
git diff --check: exit 0
cargo fmt --all --check: exit 0
check-agent-guidance --include-legacy: exit 0
check-contract-surface.py: exit 0
scripts/agent-gates.sh contract: exit 0
cargo build/test --workspace --exclude lodestar-cli --locked --offline: exit 0
cargo test -p lodestar-workspace --features test-failpoints --locked --offline: exit 0
cargo test -p lodestar-app --features test-failpoints --locked --offline: exit 0
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --exclude lodestar-cli --no-deps --locked --offline: exit 0
```

El `metadata --locked` exacto sobre el checkout y, por tanto, las compuertas `policy`/`full` no
pueden repetirse localmente: la caché no contiene `clap_lex 1.0.1` y crates.io no está resolviendo
por DNS. No se alteró el lock para ocultar esa limitación. Como comprobación suplementaria, una
copia aislada con un parche de Cargo exclusivamente hacia la fuente local de `clap_lex 1.0.1`
completó `scripts/agent-gates.sh full` (incluidos tests, failpoints, docs y política); CI debe
repetir la compuerta exacta con la fuente registry antes de crear la etiqueta.

## Issue #38 y PR #39

`issue_38_repro_exacta` conserva el transcript de la reproducción: un `initialize` que ofrece una
revisión distinta responde la baseline Legacy, con stdout MCP puro y cierre por EOF. La cobertura
de negociación incluye revisiones antiguas, futuras e inventadas y demuestra que añadir otra fecha
a una lista histórica —la solución parcial de PR #39— no implementa E34.
