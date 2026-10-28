//! Desktop integration on the host: menu entry, icon and file types. Also undoes it.

use crate::host::{argv, Runner};
use crate::launcher::{self, Paths};
use crate::recipe::Recipe;
use std::path::Path;

/// The host's `$XDG_DATA_HOME` (usually `~/.local/share`), where the desktop looks for
/// menu entries, icons and MIME types — also when Vinci runs as a Flatpak.
pub fn data_home(_home: &Path) -> std::path::PathBuf {
    crate::state::host_xdg_dir("XDG_DATA_HOME", ".local/share")
}

/// Writes the menu entry and MIME package, copies the icon out of the container, and
/// refreshes the desktop caches. Cache refresh failures are ignored: desktops pick the
/// files up on their own, just less promptly.
pub fn install(
    runner: &dyn Runner,
    recipe: &Recipe,
    distrobox: &str,
    container: &str,
    data_home: &Path,
    desktop_entry: &str,
) -> std::io::Result<Paths> {
    let paths = launcher::paths(data_home);
    for p in [&paths.desktop, &paths.mime, &paths.icon] {
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
    }
    std::fs::write(&paths.desktop, desktop_entry)?;
    std::fs::write(&paths.mime, launcher::mime_xml())?;

    if let Some(icon) = &recipe.launcher.icon {
        // The container shares $HOME, so it can write straight into the host icon dir.
        let src = format!("{}/{icon}", recipe.resolve.install_dir);
        let _ = runner.run(&argv(&[
            distrobox,
            "enter",
            "--name",
            container,
            "--",
            "install",
            "-m",
            "0644",
            &src,
            &paths.icon.to_string_lossy(),
        ]));
    }
    refresh_caches(runner, data_home);
    Ok(paths)
}

/// Removes everything [`install`] created. Missing files are fine.
pub fn remove(runner: &dyn Runner, data_home: &Path) -> std::io::Result<()> {
    let paths = launcher::paths(data_home);
    for p in [&paths.desktop, &paths.mime, &paths.icon] {
        match std::fs::remove_file(p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    refresh_caches(runner, data_home);
    Ok(())
}

fn refresh_caches(runner: &dyn Runner, data_home: &Path) {
    let apps = data_home.join("applications");
    let mime = data_home.join("mime");
    let _ = runner.run(&argv(&["update-desktop-database", &apps.to_string_lossy()]));
    let _ = runner.run(&argv(&["update-mime-database", &mime.to_string_lossy()]));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::fake::FakeRunner;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rli-integrate-{}-{name}", std::process::id()))
    }

    #[test]
    fn installs_and_removes_files() {
        let data = tmp("roundtrip");
        let runner = FakeRunner::default().respond(&[], "", true);
        let paths = install(
            &runner,
            &Recipe::bundled(),
            "distrobox",
            "vinci-free",
            &data,
            "[Desktop Entry]\n",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths.desktop).unwrap(),
            "[Desktop Entry]\n"
        );
        assert!(std::fs::read_to_string(&paths.mime)
            .unwrap()
            .contains("*.drp"));
        let calls = runner.calls();
        let copy = calls
            .iter()
            .find(|c| c.contains(&"install".to_string()))
            .unwrap();
        assert!(copy.contains(&"/opt/resolve/graphics/DV_Resolve.png".to_string()));
        assert!(copy.contains(&paths.icon.to_string_lossy().into_owned()));
        assert!(calls.iter().any(|c| c[0] == "update-desktop-database"));

        remove(&runner, &data).unwrap();
        assert!(!paths.desktop.exists() && !paths.mime.exists());
        remove(&runner, &data).unwrap();
        std::fs::remove_dir_all(&data).unwrap();
    }

    #[test]
    fn missing_cache_tools_are_not_fatal() {
        let data = tmp("notools");
        let r = install(
            &FakeRunner::default(),
            &Recipe::bundled(),
            "d",
            "c",
            &data,
            "x",
        );
        std::fs::remove_dir_all(&data).unwrap();
        assert!(r.is_ok());
    }

    #[test]
    fn real_cache_tools_accept_our_files() {
        // Only where desktop-file-utils and shared-mime-info exist.
        if std::process::Command::new("update-mime-database")
            .arg("-v")
            .output()
            .is_err()
        {
            return;
        }
        let data = tmp("realtools");
        let runner = crate::host::HostRunner {
            mode: crate::host::ExecMode::Direct,
            cancel: None,
        };
        install(&runner, &Recipe::bundled(), "/nonexistent/distrobox", "c", &data, "[Desktop Entry]\nType=Application\nName=X\nExec=true\nMimeType=application/x-resolveproj;\n").unwrap();
        let globs = std::fs::read_to_string(data.join("mime/globs2")).unwrap_or_default();
        std::fs::remove_dir_all(&data).unwrap();
        assert!(globs.contains("application/x-resolveproj:*.drp"), "{globs}");
    }
}
