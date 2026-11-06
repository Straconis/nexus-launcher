#!/usr/bin/env bash
# Stage a Linux x86_64 tarball: binary, seed catalog, licenses.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
bin=${1:-"$root/target/release/nexus-launcher"}
name=nexus-launcher-linux-x86_64
stage="$root/dist/$name"

if [[ ! -f "$bin" ]]; then
  echo "missing binary: $bin" >&2
  exit 1
fi

rm -rf "$stage"
mkdir -p "$stage/catalog"
install -m 0755 "$bin" "$stage/nexus-launcher"
if command -v strip >/dev/null 2>&1; then
  strip --strip-unneeded "$stage/nexus-launcher"
fi
cp "$root/catalog/apps.toml" "$stage/catalog/apps.toml"
cp "$root/README.md" "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$stage/"
cp "$root/assets/fonts/OFL.txt" "$stage/OFL-Orbitron.txt"
cp "$root/packaging/PORTABLE.txt" "$stage/PORTABLE.txt"

mkdir -p "$root/dist"
tar -C "$root/dist" -czf "$root/dist/${name}.tar.gz" "$name"
echo "wrote $root/dist/${name}.tar.gz"
