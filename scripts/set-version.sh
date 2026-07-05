#!/usr/bin/env bash
# set-version.sh — sincroniza la versión de lodestar en los tres sitios que la declaran.
#
# Uso:
#   ./scripts/set-version.sh X.Y.Z
#
# Si aún no tiene permiso de ejecución:
#   chmod +x scripts/set-version.sh
#
# Actualiza (con sed acotado, sin tocar otras líneas «version»):
#   1. Cargo.toml               → línea `version = "…"` dentro de [workspace.package]
#   2. src-tauri/tauri.conf.json → campo "version"
#   3. frontend/package.json     → campo "version"
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
TAURI_CONF="$ROOT_DIR/src-tauri/tauri.conf.json"
PKG_JSON="$ROOT_DIR/frontend/package.json"

for f in "$CARGO_TOML" "$TAURI_CONF" "$PKG_JSON"; do
  if [[ ! -f "$f" ]]; then
    echo "Error: no se encuentra '$f'." >&2
    exit 3
  fi
done

# `sed -i.bak` funciona tanto en BSD/macOS como en GNU/Linux (crea un respaldo .bak
# que borramos al final). Los patrones están anclados para no tocar otras «version».

# 1. Cargo.toml — solo la línea que EMPIEZA por `version = "X.Y.Z"` (la de
#    [workspace.package]); las dependencias llevan `version` dentro de llaves y
#    `rust-version` empieza por otro prefijo, así que el ancla ^ las excluye.
sed -i.bak -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"$VERSION\"/" "$CARGO_TOML"

# 2. tauri.conf.json — el único campo `"version": "X.Y.Z"` del archivo.
sed -i.bak -E "s/(\"version\": \")[0-9]+\.[0-9]+\.[0-9]+(\")/\1$VERSION\2/" "$TAURI_CONF"

# 3. package.json — la clave `"version"` de nivel superior. Las dependencias son
#    `"paquete": "^x.y.z"`, no llevan la clave literal `"version"`, así que no se tocan.
sed -i.bak -E "s/(\"version\": \")[0-9]+\.[0-9]+\.[0-9]+(\")/\1$VERSION\2/" "$PKG_JSON"

# Limpia los respaldos que dejó sed.
rm -f "$CARGO_TOML.bak" "$TAURI_CONF.bak" "$PKG_JSON.bak"

echo "Versión fijada a $VERSION en:"
echo "  - Cargo.toml ([workspace.package])"
echo "  - src-tauri/tauri.conf.json"
echo "  - frontend/package.json"
echo
echo "Recordatorio:"
echo "  1. Actualiza el lockfile:   cargo update -w"
echo "  2. Crea el tag de release:  git tag v$VERSION && git push origin v$VERSION"
echo "  (Ver RELEASING.md para el runbook completo.)"
