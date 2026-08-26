#!/bin/sh
set -eu

for command in cargo sway swaymsg grim magick jq python3; do
    command -v "$command" >/dev/null || { echo "missing required command: $command" >&2; exit 1; }
done

root=$(CDPATH=; cd -- "$(dirname -- "$0")/.." && pwd)
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
import os
import sys
from pathlib import Path

def encoded(*rgba):
    return base64.b64encode(bytes(rgba)).decode("ascii")

red = encoded(255, 0, 0, 255)
green = encoded(0, 255, 0, 255)
transparent = encoded(0, 255, 255, 0)
crop = encoded(255, 0, 0, 255, 0, 255, 0, 255)
very_negative = encoded(68, 85, 102, 255)
normal_negative = encoded(119, 136, 153, 255)
positive = encoded(171, 205, 239, 255)
half_red = encoded(255, 0, 0, 128)
half_blue = encoded(0, 0, 255, 128)
cyan = encoded(0, 255, 255, 255)
yellow = encoded(255, 255, 0, 255)
tiled = encoded(*(200, 10, 200, 255) * 4)
placeholder = "\U0010eeee\u0305"
client_frame_marker = str(Path(sys.argv[1]).with_name("client-frame-ready"))
scene = os.environ.get("KITTY_SMOKE_PASS_SCENE")
if scene:
    scenes = {
        "none": "printf '\\033[2J\\033[HNO GRAPHICS TEXT'; printf '\\033[3;3H'; sleep 30",
        "very-negative": (
            f"printf '\\033_Ga=T,q=2,f=32,s=1,v=1,i=1,c=4,r=2,z=-1073741825,C=1;{very_negative}\\033\\\\'; "
            "printf '\\033[1;1H\\033[48;2;17;34;51mTEXT\\033[2;1H    \\033[0m'; sleep 30"
        ),
        "middle-negative": (
            f"printf '\\033_Ga=T,q=2,f=32,s=1,v=1,i=1,c=4,r=2,z=-1,C=1;{normal_negative}\\033\\\\'; "
            "printf '\\033[1;1HMIDDLE NEGATIVE TEXT'; sleep 30"
        ),
        "positive": (
            f"printf '\\033[1;1H\\033_Ga=T,q=2,f=32,s=1,v=1,i=1,c=4,r=2,z=1,C=1;{positive}\\033\\\\'; "
            "printf '\\033[1;1H'; sleep 30"
        ),
    }
    script = scenes[scene]
else:
    script = (
    "printf 'Thin text must remain stable across redraws\\nSecond line 0123456789'; "
    f"printf '\\033_Ga=t,q=2,f=32,s=1,v=1,i=1;{red}\\033\\\\'; "
    "printf '\\033[5;1H\\033_Ga=p,q=2,i=1,c=4,r=4,z=-1,C=1\\033\\\\'; "
    f"printf '\\033_Ga=T,q=2,f=32,s=1,v=1,i=16711935,U=1,c=1,r=1;{transparent}\\033\\\\'; "
    f"printf '\\033[10;1H\\033[48;2;18;52;86m\\033[38;2;255;0;255m{placeholder}\\033[0m'; "
    f"printf '\\033[12;1H\\033_Ga=T,q=2,f=32,s=2,v=1,i=2,x=1,w=1,c=4,r=2,C=1;{crop}\\033\\\\'; "
    f"printf '\\033[15;1H\\033_Ga=T,q=2,f=32,s=1,v=1,i=4,c=4,r=2,z=-1073741825,C=1;{very_negative}\\033\\\\'; "
    "printf '\\033[15;1H\\033[48;2;17;34;51m    \\033[16;1H    \\033[0m'; "
    f"printf '\\033[17;1H\\033_Ga=T,q=2,f=32,s=1,v=1,i=5,c=4,r=2,z=-1,C=1;{normal_negative}\\033\\\\'; "
    "printf '\\033[17;1H\\033[48;2;34;51;68m    \\033[18;1H    \\033[0m'; "
    "printf '\\033[19;1HVISIBLE TEXT'; "
    f"printf '\\033[19;1H\\033_Ga=T,q=2,f=32,s=1,v=1,i=6,c=4,r=2,z=1,C=1;{positive}\\033\\\\'; "
    f"printf '\\033[21;1H\\033_Ga=T,q=2,f=32,s=1,v=1,i=7,c=4,r=2,C=1;{half_red}\\033\\\\'; "
    f"printf '\\033[21;1H\\033_Ga=T,q=2,f=32,s=1,v=1,i=8,c=4,r=2,C=1;{half_blue}\\033\\\\'; "
    f"printf '\\033[12;20H\\033_Ga=T,q=2,f=32,s=1,v=1,i=9,c=4,r=2,z=1,C=1;{cyan}\\033\\\\'; "
    f"printf '\\033_Ga=f,q=2,f=32,s=1,v=1,i=9,z=500;{yellow}\\033\\\\'; "
    "printf '\\033_Ga=a,q=2,i=9,r=1,z=500\\033\\\\'; "
    "printf '\\033_Ga=a,q=2,i=9,s=3\\033\\\\'; "
    f"printf '\\033[23;10H\\033_Ga=T,q=2,f=32,s=4,v=1,i=10,c=4,r=1,C=1;{tiled}\\033\\\\'; "
    "printf '\\033[24;60H'; "
    "sleep 10; "
    "printf '\\033_Ga=a,q=2,i=9,s=1,c=2\\033\\\\'; "
    f"touch '{client_frame_marker}'; sleep 25"
    )
