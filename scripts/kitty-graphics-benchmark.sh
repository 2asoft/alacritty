#!/bin/sh
set -eu

for command in cargo hyperfine python3; do
    command -v "$command" >/dev/null || { echo "missing required command: $command" >&2; exit 1; }
done

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${1:-"$root/target/kitty-graphics-benchmark.json"}
iterations=${KITTY_BENCH_ITERATIONS:-200000}
mkdir -p "$(dirname -- "$output")"

cargo build --manifest-path "$root/Cargo.toml" -p alacritty_terminal \
    --release --example kitty_parser_benchmark

binary="$root/target/release/examples/kitty_parser_benchmark"
hyperfine --warmup 3 --runs 10 --export-json "$output" \
    --command-name disabled "$binary disabled $iterations" \
    --command-name enabled "$binary enabled $iterations"

python3 - "$output" <<'PY'
import json
import sys

results = {result["command"]: result["median"] for result in json.load(open(sys.argv[1]))["results"]}
disabled = results["disabled"]
enabled = results["enabled"]
ratio = enabled / disabled
print(f"graphics-disabled baseline: {disabled:.6f}s")
print(f"graphics-enabled ordinary text: {enabled:.6f}s ({ratio:.3f}x)")
if ratio > 1.05:
    raise SystemExit("ordinary text parser regression exceeds 5%")
PY
