#!/bin/sh
# Builds Vinci-<version>-x86_64.AppImage inside an Ubuntu 24.04 container, so the result runs on
# any distro with glibc >= 2.39 (GTK and libadwaita are bundled by linuxdeploy's GTK plugin).
#
#   build-aux/appimage/build.sh            # needs podman (or ENGINE=docker)
#
# Output: build-aux/appimage/out/Vinci-<version>-x86_64.AppImage (+ .zsync for delta
# updates, + .bundled-packages.txt listing the Ubuntu packages inside; SOURCES=1 also
# writes .sources.tar with their source code, for releases). The version is the workspace one in Cargo.toml: bump it for every build you
# hand out, so the newest file is always obvious. Older builds stay in out/.
set -eu
cd "$(dirname "$0")/../.."
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
[ -n "$VERSION" ] || { echo "no version in Cargo.toml" >&2; exit 1; }
OUTPUT="Vinci-$VERSION-x86_64.AppImage"
ENGINE="${ENGINE:-podman}"
# Pinned tool versions: bump deliberately, never "continuous".
LINUXDEPLOY=1-alpha-20250213-2
GTK_PLUGIN_COMMIT=7a3fbc31a9e5075073ff8790f26effbac5f84453

exec "$ENGINE" run --rm --network host -v "$PWD:/src" -w /src docker.io/library/ubuntu:24.04 sh -euc "
export DEBIAN_FRONTEND=noninteractive
apt-get update --error-on=any -qq
apt-get install -y -qq --no-install-recommends ca-certificates curl build-essential pkg-config \
    libgtk-4-dev libadwaita-1-dev gettext file zsync librsvg2-common adwaita-icon-theme >/dev/null
curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null
. \$HOME/.cargo/env
# Own target dir: never reuse objects built on a newer host.
export CARGO_TARGET_DIR=/tmp/target
cargo build --release --locked -p vinci

APPDIR=/tmp/AppDir
install -Dm755 /tmp/target/release/vinci \$APPDIR/usr/bin/vinci
install -Dm644 data/io.github.dionatansimioni.Vinci.desktop -t \$APPDIR/usr/share/applications
install -Dm644 data/io.github.dionatansimioni.Vinci.metainfo.xml -t \$APPDIR/usr/share/metainfo
install -Dm644 data/icons/io.github.dionatansimioni.Vinci.svg -t \$APPDIR/usr/share/icons/hicolor/scalable/apps
# Symbolic icons the UI uses: KDE and other desktops may not ship Adwaita's.
install -d \$APPDIR/usr/share/icons/Adwaita
cp -r /usr/share/icons/Adwaita/index.theme /usr/share/icons/Adwaita/symbolic \$APPDIR/usr/share/icons/Adwaita/
# Fixes for the GTK plugin's hook (theme, Wayland, SVG loaders); see the file itself.
install -Dm644 build-aux/appimage/zz-vinci.sh \$APPDIR/apprun-hooks/zz-vinci.sh
for lang in \$(grep -v '^#' po/LINGUAS); do
    install -d \$APPDIR/usr/share/locale/\$lang/LC_MESSAGES
    msgfmt -o \$APPDIR/usr/share/locale/\$lang/LC_MESSAGES/vinci.mo po/\$lang.po
done

cd /tmp
curl -fsSLo linuxdeploy https://github.com/linuxdeploy/linuxdeploy/releases/download/$LINUXDEPLOY/linuxdeploy-x86_64.AppImage
curl -fsSLo linuxdeploy-plugin-gtk.sh https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/$GTK_PLUGIN_COMMIT/linuxdeploy-plugin-gtk.sh
chmod +x linuxdeploy linuxdeploy-plugin-gtk.sh
export APPIMAGE_EXTRACT_AND_RUN=1 DEPLOY_GTK_VERSION=4
export UPDATE_INFORMATION='gh-releases-zsync|dionatansimioni|vinci|latest|Vinci-*x86_64.AppImage.zsync'
export LDAI_OUTPUT=$OUTPUT VERSION=$VERSION
deploy() {
    ./linuxdeploy --appdir \$APPDIR --plugin gtk \"\$@\" \
        --desktop-file \$APPDIR/usr/share/applications/io.github.dionatansimioni.Vinci.desktop \
        --icon-file \$APPDIR/usr/share/icons/hicolor/scalable/apps/io.github.dionatansimioni.Vinci.svg
}
deploy
# Licenses travel with the binaries: ours, the crates', and the copyright file of every
# Ubuntu package whose libraries or icons were bundled (GTK, libadwaita, GLib... are LGPL).
DOC=\$APPDIR/usr/share/doc/vinci
install -Dm644 /src/LICENSE /src/THIRD-PARTY.md -t \$DOC
find \$APPDIR/usr/lib -name '*.so*' -type f -printf '%f\\n' | while read -r lib; do
    dpkg -S \"*/\$lib\" 2>/dev/null | cut -d: -f1 | head -1
done > /tmp/pkgs
dpkg -S /usr/share/icons/Adwaita/index.theme | cut -d: -f1 >> /tmp/pkgs
install -d \$DOC/packages
sort -u /tmp/pkgs | while read -r pkg; do
    dpkg-query -W -f='\${Package} \${Version} (source: \${source:Package} \${source:Version})\\n' \$pkg
    cp /usr/share/doc/\$pkg/copyright \$DOC/packages/\$pkg.copyright 2>/dev/null || true
done > \$DOC/bundled-packages.txt
cp \$DOC/bundled-packages.txt /tmp/bundled-packages.txt
deploy --output appimage
mkdir -p /src/build-aux/appimage/out
mv $OUTPUT* /src/build-aux/appimage/out/
cp /tmp/bundled-packages.txt /src/build-aux/appimage/out/$OUTPUT.bundled-packages.txt
# The exact Ubuntu sources of everything bundled, published next to each release
# (the LGPL asks for the library sources to be available alongside the binaries).
if [ '${SOURCES:-0}' = 1 ]; then
    sed -i 's/^Types: deb\$/Types: deb deb-src/' /etc/apt/sources.list.d/ubuntu.sources
    apt-get update -qq
    mkdir -p /tmp/sources && cd /tmp/sources
    sed -n 's/.*(source: \\([^ ]*\\) \\([^)]*\\)).*/\\1=\\2/p' /tmp/bundled-packages.txt | sort -u |
        xargs apt-get source --download-only -qq
    cd /tmp && tar -cf /src/build-aux/appimage/out/$OUTPUT.sources.tar sources
fi
"
