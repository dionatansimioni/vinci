# Vinci's AppRun hook. Sourced after linuxdeploy-plugin-gtk.sh (hooks run in name order).
#
# That hook forces GTK_THEME, which breaks libadwaita's styling and dark mode, and
# GDK_BACKEND=x11, a GTK 3 workaround that only blocks native Wayland in GTK 4.
unset GTK_THEME GTK_DATA_PREFIX
if [ -n "${WAYLAND_DISPLAY:-}" ]; then
    unset GDK_BACKEND
fi
