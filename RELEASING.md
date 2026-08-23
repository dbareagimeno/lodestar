# Publicar una versión de lodestar

Runbook para cerrar una versión y publicar los binarios de línea de comandos (CLI +
MCP) para las tres plataformas. El pipeline (`.github/workflows/release.yml`) se
dispara al empujar un tag `vX.Y.Z` y deja el GitHub Release en **borrador** para que lo
revises antes de publicarlo.

> La UI de escritorio (Tauri) se movió a la rama `experimental/ui-desktop`; este runbook
> y el pipeline de `main` ya no publican el bundle de escritorio (dmg/deb/appimage/nsis).

## Modelo de ramas

**`develop` es la rama de integración** (y la rama por defecto del repo): todo el trabajo
—épicas, historias, fixes— sale de `develop` y vuelve a `develop` por PR.

**`main` solo recibe releases.** No se commitea en `main` ni se mergea trabajo suelto
en `main`: lo único que entra es un PR de release desde `develop`. Por tanto `main`
siempre apunta a la última versión publicada, y el tag `vX.Y.Z` siempre cuelga de un
commit de `main`.

```
feature/… ──PR──► develop ──PR de release──► main ──tag vX.Y.Z──► binarios
```

## Requisitos previos

- Estar en `develop`, al día con `origin/develop`, con el árbol limpio y el CI en verde.
- `git` y `cargo` configurados; permisos de push al repo.

## Pasos

1. **Fija la versión** donde se declara (`Cargo.toml`, `[workspace.package]`) y
   actualiza el lockfile:

   ```bash
   ./scripts/set-version.sh X.Y.Z   # p. ej. 0.1.0 (chmod +x la primera vez)
   cargo update -w                  # propaga la versión al Cargo.lock
   ```

2. **Actualiza `CHANGELOG.md`**: mueve lo que haya en `## [No publicado]` a una
   nueva sección `## [X.Y.Z] - AAAA-MM-DD`, agrupando en Añadido/Cambiado/Corregido/…
   Actualiza también los enlaces de comparación al pie del archivo.

