#!/usr/bin/env bash
set -euo pipefail

# Ubuntu 24.04 does not publish libgtk4-layer-shell-dev. Build the exact
# upstream revision used by CI instead of depending on an unavailable package
# or an unpinned default branch.
readonly upstream="https://github.com/wmww/gtk4-layer-shell.git"
readonly revision="536ff516ed68b9bb34afc4c07f942a54b2b4b03f" # v1.0.4
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

git -C "$workdir" init --quiet
git -C "$workdir" remote add origin "$upstream"
git -C "$workdir" fetch --quiet --depth 1 origin "$revision"
git -C "$workdir" checkout --quiet --detach FETCH_HEAD

meson setup "$workdir/build" "$workdir" \
  --prefix=/usr \
  -Dexamples=false \
  -Ddocs=false \
  -Dtests=false \
  -Dsmoke-tests=false \
  -Dintrospection=false \
  -Dvapi=false
ninja -C "$workdir/build"
sudo ninja -C "$workdir/build" install
sudo ldconfig

pkg-config --modversion gtk4-layer-shell-0