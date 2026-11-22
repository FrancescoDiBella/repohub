#!/usr/bin/env bash
# Prints the size of what RepoHub ships that this machine can build: the desktop program, the web
# build (the .wasm cargo makes, after wasm-bindgen, after wasm-opt, and what a browser downloads
# with gzip and with brotli), and the fonts and other files compiled in, with how many copies of
# each the program and the .wasm carry. It builds nothing: build first (CONTRIBUTING, "Size").
#
#   scripts/size.sh                       print the table
#   scripts/size.sh --json FILE           also write the numbers as JSON
#   scripts/size.sh --compare FILE        add a column comparing with FILE (JSON from --json), and
#                                         warn when the program, the .wasm or the download grows
#                                         by more than 3% (a GitHub annotation in Actions)
#   scripts/size.sh --crates              also list the biggest crates: cargo-bloat for the
#                                         program (it rebuilds it with symbols, a full build) and
#                                         twiggy for the .wasm, which needs its function names:
#                                         build it with CARGO_PROFILE_WEB_STRIP=false first
#
# Where it looks; set these to measure something else:
#   PROGRAM  the desktop program        target/dist/repohub, else target/release/repohub
#   WASM     the .wasm from cargo       target/wasm32-unknown-unknown/web/repohub.wasm
#   WEB      scripts/web-build.sh's     dist/web
#            folder
# Anything missing is left out. Brotli needs the `brotli` command or Python's brotli module.
# In GitHub Actions the table also goes to the job summary.
set -euo pipefail
cd "$(dirname "$0")/.."

json='' compare='' crates=''
while [ $# -gt 0 ]; do
    case "$1" in
        --json) json=${2:?--json needs a file}; shift 2 ;;
        --compare) compare=${2:?--compare needs a file}; shift 2 ;;
        --crates) crates=1; shift ;;
        *) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 2 ;;
    esac
done

if [ -z "${PROGRAM:-}" ]; then
    PROGRAM=target/release/repohub
    [ -f target/dist/repohub ] && PROGRAM=target/dist/repohub
fi
WASM=${WASM:-target/wasm32-unknown-unknown/web/repohub.wasm}
WEB=${WEB:-dist/web}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

bytes() { [ -f "$1" ] && stat -c %s "$1" || true; }
gz() { [ -f "$1" ] && gzip -9 -n -c "$1" | wc -c || true; }
br() {
    [ -f "$1" ] || return 0
    if command -v brotli > /dev/null; then
        brotli -q 11 -c "$1" | wc -c
    elif python3 -c 'import brotli' 2> /dev/null; then
        python3 -c 'import brotli, sys; print(len(brotli.compress(open(sys.argv[1], "rb").read(), quality=11)))' "$1"
    fi
}
sum() { local t=0 v; for v in "$@"; do [ -n "$v" ] && t=$((t + v)); done; [ "$t" -gt 0 ] && echo "$t" || true; }

# The numbers, by key (the JSON uses the same keys).
declare -A n
n[program]=$(bytes "$PROGRAM")
n[program_gz]=$(gz "$PROGRAM")
n[wasm_cargo]=$(bytes "$WASM")
if [ -f "$WASM" ] && command -v wasm-bindgen > /dev/null; then
    wasm-bindgen --target web --no-typescript --out-dir "$tmp/bindgen" --out-name app "$WASM"
    n[wasm_bindgen]=$(bytes "$tmp/bindgen/app_bg.wasm")
fi
if [ -f "$WEB/app_bg.wasm" ]; then
    n[wasm]=$(bytes "$WEB/app_bg.wasm")
    n[wasm_gz]=$(gz "$WEB/app_bg.wasm")
    n[wasm_br]=$(br "$WEB/app_bg.wasm")
    # Everything the page loads besides the .wasm: the page, app.js and wasm-bindgen's snippets.
    js_br=() js_gz=()
    while IFS= read -r f; do js_br+=("$(br "$f")"); js_gz+=("$(gz "$f")"); done < <(find "$WEB" -type f \( -name '*.js' -o -name index.html \))
    n[js_br]=$(sum "${js_br[@]}")
    n[download_gz]=$(sum "${n[wasm_gz]}" "${js_gz[@]}")
    [ -n "${n[wasm_br]}" ] && n[download_br]=$(sum "${n[wasm_br]}" "${js_br[@]}")
fi

label() {
    case "$1" in
        program) echo "Desktop program ($(uname -s) $(uname -m), $PROGRAM)" ;;
        program_gz) echo "Desktop program, gzip -9 (about the .tar.gz)" ;;
        wasm_cargo) echo ".wasm from cargo ($WASM)" ;;
        wasm_bindgen) echo ".wasm after wasm-bindgen" ;;
        wasm) echo ".wasm after wasm-opt ($WEB/app_bg.wasm)" ;;
        wasm_gz) echo ".wasm, gzip -9" ;;
        wasm_br) echo ".wasm, brotli -q 11" ;;
        js_br) echo "Page, app.js and snippets, brotli" ;;
        download_gz) echo "**Web download, gzip**" ;;
        download_br) echo "**Web download, brotli**" ;;
    esac
}
keys=(program program_gz wasm_cargo wasm_bindgen wasm wasm_gz wasm_br js_br download_gz download_br)
# Growing by more than this, in percent, is worth a warning.
watched=(program wasm download_br)
limit=3

