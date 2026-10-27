//! Host distro detection from `/etc/os-release` and the matching package manager.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Apt,
    Dnf,
    Pacman,
    Zypper,
    /// Fedora Atomic family (Silverblue, Kinoite, Bazzite…). Layering needs a reboot.
    RpmOstree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distro {
    pub id: String,
    pub id_like: Vec<String>,
    pub pretty_name: String,
    pub version_id: Option<String>,
    pub variant_id: Option<String>,
}

/// Distros whose root filesystem is read-only and that ship podman out of the box.
const IMMUTABLE_IDS: &[&str] = &["bazzite", "aurora", "bluefin", "steamos", "endless"];
const ATOMIC_VARIANTS: &[&str] = &["silverblue", "kinoite", "sericea", "onyx", "cosmic-atomic"];

impl Distro {
    pub fn parse(os_release: &str) -> Self {
        let kv: HashMap<&str, String> = os_release
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim(), unquote(v.trim())))
            .collect();
        let get = |k: &str| kv.get(k).cloned().filter(|s| !s.is_empty());
        let id = get("ID").unwrap_or_else(|| "linux".into()).to_lowercase();
        Self {
            pretty_name: get("PRETTY_NAME")
                .or_else(|| get("NAME"))
                .unwrap_or_else(|| id.clone()),
            id_like: get("ID_LIKE")
                .map(|s| s.split_whitespace().map(str::to_lowercase).collect())
                .unwrap_or_default(),
            version_id: get("VERSION_ID"),
            variant_id: get("VARIANT_ID").map(|s| s.to_lowercase()),
            id,
        }
    }

    pub fn detect() -> std::io::Result<Self> {
        let text = std::fs::read_to_string("/etc/os-release")
            .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))?;
        Ok(Self::parse(&text))
    }

    fn is(&self, id: &str) -> bool {
        self.id == id || self.id_like.iter().any(|l| l == id)
    }

    pub fn is_immutable(&self) -> bool {
        IMMUTABLE_IDS.contains(&self.id.as_str())
            || self
                .variant_id
                .as_deref()
                .is_some_and(|v| ATOMIC_VARIANTS.contains(&v))
    }

    /// The package manager we know how to drive here, or `None` (show manual instructions).
    pub fn package_manager(&self) -> Option<PackageManager> {
        if self.id == "steamos" {
            return None; // read-only, no supported layering; podman ships preinstalled
        }
        if self.is_immutable() {
            return Some(PackageManager::RpmOstree);
        }
        if self.is("debian") || self.is("ubuntu") {
            Some(PackageManager::Apt)
        } else if self.is("fedora") || self.is("rhel") || self.is("centos") {
            Some(PackageManager::Dnf)
        } else if self.is("arch") {
            Some(PackageManager::Pacman)
        } else if self.is("suse") || self.id.starts_with("opensuse") {
            Some(PackageManager::Zypper)
        } else {
            None
        }
    }
}

impl PackageManager {
    /// Command (run as root via pkexec) that installs `packages`.
    pub fn install_command(self, packages: &[&str]) -> Vec<String> {
        let base: &[&str] = match self {
            Self::Apt => &["apt-get", "install", "-y"],
            Self::Dnf => &["dnf", "install", "-y"],
            Self::Pacman => &["pacman", "-S", "--needed", "--noconfirm"],
            Self::Zypper => &["zypper", "--non-interactive", "install"],
            Self::RpmOstree => &["rpm-ostree", "install", "--idempotent"],
        };
        base.iter().chain(packages).map(|s| s.to_string()).collect()
    }

    pub fn remove_command(self, packages: &[&str]) -> Vec<String> {
        let base: &[&str] = match self {
            Self::Apt => &["apt-get", "remove", "-y"],
            Self::Dnf => &["dnf", "remove", "-y"],
            Self::Pacman => &["pacman", "-R", "--noconfirm"],
            Self::Zypper => &["zypper", "--non-interactive", "remove"],
            Self::RpmOstree => &["rpm-ostree", "uninstall"],
        };
        base.iter().chain(packages).map(|s| s.to_string()).collect()
    }

    pub fn needs_reboot(self) -> bool {
        self == Self::RpmOstree
    }
}

fn unquote(v: &str) -> String {
    v.trim_matches(|c| c == '"' || c == '\'').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Distro {
        let path = format!(
            "{}/tests/fixtures/os-release/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        Distro::parse(&std::fs::read_to_string(path).unwrap())
    }

    #[test]
    fn maps_distros_to_package_managers() {
        let cases = [
            ("ubuntu", Some(PackageManager::Apt)),
            ("linuxmint", Some(PackageManager::Apt)),
            ("debian", Some(PackageManager::Apt)),
            ("fedora", Some(PackageManager::Dnf)),
            ("arch", Some(PackageManager::Pacman)),
            ("manjaro", Some(PackageManager::Pacman)),
            ("opensuse-tumbleweed", Some(PackageManager::Zypper)),
            ("silverblue", Some(PackageManager::RpmOstree)),
            ("bazzite", Some(PackageManager::RpmOstree)),
            ("steamos", None),
            ("nixos", None),
        ];
        for (name, expected) in cases {
            assert_eq!(fixture(name).package_manager(), expected, "{name}");
        }
    }

    #[test]
    fn detects_immutable() {
        assert!(fixture("silverblue").is_immutable());
        assert!(fixture("bazzite").is_immutable());
        assert!(!fixture("fedora").is_immutable());
    }

    #[test]
    fn parses_fields() {
        let d = fixture("linuxmint");
        assert_eq!(d.id, "linuxmint");
        assert_eq!(d.id_like, vec!["ubuntu", "debian"]);
        assert_eq!(d.pretty_name, "Linux Mint 22.2");
    }

    #[test]
    fn builds_install_commands() {
        assert_eq!(
            PackageManager::Apt.install_command(&["podman"]),
            ["apt-get", "install", "-y", "podman"]
        );
        assert_eq!(
            PackageManager::Pacman.remove_command(&["podman"]),
            ["pacman", "-R", "--noconfirm", "podman"]
        );
        assert!(PackageManager::RpmOstree.needs_reboot());
    }

    #[test]
    fn tolerates_empty_input() {
        let d = Distro::parse("");
        assert_eq!(d.id, "linux");
        assert_eq!(d.package_manager(), None);
    }
}
