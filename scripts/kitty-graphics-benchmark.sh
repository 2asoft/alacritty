#!/bin/sh
set -eu

for command in cargo hyperfine; do
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
    --command-name ordinary-text "$binary $iterations"

printf 'Ordinary-text parser benchmark written to %s\n' "$output"