3. **Corre el banco de evidencia completo** antes de abrir el PR a `develop`. Este paso se ejecuta
   en la máquina que corresponde a la baseline. Los JSON generados son raws temporales dentro de
   un directorio persistente fuera del checkout, indicado por `LODESTAR_EVIDENCE_DIR`; sus gzip
   deterministas se conservan allí hasta que exista el draft de GitHub Release. En este momento el
   PR solo lleva los resúmenes Markdown y `manifest.json`: el manifiesto puede declarar ya las URL
   deterministas futuras de `v${VERSION}`, pero el resultado no se considera evidencia permanente
   hasta que el gzip se haya subido y verificado en el draft.
   El banco de conformidad es el corpus canónico completo; el rendimiento es full, y el último
   comando juzga el gate absoluto con el binario release de `lodestar-bench`:

   ```bash
   set -euo pipefail
   VERSION="X.Y.Z"
   : "${LODESTAR_EVIDENCE_DIR:?Define LODESTAR_EVIDENCE_DIR fuera del checkout}"
   EVIDENCE_DIR="${LODESTAR_EVIDENCE_DIR%/}/v${VERSION}"
   RUN_DIR="$EVIDENCE_DIR/run"
   ASSET_DIR="$EVIDENCE_DIR/assets"
   MANIFEST="$EVIDENCE_DIR/manifest.json"
   mkdir -p "$RUN_DIR" "$ASSET_DIR"
   mkdir -p "docs/qa/corridas/v${VERSION}"
   cargo build --release --locked -p lodestar-cli -p lodestar-mcp -p lodestar-bench

   CORPUS="$EVIDENCE_DIR/conformidad-corpus"
   ./docs/qa/testbench/make_corpus.py "$CORPUS"
   python3 docs/qa/testbench/lodestar_harness.py \
     --run-all \
     --root-corpus "$CORPUS" \
     --binary target/release/lodestar-mcp \
     --binary-cli target/release/lodestar \
     --out "$RUN_DIR/conformidad.json" | tee "$RUN_DIR/conformidad.md"

   WIRE_ROOT="$EVIDENCE_DIR/realista-10k"
   WIRE_INPUT="$EVIDENCE_DIR/wire-calibration-realista-10k.json"
   cargo run --release --locked -p lodestar-fixtures --example release_realista -- \
     "$WIRE_ROOT" 33
   python3 docs/qa/testbench/make_wire_calibration.py \
     --root "$WIRE_ROOT" \
     --binary target/release/lodestar-mcp \
     --out "$WIRE_INPUT" > /dev/null
   env -u LODESTAR_BENCH_TEST_ITERATIONS cargo run --release -p lodestar-bench --locked -- --seed 33 \
     --wire-calibration-input "$WIRE_INPUT" \
     --json-output "$RUN_DIR/rendimiento.json" \
     --markdown-output "$RUN_DIR/rendimiento.md" > /dev/null

   target/release/lodestar-bench --gate \
     --report "$RUN_DIR/rendimiento.json" \
     --thresholds docs/qa/testbench/umbrales.json \
     --baseline docs/qa/e33-h05-baseline-release-macbook-2026-08.json \
     --machine-id release-macbook-2026-08 | tee -a "$RUN_DIR/rendimiento.md"
   ```

   Tras un PASS, crea los gzip con cabecera y compresión deterministas. Conserva el raw solo hasta
   haber calculado su SHA/tamaño para `raw`; el SHA/tamaño de `artifact` siempre corresponde al
   `.json.gz`. Repite el patrón para cada JSON que catalogue el manifiesto, usando como nombre del
   asset el nombre final que tendrá en la URL de la release:

   ```bash
   gzip -n -9 -c "$RUN_DIR/conformidad.json" \
     > "$ASSET_DIR/e33-h07-v${VERSION}-conformidad.json.gz"
   gzip -n -9 -c "$RUN_DIR/rendimiento.json" \
     > "$ASSET_DIR/e33-h07-v${VERSION}-rendimiento.json.gz"
   shasum -a 256 "$RUN_DIR"/*.json "$ASSET_DIR"/*.json.gz
   wc -c "$RUN_DIR"/*.json "$ASSET_DIR"/*.json.gz
   cp "$RUN_DIR/conformidad.md" "docs/qa/corridas/v${VERSION}/conformidad.md"
   cp "$RUN_DIR/rendimiento.md" "docs/qa/corridas/v${VERSION}/rendimiento.md"
   # Crea $MANIFEST después de obtener los hashes/tamaños. Para cada resultado, usa esta forma
   # conceptual (rellenando los valores calculados; no copies un manifiesto desde el checkout):
   # {"id":"...", "summary":"...",
   #  "artifact":{"url":"https://github.com/dbareagimeno/lodestar/releases/download/v${VERSION}/...json.gz",
   #    "sha256":"<SHA-DEL-GZIP>", "size_bytes":<BYTES-DEL-GZIP>,
   #    "media_type":"application/gzip", "compression":"gzip"},
   #  "raw":{"sha256":"<SHA-DEL-JSON>", "size_bytes":<BYTES-DEL-JSON>,
   #    "schema_version":"<SCHEMA-DEL-JSON>"}}
   cp "$MANIFEST" "docs/qa/corridas/v${VERSION}/manifest.json"
   # Solo después de validar y copiar el manifiesto se eliminan los raws locales.
   rm "$RUN_DIR"/*.json
   ```

   No uses `gh release upload` en este paso: todavía no existe el tag ni el release. No copies
   raws ni gzip al checkout; solo los dos resúmenes y el manifiesto entran en el PR. Mantén
   `"media_type": "application/gzip"` y `"compression": "gzip"` en cada `artifact`.

   `--machine-id release-macbook-2026-08` solo es válido en la máquina identificada por esa
   baseline: ahí juzga los máximos absolutos ratificados (`p95 ≤ 1 s` por lectura y
   `cold-open ≤ 5 s`). En cualquier otra máquina el gate solo compara tendencia, nunca absolutos.
   Si conformidad, rendimiento o `--gate` sale distinto de cero, es **stop-the-line**: no se abre
   ni se mergea el PR de release hasta corregirlo y repetir la corrida. Antes de añadir los
   artefactos, sustituye cualquier raíz efímera de la máquina por `<ephemeral-root>` y la raíz del
   checkout por `<repo>`; no se versionan rutas privadas.

   El workflow manual [`testbench.yml`](.github/workflows/testbench.yml) es una comprobación
   complementaria: corre conformidad canónica y rendimiento smoke en un runner compartido,
   deliberadamente sin `--gate`, `--thresholds` ni `--baseline`, y publica el resultado como
   artefacto. No sustituye este paso ni habilita el gate absoluto.

4. **Commit + PR a `develop`**: abre un PR con los cambios de versión y del changelog
   **contra `develop`**, pásalo por CI y mergéalo. Los pasos 1–3 son trabajo como
   cualquier otro y entran por la rama de integración, no directamente en `main`.

5. **PR de release `develop` → `main`**: abre un segundo PR con `develop` como origen y
   `main` como destino, sin más cambios que los que ya lleva `develop`. Mergéalo cuando
   el CI esté en verde. El tag debe apuntar a un commit ya en `main`.

   ```bash
   gh pr create --base main --head develop --title "release: vX.Y.Z"
   ```

