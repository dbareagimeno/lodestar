#!/bin/sh
# Guardarraíl del pipeline de release (E27-H01): el tag debe ser exactamente
# `v` + la versión declarada en [workspace.package] de Cargo.toml. Sin esta
# guarda, un tag sin prefijo no dispara el workflow (pasó con 0.5.0: release
# publicada con 0 assets) y un tag desincronizado nombraría artefactos con una
# versión que el binario no declara.
#
# Uso: verifica-tag-release.sh <tag>
# Sale 0 si el tag es exactamente v<version>; ≠0 en cualquier otro caso.
# Solo POSIX sh + sed: corre en el runner de ubuntu sin instalar nada.
set -eu

if [ $# -ne 1 ]; then
    echo "uso: $0 <tag>" >&2
    exit 2
fi

tag="$1"

version=$(sed -n '/^\[workspace\.package\]/,/^\[/{ s/^version *= *"\(.*\)".*/\1/p; }' Cargo.toml | head -n 1)

if [ -z "$version" ]; then
    echo "error: no se pudo extraer 'version' de [workspace.package] en Cargo.toml" >&2
    exit 3
fi

case "$tag" in
v*) ;;
*)
    echo "error: el tag '$tag' no lleva el prefijo 'v' obligatorio (esperado: v$version)." >&2
    echo "Un tag sin prefijo no matchea el filtro tags: [\"v*\"] del workflow — con 0.5.0" >&2
    echo "dejó la release publicada sin binarios. Re-tagea como v$version." >&2
    exit 1
    ;;
esac

if [ "$tag" != "v$version" ]; then
    echo "error: el tag '$tag' no coincide con la versión del workspace ('$version' en Cargo.toml)." >&2
    echo "Esperado: v$version. Sincroniza Cargo.toml (scripts/set-version.sh) o corrige el tag." >&2
    exit 1
fi

echo "ok: tag '$tag' coincide con la versión del workspace ($version)"
