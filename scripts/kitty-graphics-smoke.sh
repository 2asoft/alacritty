#!/bin/sh
set -eu

for command in cargo sway swaymsg grim magick jq python3; do
    command -v "$command" >/dev/null || { echo "missing required command: $command" >&2; exit 1; }
done

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${1:-"$root/target/kitty-graphics-smoke"}
runtime="$output/runtime"
rm -rf "$output"
mkdir -p "$runtime"
chmod 700 "$runtime"

cleanup() {
    [ ! -f "$output/alacritty.pid" ] || kill "$(cat "$output/alacritty.pid")" 2>/dev/null || true
    [ ! -f "$output/sway.pid" ] || kill "$(cat "$output/sway.pid")" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

cargo build --manifest-path "$root/Cargo.toml" -p alacritty

python3 - "$output/alacritty.toml" <<'PY'
import base64
import sys
from pathlib import Path

pixel = base64.b64encode(bytes([255, 0, 0, 255])).decode("ascii")
transparent = base64.b64encode(bytes([0, 255, 255, 0])).decode("ascii")
placeholder = "\U0010eeee\u0305"
script = (
    "printf 'Thin text must remain stable across redraws\\nSecond line 0123456789'; "
    f"printf '\\033_Ga=t,q=2,f=32,s=1,v=1,i=1;{pixel}\\033\\\\'; "
    "printf '\\033[5;1H\\033_Ga=p,q=2,i=1,c=4,r=4,z=-1,C=1\\033\\\\'; "
    f"printf '\\033_Ga=T,q=2,f=32,s=1,v=1,i=16711935,U=1,c=1,r=1;{transparent}\\033\\\\'; "
    f"printf '\\033[10;1H\\033[38;2;255;0;255m{placeholder}\\033[0m\\033[20;20H'; "
    "sleep 30"
)
quoted = '"' + script.replace('\\', '\\\\').replace('"', '\\"') + '"'
Path(sys.argv[1]).write_text(
    '[window]\n'
    'dimensions = { columns = 60, lines = 24 }\n'
    '[terminal.graphics]\n'
    'enabled = true\n'
    '[terminal.shell]\n'
    'program = "/bin/sh"\n'
    f'args = ["-c", {quoted}]\n',
    encoding="utf-8",
)
PY

cat >"$output/sway.conf" <<'EOF'
output * mode 800x600
seat * hide_cursor 1000
focus_follows_mouse no
font monospace 10
EOF

(
    export XDG_RUNTIME_DIR="$runtime"
    unset SWAYSOCK WAYLAND_DISPLAY
    WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1 WLR_LIBINPUT_NO_DEVICES=1 \
        sway -c "$output/sway.conf" >"$output/sway.log" 2>&1 &
    echo $! >"$output/sway.pid"
)

socket=
wayland=
for _ in $(seq 1 100); do
    socket=$(find "$runtime" -name 'sway-ipc.*.sock' -print -quit)
    wayland=$(find "$runtime" -name 'wayland-*' ! -name '*.lock' -printf '%f\n' | head -1)
    [ -z "$socket" ] || [ -z "$wayland" ] || break
    sleep 0.05
done
[ -n "$socket" ] && [ -n "$wayland" ] || { echo "headless sway did not start" >&2; exit 1; }

XDG_RUNTIME_DIR="$runtime" SWAYSOCK="$socket" WAYLAND_DISPLAY="$wayland" \
    ALACRITTY_TEST_DISCARD_IMAGE_TEXTURES=1 "$root/target/debug/alacritty" --config-file "$output/alacritty.toml" \
    >"$output/alacritty.log" 2>&1 &
echo $! >"$output/alacritty.pid"

for _ in $(seq 1 100); do
    if XDG_RUNTIME_DIR="$runtime" SWAYSOCK="$socket" swaymsg -t get_tree -r \
        | jq -e '.. | objects | select(.app_id? == "Alacritty")' >/dev/null; then
        break
    fi
    sleep 0.03
done
sleep 1

for frame in 1 2 3 4; do
    XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$wayland" \
        grim "$output/frame-$frame.png"
    magick "$output/frame-$frame.png" -crop 600x70+0+25 +repage "$output/text-$frame.png"
    sleep 0.6
done

text_pixels=$(magick "$output/text-1.png" -format %c histogram:info:- \
    | awk '/#D8D8D8 / { gsub(":", "", $1); print $1 }')
[ "${text_pixels:-0}" -ge 40 ] || {
    echo "expected text above negative-z image was missing from framebuffer" >&2
    exit 1
}

placeholder_pixels=$(magick "$output/frame-1.png" -format %c histogram:info:- \
    | awk '/#FF00FF / { gsub(":", "", $1); print $1 }')
[ "${placeholder_pixels:-0}" -eq 0 ] || {
    echo "unicode placeholder glyph leaked through its transparent image tile" >&2
    exit 1
}

red_pixels=$(magick "$output/frame-1.png" -format %c histogram:info:- \
    | awk '/#FF0000 / { gsub(":", "", $1); print $1 }')
[ "${red_pixels:-0}" -ge 500 ] || {
    echo "expected rendered red image was missing from framebuffer" >&2
    exit 1
}

for frame in 2 3 4; do
    changed=$(magick compare -metric AE "$output/text-1.png" "$output/text-$frame.png" null: 2>&1 || true)
    [ "$changed" = "0 (0)" ] || {
        echo "text pixels changed across graphics redraws: $changed" >&2
        exit 1
    }
done

printf 'Kitty graphics smoke test passed. Screenshots: %s/frame-*.png\n' "$output"