quoted = '"' + script.replace('\\', '\\\\').replace('"', '\\"') + '"'
Path(sys.argv[1]).write_text(
    '[window]\n'
    'dimensions = { columns = 60, lines = 24 }\n'
    '[terminal.graphics]\n'
    'storage_limit = 64\n'
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
    ALACRITTY_TEST_DISCARD_IMAGE_TEXTURES=once \
    ALACRITTY_TEST_ANIMATION_STATE_FILE="$output/animation-state" \
    ALACRITTY_TEST_IMAGE_CACHE_FILE="$output/image-cache" \
    ALACRITTY_TEST_MAX_TEXTURE_SIZE=3 \
    "$root/target/debug/alacritty" --config-file "$output/alacritty.toml" \
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
    sleep 0.07
done
for frame in 1 2 3 4; do
    magick "$output/frame-$frame.png" -crop 600x70+0+25 +repage "$output/text-$frame.png"
done

if [ -n "${KITTY_SMOKE_PASS_SCENE:-}" ]; then
    case "$KITTY_SMOKE_PASS_SCENE" in
        none)
            magick "$output/frame-1.png" -format %c histogram:info:- | grep -q '#D8D8D8 ' || {
                echo "no-graphics cell pass lost text or cursor" >&2; exit 1;
            }
            ;;
        very-negative)
            ! magick "$output/frame-1.png" -format %c histogram:info:- | grep -q '#445566 ' || {
                echo "very-negative image escaped cell backgrounds" >&2; exit 1;
            }
            ;;
        middle-negative)
            magick "$output/frame-1.png" -format %c histogram:info:- | grep -q '#778899 ' || {
                echo "middle-negative image was not rendered between cell passes" >&2; exit 1;
            }
            ;;
        positive)
            histogram=$(magick "$output/frame-1.png" -format %c histogram:info:-)
            printf '%s\n' "$histogram" | grep -q '#ABCDEF ' || {
                echo "positive image was not rendered above cells" >&2; exit 1;
            }
            printf '%s\n' "$histogram" | grep -q '#D8D8D8 ' || {
                echo "cursor was not rendered above the positive image" >&2; exit 1;
            }
            ;;
    esac
    printf 'Kitty graphics %s cell-pass smoke test passed. Screenshot: %s/frame-1.png\n' \
        "$KITTY_SMOKE_PASS_SCENE" "$output"
    exit 0
fi

wait_animation_state() {
    expected=$1
    for _ in $(seq 1 200); do
        [ "$(cat "$output/animation-state" 2>/dev/null || true)" = "$expected" ] && return 0
        sleep 0.03
    done
    echo "animation did not reach frame $expected" >&2
    return 1
}
wait_animation_state 1
sleep 0.2
XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$wayland" grim "$output/animation-frame.png"
wait_animation_state 0
sleep 0.2
XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$wayland" grim "$output/animation-root.png"
for _ in $(seq 1 400); do
    [ -e "$output/client-frame-ready" ] && break
    sleep 0.03
done
[ -e "$output/client-frame-ready" ] || { echo "client frame selection did not complete" >&2; exit 1; }
sleep 0.2
XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$wayland" grim "$output/client-frame.png"

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

for color in 123456 00FF00 112233 778899 ABCDEF; do
    pixels=$(magick "$output/frame-1.png" -format %c histogram:info:- \
        | awk -v color="#$color " 'index($0, color) { gsub(":", "", $1); print $1 }')
    [ "${pixels:-0}" -ge 20 ] || {
        echo "expected protocol framebuffer color #$color was missing" >&2
        exit 1
    }
done

alpha_pixels=$(magick "$output/frame-1.png" -format %c histogram:info:- \
    | awk '/#460686 / { gsub(":", "", $1); print $1 }')
[ "${alpha_pixels:-0}" -ge 500 ] || {
    echo "overlapping translucent images did not use source-over composition" >&2
    exit 1
}

very_negative_pixels=$(magick "$output/frame-1.png" -format %c histogram:info:- \
    | awk '/#445566 / { gsub(":", "", $1); print $1 }')
[ "${very_negative_pixels:-0}" -eq 0 ] || {
    echo "very-negative image rendered above a non-default cell background" >&2
    exit 1
}

magick "$output/animation-root.png" -format %c histogram:info:- | grep -q '#00FFFF ' || {
    echo "animation root frame was missing from framebuffer" >&2
    exit 1
}
magick "$output/animation-frame.png" -format %c histogram:info:- | grep -q '#FFFF00 ' || {
    echo "animation second frame was missing from framebuffer" >&2
    exit 1
}

read -r cache_bytes _ cache_evictions <"$output/image-cache"
[ "$cache_bytes" -le 64 ] && [ "$cache_evictions" -gt 0 ] || {
    echo "image cache did not remain bounded under animation churn" >&2
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
