#!/usr/bin/env bash
set -euo pipefail

PREFIX="${PREFIX:-/usr/local}"
BUILD_FLAGS=()
if [[ "${TOPMONITORING_NO_WAYLAND:-0}" == "1" ]]; then
  BUILD_FLAGS+=(--no-default-features)
fi

as_root() {
  if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
    "$@"
  else
    sudo "$@"
  fi
}

echo "Building TopMonitoring 2..."
cargo build --release --locked "${BUILD_FLAGS[@]}"

echo "Installing under ${PREFIX}..."
as_root install -Dm755 target/release/topmonitoring "${PREFIX}/bin/topmonitoring"
as_root install -Dm644 topmonitoring.desktop   "${PREFIX}/share/applications/io.github.fakedevbagus.TopMonitoring.desktop"
as_root install -Dm644 data/io.github.fakedevbagus.TopMonitoring.metainfo.xml   "${PREFIX}/share/metainfo/io.github.fakedevbagus.TopMonitoring.metainfo.xml"
as_root install -Dm644 assets/topmonitoring.svg   "${PREFIX}/share/icons/hicolor/scalable/apps/io.github.fakedevbagus.TopMonitoring.svg"
as_root install -Dm644 assets/topmonitoring-256.png   "${PREFIX}/share/icons/hicolor/256x256/apps/io.github.fakedevbagus.TopMonitoring.png"

command -v update-desktop-database >/dev/null &&   as_root update-desktop-database "${PREFIX}/share/applications" || true
command -v gtk-update-icon-cache >/dev/null &&   as_root gtk-update-icon-cache -f -t "${PREFIX}/share/icons/hicolor" || true

echo "Installed. Run: topmonitoring"
