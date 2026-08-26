#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${1:-"$root/target/kitty-graphics-geometry"}
mkdir -p "$output"
work=$(mktemp -d "$output/run.XXXXXX")
runtime=$(mktemp -d /tmp/kgp-geometry.XXXXXX)
sway_pid=
terminal_pid=
cleanup() {
    [ -z "$terminal_pid" ] || kill "$terminal_pid" 2>/dev/null || true
    [ -z "$sway_pid" ] || kill "$sway_pid" 2>/dev/null || true
    rm -rf "$runtime"
}
trap cleanup EXIT INT TERM
export PYTHONDONTWRITEBYTECODE=1
cargo build --manifest-path "$root/Cargo.toml" -p alacritty
export XDG_RUNTIME_DIR="$runtime"
unset SWAYSOCK WAYLAND_DISPLAY
cat > "$work/sway.conf" <<'CONFIG'
output * mode 800x600
seat * hide_cursor 100
focus_follows_mouse no
default_border none
CONFIG
WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1 WLR_LIBINPUT_NO_DEVICES=1 \
    sway -c "$work/sway.conf" > "$work/sway.log" 2>&1 &
sway_pid=$!
for _ in $(seq 1 100); do
    SWAYSOCK=$(find "$runtime" -name 'sway-ipc.*.sock' -print -quit)
    WAYLAND_DISPLAY=$(find "$runtime" -name 'wayland-*' ! -name '*.lock' -print -quit)
    [ -z "$SWAYSOCK" ] || [ -z "$WAYLAND_DISPLAY" ] || break
    sleep 0.05
done
[ -n "$SWAYSOCK" ] && [ -n "$WAYLAND_DISPLAY" ]
export SWAYSOCK WAYLAND_DISPLAY

matches_expected_colors() {
    result=0
    : > "$scene/observed.tsv"
    while read -r color expected; do
        actual=$(awk -v color="#$color " 'index($0, color) {gsub(":", "", $1); print $1}' "$scene/histogram.txt")
        printf '%s\t%s\t%s\n' "$color" "${actual:-0}" "$expected" >> "$scene/observed.tsv"
        [ "${actual:-0}" = "$expected" ] || result=1
    done < "$scene/expected.tsv"
    return "$result"
}

for padding in 0 40; do
    scene="$work/padding-$padding"
    mkdir "$scene"
    cat > "$scene/alacritty.toml" <<CONFIG
[general]
ipc_socket = false
[window]
padding = { x = $padding, y = $padding }
dynamic_padding = false
decorations = "None"
[font]
size = 11.0
[font.normal]
family = "DejaVu Sans Mono"
[colors.primary]
background = "#000000"
[colors.normal]
blue = "#0000ff"
[terminal.shell]
program = "$(command -v python3)"
args = ["$root/tests/kitty_graphics/geometry.py", "$scene"]
CONFIG
    "$root/target/debug/alacritty" --hold --config-file "$scene/alacritty.toml" \
        > "$scene/alacritty.log" 2>&1 &
    terminal_pid=$!
    for _ in $(seq 1 100); do
        if swaymsg -t get_tree -r | jq -e '.. | objects | select(.app_id? == "Alacritty")' >/dev/null; then
            touch "$scene/start"
            break
        fi
        sleep 0.05
    done
    for _ in $(seq 1 200); do
        [ ! -e "$scene/ready" ] || break
        sleep 0.05
    done
    [ -e "$scene/ready" ] || { grim "$scene/frame.png"; echo "geometry fixture failed: $scene" >&2; exit 1; }
    matched=false
    for _ in $(seq 1 100); do
        grim "$scene/frame.png"
        magick "$scene/frame.png" -format %c histogram:info:- > "$scene/histogram.txt"
        if matches_expected_colors; then matched=true; break; fi
        sleep 0.05
    done
    [ "$matched" = true ] || { echo "geometry framebuffer mismatch: $scene/observed.tsv" >&2; exit 1; }
    kill "$terminal_pid"
    wait "$terminal_pid" || true
    terminal_pid=
done
printf 'Graphics geometry and CPU composition passed at both padding values: %s\n' "$work"
