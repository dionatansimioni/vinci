#!/bin/sh
# Builds Vinci-x86_64.AppImage inside an Ubuntu 24.04 container, so the result runs on
# any distro with glibc >= 2.39 (GTK and libadwaita are bundled by linuxdeploy's GTK plugin).
#
#   build-aux/appimage/build.sh            # needs podman (or ENGINE=docker)
#
# Output: build-aux/appimage/out/Vinci-x86_64.AppImage (+ .zsync for delta updates)
set -eu
cd "$(dirname "$0")/../.."
ENGINE="${ENGINE:-podman}"
# Pinned tool versions: bump deliberately, never "continuous".
LINUXDEPLOY=1-alpha-20250213-2
GTK_PLUGIN_COMMIT=7a3fbc31a9e5075073ff8790f26effbac5f84453

exec "$ENGINE" run --rm --network host -v "$PWD:/src" -w /src docker.io/library/ubuntu:24.04 sh -euc "
export DEBIAN_FRONTEND=noninteractive
apt-get update --error-on=any -qq
apt-get install -y -qq --no-install-recommends ca-certificates curl build-essential pkg-config \
    libgtk-4-dev libadwaita-1-dev gettext file zsync librsvg2-common >/dev/null
curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null
. \$HOME/.cargo/env
cargo build --release --locked -p vinci

APPDIR=/tmp/AppDir
install -Dm755 target/release/vinci \$APPDIR/usr/bin/vinci
install -Dm644 data/io.github.dionatansimioni.Vinci.desktop -t \$APPDIR/usr/share/applications
install -Dm644 data/io.github.dionatansimioni.Vinci.metainfo.xml -t \$APPDIR/usr/share/metainfo
install -Dm644 data/icons/io.github.dionatansimioni.Vinci.svg -t \$APPDIR/usr/share/icons/hicolor/scalable/apps
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
export LDAI_OUTPUT=Vinci-x86_64.AppImage
./linuxdeploy --appdir \$APPDIR --plugin gtk --output appimage \
    --desktop-file \$APPDIR/usr/share/applications/io.github.dionatansimioni.Vinci.desktop \
    --icon-file \$APPDIR/usr/share/icons/hicolor/scalable/apps/io.github.dionatansimioni.Vinci.svg
mkdir -p /src/build-aux/appimage/out
mv Vinci-x86_64.AppImage* /src/build-aux/appimage/out/
"
