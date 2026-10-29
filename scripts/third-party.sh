#!/bin/sh
# Regenerates the Rust crate table in THIRD-PARTY.md from Cargo.lock: every crate compiled
# into the vinci binary on Linux, with its version and license. Run after changing
# dependencies; CI fails if the table is stale.
#
#   scripts/third-party.sh          # rewrite THIRD-PARTY.md
#   scripts/third-party.sh --check  # exit 1 if it would change
set -eu
cd "$(dirname "$0")/.."
crates=$(cargo tree -p vinci -e normal --prefix none --target x86_64-unknown-linux-gnu \
    | sed 's/ (.*//' | sort -u)
table=$(cargo metadata --format-version 1 --locked | CRATES="$crates" python3 -c '
import json, os, sys
wanted = set(os.environ["CRATES"].split("\n"))
meta = json.load(sys.stdin)
rows = sorted(
    (p["name"], p["version"], p.get("license") or "see crate")
    for p in meta["packages"]
    if p["source"] and p["name"] + " v" + p["version"] in wanted
)
print("| Crate | Version | License |")
print("|---|---|---|")
for n, v, l in rows:
    print(f"| [{n}](https://crates.io/crates/{n}) | {v} | {l} |")
')
new=$(awk -v table="$table" '
    /<!-- crates:start -->/ { print; print table; skip = 1; next }
    /<!-- crates:end -->/ { skip = 0 }
    !skip { print }
' THIRD-PARTY.md)
if [ "${1:-}" = "--check" ]; then
    [ "$new" = "$(cat THIRD-PARTY.md)" ] || { echo "THIRD-PARTY.md is stale: run scripts/third-party.sh" >&2; exit 1; }
else
    printf '%s\n' "$new" > THIRD-PARTY.md
fi
