#!/usr/bin/env bash
# Regenerates the README's screenshots in docs/ from the sample data, headlessly:
#   scripts/screenshots.sh            (builds the release binary first)
# Nothing here talks to GitHub or reads your settings: the app runs on its sample data, with a
# throwaway data folder and a made-up local clone (for the Changes tab).
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --quiet
bin=target/release/repohub
out=docs
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# A made-up clone of the sample repository, with uncommitted changes.
repo="$tmp/clones/francescodibella.github.io"
mkdir -p "$repo/src"
git -C "$repo" init -q -b main
# RepoHub takes a folder as the clone only when a remote is the repository.
git -C "$repo" remote add origin https://github.com/francescodibella/francescodibella.github.io.git
cat > "$repo/src/scene.js" <<'JS'
import * as THREE from "three";

export function scene() {
  const s = new THREE.Scene();
  s.background = new THREE.Color(0x101418);
  return s;
}
JS
printf '# francescodibella.github.io\n\nPersonal site and playground.\n' > "$repo/README.md"
git -C "$repo" add -A
git -C "$repo" -c user.name=demo -c user.email=demo@example.com commit -qm "First version"
cat > "$repo/src/scene.js" <<'JS'
import * as THREE from "three";

export function scene(fog = true) {
  const s = new THREE.Scene();
  s.background = new THREE.Color(0x0b0f14);
  if (fog) s.fog = new THREE.Fog(0x0b0f14, 4, 18);
  return s;
}
JS
printf 'export const bunny = { ears: 2, hue: 210 };\n' > "$repo/src/bunny.js"

# snap NAME SIZE [ENV=VALUE...] -- ARGS: one still, with its own empty data folder.
snap() {
  local name=$1 size=$2; shift 2
  local env=()
  while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do env+=("$1"); shift; done
  shift
  local data="$tmp/data-$name"
  mkdir -p "$data"
  [ -n "${PREFS:-}" ] && printf '%s' "$PREFS" > "$data/prefs.json"
  env REPOHUB_DEMO=1 REPOHUB_DATA_DIR="$data" REPOHUB_REPOS_DIR="$tmp/clones" "${env[@]}" \
    "$bin" --snap "$out/$name.png" --size "$size" --scroll-bars always -- "$@" >/dev/null 2>&1
  echo "$out/$name.png"
}

snap overview     1180x760 REPOHUB_THEME=dark  -- page overview
snap repository   1180x760 REPOHUB_THEME=light -- page repo
snap changes      1180x900 REPOHUB_THEME=dark  -- page repo tab changes
snap pull-diff    1180x1100 REPOHUB_THEME=dark -- page pulls diff 14
PREFS='{"theme":"dark","theme.dark":"neon"}' \
snap themes       1180x760 REPOHUB_THEME=dark  -- page settings section appearance
snap mobile       390x844  REPOHUB_THEME=dark REPOHUB_COMPACT=1 -- page pulls
# docs/notes.png, docs/terminal.png and docs/claude.png need a drawing, a running shell and
# Claude Code: they're taken by hand (see CONTRIBUTING).
