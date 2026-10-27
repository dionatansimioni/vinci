#!/bin/sh
# Lists shared libraries that DaVinci Resolve's files need but that neither Resolve
# nor the container provides. Run inside the Vinci container after installing:
#   distrobox enter --name vinci-free -- sh scripts/missing-libs.sh
# Then map each library to an Ubuntu package and add it to data/recipe.toml (or note
# why it's not needed, like tools Resolve never loads on X11).
#
# Empty output means nothing is missing. Files Resolve never loads in our setup are
# skipped: the control-panel setup tool (needs OpenSSL 1.1), the helper sqlite3 binary,
# Qt's framebuffer platform plugin (we use xcb) and optional onboarding QML modules.
export LD_LIBRARY_PATH=/opt/resolve/libs:/opt/resolve/bin
IGNORE='^(DaVinci Control Panels Setup/|bin/sqlite3$|libs/plugins/platforms/libqlinuxfb\.so$|Onboarding/qml/)'
find /opt/resolve \( -name '*.so*' -o -name '*.plugin' -o -path '*/bin/*' \) -type f 2>/dev/null |
while read -r f; do
    ldd "$f" 2>/dev/null | awk -v f="${f#/opt/resolve/}" '/not found/ {print $1 "\t" f}'
done | sort -u | while IFS="$(printf '\t')" read -r lib file; do
    # Libraries Resolve ships itself are found at runtime through its own paths.
    printf '%s\n' "$file" | grep -Eq "$IGNORE" && continue
    find /opt/resolve -name "$lib" -print -quit | grep -q . || printf '%s\t%s\n' "$lib" "$file"
done
