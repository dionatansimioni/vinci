//! Desktop integration on the host: menu entry, icon and file types. Also undoes it.

use crate::host::{argv, Runner};
use crate::installer_file::Edition;
use crate::launcher::{self, Paths};
use crate::recipe::Recipe;
use std::path::{Path, PathBuf};

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
    edition: Edition,
    data_home: &Path,
    desktop_entry: &str,
) -> std::io::Result<Paths> {
    let paths = launcher::paths(data_home, edition);
    // A Studio install from Vinci 0.3 or older has its entry under the Free id.
    remove_legacy_entry(data_home, edition, container)?;
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

fn remove_if_present(p: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(p) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Deletes the entry under the Free id when it actually starts `container` of another
/// edition (written by Vinci 0.3 or older, which had one entry for whatever was installed).
fn remove_legacy_entry(data_home: &Path, edition: Edition, container: &str) -> std::io::Result<()> {
    if edition == Edition::Free {
        return Ok(());
    }
    let legacy = launcher::paths(data_home, Edition::Free);
    if launcher::entry_runs(&legacy.desktop, container) {
        remove_if_present(&legacy.desktop)?;
        remove_if_present(&legacy.icon)?;
    }
    Ok(())
}

/// Whether `container`'s menu entry still sits under the Free id (Vinci 0.3 or older).
pub fn has_legacy_entry(data_home: &Path, edition: Edition, container: &str) -> bool {
    edition != Edition::Free
        && launcher::entry_runs(
            &launcher::paths(data_home, Edition::Free).desktop,
            container,
        )
}

/// Removes what [`install`] created for one install. The file types go only with the
/// `last` one, since every install shares them. Missing files are fine.
pub fn remove(
    runner: &dyn Runner,
    data_home: &Path,
    edition: Edition,
    container: &str,
    last: bool,
) -> std::io::Result<()> {
    let paths = launcher::paths(data_home, edition);
    remove_if_present(&paths.desktop)?;
    remove_if_present(&paths.icon)?;
    remove_legacy_entry(data_home, edition, container)?;
    if last {
        remove_if_present(&paths.mime)?;
    }
    refresh_caches(runner, data_home);
    Ok(())
}

/// Shortcuts Blackmagic's installer script writes for Resolve and its utilities (RAW
/// Player, RAW Speed Test, Capture Logs, Panels Setup, its own uninstaller). The home
/// folder is shared with the container, so when they land there the host menu or desktop
/// shows them, and none of them work outside the container. Vinci only offers Resolve.
const BLACKMAGIC_ENTRIES: [&str; 7] = [
    "com.blackmagicdesign.resolve.desktop",
    "com.blackmagicdesign.resolve-Installer.desktop",
    "com.blackmagicdesign.resolve-CaptureLogs.desktop",
    "com.blackmagicdesign.resolve-Panels.desktop",
    "com.blackmagicdesign.resolve-DaVinciRemoteMonitoring.desktop",
    "com.blackmagicdesign.rawplayer.desktop",
    "com.blackmagicdesign.rawspeedtest.desktop",
];

fn blackmagic_shortcuts(home: &Path, data_home: &Path, config_home: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = BLACKMAGIC_ENTRIES
        .iter()
        .map(|e| data_home.join("applications").join(e))
        .collect();
    out.push(home.join("Desktop/com.blackmagicdesign.resolve.desktop"));
    out.push(config_home.join("menus/applications-merged/com.blackmagicdesign.resolve.menu"));
    out.push(config_home.join("menus/com.blackmagicdesign.resolve.menu"));
    out.push(data_home.join("desktop-directories/com.blackmagicdesign.resolve.directory"));
    out
}

/// Deletes the shortcuts of [`BLACKMAGIC_ENTRIES`] that point into the container (the
/// files mention `install_dir`). Left alone when Resolve is also installed natively on
/// the host, where the same files are that install's own. Returns what was deleted.
pub fn remove_blackmagic_shortcuts(
    runner: &dyn Runner,
    recipe: &Recipe,
    home: &Path,
    data_home: &Path,
    config_home: &Path,
) -> Vec<PathBuf> {
    let install_dir = &recipe.resolve.install_dir;
    let native = format!("{install_dir}/bin/resolve");
    if runner
        .run(&argv(&["test", "-e", &native]))
        .is_ok_and(|o| o.success)
    {
        return Vec::new();
    }
    let removed: Vec<PathBuf> = blackmagic_shortcuts(home, data_home, config_home)
        .into_iter()
        .filter(|p| {
            std::fs::read_to_string(p).is_ok_and(|text| {
                text.contains(install_dir.as_str()) || text.contains("com.blackmagicdesign")
            })
        })
        .filter(|p| std::fs::remove_file(p).is_ok())
        .collect();
    if !removed.is_empty() {
        crate::log::info(&format!(
            "removed {} shortcut(s) left by Blackmagic's installer",
            removed.len()
        ));
        refresh_caches(runner, data_home);
    }
    removed
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
            Edition::Free,
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

        remove(&runner, &data, Edition::Free, "vinci-free", true).unwrap();
        assert!(!paths.desktop.exists() && !paths.mime.exists());
        remove(&runner, &data, Edition::Free, "vinci-free", true).unwrap();
        std::fs::remove_dir_all(&data).unwrap();
    }

    #[test]
    fn free_and_studio_get_their_own_entries() {
        let data = tmp("both");
        let runner = FakeRunner::default().respond(&[], "", true);
        let r = Recipe::bundled();
        let entry = |c: &str| format!("[Desktop Entry]\nX-Vinci-Container={c}\n");
        let free = install(
            &runner,
            &r,
            "d",
            "vinci-free",
            Edition::Free,
            &data,
            &entry("vinci-free"),
        )
        .unwrap();
        let studio = install(
            &runner,
            &r,
            "d",
            "vinci-studio",
            Edition::Studio,
            &data,
            &entry("vinci-studio"),
        )
        .unwrap();
        assert!(free.desktop.exists() && studio.desktop.exists());
        assert_ne!(free.desktop, studio.desktop);

        // Removing one keeps the other and the shared file types.
        remove(&runner, &data, Edition::Studio, "vinci-studio", false).unwrap();
        assert!(!studio.desktop.exists());
        assert!(free.desktop.exists() && free.mime.exists());
        remove(&runner, &data, Edition::Free, "vinci-free", true).unwrap();
        assert!(!free.mime.exists());
        std::fs::remove_dir_all(&data).unwrap();
    }

    /// Vinci 0.3 wrote a Studio install's entry under the Free id.
    #[test]
    fn moves_an_older_studio_entry_to_its_own_id() {
        let data = tmp("legacy");
        let runner = FakeRunner::default().respond(&[], "", true);
        let legacy = launcher::paths(&data, Edition::Free);
        std::fs::create_dir_all(legacy.desktop.parent().unwrap()).unwrap();
        std::fs::write(&legacy.desktop, "X-Vinci-Container=vinci-studio\n").unwrap();
        assert!(has_legacy_entry(&data, Edition::Studio, "vinci-studio"));
        assert!(!has_legacy_entry(&data, Edition::Free, "vinci-free"));

        let entry = "[Desktop Entry]\nX-Vinci-Container=vinci-studio\n";
        let r = Recipe::bundled();
        let studio = install(
            &runner,
            &r,
            "d",
            "vinci-studio",
            Edition::Studio,
            &data,
            entry,
        )
        .unwrap();
        assert!(studio.desktop.exists());
        assert!(
            !legacy.desktop.exists(),
            "no second Studio entry under the Free id"
        );
        assert!(!has_legacy_entry(&data, Edition::Studio, "vinci-studio"));

        // A Free entry under that id is the Free install's own: never touched.
        std::fs::write(&legacy.desktop, "X-Vinci-Container=vinci-free\n").unwrap();
        install(
            &runner,
            &r,
            "d",
            "vinci-studio",
            Edition::Studio,
            &data,
            entry,
        )
        .unwrap();
        assert!(legacy.desktop.exists());
        std::fs::remove_dir_all(&data).unwrap();
    }

    #[test]
    fn removes_only_blackmagic_shortcuts_that_point_into_the_container() {
        let root = tmp("bmd");
        let (home, data, config) = (root.join("home"), root.join("data"), root.join("config"));
        let apps = data.join("applications");
        for d in [&apps, &home.join("Desktop"), &config.join("menus")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let entry = "[Desktop Entry]\nExec=/opt/resolve/bin/BlackmagicRAWSpeedTest\n";
        std::fs::write(
            apps.join("com.blackmagicdesign.rawspeedtest.desktop"),
            entry,
        )
        .unwrap();
        std::fs::write(
            home.join("Desktop/com.blackmagicdesign.resolve.desktop"),
            "Exec=/opt/resolve/bin/resolve\n",
        )
        .unwrap();
        std::fs::write(
            config.join("menus/com.blackmagicdesign.resolve.menu"),
            "<Filename>com.blackmagicdesign.resolve.desktop</Filename>",
        )
        .unwrap();
        // Same name, unrelated content: not ours to delete.
        let other = apps.join("com.blackmagicdesign.rawplayer.desktop");
        std::fs::write(&other, "Exec=/usr/bin/rawplayer\n").unwrap();
        let ours = apps.join("io.github.dionatansimioni.Vinci.Resolve.desktop");
        std::fs::write(&ours, entry).unwrap();

        // Resolve installed natively on the host: its shortcuts are real, keep them all.
        let native = FakeRunner::default().respond(&["test"], "", true);
        let r = Recipe::bundled();
        assert!(remove_blackmagic_shortcuts(&native, &r, &home, &data, &config).is_empty());

        let runner = FakeRunner::default().respond(&["test"], "", false);
        let removed = remove_blackmagic_shortcuts(&runner, &r, &home, &data, &config);
        assert_eq!(removed.len(), 3, "{removed:?}");
        assert!(other.exists() && ours.exists());
        assert!(runner
            .calls()
            .iter()
            .any(|c| c[0] == "update-desktop-database"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_cache_tools_are_not_fatal() {
        let data = tmp("notools");
        let r = install(
            &FakeRunner::default(),
            &Recipe::bundled(),
            "d",
            "c",
            Edition::Free,
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
        install(&runner, &Recipe::bundled(), "/nonexistent/distrobox", "c", Edition::Free, &data, "[Desktop Entry]\nType=Application\nName=X\nExec=true\nMimeType=application/x-resolveproj;\n").unwrap();
        let globs = std::fs::read_to_string(data.join("mime/globs2")).unwrap_or_default();
        std::fs::remove_dir_all(&data).unwrap();
        assert!(globs.contains("application/x-resolveproj:*.drp"), "{globs}");
    }
}
