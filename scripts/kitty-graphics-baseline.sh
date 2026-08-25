#!/usr/bin/env bash
set -euo pipefail

export LC_ALL=C
export PYTHONDONTWRITEBYTECODE=1
for command in awk b3sum cargo git grim jq kitty magick python3 rustc setsid sway swaymsg; do
    command -v "$command" >/dev/null || {
        echo "missing required command: $command" >&2
        exit 1
    }
done

root=$(cd -- "$(dirname -- "$0")/.." && pwd)
baseline="$root/tests/kitty_graphics/baseline"
generated="$root/tests/kitty_graphics/.baseline.generated"
previous="$root/tests/kitty_graphics/.baseline.previous"
work="$root/target/kitty-graphics-baseline"
runtime="$work/runtime"
client="$root/tests/kitty_graphics/client.py"
reporter="$root/tests/kitty_graphics/report.py"
python=$(command -v python3)
terminal_pid=

if [[ ! -e "$baseline" && -e "$previous" ]]; then
    mv "$previous" "$baseline"
elif [[ -e "$baseline" && -e "$previous" ]]; then
    rm -rf "$previous"
fi
rm -rf "$work" "$generated"
mkdir -p "$runtime" "$generated/alacritty" "$generated/kitty"
chmod 700 "$runtime"

stop_process_group() {
    pid=$1
    kill -TERM -- "-$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
}

cleanup() {
    [[ -z "$terminal_pid" ]] || stop_process_group "$terminal_pid"
    [[ ! -f "$work/sway.pid" ]] || stop_process_group "$(cat "$work/sway.pid")"
    if [[ ! -e "$baseline" && -e "$previous" ]]; then
        mv "$previous" "$baseline"
    fi
}
trap cleanup EXIT INT TERM

python3 -m unittest discover -s "$root/tests/kitty_graphics" -p 'test_*.py'
build_log="$work/alacritty-build.log"
if (cd "$root" && cargo build --quiet --color never -p alacritty) \
    >"$build_log" 2>&1; then
    alacritty_build_status=success
    printf 'success\n' >"$generated/alacritty/build.txt"
else
    alacritty_build_status=failure
    {
        printf 'failure\n'
        sed "s|$root|\$REPOSITORY|g" "$build_log"
    } >"$generated/alacritty/build.txt"
fi
cat "$build_log"
python3 "$client" unused --write-stimulus "$generated/stimulus.hex"

cat >"$work/alacritty.toml" <<EOF
[general]
ipc_socket = false

[colors.primary]
background = "#000000"
foreground = "#ffffff"

[font]
size = 11.0

[font.normal]
family = "DejaVu Sans Mono"

[window]
decorations = "None"
dimensions = { columns = 80, lines = 24 }
padding = { x = 0, y = 0 }

[terminal.shell]
program = "$python"
args = ["$client", "$work/alacritty"]
EOF

cat >"$work/kitty.conf" <<EOF
background #000000
foreground #ffffff
font_family DejaVu Sans Mono
font_size 11.0
remember_window_size no
initial_window_width 640
initial_window_height 480
window_border_width 0
window_margin_width 0
window_padding_width 0
hide_window_decorations yes
cursor_shape block
shell .
EOF

cat >"$work/sway.conf" <<'EOF'
output * mode 800x600
seat * hide_cursor 100
focus_follows_mouse no
font monospace 10
default_border none
default_floating_border none
for_window [app_id="Alacritty"] floating enable, resize set 640 480, move position 0 0
for_window [app_id="kitty-kgp-baseline"] floating enable, resize set 640 480, move position 0 0
EOF

XDG_RUNTIME_DIR="$runtime" WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1 \
    WLR_LIBINPUT_NO_DEVICES=1 \
    setsid sway -c "$work/sway.conf" >"$work/sway.log" 2>&1 &
echo $! >"$work/sway.pid"

