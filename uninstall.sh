#!/usr/bin/env bash
sudo rm -f /usr/local/bin/topmonitoring
rm -f "$HOME/.local/share/applications/topmonitoring.desktop"
rm -f "$HOME/.config/autostart/topmonitoring.desktop"
echo "TopMonitoring removed. (Your config at ~/.config/topmonitoring is kept.)"