//! Uninstall: removes exactly what Vinci created (SPEC §3 and §7).
//!
//! Projects and the Resolve database are kept unless the user explicitly asks; podman
//! is only offered for removal when Vinci installed it and no other containers use it.

use crate::distro::Distro;
use crate::host::{argv, Runner};
use crate::installer_file::Edition;
use crate::integrate;
use crate::recipe::Recipe;
use crate::state::{InstalledTool, State, ToolSource};
use std::path::{Path, PathBuf};

/// Where Resolve keeps its database, project library and logs.
pub fn resolve_data_dir(home: &Path) -> PathBuf {
    home.join(".local/share/DaVinciResolve")
}

/// Vinci's own data: kept installers, private distrobox, caches.
pub fn vinci_dirs(home: &Path) -> [PathBuf; 2] {
    [home.join(".local/share/vinci"), home.join(".cache/vinci")]
}

/// Containers that are not ours (the user's own distroboxes, etc.).
pub fn foreign_containers(runner: &dyn Runner, recipe: &Recipe) -> usize {
    let prefix = format!("{}-", recipe.container.name_prefix);
    runner
        .run(&argv(&["podman", "ps", "--all", "--format", "{{.Names}}"]))
        .ok()
        .filter(|o| o.success)
        .map_or(0, |o| {
            o.stdout
                .lines()
                .map(str::trim)
                .filter(|n| !n.is_empty() && !n.starts_with(&prefix))
                .count()
        })
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Also delete ~/.local/share/DaVinciResolve. Off by default (SPEC §7).
    pub delete_projects: bool,
    /// Tools (by name) the user ticked for removal; only ones Vinci installed count.
    pub remove_tools: Vec<String>,
}

/// Where things live for this user.
pub struct Locations<'a> {
    pub home: &'a Path,
    pub data_home: &'a Path,
    pub state_path: &'a Path,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub removed_containers: Vec<String>,
    pub removed_tools: Vec<String>,
    /// Steps that failed but didn't stop the uninstall, for the report.
    pub warnings: Vec<String>,
}

/// Everything the uninstall screen needs to show before asking.
pub fn removable_tools(state: &State, runner: &dyn Runner, recipe: &Recipe) -> Vec<InstalledTool> {
    state
        .removable_tools(foreign_containers(runner, recipe))
        .into_iter()
        .cloned()
        .collect()
}

/// Removes tools Vinci installed: a private copy is deleted, a package goes through the
/// package manager as root. Returns the names removed and the names that failed; the
/// caller updates the state.
pub fn remove_tools(
    tools: &[InstalledTool],
    run_root: &dyn Fn(&[String]) -> bool,
    distro: &Distro,
) -> (Vec<String>, Vec<String>) {
    let (mut removed, mut failed) = (Vec::new(), Vec::new());
    for tool in tools {
        let ok = match &tool.source {
            ToolSource::UserLocal { prefix } => {
                std::fs::remove_dir_all(prefix).is_ok() || !prefix.exists()
            }
            ToolSource::PackageManager => distro
                .package_manager()
                .is_some_and(|pm| run_root(&pm.remove_command(&[&tool.name]))),
        };
        if ok { &mut removed } else { &mut failed }.push(tool.name.clone());
    }
    (removed, failed)
}

/// Removes one install while another stays: its container, its menu entry, and the
/// restore point when it holds this install. Tools, projects, USB rules and Vinci's own
/// folders are shared, so they stay; the last install goes through [`uninstall`].
pub fn remove_installation(
    runner: &dyn Runner,
    data_home: &Path,
    state_path: &Path,
    container: &str,
) -> Summary {
    let mut sum = Summary::default();
    let mut state = State::load(state_path).unwrap_or_default();
    match runner.run(&argv(&["podman", "rm", "--force", "--ignore", container])) {
        Ok(o) if o.success => sum.removed_containers.push(container.to_string()),
        _ => sum.warnings.push(format!("container {container}")),
    }
    let removed = state.remove_installation(container);
    if let Some(inst) = &removed {
        let last = state.installations.is_empty();
        if let Err(e) = integrate::remove(runner, data_home, inst.edition, container, last) {
            sum.warnings.push(format!("menu entry: {e}"));
        }
    }
    if state
        .backup
        .as_ref()
        .is_some_and(|b| b.container == container)
    {
        // Fails harmlessly while another container still runs from the image.
        let _ = runner.run(&argv(&[
            "podman",
            "image",
            "rm",
            "--ignore",
            crate::backup::IMAGE,
        ]));
        state.backup = None;
    }
    if let Err(e) = state.save(state_path) {
        sum.warnings.push(format!("state: {e}"));
    }
    sum
}

