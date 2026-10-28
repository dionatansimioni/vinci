#!/bin/sh
# Opens the built AppImage in a clean Ubuntu 24.04 container that has only what every
# desktop has (X11/Wayland client libs, fonts, the shared-mime-info database that
# gdk-pixbuf needs to recognise SVG icons) and NO GTK/libadwaita, and checks that a
# "Vinci" window appears with no icon errors. Proves the AppImage is self-contained.
#
#   build-aux/appimage/smoke-test.sh [DISPLAY]     # default :99 (an Xvfb is started)
#
# Tests the newest build in out/ (highest version), or the one in IMAGE=out/<file>.
#
# Needs podman (or ENGINE=docker), Xvfb and xdotool on the host. Optional: SHOT=file.png
# saves a screenshot (needs ffmpeg on the host).
set -eu
cd "$(dirname "$0")"
ENGINE="${ENGINE:-podman}"
DISPLAY_NUM="${1:-:99}"
IMAGE="${IMAGE:-$(ls out/Vinci-*-x86_64.AppImage 2>/dev/null | sort -V | tail -1)}"
[ -x "$IMAGE" ] || { echo "build first: $IMAGE not found" >&2; exit 2; }

started_x=""
if ! DISPLAY="$DISPLAY_NUM" xdotool getdisplaygeometry >/dev/null 2>&1; then
    Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 >/dev/null 2>&1 &
    started_x=$!
    sleep 2
fi
name="vinci-smoke-$$"
cleanup() {
    "$ENGINE" rm -f "$name" >/dev/null 2>&1 || true
    [ -n "$started_x" ] && kill "$started_x" 2>/dev/null || true
}
trap cleanup EXIT

"$ENGINE" run -d --name "$name" --network host -e DISPLAY="$DISPLAY_NUM" \
    -e APPIMAGE_EXTRACT_AND_RUN=1 -e VINCI_IMAGE="/app/$(basename "$IMAGE")" -v /tmp/.X11-unix:/tmp/.X11-unix \
    -v "$PWD/out:/app:ro" docker.io/library/ubuntu:24.04 sh -c '
    apt-get update -qq --error-on=any &&
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
        shared-mime-info libx11-6 libxcb1 libwayland-client0 libfontconfig1 libfreetype6 \
        libharfbuzz0b libfribidi0 fonts-dejavu-core >/dev/null &&
    exec "$VINCI_IMAGE"' >/dev/null

i=0
until DISPLAY="$DISPLAY_NUM" xdotool search --name '^Vinci$' >/dev/null 2>&1; do
    i=$((i + 1))
    if [ "$i" -gt 90 ] || [ "$("$ENGINE" inspect -f '{{.State.Running}}' "$name")" != true ]; then
        echo "FAIL: no Vinci window" >&2
        "$ENGINE" logs "$name" 2>&1 | tail -20 >&2
        exit 1
    fi
    sleep 2
done
sleep 3
if [ -n "${SHOT:-}" ]; then
    ffmpeg -loglevel error -y -f x11grab -video_size 1280x800 -i "$DISPLAY_NUM" -frames:v 1 "$SHOT"
fi
if "$ENGINE" logs "$name" 2>&1 | grep -q 'Failed to load icon'; then
    echo "FAIL: icons did not load" >&2
    exit 1
fi
echo "OK: Vinci window opened from the AppImage on a system without GTK/libadwaita"
