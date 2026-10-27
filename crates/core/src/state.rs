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
    /// The active Resolve install. Only one exists at a time.
    #[serde(default)]
    pub installation: Option<Installation>,
    /// Studio dongle udev rule file we wrote, if any.
    #[serde(default)]
    pub udev_rule: Option<PathBuf>,
}

impl State {
    /// Default location: `$XDG_STATE_HOME/vinci/state.json`.
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home().join(".local/state"));
        base.join("vinci/state.json")
    }

    /// Loads the state; a missing file means a fresh system.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(std::io::Error::other),
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
        s.installation = Some(Installation {
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