/// Runs the uninstall. Best effort: keeps going after a failed step and reports it.
/// `run_root` runs a command as root (pkexec), used only to remove podman.
pub fn uninstall(
    runner: &dyn Runner,
    run_root: &dyn Fn(&[String]) -> bool,
    recipe: &Recipe,
    distro: &Distro,
    at: &Locations,
    opts: &Options,
) -> Summary {
    let (home, data_home, state_path) = (at.home, at.data_home, at.state_path);
    let mut sum = Summary::default();
    let mut state = State::load(state_path).unwrap_or_default();
    let prefix = format!("{}-", recipe.container.name_prefix);

    // Every vinci-* container (the active one, a leftover Free/Studio, backups).
    let names: Vec<String> = runner
        .run(&argv(&["podman", "ps", "--all", "--format", "{{.Names}}"]))
        .ok()
        .map(|o| o.stdout.lines().map(|l| l.trim().to_string()).collect())
        .unwrap_or_default();
    for name in names.iter().filter(|n| n.starts_with(&prefix)) {
        match runner.run(&argv(&["podman", "rm", "--force", "--ignore", name])) {
            Ok(o) if o.success => sum.removed_containers.push(name.clone()),
            _ => sum.warnings.push(format!("container {name}")),
        }
    }
    let _ = runner.run(&argv(&[
        "podman",
        "image",
        "rm",
        "--force",
        "--ignore",
        crate::backup::IMAGE,
    ]));

    for edition in [Edition::Free, Edition::Studio] {
        let container = crate::container::container_name(recipe, edition);
        if let Err(e) = integrate::remove(runner, data_home, edition, &container, true) {
            sum.warnings.push(format!("menu entry: {e}"));
        }
    }
    if opts.delete_projects {
        let dir = resolve_data_dir(home);
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            if e.kind() != std::io::ErrorKind::NotFound {
                sum.warnings.push(format!("projects: {e}"));
            }
        }
    }

    let allowed = state.removable_tools(foreign_containers(runner, recipe));
    let to_remove: Vec<InstalledTool> = allowed
        .into_iter()
        .filter(|t| opts.remove_tools.contains(&t.name))
        .cloned()
        .collect();
    let (removed, failed) = remove_tools(&to_remove, run_root, distro);
    state.tools.retain(|t| !removed.contains(&t.name));
    sum.removed_tools = removed;
    sum.warnings
        .extend(failed.into_iter().map(|name| format!("tool {name}")));

    // Vinci's own folders go last: the private distrobox may live inside them.
    for dir in vinci_dirs(home) {
        let _ = std::fs::remove_dir_all(dir);
    }
    state.installations.clear();
    if !state.udev_rules.is_empty() {
        if run_root(&crate::udev::remove_command(&state.udev_rules)) {
            state.udev_rules.clear();
        } else {
            sum.warnings.push("udev rules".into());
        }
    }
    if state.tools.is_empty() {
        let _ = std::fs::remove_file(state_path);
    } else if let Err(e) = state.save(state_path) {
        sum.warnings.push(format!("state: {e}"));
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::fake::FakeRunner;
    use crate::installer_file::Edition;
    use crate::state::Installation;

    struct Home(PathBuf);
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup(name: &str) -> (Home, PathBuf) {
        let home = std::env::temp_dir().join(format!("rli-uninst-{}-{name}", std::process::id()));
        std::fs::create_dir_all(resolve_data_dir(&home).join("Resolve Project Library")).unwrap();
        let private = home.join(".local/share/vinci/distrobox/bin");
        std::fs::create_dir_all(&private).unwrap();
        std::fs::create_dir_all(home.join(".cache/vinci/recipe")).unwrap();
        let mut st = State::default();
        st.record_tool("podman", ToolSource::PackageManager);
        st.record_tool(
            "distrobox",
            ToolSource::UserLocal {
                prefix: home.join(".local/share/vinci/distrobox"),
            },
        );
        st.udev_rules = vec!["/etc/udev/rules.d/99-BlackmagicDevices.rules".into()];
        st.set_installation(Installation {
            container: "vinci-free".into(),
            edition: Edition::Free,
            version: "21.1.2".parse().unwrap(),
        });
        let state_path = home.join("state.json");
        st.save(&state_path).unwrap();
        (Home(home), state_path)
    }

    fn runner(containers: &str) -> FakeRunner {
        FakeRunner::default()
            .respond(&["podman", "ps"], containers, true)
            .respond(&[], "", true)
    }

    fn ubuntu() -> Distro {
        Distro::parse("ID=ubuntu\nID_LIKE=debian\n")
    }

    #[test]
    fn default_uninstall_keeps_projects_and_tools() {
        let (home, state_path) = setup("default");
        let r = runner("vinci-free\nvinci-studio\n");
        let sum = uninstall(
            &r,
            &|a: &[String]| {
                assert!(
                    a[2].contains("udev"),
                    "only the udev rules need root here: {a:?}"
                );
                true
            },
            &Recipe::bundled(),
            &ubuntu(),
            &Locations {
                home: &home.0,
                data_home: &home.0.join(".local/share"),
                state_path: &state_path,
            },
            &Options::default(),
        );
        assert_eq!(sum.removed_containers, ["vinci-free", "vinci-studio"]);
        assert!(sum.removed_tools.is_empty());
        assert!(
            resolve_data_dir(&home.0).exists(),
            "projects kept by default"
        );
        let st = State::load(&state_path).unwrap();
        assert!(st.installations.is_empty());
        assert_eq!(st.tools.len(), 2, "tool records kept for a later uninstall");
    }

    #[test]
    fn removing_one_of_two_installs_leaves_the_other_and_shared_things() {
        let (home, state_path) = setup("one");
        let mut st = State::load(&state_path).unwrap();
        let studio = Installation {
            container: "vinci-studio".into(),
            edition: Edition::Studio,
            version: "21.1.2".parse().unwrap(),
        };
        st.set_installation(studio.clone());
        st.backup = Some(studio);
        st.save(&state_path).unwrap();
        let data = home.0.join(".local/share");
        let entry = |c: &str| format!("X-Vinci-Container={c}\n");
        for (e, c) in [
            (Edition::Free, "vinci-free"),
            (Edition::Studio, "vinci-studio"),
        ] {
            integrate::install(&runner(""), &Recipe::bundled(), "d", c, e, &data, &entry(c))
                .unwrap();
        }
        let r = runner("");

        let sum = remove_installation(&r, &data, &state_path, "vinci-studio");
        assert_eq!(sum.removed_containers, ["vinci-studio"]);
        assert!(sum.warnings.is_empty(), "{:?}", sum.warnings);
        let calls = r.calls();
        assert!(!calls.iter().any(|c| c.contains(&"vinci-free".to_string())));
        assert!(calls.contains(&argv(&[
            "podman",
            "image",
            "rm",
            "--ignore",
            crate::backup::IMAGE
        ])));
        let st = State::load(&state_path).unwrap();
        assert_eq!(st.installations.len(), 1);
        assert_eq!(st.installations[0].container, "vinci-free");
        assert!(st.backup.is_none(), "its restore point went with it");
        assert_eq!(st.tools.len(), 2, "tools stay for the other install");
        let free = crate::launcher::paths(&data, Edition::Free);
        assert!(free.desktop.exists() && free.mime.exists());
        assert!(!crate::launcher::paths(&data, Edition::Studio)
            .desktop
            .exists());
        assert!(resolve_data_dir(&home.0).exists());
        assert!(home.0.join(".local/share/vinci").exists());
    }

    #[test]
    fn full_uninstall_removes_everything_we_made() {
        let (home, state_path) = setup("full");
        let r = runner("vinci-free\n");
        let roots = std::cell::RefCell::new(Vec::new());
        let sum = uninstall(
            &r,
            &|a: &[String]| {
                roots.borrow_mut().push(a.to_vec());
                true
            },
            &Recipe::bundled(),
            &ubuntu(),
            &Locations {
                home: &home.0,
                data_home: &home.0.join(".local/share"),
                state_path: &state_path,
            },
            &Options {
                delete_projects: true,
                remove_tools: vec!["podman".into(), "distrobox".into()],
            },
        );
        assert_eq!(sum.removed_tools, ["podman", "distrobox"]);
        assert_eq!(roots.borrow()[0], ["apt-get", "remove", "-y", "podman"]);
        assert!(
            roots.borrow()[1][2].contains("rm -f"),
            "udev rules removed as root"
        );
        assert!(!resolve_data_dir(&home.0).exists());
        assert!(!home.0.join(".local/share/vinci").exists());
        assert!(!state_path.exists(), "nothing left to remember");
        assert!(sum.warnings.is_empty(), "{:?}", sum.warnings);
    }

    #[test]
    fn keeps_podman_when_user_has_other_containers() {
        let (home, state_path) = setup("foreign");
        let r = runner("vinci-free\nmy-arch-box\n");
        let sum = uninstall(
            &r,
            &|a: &[String]| {
                assert!(
                    !a.contains(&"podman".to_string()),
                    "podman must not be removed"
                );
                true
            },
            &Recipe::bundled(),
            &ubuntu(),
            &Locations {
                home: &home.0,
                data_home: &home.0.join(".local/share"),
                state_path: &state_path,
            },
            &Options {
                delete_projects: false,
                remove_tools: vec!["podman".into(), "distrobox".into()],
            },
        );
        assert_eq!(
            sum.removed_containers,
            ["vinci-free"],
            "user's box untouched"
        );
        assert_eq!(sum.removed_tools, ["distrobox"]);
        let st = State::load(&state_path).unwrap();
        assert_eq!(st.tools.len(), 1);
        assert_eq!(st.tools[0].name, "podman");
    }

    #[test]
    fn counts_only_foreign_containers() {
        let r = runner("vinci-free\nfedora-box\nubuntu\n\n");
        assert_eq!(foreign_containers(&r, &Recipe::bundled()), 2);
    }
}
