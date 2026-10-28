//! Making Vinci itself look right on any desktop: its icon in the taskbar and the window
//! buttons on KDE and other desktops that don't hand GTK a usable button layout.

use crate::APP_ID;
use relm4::gtk;
use std::path::Path;

const ICON: &[u8] = include_bytes!("../../../data/icons/io.github.dionatansimioni.Vinci.svg");
const DESKTOP_ENTRY: &str = include_str!("../../../data/io.github.dionatansimioni.Vinci.desktop");

/// Keeps the user's layout (side, app menu/icon) but makes sure minimize, maximize and
/// close are all there. GNOME defaults to close only, and some KDE setups give GTK no
/// buttons at all.
pub fn decoration_layout(current: &str) -> String {
    const BUTTONS: [&str; 3] = ["minimize", "maximize", "close"];
    let (left, right) = current.split_once(':').unwrap_or((current, ""));
    let keep = |side: &str| -> Vec<String> {
        side.split(',')
            .map(str::trim)
            .filter(|b| !b.is_empty() && !BUTTONS.contains(b))
            .map(String::from)
            .collect()
    };
    let has_close = |side: &str| side.split(',').any(|b| b.trim() == "close");
    let (mut l, mut r) = (keep(left), keep(right));
    if has_close(left) && !has_close(right) {
        // macOS-style layouts put close first.
        let mut buttons: Vec<String> = ["close", "minimize", "maximize"].map(String::from).into();
        buttons.append(&mut l);
        l = buttons;
    } else {
        r.extend(BUTTONS.map(String::from));
    }
    format!("{}:{}", l.join(","), r.join(","))
}

/// Applies [`decoration_layout`] now and whenever the desktop changes the setting.
pub fn force_window_buttons() {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    let apply = |s: &gtk::Settings| {
        let current = s.gtk_decoration_layout().map(|g| g.to_string());
        let wanted = decoration_layout(current.as_deref().unwrap_or(""));
        // Only on change, so our own write doesn't trigger another round.
        if current.as_deref() != Some(wanted.as_str()) {
            s.set_gtk_decoration_layout(Some(&wanted));
        }
    };
    apply(&settings);
    settings.connect_gtk_decoration_layout_notify(apply);
}

/// Quotes a path for the `Exec` key (Desktop Entry spec: double quotes, with `"`, `` ` ``,
/// `$` and `\` escaped, then `\` escaped again as a string value).
fn exec_quote(path: &str) -> String {
    let mut q = String::from("\"");
    for c in path.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            q.push('\\');
        }
        q.push(c);
    }
    q.push('"');
    q.replace('\\', "\\\\")
}

/// Vinci's own menu entry pointing at the AppImage. `TryExec` hides it once the file is
/// moved or deleted, until Vinci is opened again from the new place.
pub fn appimage_entry(appimage: &str) -> String {
    DESKTOP_ENTRY
        .lines()
        .map(|l| {
            if l == "Exec=vinci" {
                format!("TryExec={appimage}\nExec={}", exec_quote(appimage))
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// Installs the icon, and for the AppImage the menu entry, in the user's data dir.
/// Wayland taskbars find a window's icon through the `.desktop` file named after its app
/// ID, so without these the AppImage shows a generic icon. Flatpak installs both itself.
pub fn integrate(data_home: &Path, appimage: Option<&str>) -> std::io::Result<()> {
    let icon = data_home.join(format!("icons/hicolor/scalable/apps/{APP_ID}.svg"));
    write_if_changed(&icon, ICON)?;
    if let Some(path) = appimage {
        let entry = data_home.join(format!("applications/{APP_ID}.desktop"));
        write_if_changed(&entry, appimage_entry(path).as_bytes())?;
    }
    Ok(())
}

fn write_if_changed(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if std::fs::read(path).is_ok_and(|c| c == content) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_the_missing_window_buttons() {
        assert_eq!(
            decoration_layout("appmenu:close"),
            "appmenu:minimize,maximize,close"
        );
        assert_eq!(decoration_layout(""), ":minimize,maximize,close");
        assert_eq!(decoration_layout("icon:"), "icon:minimize,maximize,close");
        // Already complete: unchanged, so the notify handler stops there.
        let full = "icon:minimize,maximize,close";
        assert_eq!(decoration_layout(full), full);
        assert_eq!(
            decoration_layout(&decoration_layout("menu:close")),
            "menu:minimize,maximize,close"
        );
    }

    #[test]
    fn keeps_buttons_on_the_left_when_the_user_put_them_there() {
        assert_eq!(
            decoration_layout("close,minimize:appmenu"),
            "close,minimize,maximize:appmenu"
        );
    }

    #[test]
    fn appimage_entry_points_at_the_file() {
        let e = appimage_entry("/home/u/Apps/Vinci-0.2.0-x86_64.AppImage");
        assert!(e.contains("\nTryExec=/home/u/Apps/Vinci-0.2.0-x86_64.AppImage\n"));
        assert!(e.contains("\nExec=\"/home/u/Apps/Vinci-0.2.0-x86_64.AppImage\"\n"));
        assert!(!e.contains("Exec=vinci"));
        assert!(e.contains(&format!("Icon={APP_ID}\n")));
        // Shell-special characters can't break out of the quotes.
        assert!(appimage_entry("/a/$x\"y").contains("Exec=\"/a/\\\\$x\\\\\"y\""));
    }

    #[test]
    fn integrate_writes_icon_and_entry() {
        let dir = std::env::temp_dir().join(format!("vinci-desktop-{}", std::process::id()));
        integrate(&dir, Some("/opt/Vinci.AppImage")).unwrap();
        assert!(dir
            .join(format!("icons/hicolor/scalable/apps/{APP_ID}.svg"))
            .exists());
        let entry = std::fs::read_to_string(dir.join(format!("applications/{APP_ID}.desktop")));
        assert!(entry.unwrap().contains("/opt/Vinci.AppImage"));
        // Native builds: icon only.
        let native = dir.join("native");
        integrate(&native, None).unwrap();
        assert!(!native.join("applications").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
