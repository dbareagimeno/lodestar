#!/usr/bin/env bash
# set-version.sh — sincroniza la versión de lodestar donde se declara.
#
# Uso:
#   ./scripts/set-version.sh X.Y.Z
#
# Si aún no tiene permiso de ejecución:
#   chmod +x scripts/set-version.sh
#
# Actualiza (con sed acotado, sin tocar otras líneas «version»):
#   1. Cargo.toml → línea `version = "…"` dentro de [workspace.package]
#   2. Cargo.toml → el `version` de las deps internas de [workspace.dependencies]
#      (las `lodestar-* = { path = "crates/…", version = "…" }`); si no se suben a
#      la vez, `cargo update -w` falla porque el requisito ^X.Y.Z ya no casa.
#
# (La UI de escritorio —tauri.conf.json / frontend/package.json— se movió a la
#  rama `experimental/ui-desktop`; este script ya no la versiona.)
#
# Tras correrlo hay que actualizar el lockfile de Cargo y crear el tag (ver RELEASING.md).

set -euo pipefail

# --- Argumentos y validación -------------------------------------------------
if [[ $# -ne 1 ]]; then
  echo "Uso: $0 X.Y.Z" >&2
  exit 2
fi

VERSION="$1"

# Valida el formato semver básico X.Y.Z (solo números; sin prefijo «v» ni sufijos).
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Error: versión inválida '$VERSION'. Formato esperado: X.Y.Z (p. ej. 0.1.0)." >&2
  exit 2
fi

# --- Localiza la raíz del repo (este script vive en scripts/) ----------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

CARGO_TOML="$ROOT_DIR/Cargo.toml"

if [[ ! -f "$CARGO_TOML" ]]; then
  echo "Error: no se encuentra '$CARGO_TOML'." >&2
  exit 3
fi

# `sed -i.bak` funciona tanto en BSD/macOS como en GNU/Linux (crea un respaldo .bak
# que borramos al final). El patrón está anclado para no tocar otras «version».

# Cargo.toml — solo la línea que EMPIEZA por `version = "X.Y.Z"` (la de
# [workspace.package]); las dependencias llevan `version` dentro de llaves y
# `rust-version` empieza por otro prefijo, así que el ancla ^ las excluye.
sed -i.bak -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"$VERSION\"/" "$CARGO_TOML"

# Deps internas de [workspace.dependencies]: `lodestar-x = { path = "crates/…", version = "…" }`.
# El requisito semver debe seguir a la versión del workspace o `cargo update -w` no resuelve.
sed -i.bak -E \
  "s|(path = \"crates/lodestar-[a-z-]+\", version = \")[0-9]+\.[0-9]+\.[0-9]+\"|\1$VERSION\"|" \
  "$CARGO_TOML"

# Limpia el respaldo que dejó sed.
rm -f "$CARGO_TOML.bak"

echo "Versión fijada a $VERSION en:"
echo "  - Cargo.toml ([workspace.package] + deps internas de [workspace.dependencies])"
echo
echo "Recordatorio:"
echo "  1. Actualiza el lockfile:   cargo update -w"
echo "  2. Crea el tag de release:  git tag v$VERSION && git push origin v$VERSION"
echo "  (Ver RELEASING.md para el runbook completo.)"