old() { [ -n "$compare" ] && sed -n "s/.*\"$1\": *\([0-9][0-9]*\).*/\1/p" "$compare" | head -n1 || true; }
mb() { awk -v b="$1" 'BEGIN { printf "%.2f", b / 1000000 }'; }
pct() { awk -v a="$1" -v b="$2" 'BEGIN { printf "%+.1f%%", (b - a) * 100 / a }'; }

{
    echo "### Size"
    echo
    if [ -n "$compare" ]; then
        echo "| What | Bytes | MB | Against $(basename "$compare") |"
        echo "| --- | ---: | ---: | ---: |"
    else
        echo "| What | Bytes | MB |"
        echo "| --- | ---: | ---: |"
    fi
    for k in "${keys[@]}"; do
        v=${n[$k]:-}
        [ -n "$v" ] || continue
        row="| $(label "$k") | $v | $(mb "$v") |"
        if [ -n "$compare" ]; then
            o=$(old "$k")
            if [ -n "$o" ]; then row="$row $(mb "$o") → $(pct "$o" "$v") |"; else row="$row – |"; fi
        fi
        echo "$row"
    done
    echo
    # Fonts and other files compiled in. More than one copy means the bytes are there twice.
    echo "| Compiled-in file | Bytes | Copies in the program | Copies in the .wasm |"
    echo "| --- | ---: | ---: | ---: |"
    python3 - "$PROGRAM" "$WASM" assets/fonts/*.ttf assets/*.rgba <<'EOF'
import sys
def load(p):
    try:
        return open(p, "rb").read()
    except OSError:
        return None
program, wasm = load(sys.argv[1]), load(sys.argv[2])
for path in sys.argv[3:]:
    data = open(path, "rb").read()
    copies = lambda blob: "–" if blob is None else str(blob.count(data))
    print(f"| {path} | {len(data)} | {copies(program)} | {copies(wasm)} |")
EOF
} > "$tmp/table.md"
cat "$tmp/table.md"
[ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$tmp/table.md" >> "$GITHUB_STEP_SUMMARY"

if [ -n "$compare" ]; then
    for k in "${watched[@]}"; do
        v=${n[$k]:-} o=$(old "$k")
        [ -n "$v" ] && [ -n "$o" ] || continue
        if awk -v a="$o" -v b="$v" -v l="$limit" 'BEGIN { exit !(b > a * (1 + l / 100)) }'; then
            msg="$(label "$k" | tr -d '*') grew $(pct "$o" "$v") ($(mb "$o") → $(mb "$v") MB), more than $limit%. If that's expected, update $compare (CONTRIBUTING, \"Size\")."
            if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::warning title=Size::$msg"; else echo "warning: $msg" >&2; fi
        fi
    done
fi

if [ -n "$json" ]; then
    {
        echo "{"
        echo "  \"commit\": \"$(git describe --always --dirty 2> /dev/null || echo unknown)\","
        echo "  \"rustc\": \"$(rustc --version 2> /dev/null | cut -d' ' -f2)\","
        for k in "${keys[@]}"; do [ -n "${n[$k]:-}" ] && echo "  \"$k\": ${n[$k]},"; done
        echo "  \"platform\": \"$(uname -s)-$(uname -m)\""
        echo "}"
    } > "$json"
fi

if [ -n "$crates" ]; then
    profile=$(basename "$(dirname "$PROGRAM")")
    if command -v cargo-bloat > /dev/null && [ -f "$PROGRAM" ]; then
        echo
        echo "Biggest crates in the program (cargo bloat, profile $profile, rebuilt with symbols):"
        var=CARGO_PROFILE_$(echo "$profile" | tr 'a-z-' 'A-Z_')_STRIP
        env "$var=false" cargo bloat --profile "$profile" --bin repohub --crates -n 25
    else
        echo "cargo-bloat is not installed (cargo install cargo-bloat), or there is no program." >&2
    fi
    if command -v twiggy > /dev/null && [ -f "$WASM" ]; then
        echo
        echo "Biggest crates in the .wasm (twiggy, $WASM; code by function name, data segments apart):"
        twiggy top -n 1000000 --format csv "$WASM" | python3 -c '
import csv, re, sys
total, by = 0, {}
for row in csv.DictReader(sys.stdin):
    name, size = row["Name"], int(row["ShallowSize"])
    total += size
    if name.startswith("data["):
        crate = "(data: fonts, tables, strings)"
    elif name.startswith(("code[", "elem[", "table[", "type[", "import ", "export ", "global[", "func[")) or "::" not in name:
        crate = "(no name, headers, tables)"
    else:
        crate = re.sub(r"^[<&*\s]*(dyn |impl |mut |const )*", "", name).split("::")[0].split("<")[0] or "?"
    by[crate] = by.get(crate, 0) + size
for crate, size in sorted(by.items(), key=lambda kv: -kv[1])[:25]:
    print(f"{size:>10}  {size * 100 / total:5.1f}%  {crate}")
print(f"{total:>10}  100.0%  total")
if by.get("(no name, headers, tables)", 0) > total / 2:
    print("Most code has no name: build with CARGO_PROFILE_WEB_STRIP=false to see the crates.")
'
    else
        echo "twiggy is not installed (cargo install twiggy), or there is no .wasm." >&2
    fi
fi
