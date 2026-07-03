#!/usr/bin/env bash
set -e
echo "Building release..."
cargo build --release
echo "Installing binary..."
sudo install -Dm755 target/release/topmonitoring /usr/local/bin/topmonitoring
install -Dm644 topmonitoring.desktop "$HOME/.local/share/applications/topmonitoring.desktop"
echo "TopMonitoring terpasang. Jalankan dengan: topmonitoring"
