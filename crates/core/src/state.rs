//! Persistent record of what the app changed on the system, so uninstall can undo
//! exactly that and nothing else.

use crate::installer_file::Edition;
use crate::version::Version;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    /// Installed through the distro package manager with pkexec.
    PackageManager,
    /// Installed without root under `~/.local`.
    UserLocal { prefix: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledTool {
    /// `podman` or `distrobox`
    pub name: String,
    pub source: ToolSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installation {
    pub container: String,
    pub edition: Edition,
    pub version: Version,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub tools: Vec<InstalledTool>,
    /// Every Resolve install, one container each (Free and Studio can live side by side).
    #[serde(default)]
    pub installations: Vec<Installation>,
    /// Vinci 0.3 and older kept a single install here; [`State::load`] moves it over.
    #[serde(default, rename = "installation", skip_serializing)]
    pub(crate) legacy_installation: Option<Installation>,
    /// What the restore point (`backup::IMAGE`) contains, if one exists.
    #[serde(default)]
    pub backup: Option<Installation>,
    /// udev rules Vinci copied to the host (Studio dongle, panels), for uninstall.
    #[serde(default)]
    pub udev_rules: Vec<PathBuf>,
}

impl State {
    /// Default location: `$XDG_STATE_HOME/vinci/state.json` of the host, so the Flatpak and
    /// AppImage builds share it.
    pub fn default_path() -> PathBuf {
        host_xdg_dir("XDG_STATE_HOME", ".local/state").join("vinci/state.json")
    }

    /// Loads the state; a missing file means a fresh system.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut state: Self = serde_json::from_str(&text).map_err(std::io::Error::other)?;
                if let Some(old) = state.legacy_installation.take() {
                    state.set_installation(old);
                }
                Ok(state)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Writes atomically (temp file + rename) so a crash never leaves half a file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }

    pub fn installation(&self, container: &str) -> Option<&Installation> {
        self.installations.iter().find(|i| i.container == container)
    }

    /// Adds `inst`, or replaces the install living in the same container.
    pub fn set_installation(&mut self, inst: Installation) {
        match self
            .installations
            .iter_mut()
            .find(|i| i.container == inst.container)
        {
            Some(slot) => *slot = inst,
            None => {
                self.installations.push(inst);
                // Free before Studio, whatever order they were installed in.
                self.installations
                    .sort_by_key(|i| i.edition == Edition::Studio);
            }
        }
    }

    pub fn remove_installation(&mut self, container: &str) -> Option<Installation> {
        let i = self
            .installations
            .iter()
            .position(|i| i.container == container)?;
        Some(self.installations.remove(i))
    }

    pub fn record_tool(&mut self, name: &str, source: ToolSource) {
        if !self.installed_by_us(name) {
            self.tools.push(InstalledTool {
                name: name.into(),
                source,
            });
        }
    }

    pub fn installed_by_us(&self, name: &str) -> bool {
        self.tools.iter().any(|t| t.name == name)
    }

    /// Tools the uninstaller may offer to remove. Podman is kept when the user has
    /// containers that aren't ours.
    pub fn removable_tools(&self, foreign_containers: usize) -> Vec<&InstalledTool> {
        self.tools
            .iter()
            .filter(|t| !(t.name == "podman" && foreign_containers > 0))
            .collect()
    }
}

/// An XDG base directory *of the host*. Inside Flatpak, `XDG_*_HOME` point into the
/// app's private `~/.var/app/<id>/`, which the desktop never reads; Flatpak exposes the
/// real ones as `HOST_XDG_*_HOME`.
pub fn host_xdg_dir(var: &str, default_rel: &str) -> PathBuf {
    resolve_xdg_dir(
        var,
        default_rel,
        &home(),
        std::path::Path::new("/.flatpak-info").exists(),
        |k| std::env::var_os(k),
    )
}

fn resolve_xdg_dir(
    var: &str,
    default_rel: &str,
    home: &std::path::Path,
    in_flatpak: bool,
    get: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> PathBuf {
    let absolute = |v: Option<std::ffi::OsString>| v.map(PathBuf::from).filter(|p| p.is_absolute());
    if let Some(p) = absolute(get(&format!("HOST_{var}"))) {
        return p;
    }
    if !in_flatpak {
        if let Some(p) = absolute(get(var)) {
            return p;
        }
    }
    home.join(default_rel)
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rli-state-{}-{name}/state.json",
            std::process::id()
        ))
    }

    #[test]
    fn xdg_dirs_point_at_the_host_even_inside_flatpak() {
        let home = std::path::Path::new("/home/u");
        fn env<'a>(
            pairs: &'a [(&'a str, &'a str)],
        ) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
            move |k| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).into())
            }
        }
        let sandboxed = [("XDG_DATA_HOME", "/home/u/.var/app/io.x/data")];
        // Flatpak without HOST_ vars: ignore the sandbox path, use the host default.
        assert_eq!(
            resolve_xdg_dir("XDG_DATA_HOME", ".local/share", home, true, env(&sandboxed)),
            PathBuf::from("/home/u/.local/share")
        );
        // Flatpak with HOST_ vars: use them.
        let with_host = [
            ("XDG_DATA_HOME", "/home/u/.var/app/io.x/data"),
            ("HOST_XDG_DATA_HOME", "/data/u"),
        ];
        assert_eq!(
            resolve_xdg_dir("XDG_DATA_HOME", ".local/share", home, true, env(&with_host)),
            PathBuf::from("/data/u")
        );
        // Native: honour XDG_*, ignore relative values.
        let native = [("XDG_STATE_HOME", "/st")];
        assert_eq!(
            resolve_xdg_dir("XDG_STATE_HOME", ".local/state", home, false, env(&native)),
            PathBuf::from("/st")
        );
        let relative = [("XDG_STATE_HOME", "rel")];
        assert_eq!(
            resolve_xdg_dir(
                "XDG_STATE_HOME",
                ".local/state",
                home,
                false,
                env(&relative)
            ),
            PathBuf::from("/home/u/.local/state")
        );
    }

    #[test]
    fn missing_file_is_empty_state() {
        assert_eq!(
            State::load(Path::new("/nonexistent/rli/state.json")).unwrap(),
            State::default()
        );
    }

    #[test]
    fn roundtrips_through_disk() {
        let path = tmp("roundtrip");
        let mut s = State::default();
        s.record_tool("podman", ToolSource::PackageManager);
        s.record_tool("podman", ToolSource::PackageManager);
        s.set_installation(Installation {
            container: "resolve-studio".into(),
            edition: Edition::Studio,
            version: "20.2.1".parse().unwrap(),
        });
        s.save(&path).unwrap();
        let loaded = State::load(&path).unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert_eq!(loaded, s);
        assert_eq!(loaded.tools.len(), 1, "duplicates are not recorded");
    }

    fn inst(container: &str, edition: Edition, version: &str) -> Installation {
        Installation {
            container: container.into(),
            edition,
            version: version.parse().unwrap(),
        }
    }

    #[test]
    fn keeps_free_and_studio_side_by_side() {
        let mut s = State::default();
        s.set_installation(inst("vinci-studio", Edition::Studio, "21.1.2"));
        s.set_installation(inst("vinci-free", Edition::Free, "21.1.2"));
        s.set_installation(inst("vinci-studio", Edition::Studio, "21.2"));
        let names: Vec<&str> = s
            .installations
            .iter()
            .map(|i| i.container.as_str())
            .collect();
        assert_eq!(
            names,
            ["vinci-free", "vinci-studio"],
            "Free first, no duplicate"
        );
        assert_eq!(
            s.installation("vinci-studio").unwrap().version.to_string(),
            "21.2"
        );
        assert_eq!(
            s.remove_installation("vinci-free").unwrap().edition,
            Edition::Free
        );
        assert!(s.remove_installation("vinci-free").is_none());
        assert_eq!(s.installations.len(), 1);
    }

    /// state.json written by Vinci 0.3 and older had a single `installation`.
    #[test]
    fn reads_the_single_install_of_older_versions() {
        let path = tmp("legacy");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"tools":[],"installation":{"container":"vinci-studio","edition":"Studio","version":"21.1.2"},"backup":null}"#,
        )
        .unwrap();
        let s = State::load(&path).unwrap();
        assert_eq!(
            s.installations,
            [inst("vinci-studio", Edition::Studio, "21.1.2")]
        );
        s.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert!(!text.contains("\"installation\""), "{text}");
        assert!(text.contains("\"installations\""));
    }

    #[test]
    fn keeps_podman_when_user_has_other_containers() {
        let mut s = State::default();
        s.record_tool("podman", ToolSource::PackageManager);
        s.record_tool(
            "distrobox",
            ToolSource::UserLocal {
                prefix: "/home/u/.local".into(),
            },
        );
        assert_eq!(s.removable_tools(0).len(), 2);
        let names: Vec<_> = s
            .removable_tools(2)
            .iter()
            .map(|t| t.name.clone())
            .collect();
        assert_eq!(names, ["distrobox"]);
    }

    #[test]
    fn tool_we_did_not_install_is_never_removable() {
        let s = State::default();
        assert!(!s.installed_by_us("podman"));
        assert!(s.removable_tools(0).is_empty());
    }

    #[test]
    fn corrupt_file_is_an_error() {
        let path = tmp("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        let r = State::load(&path);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert!(r.is_err());
    }
}
