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

3. **Commit + PR a `develop`**: abre un PR con los cambios de versión y del changelog
   **contra `develop`**, pásalo por CI y mergéalo. Los pasos 1–2 son trabajo como
   cualquier otro y entran por la rama de integración, no directamente en `main`.

4. **PR de release `develop` → `main`**: abre un segundo PR con `develop` como origen y
   `main` como destino, sin más cambios que los que ya lleva `develop`. Mergéalo cuando
   el CI esté en verde. El tag debe apuntar a un commit ya en `main`.

   ```bash
   gh pr create --base main --head develop --title "release: vX.Y.Z"
   ```

5. **Crea y empuja el tag**:

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

6. **El workflow `release.yml` compila las tres plataformas** (macOS Apple Silicon,
   Windows y Linux) y crea un **GitHub Release en borrador** con los tarballs/zip de
   los binarios de CLI y MCP más un `SHA256SUMS-<target>.txt` por plataforma. Cada
   checksum se verifica dentro del propio job antes de subirse.

7. **Revisa el borrador y publícalo**: en GitHub → *Releases*, comprueba que están
   los **6 artefactos** (3 tarballs/zip + 3 ficheros de checksums) y las notas,
   ajusta el texto si hace falta y pulsa **Publish**. El release solo es visible
   tras publicarlo. Quien descargue un binario puede verificarlo con:

   ```bash
   shasum -a 256 -c SHA256SUMS-<target>.txt   # sha256sum -c en Linux
   ```

8. **Devuelve `main` a `develop`**: el merge del paso 4 crea en `main` un commit que
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
en `DECISIONES.md` (packaging/firma).

## Publicar en crates.io (opcional)

> **AVISO**: publicar en crates.io es **permanente** (crates.io no permite despublicar
> de verdad, solo *yank*). Hazlo solo si esa permanencia es intencional; la decisión
> sigue abierta en `DECISIONES.md`.

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
