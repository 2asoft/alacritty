#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${1:-"$root/target/kitty-graphics-memory"}
mkdir -p "$output"
work=$(mktemp -d "$output/run.XXXXXX")
cargo build --manifest-path "$root/Cargo.toml" -p alacritty_terminal \
    --example kitty_memory_measurement --features fuzzing --release > "$work/build.txt" 2>&1
rustc --version > "$work/toolchain.txt"
for case in animation_frame compose_frames edit_frame direct_rgb direct_rgba; do
    "$root/target/release/examples/kitty_memory_measurement" "$case" > "$work/$case.txt"
    quota=$(awk -F= '$1 == "quota_bytes" { print $2 }' "$work/$case.txt")
    peak=$(awk -F= '$1 == "peak_additional_live_allocation_bytes" { print $2 }' "$work/$case.txt")
    case "$case" in
        compose_frames) limit=$((quota + quota / 2 + 1048576)) ;;
        edit_frame) limit=$((3 * quota + 1048576)) ;;
        *) limit=$((2 * quota + 1048576)) ;;
    esac
    [ "$peak" -le "$limit" ] || {
        printf '%s exceeded fixture allocation bound: %s > %s\n' "$case" "$peak" "$limit" >&2
        exit 1
    }
    printf '%s peak=%s limit=%s\n' "$case" "$peak" "$limit"
done
printf 'Graphics memory checks passed: %s\n' "$work"
