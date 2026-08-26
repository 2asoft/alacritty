#!/bin/sh
set -eu

for command in cargo python3; do
    command -v "$command" >/dev/null || {
        echo "missing required command: $command" >&2
        exit 1
    }
done

root=$(CDPATH=; cd -- "$(dirname -- "$0")/.." && pwd)
output=${1:-"$root/target/kitty-graphics-measurements.json"}
runs=${KITTY_MEASUREMENT_RUNS:-3}
case "$runs" in
    ''|*[!0-9]*) echo "KITTY_MEASUREMENT_RUNS must be a positive odd integer" >&2; exit 1 ;;
esac
if [ "$runs" -le 0 ] || [ $((runs % 2)) -eq 0 ]; then
    echo "KITTY_MEASUREMENT_RUNS must be a positive odd integer" >&2
    exit 1
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$(dirname -- "$output")"

cargo build --manifest-path "$root/Cargo.toml" -p alacritty_terminal \
    --release --example kitty_parser_benchmark
cargo test --manifest-path "$root/Cargo.toml" -p alacritty_terminal \
    --release --no-run --message-format=json >"$tmp/cargo-test.json"

ordinary="$root/target/release/examples/kitty_parser_benchmark"
test_binary=$(python3 - "$tmp/cargo-test.json" <<'PY'
import json
import sys

for line in open(sys.argv[1], encoding="utf-8"):
    message = json.loads(line)
    target = message.get("target", {})
    if (
        message.get("reason") == "compiler-artifact"
        and target.get("name") == "alacritty_terminal"
        and "lib" in target.get("kind", [])
        and message.get("profile", {}).get("test")
    ):
        print(message["executable"])
        break
else:
    raise SystemExit("alacritty_terminal test binary not found")
PY
)
cat >"$tmp/run-with-rss.py" <<'PY'
import os
import sys

stdout_path, rss_path, separator, *command = sys.argv[1:]
if separator != "--" or not command:
    raise SystemExit("usage: run-with-rss.py STDOUT RSS -- COMMAND...")
pid = os.fork()
if pid == 0:
    output = os.open(stdout_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    os.dup2(output, 1)
    os.execvp(command[0], command)
_, status, usage = os.wait4(pid, 0)
with open(rss_path, "w", encoding="ascii") as output:
    output.write(f"{usage.ru_maxrss}\n")
if not os.WIFEXITED(status):
    raise SystemExit(128 + os.WTERMSIG(status))
raise SystemExit(os.WEXITSTATUS(status))
PY
cases='ordinary_text
direct_rgba_64_mib
direct_rgb_64_mib
animation_insert_4096_square
animation_compose_4096_square
history_none_100000_lines
history_virtual_100000_lines
history_relative_100000_lines
virtual_placements_65536'

: >"$tmp/samples.tsv"
run_case() {
    case_name=$1
    run=$2
    stdout="$tmp/$case_name-$run.stdout"
    rss="$tmp/$case_name-$run.rss"

    case "$case_name" in
        ordinary_text)
            python3 "$tmp/run-with-rss.py" "$stdout" "$rss" -- "$ordinary" 200000
            elapsed=$(tail -n 1 "$stdout")
            printf '%s\t%s\telapsed_ns\t%s\t%s\n' \
                "$case_name" "$run" "$elapsed" "$(cat "$rss")" >>"$tmp/samples.tsv"
            ;;
        *)
            case "$case_name" in
                direct_*) test_name="measurement_$case_name" ;;
                animation_*) test_name="measurement_$case_name" ;;
                history_*) test_name="measurement_$case_name" ;;
                virtual_placements_65536) test_name=measurement_65536_virtual_placements ;;
            esac
            python3 "$tmp/run-with-rss.py" "$stdout" "$rss" -- \
                "$test_binary" "term::tests::$test_name" --ignored --exact --nocapture
            awk -v case_name="$case_name" -v run="$run" -v rss="$(cat "$rss")" '
                /KGP_MEASUREMENT/ {
                    label="elapsed_ns"
                    if ($1 != "KGP_MEASUREMENT") {
                        label=$1
                        sub(/^KGP_MEASUREMENT_/, "", label)
                    }
                    split($2, value, "=")
                    print case_name "\t" run "\t" label "\t" value[2] "\t" rss
                }
            ' "$stdout" >>"$tmp/samples.tsv"
            ;;
    esac
}

for case_name in $cases; do
    run=1
    while [ "$run" -le "$runs" ]; do
        run_case "$case_name" "$run"
        run=$((run + 1))
    done
done

python3 - "$tmp/samples.tsv" "$output" "$runs" <<'PY'
import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path

samples_path = Path(sys.argv[1])
output_path = Path(sys.argv[2])
groups = defaultdict(list)
for line in samples_path.read_text().splitlines():
    case, run, label, elapsed_ns, peak_rss_kib = line.split("\t")
    key = case if label == "elapsed_ns" else f"{case}:{label}"
    groups[key].append(
        {
            "run": int(run),
            "elapsed_ns": int(elapsed_ns),
            "peak_rss_kib": int(peak_rss_kib),
        }
    )

expected = {
    "ordinary_text",
    "direct_rgba_64_mib",
    "direct_rgb_64_mib",
    "animation_insert_4096_square",
    "animation_compose_4096_square",
    "history_none_100000_lines",
    "history_virtual_100000_lines",
    "history_relative_100000_lines",
    "virtual_placements_65536",
}
if set(groups) != expected:
    missing = sorted(expected - set(groups))
    extra = sorted(set(groups) - expected)
    raise SystemExit(f"measurement case mismatch: missing={missing}, extra={extra}")
if any(len(samples) != int(sys.argv[3]) for samples in groups.values()):
    raise SystemExit("a measurement case did not produce one sample per run")

results = []
for name, samples in groups.items():
    results.append(
        {
            "case": name,
            "samples": samples,
            "median_elapsed_ns": int(statistics.median(s["elapsed_ns"] for s in samples)),
            "median_peak_rss_kib": int(statistics.median(s["peak_rss_kib"] for s in samples)),
        }
    )

output_path.write_text(json.dumps({"schema": 1, "results": results}, indent=2) + "\n")
PY

printf 'Kitty graphics measurements written to %s\n' "$output"
