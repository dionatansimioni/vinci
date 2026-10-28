#!/bin/sh
# Builds Vinci-<version>-x86_64.AppImage inside an Ubuntu 24.04 container, so the result runs on
# any distro with glibc >= 2.39 (GTK and libadwaita are bundled by linuxdeploy's GTK plugin).
#
#   build-aux/appimage/build.sh            # needs podman (or ENGINE=docker)
#
# Output: build-aux/appimage/out/Vinci-<version>-x86_64.AppImage (+ .zsync for delta
# updates). The version is the workspace one in Cargo.toml: bump it for every build you
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
./linuxdeploy --appdir \$APPDIR --plugin gtk --output appimage \
    --desktop-file \$APPDIR/usr/share/applications/io.github.dionatansimioni.Vinci.desktop \
    --icon-file \$APPDIR/usr/share/icons/hicolor/scalable/apps/io.github.dionatansimioni.Vinci.svg
mkdir -p /src/build-aux/appimage/out
mv $OUTPUT* /src/build-aux/appimage/out/
"