socket=
wayland=
for _ in $(seq 1 200); do
    socket=$(find "$runtime" -name 'sway-ipc.*.sock' -print -quit)
    wayland=$(find "$runtime" -name 'wayland-*' ! -name '*.lock' -print -quit)
    wayland=${wayland##*/}
    [ -z "$socket" ] || [ -z "$wayland" ] || break
    sleep 0.05
done
[ -n "$socket" ] && [ -n "$wayland" ] || {
    echo "headless Sway did not start" >&2
    exit 1
}

sway_command() {
    XDG_RUNTIME_DIR="$runtime" SWAYSOCK="$socket" swaymsg "$@" >/dev/null
}

window_exists() {
    app_id=$1
    XDG_RUNTIME_DIR="$runtime" SWAYSOCK="$socket" swaymsg -t get_tree -r \
        | jq -e --arg app_id "$app_id" '.. | objects | select(.app_id? == $app_id)' \
        >/dev/null
}

run_terminal() {
    name=$1
    app_id=$2
    shift 2
    rm -rf "${work:?}/$name"
    mkdir -p "$work/$name"
    XDG_RUNTIME_DIR="$runtime" SWAYSOCK="$socket" WAYLAND_DISPLAY="$wayland" \
        setsid "$@" >"$work/$name.log" 2>&1 &
    terminal_pid=$!

    for _ in $(seq 1 300); do
        window_exists "$app_id" && break
        sleep 0.05
    done
    window_exists "$app_id" || {
        echo "$name window did not appear" >&2
        exit 1
    }

    sway_command "[app_id=\"$app_id\"]" resize set width 640 px height 480 px
    sway_command "[app_id=\"$app_id\"]" move position 0 px 0 px
    sleep 0.2
    touch "$work/$name/start"

    for _ in $(seq 1 300); do
        [ -e "$work/$name/ready" ] && break
        sleep 0.05
    done
    [ -e "$work/$name/ready" ] || {
        echo "$name fixture did not become ready" >&2
        exit 1
    }

    sleep 0.2
    XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$wayland" \
        grim "$generated/$name/frame.png"
    magick "$generated/$name/frame.png" -strip "$generated/$name/frame.png"
    cp "$work/$name/transcript.hex" "$generated/$name/transcript.hex"

    sway_command "[app_id=\"$app_id\"]" kill || true
    stop_process_group "$terminal_pid"
    terminal_pid=
    for _ in $(seq 1 100); do
        window_exists "$app_id" || break
        sleep 0.02
    done
    if window_exists "$app_id"; then
        echo "$name window did not close" >&2
        exit 1
    fi
}

if [[ "$alacritty_build_status" == success ]]; then
    run_terminal alacritty Alacritty "$root/target/debug/alacritty" \
        --config-file "$work/alacritty.toml"
else
    : >"$generated/alacritty/transcript.hex"
fi
run_terminal kitty kitty-kgp-baseline kitty --config "$work/kitty.conf" \
    --class kitty-kgp-baseline "$python" "$client" "$work/kitty"

semantic_count() {
    image=$1
    predicate=$2
    magick "$image" -fx "$predicate ? 1 : 0" -format '%[fx:mean*w*h]\n' info: \
        | awk '{printf "%.0f\n", $1}'
}

terminals=(kitty)
if [[ "$alacritty_build_status" == success ]]; then
    terminals+=(alacritty)
fi
for terminal in "${terminals[@]}"; do
    image="$generated/$terminal/frame.png"
    {
        printf 'red=%s\n' "$(semantic_count "$image" 'r > 0.7 && g < 0.2 && b < 0.2')"
        printf 'green=%s\n' "$(semantic_count "$image" 'g > 0.7 && r < 0.2 && b < 0.2')"
        printf 'magenta=%s\n' "$(semantic_count "$image" 'r > 0.7 && b > 0.7 && g < 0.2')"
    } >"$generated/$terminal/metrics.txt"
done

if [[ "$alacritty_build_status" == success ]]; then
    different_pixels=$(magick "$generated/alacritty/frame.png" "$generated/kitty/frame.png" \
        -compose difference -composite -threshold 0 -format '%[fx:mean*w*h]\n' info: \
        | awk '{printf "%.0f\n", $1}')
    magick "$generated/alacritty/frame.png" "$generated/kitty/frame.png" +append \
        -strip "$generated/comparison.png"
else
    different_pixels=unavailable
fi

source_digest=$(
    cd "$root"
    git ls-files \
        | grep -v '^tests/kitty_graphics/baseline/' \
        | LC_ALL=C sort \
        | while IFS= read -r path; do
            if [[ -L "$path" ]]; then
                printf '%s  %s\n' "$(readlink "$path" | b3sum | awk '{print $1}')" "$path"
            elif [[ -f "$path" ]]; then
                b3sum "$path"
            fi
        done \
        | b3sum \
        | awk '{print $1}'
)
if [[ "$alacritty_build_status" == success ]]; then
    alacritty_version=$("$root/target/debug/alacritty" --version | sed 's/ ([0-9a-f][0-9a-f]*)$//')
else
    alacritty_version=unavailable
fi
kitty_version=$(kitty --version | head -1)
rust_version=$(rustc --version)
python3 "$reporter" \
    --root "$generated" \
    --source-digest "$source_digest" \
    --alacritty-version "$alacritty_version" \
    --kitty-version "$kitty_version" \
    --rust-version "$rust_version" \
    --alacritty-build-status "$alacritty_build_status" \
    --comparison-different-pixels "$different_pixels"

manifest="$work/manifest.b3"
(
    cd "$generated"
    find . -type f -print \
        | LC_ALL=C sort \
        | while IFS= read -r path; do b3sum "$path"; done
) >"$manifest"
mv "$manifest" "$generated/manifest.b3"

if [[ -e "$baseline" ]]; then
    mv "$baseline" "$previous"
fi
if ! mv "$generated" "$baseline"; then
    [[ ! -e "$previous" ]] || mv "$previous" "$baseline"
    exit 1
fi
rm -rf "$previous"
printf 'Observational KGP baseline written to %s\n' "$baseline"
