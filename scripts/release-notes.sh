#!/usr/bin/env sh
# Prints one section of CHANGELOG.md, without its heading: the release notes of a version.
#   scripts/release-notes.sh             the Unreleased section
#   scripts/release-notes.sh 0.2.0       the section "## 0.2.0" (a leading "v" is fine too)
# Exits with status 1, printing nothing, when the section is missing or empty.
set -eu
want=${1:-Unreleased}
want=${want#v}
file=${CHANGELOG:-$(dirname "$0")/../CHANGELOG.md}
awk -v want="$want" '
  /^## / {
    if (inside) exit
    h = substr($0, 4)
    sub(/^\[/, "", h); sub(/\].*$/, "", h); sub(/[ \t].*$/, "", h)
    if (tolower(h) == tolower(want)) { inside = 1; next }
  }
  inside { print }
' "$file" | sed -e '/./,$!d' | awk '{ lines[NR] = $0 } END { n = NR; while (n > 0 && lines[n] ~ /^[ \t]*$/) n--; for (i = 1; i <= n; i++) print lines[i]; if (n == 0) exit 1 }'
