#!/usr/bin/env sh
# Installs RepoHub for the current user: the program in ~/.local/bin and a launcher entry.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
install -Dm755 "$here/repohub" "$HOME/.local/bin/repohub"
install -Dm644 "$here/repohub.png" "$HOME/.local/share/icons/hicolor/256x256/apps/repohub.png"
install -Dm644 "$here/repohub.desktop" "$HOME/.local/share/applications/repohub.desktop"
echo "Installed. Start RepoHub from your applications menu, or run: repohub"