6. **Crea y empuja el tag**:

   ```bash
   git checkout main && git pull
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

   > **El prefijo `v` es obligatorio.** El workflow solo se dispara con tags `v*`: un tag
   > `X.Y.Z` a secas no construye nada y deja la release sin binarios (pasó con `0.5.0`,
   > que hubo que re-tagear como `v0.5.0`).

   > **El tag debe coincidir con la versión del workspace.** El primer step del workflow
   > ejecuta `scripts/verifica-tag-release.sh`: si el tag no es exactamente `v` + la
   > `version` de `[workspace.package]` en `Cargo.toml`, el CI falla **antes** de crear el
   > release o compilar nada. Un `v0.6.0` empujado con `Cargo.toml` aún en `0.5.0` no
   > publica artefactos con una versión que el binario no declara.

7. **El workflow `release.yml` compila las tres plataformas** (macOS Apple Silicon,
   Windows y Linux) y crea un **GitHub Release en borrador** con los tarballs/zip de
   los binarios de CLI y MCP más un `SHA256SUMS-<target>.txt` por plataforma. Cada
   checksum se verifica dentro del propio job antes de subirse.

8. **Revisa el borrador, sube la evidencia y publícalo**. Antes de pulsar **Publish**, conserva
   los gzip del paso 3 y súbelos al draft que acaba de crear el workflow:

   ```bash
   set -euo pipefail
   : "${LODESTAR_EVIDENCE_DIR:?Define LODESTAR_EVIDENCE_DIR fuera del checkout}"
   VERSION="X.Y.Z"
   EVIDENCE_DIR="${LODESTAR_EVIDENCE_DIR%/}/v${VERSION}"
   ASSET_DIR="$EVIDENCE_DIR/assets"
   MANIFEST="docs/qa/corridas/v${VERSION}/manifest.json"
   gh release upload "v${VERSION}" "$ASSET_DIR"/*.json.gz

   VERIFY_DIR="$(mktemp -d "$EVIDENCE_DIR/verified-assets.XXXXXX")"
   gh release download "v${VERSION}" --pattern '*.json.gz' \
     --dir "$VERIFY_DIR" --clobber
   python3 - "$MANIFEST" "$VERIFY_DIR" <<'PY'
   import hashlib
   import json
   import pathlib
   import sys
   from urllib.parse import urlparse

   manifest_path, asset_dir = map(pathlib.Path, sys.argv[1:])
   manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
   expected = {}
   for result in manifest["results"]:
       artifact = result["artifact"]
       name = pathlib.PurePosixPath(urlparse(artifact["url"]).path).name
       expected[name] = (artifact["sha256"], artifact["size_bytes"])
   actual = {path.name: path for path in asset_dir.glob("*.json.gz")}
   if set(actual) != set(expected):
       raise SystemExit(f"assets distintos del manifest: {sorted(actual)}")
   for name, (sha, size) in expected.items():
       path = actual[name]
       got_sha = hashlib.sha256(path.read_bytes()).hexdigest()
       got_size = path.stat().st_size
       if (got_sha, got_size) != (sha, size):
           raise SystemExit(
               f"{name}: manifest {(sha, size)} != asset {(got_sha, got_size)}"
           )
   print(f"OK: {len(expected)} assets coinciden con artifact.sha256/size_bytes")
   PY
   ```

   Comprueba también que están los **6 artefactos** de binarios (3 tarballs/zip + 3 ficheros de
   checksums), los gzip de evidencia y las notas. Publica solo después de que la verificación
   anterior termine correctamente (en GitHub → *Publish* o con `gh release edit
   "v${VERSION}" --draft=false`). Hasta esa subida y verificación, el contenido del PR es un
   estado transitorio de preparación, no evidencia permanente. El release solo es visible tras
   publicarlo. Quien descargue un binario puede verificarlo con:

   ```bash
   shasum -a 256 -c SHA256SUMS-<target>.txt   # sha256sum -c en Linux
   ```

9. **Devuelve `main` a `develop`**: el merge del paso 5 crea en `main` un commit que
   `develop` no tiene, y a partir de ahí las dos ramas divergen. Ciérralo en el momento,
   no la próxima release:

   ```bash
   git checkout develop && git pull
   git merge --ff-only main || git merge main   # ff si no hubo trabajo nuevo en develop
   git push origin develop
   ```

## Firma de código (diferida)

Los binarios de CLI/MCP salen **sin firmar** para macOS (arm64), Windows y Linux. Esto
puede implicar avisos del SO al ejecutarlos (Gatekeeper en macOS, SmartScreen en
Windows). La firma y notarización están **diferidas, no descartadas**: cuando se
aborde, el pipeline añadirá los certificados/secretos correspondientes. Ver el estado
en `decisiones §1` (packaging/firma).

## Publicar en crates.io (opcional)

> **AVISO**: publicar en crates.io es **permanente** (crates.io no permite despublicar
> de verdad, solo *yank*). Hazlo solo si esa permanencia es intencional; la decisión
> sigue abierta en `decisiones §17-DA`.

Requiere autenticarse una vez con un token de crates.io:

```bash
cargo login   # pega el token de https://crates.io/settings/tokens
```

Publica en **orden topológico** (una dependencia debe existir en el registry antes que
quien la consume). `lodestar-fixtures` es `publish = false` y no se publica:

```bash
cargo publish -p lodestar-core
cargo publish -p lodestar-store
cargo publish -p lodestar-workspace
cargo publish -p lodestar-app
cargo publish -p lodestar-cli
cargo publish -p lodestar-mcp
```

Espera a que cada crate esté indexado antes de publicar el siguiente (a veces hay unos
segundos de retardo).
