#!/usr/bin/env bash
set -euo pipefail

PREFIX="${PREFIX:-/usr/local}"
as_root() {
  if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
    "$@"
  else
    sudo "$@"
  fi
}

as_root rm -f   "${PREFIX}/bin/topmonitoring"   "${PREFIX}/share/applications/io.github.fakedevbagus.TopMonitoring.desktop"   "${PREFIX}/share/metainfo/io.github.fakedevbagus.TopMonitoring.metainfo.xml"   "${PREFIX}/share/icons/hicolor/scalable/apps/io.github.fakedevbagus.TopMonitoring.svg"   "${PREFIX}/share/icons/hicolor/256x256/apps/io.github.fakedevbagus.TopMonitoring.png"
rm -f "${HOME}/.config/autostart/io.github.fakedevbagus.TopMonitoring.desktop"       "${HOME}/.config/autostart/topmonitoring.desktop"

echo "TopMonitoring removed. User data in ~/.config/topmonitoring was kept."
