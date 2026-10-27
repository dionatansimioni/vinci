//! Host prerequisites: podman and distrobox. Detects what is there and plans what to add.

use crate::distro::{Distro, PackageManager};
use crate::host::{argv, Runner};
use crate::recipe::Recipe;
use crate::version::Version;
use std::path::{Path, PathBuf};

/// Our private distrobox prefix: `~/.local/share/vinci/distrobox`.
pub fn private_prefix(home: &Path) -> PathBuf {
    home.join(".local/share/vinci/distrobox")
}

/// `podman version 5.7.0` → 5.7.0
pub fn parse_podman_version(out: &str) -> Option<Version> {
    out.split_whitespace().last()?.parse().ok()
}

/// `distrobox: 1.8.2.5` → 1.8.2.5
pub fn parse_distrobox_version(out: &str) -> Option<Version> {
    out.lines()
        .find_map(|l| l.trim().strip_prefix("distrobox:"))?
        .trim()
        .parse()
        .ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DistroboxChoice {
    /// A compatible distrobox is on the host PATH.
    Host { version: Version },
    /// Our pinned copy is already installed.
    Private { path: PathBuf },
    /// Neither: install the pinned version into the private prefix.
    Install,
}

impl DistroboxChoice {
    /// The executable to call, if one is available.
    pub fn executable(&self) -> Option<String> {
        match self {
            Self::Host { .. } => Some("distrobox".into()),
            Self::Private { path } => Some(path.to_string_lossy().into_owned()),
            Self::Install => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostTools {
    pub podman: Option<Version>,
    pub distrobox: DistroboxChoice,
}

pub fn detect(runner: &dyn Runner, recipe: &Recipe, home: &Path) -> HostTools {
    let podman = runner
        .run(&argv(&["podman", "--version"]))
        .ok()
        .filter(|o| o.success)
        .and_then(|o| parse_podman_version(&o.stdout));

    let host_version = runner
        .run(&argv(&["distrobox", "version"]))
        .ok()
        .filter(|o| o.success)
        .and_then(|o| parse_distrobox_version(&o.stdout));
    let private_bin = private_prefix(home).join("bin/distrobox");
    let distrobox = match host_version {
        Some(v) if recipe.distrobox.host_compatible(&v) => DistroboxChoice::Host { version: v },
        _ if private_bin.exists() => DistroboxChoice::Private { path: private_bin },
        _ => DistroboxChoice::Install,
    };
    HostTools { podman, distrobox }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Run as root (the caller wraps with pkexec).
    InstallPodman {
        manager: PackageManager,
        argv: Vec<String>,
    },
    /// No package manager we know: show this command to the user.
    ManualPodman { hint: String },
    /// Runs as the user, no root needed.
    InstallDistrobox { argv: Vec<String>, prefix: PathBuf },
}

/// What has to happen on the host before containers can be created.
pub fn plan(tools: &HostTools, distro: &Distro, recipe: &Recipe, home: &Path) -> Vec<Action> {
    let mut actions = Vec::new();
    if tools.podman.is_none() {
        actions.push(match distro.package_manager() {
            Some(pm) => Action::InstallPodman {
                manager: pm,
                argv: pm.install_command(&["podman"]),
            },
            None => Action::ManualPodman {
                hint: "Install podman with your distribution's package manager, then reopen Vinci."
                    .into(),
            },
        });
    }
    if tools.distrobox == DistroboxChoice::Install {
        let prefix = private_prefix(home);
        // URL and prefix are positional arguments: never spliced into the script text.
        let script = r#"set -e; tmp=$(mktemp); trap 'rm -f "$tmp"' EXIT; if command -v curl >/dev/null; then curl -fsSL "$1" -o "$tmp"; else wget -qO "$tmp" "$1"; fi; sh "$tmp" --prefix "$2""#;
        actions.push(Action::InstallDistrobox {
            argv: argv(&[
                "sh",
                "-c",
                script,
                "sh",
                &recipe.distrobox.install_url,
                &prefix.to_string_lossy(),
            ]),
            prefix,
        });
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::fake::FakeRunner;

    fn ubuntu() -> Distro {
        Distro::parse("ID=ubuntu\nID_LIKE=debian\nPRETTY_NAME=\"Ubuntu\"\n")
    }

    #[test]
    fn parses_versions() {
        assert_eq!(
            parse_podman_version("podman version 5.7.0\n")
                .unwrap()
                .to_string(),
            "5.7.0"
        );
        assert_eq!(
            parse_distrobox_version("distrobox: 1.8.2.5\n")
                .unwrap()
                .to_string(),
            "1.8.2.5"
        );
        assert!(parse_distrobox_version("garbage").is_none());
    }

    #[test]
    fn uses_compatible_host_distrobox() {
        let r = FakeRunner::default()
            .respond(&["podman", "--version"], "podman version 5.7.0", true)
            .respond(&["distrobox", "version"], "distrobox: 1.8.1", true);
        let t = detect(&r, &Recipe::bundled(), Path::new("/nonexistent"));
        assert_eq!(t.podman.as_ref().unwrap().to_string(), "5.7.0");
        assert_eq!(t.distrobox.executable().as_deref(), Some("distrobox"));
        assert!(plan(&t, &ubuntu(), &Recipe::bundled(), Path::new("/h")).is_empty());
    }

    #[test]
    fn ignores_incompatible_host_distrobox_and_plans_install() {
        let r = FakeRunner::default().respond(&["distrobox", "version"], "distrobox: 2.0.0", true);
        let t = detect(&r, &Recipe::bundled(), Path::new("/nonexistent"));
        assert_eq!(t.podman, None);
        assert_eq!(t.distrobox, DistroboxChoice::Install);
        let actions = plan(&t, &ubuntu(), &Recipe::bundled(), Path::new("/home/u"));
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            &actions[0],
            Action::InstallPodman { manager: PackageManager::Apt, argv } if argv.last().unwrap() == "podman"
        ));
        let Action::InstallDistrobox { argv, prefix } = &actions[1] else {
            panic!()
        };
        assert_eq!(prefix, Path::new("/home/u/.local/share/vinci/distrobox"));
        assert_eq!(argv[4], Recipe::bundled().distrobox.install_url);
        assert_eq!(argv[5], "/home/u/.local/share/vinci/distrobox");
    }

    #[test]
    fn finds_private_copy() {
        let home = std::env::temp_dir().join(format!("rli-tools-{}", std::process::id()));
        let bin = private_prefix(&home).join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("distrobox"), "").unwrap();
        let t = detect(&FakeRunner::default(), &Recipe::bundled(), &home);
        std::fs::remove_dir_all(&home).unwrap();
        assert_eq!(
            t.distrobox,
            DistroboxChoice::Private {
                path: bin.join("distrobox")
            }
        );
    }

    #[test]
    fn unknown_distro_gets_manual_podman_hint() {
        let t = HostTools {
            podman: None,
            distrobox: DistroboxChoice::Host {
                version: "1.8.2".parse().unwrap(),
            },
        };
        let nix = Distro::parse("ID=nixos\n");
        let actions = plan(&t, &nix, &Recipe::bundled(), Path::new("/h"));
        assert!(matches!(actions.as_slice(), [Action::ManualPodman { .. }]));
    }

    /// Runs the generated wrapper against a fake installer served from a file:// URL,
    /// proving the URL and prefix reach the script intact.
    #[test]
    fn install_script_wrapper_runs() {
        let dir = std::env::temp_dir().join(format!("rli-dbx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("install");
        std::fs::write(&fake, "printf '%s\\n' \"$@\" > \"$RLI_ARGS_OUT\"\n").unwrap();
        let out = dir.join("args");
        let t = HostTools {
            podman: Some("5.0".parse().unwrap()),
            distrobox: DistroboxChoice::Install,
        };
        let mut recipe = Recipe::bundled();
        recipe.distrobox.install_url = format!("file://{}", fake.display());
        let home = dir.join("my home");
        let actions = plan(&t, &ubuntu(), &recipe, &home);
        let Action::InstallDistrobox { argv, .. } = &actions[0] else {
            panic!()
        };
        let ok = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .env("RLI_ARGS_OUT", &out)
            .status()
            .unwrap()
            .success();
        let written = std::fs::read_to_string(&out).unwrap_or_default();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(ok);
        let expected = format!("--prefix\n{}\n", private_prefix(&home).display());
        assert_eq!(
            written, expected,
            "prefix with a space must arrive as one argument"
        );
    }
}
