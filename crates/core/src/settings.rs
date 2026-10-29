//! User settings, in the host's `$XDG_CONFIG_HOME/vinci/settings.toml` (shared by the
//! Flatpak and AppImage builds). Unknown or missing keys fall back to defaults, so old
//! files keep working after upgrades.

use crate::gpu::Gpu;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// UI language (`pt_BR`, `en`); `None` follows the system.
    pub language: Option<String>,
    /// PCI slot of the GPU to run Resolve on; `None` picks automatically.
    pub preferred_gpu: Option<String>,
    /// Keep the last working installer for offline reinstalls (SPEC §12.5).
    pub keep_installer: bool,
    /// Save a restore point before updates (SPEC §12.4).
    pub keep_restore_point: bool,
    /// Convert in the background, with a tray icon (SPEC §9).
    pub background_service: bool,
    /// Watch folders; `None` disables each one.
    pub watch_import: Option<PathBuf>,
    pub watch_export: Option<PathBuf>,
    /// Keep a local activity log (`log::default_path`). Off unless the user turns it on.
    pub save_logs: bool,
    /// Closing the window leaves Vinci in the system tray instead of quitting.
    pub close_to_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: None,
            preferred_gpu: None,
            keep_installer: true,
            keep_restore_point: true,
            background_service: true,
            watch_import: None,
            watch_export: None,
            save_logs: false,
            close_to_tray: true,
        }
    }
}

/// Languages the UI ships, as (gettext code, native name).
pub const LANGUAGES: &[(&str, &str)] = &[("en", "English"), ("pt_BR", "Português (Brasil)")];

impl Settings {
    pub fn default_path() -> PathBuf {
        crate::state::host_xdg_dir("XDG_CONFIG_HOME", ".config").join("vinci/settings.toml")
    }

    /// Never fails: a missing or broken file means defaults (the app must always open).
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }

    /// The GPU to use: the user's choice if it is still present and usable, otherwise
    /// the automatic pick (`gpu::preferred`).
    pub fn choose_gpu<'a>(&self, gpus: &'a [Gpu]) -> Option<&'a Gpu> {
        self.preferred_gpu
            .as_deref()
            .and_then(|slot| {
                gpus.iter()
                    .find(|g| g.pci_slot.as_deref() == Some(slot) && g.is_usable())
            })
            .or_else(|| crate::gpu::preferred(gpus))
    }
}

/// Default watch folders under the user's Videos folder.
pub fn default_watch_dirs(videos: &Path) -> (PathBuf, PathBuf) {
    (videos.join("Vinci/Import"), videos.join("Vinci/Export"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::Vendor;

    fn gpu(vendor: Vendor, slot: &str, driver: &str, id: u16) -> Gpu {
        Gpu {
            vendor,
            device_id: id,
            driver: Some(driver.into()),
            pci_slot: Some(slot.into()),
            render_node: None,
            amd_gfx_target: None,
        }
    }

    #[test]
    fn roundtrip_and_defaults() {
        let path =
            std::env::temp_dir().join(format!("rli-settings-{}/settings.toml", std::process::id()));
        assert_eq!(Settings::load(&path), Settings::default());
        let s = Settings {
            language: Some("pt_BR".into()),
            keep_installer: false,
            watch_import: Some("/home/u/Videos/Vinci/Import".into()),
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        std::fs::write(&path, "language = \"en\"\nfuture_key = 1\n").unwrap();
        let partial = Settings::load(&path);
        std::fs::write(&path, "not toml [[[").unwrap();
        let broken = Settings::load(&path);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert_eq!(partial.language.as_deref(), Some("en"));
        assert!(partial.keep_restore_point, "missing keys use defaults");
        assert_eq!(
            broken,
            Settings::default(),
            "a broken file never blocks the app"
        );
    }

    #[test]
    fn honours_gpu_choice_only_when_valid() {
        let igpu = gpu(Vendor::Intel, "0000:00:02.0", "i915", 0x46a6);
        let nv = gpu(Vendor::Nvidia, "0000:01:00.0", "nvidia", 0x2571);
        let old = gpu(Vendor::Intel, "0000:00:03.0", "i915", 0x5917);
        let gpus = [igpu.clone(), nv.clone(), old];
        let mut s = Settings::default();
        assert_eq!(s.choose_gpu(&gpus), Some(&nv), "automatic: discrete first");
        s.preferred_gpu = Some("0000:00:02.0".into());
        assert_eq!(s.choose_gpu(&gpus), Some(&igpu));
        s.preferred_gpu = Some("0000:00:03.0".into());
        assert_eq!(
            s.choose_gpu(&gpus),
            Some(&nv),
            "unsupported choice falls back"
        );
        s.preferred_gpu = Some("0000:09:00.0".into());
        assert_eq!(s.choose_gpu(&gpus), Some(&nv), "removed GPU falls back");
    }

    #[test]
    fn watch_dirs_live_under_videos() {
        let (i, e) = default_watch_dirs(Path::new("/home/u/Vídeos"));
        assert_eq!(i, Path::new("/home/u/Vídeos/Vinci/Import"));
        assert_eq!(e, Path::new("/home/u/Vídeos/Vinci/Export"));
    }
}
