# Publicar una versión de lodestar

Runbook para cerrar una versión y publicar un release multiplataforma. El pipeline
(`.github/workflows/release.yml`) se dispara al empujar un tag `vX.Y.Z` y deja el
GitHub Release en **borrador** para que lo revises antes de publicarlo.

## Requisitos previos

- Estar en `main` con el árbol limpio y el CI en verde.
- `git` y `cargo` configurados; permisos de push al repo.

## Pasos

1. **Fija la versión** en los tres sitios que la declaran (`Cargo.toml`,
   `src-tauri/tauri.conf.json`, `frontend/package.json`) y actualiza el lockfile:

   ```bash
   ./scripts/set-version.sh X.Y.Z   # p. ej. 0.1.0 (chmod +x la primera vez)
   cargo update -w                  # propaga la versión al Cargo.lock
   ```

2. **Actualiza `CHANGELOG.md`**: mueve lo que haya en `## [No publicado]` a una
   nueva sección `## [X.Y.Z] - AAAA-MM-DD`, agrupando en Añadido/Cambiado/Corregido/…
   Actualiza también los enlaces de comparación al pie del archivo.

3. **Commit + PR + merge a `main`**: abre un PR con los cambios de versión y del
   changelog, pásalo por CI y mergéalo. El tag debe apuntar a un commit ya en `main`.

4. **Crea y empuja el tag**:

   ```bash
   git checkout main && git pull
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

5. **El workflow `release.yml` compila las tres plataformas** (macOS Apple Silicon,
   Windows y Linux) y crea un **GitHub Release en borrador** con los instaladores
   (dmg / deb / appimage / nsis) más los binarios de CLI y MCP. Los bundles salen
   **sin firmar** (ver «Firma de código» abajo).

6. **Revisa el borrador y publícalo**: en GitHub → *Releases*, comprueba que están
   todos los artefactos de las tres plataformas y las notas, ajusta el texto si hace
   falta y pulsa **Publish**. El release solo es visible tras publicarlo.

## Firma de código (diferida)

Los bundles de v0.1.0 salen **sin firmar** para macOS (arm64), Windows y Linux. Esto
implica avisos del SO al instalar (Gatekeeper en macOS, SmartScreen en Windows). La
firma y notarización están **diferidas, no descartadas**: cuando se aborde, el
pipeline añadirá los certificados/secretos correspondientes. Ver el estado en
`DECISIONES.md` (packaging/firma).

## Publicar en crates.io (opcional)

> **AVISO**: el repositorio es **privado**. Publicar en crates.io hace el código
> **público y permanente** (crates.io no permite despublicar de verdad, solo *yank*).
> Hazlo solo si esa exposición es intencional.

Requiere autenticarse una vez con un token de crates.io:

```bash
cargo login   # pega el token de https://crates.io/settings/tokens
```

Publica en **orden topológico** (una dependencia debe existir en el registry antes que
quien la consume). `lodestar-fixtures` y `src-tauri` son `publish = false` y no se
publican:

```bash
cargo publish -p lodestar-core
cargo publish -p lodestar-store
cargo publish -p lodestar-vcs
cargo publish -p lodestar-workspace
cargo publish -p lodestar-cli
cargo publish -p lodestar-mcp
```

Espera a que cada crate esté indexado antes de publicar el siguiente (a veces hay unos
segundos de retardo).
