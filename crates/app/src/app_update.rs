//! The banner offering a new version of Vinci itself (AppImage only), and what its button
//! does at each step: update in place, restart into the new version, or open the
//! releases page when updating in place isn't possible.

use crate::i18n::tr;
use relm4::adw;
use relm4::gtk::prelude::*;
use rli_core::app_update::{self, Release};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub enum AppUpdate {
    #[default]
    None,
    Available(Release),
    Downloading(Release),
    /// Installed at this path; restarting runs it.
    Ready(PathBuf),
    Failed(String),
}

/// What the banner button should do now.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Download,
    OpenReleases,
    Restart(PathBuf),
    Nothing,
}

/// The running AppImage, when this is one (not Flatpak, not a native build).
pub fn running_appimage() -> Option<PathBuf> {
    if Path::new("/.flatpak-info").exists() {
        return None;
    }
    std::env::var_os("APPIMAGE").map(PathBuf::from)
}

impl AppUpdate {
    pub fn action(&self, appimage: Option<&Path>) -> Action {
        match self {
            Self::Available(r)
                if r.appimage.is_some() && appimage.is_some_and(app_update::can_replace) =>
            {
                Action::Download
            }
            Self::Available(_) | Self::Failed(_) => Action::OpenReleases,
            Self::Ready(path) => Action::Restart(path.clone()),
            Self::None | Self::Downloading(_) => Action::Nothing,
        }
    }

    pub fn render(&self, banner: &adw::Banner, appimage: Option<&Path>) {
        let version = |r: &Release| r.version.to_string();
        let (title, button) = match self {
            Self::None => {
                banner.set_revealed(false);
                return;
            }
            Self::Available(r) => (
                tr("Vinci {version} is available").replace("{version}", &version(r)),
                Some(match self.action(appimage) {
                    Action::Download => tr("Update Vinci"),
                    _ => tr("Download"),
                }),
            ),
            Self::Downloading(r) => (
                tr("Downloading Vinci {version}…").replace("{version}", &version(r)),
                None,
            ),
            Self::Ready(_) => (
                tr("Vinci was updated. Restart it to use the new version."),
                Some(tr("Restart")),
            ),
            Self::Failed(e) => (
                format!("{} ({e})", tr("Vinci could not update itself.")),
                Some(tr("Download")),
            ),
        };
        banner.set_title(&title);
        banner.set_button_label(button.as_deref());
        banner.set_revealed(true);
    }
}

/// Downloads and installs `release` over `appimage`. Runs off the main thread.
pub fn install(release: &Release, appimage: &Path) -> Result<PathBuf, String> {
    let asset = release
        .appimage
        .as_ref()
        .ok_or("no AppImage in the release")?;
    let current: rli_core::version::Version = env!("CARGO_PKG_VERSION")
        .parse()
        .map_err(|e| format!("{e}"))?;
    let mut body = rli_core::remote::download(&asset.url)?;
    app_update::install(&mut body, appimage, &current, asset).map_err(|e| e.to_string())
}

/// Starts the new version once this process has quit (one app per session: started
/// earlier, it would just hand its window to this one).
pub fn restart(path: &Path) {
    let _ = std::process::Command::new("sh")
        .args(["-c", "sleep 1; exec \"$1\"", "sh"])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    relm4::main_application().quit();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rli_core::app_update::Asset;

    fn release(with_asset: bool) -> Release {
        Release {
            version: "9.0.0".parse().unwrap(),
            appimage: with_asset.then(|| Asset {
                name: "Vinci-9.0.0-x86_64.AppImage".into(),
                url: "https://github.com/x".into(),
                sha256: "0".repeat(64),
            }),
        }
    }

    #[test]
    fn button_updates_in_place_only_when_it_can() {
        let dir = std::env::temp_dir().join(format!("vinci-appupdate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Vinci.AppImage");
        std::fs::write(&file, "x").unwrap();
        let ok = Some(file.as_path());
        assert_eq!(
            AppUpdate::Available(release(true)).action(ok),
            Action::Download
        );
        // No checksum, not an AppImage, or a folder we can't write: the releases page.
        assert_eq!(
            AppUpdate::Available(release(false)).action(ok),
            Action::OpenReleases
        );
        assert_eq!(
            AppUpdate::Available(release(true)).action(None),
            Action::OpenReleases
        );
        let missing = dir.join("gone/Vinci.AppImage");
        assert_eq!(
            AppUpdate::Available(release(true)).action(Some(&missing)),
            Action::OpenReleases
        );
        assert_eq!(
            AppUpdate::Failed("x".into()).action(ok),
            Action::OpenReleases
        );
        assert_eq!(
            AppUpdate::Ready(file.clone()).action(ok),
            Action::Restart(file.clone())
        );
        assert_eq!(
            AppUpdate::Downloading(release(true)).action(ok),
            Action::Nothing
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
