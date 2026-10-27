//! Turns a recipe into the ordered commands that build a Resolve container.
//!
//! Nothing here runs anything: each [`Step`] carries an argv for the host runner, so
//! the plan is testable and the UI can show progress step by step.

use crate::gpu::Vendor;
use crate::installer_file::Edition;
use crate::recipe::{Recipe, VendorSetup};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    PullImage,
    /// Saves the current container as the restore point before it is replaced.
    Snapshot,
    /// Drops an earlier container with the same name (reinstall/update). Runs after the
    /// pull so a network failure never destroys a working install.
    RemoveExisting,
    Create,
    /// First entry runs distrobox's init (users, sudo); wait for it before using sudo.
    Initialize,
    AddRepository,
    InstallPackages,
    ExtractInstaller,
    InstallResolve,
    /// The installer exits 0 even on abort, so check the binary actually landed.
    Verify,
    RemoveConflictingLibs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub kind: StepKind,
    pub argv: Vec<String>,
}

/// Everything the plan depends on besides the recipe.
#[derive(Debug, Clone)]
pub struct Target<'a> {
    pub edition: Edition,
    pub vendor: Vendor,
    /// The distrobox executable (host copy or our pinned one).
    pub distrobox: &'a str,
    /// The `.zip` or `.run` from Blackmagic, somewhere under `$HOME` (shared with the container).
    pub installer: &'a Path,
    /// Scratch directory under `$HOME` where the zip is extracted.
    pub work_dir: &'a Path,
    /// Keep the container being replaced as a restore point.
    pub snapshot: bool,
}

pub fn container_name(recipe: &Recipe, edition: Edition) -> String {
    let edition = match edition {
        Edition::Free => "free",
        Edition::Studio => "studio",
    };
    format!("{}-{edition}", recipe.container.name_prefix)
}

fn vendor_setup(recipe: &Recipe, vendor: Vendor) -> Option<&VendorSetup> {
    match vendor {
        Vendor::Nvidia => Some(&recipe.gpu.nvidia),
        Vendor::Amd => Some(&recipe.gpu.amd),
        Vendor::Intel => Some(&recipe.gpu.intel),
        Vendor::Other(_) => None,
    }
}

fn s(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

/// Wraps a command to run as the user inside the container.
fn in_box_user(t: &Target, name: &str, cmd: Vec<String>) -> Vec<String> {
    let mut argv = s(&[t.distrobox, "enter", "--name", name, "--"]);
    argv.extend(cmd);
    argv
}

/// Wraps a command to run as root inside the container.
fn in_box(t: &Target, name: &str, cmd: Vec<String>) -> Vec<String> {
    let mut sudo = vec!["sudo".to_string()];
    sudo.extend(cmd);
    in_box_user(t, name, sudo)
}

/// Full install plan, in order.
pub fn install_plan(recipe: &Recipe, t: &Target) -> Vec<Step> {
    let name = container_name(recipe, t.edition);
    let setup = vendor_setup(recipe, t.vendor);
    let mut steps = vec![Step {
        kind: StepKind::PullImage,
        argv: s(&["podman", "pull", &recipe.container.image]),
    }];

    if t.snapshot {
        steps.push(crate::backup::snapshot_step(&name));
    }
    steps.push(Step {
        kind: StepKind::RemoveExisting,
        // --ignore: succeeds when there is nothing to remove.
        argv: s(&["podman", "rm", "--force", "--ignore", &name]),
    });

    let mut create = s(&[
        t.distrobox,
        "create",
        "--yes",
        "--name",
        &name,
        "--image",
        &recipe.container.image,
    ]);
    if t.vendor == Vendor::Nvidia {
        create.push("--nvidia".into());
    }
    steps.push(Step {
        kind: StepKind::Create,
        argv: create,
    });
    steps.push(Step {
        kind: StepKind::Initialize,
        argv: in_box_user(t, &name, s(&["sudo", "-n", "true"])),
    });

    if let Some(repo) = setup.and_then(|v| v.apt_repo.as_ref()) {
        // Fields are validated by the recipe parser; the source line is passed as a
        // positional argument so the shell never interprets it.
        let script = r#"set -e; install -d -m 0755 /etc/apt/keyrings; curl -fsSL "$1" -o "$2"; printf '%s\n' "$3" > "/etc/apt/sources.list.d/$4.list""#;
        steps.push(Step {
            kind: StepKind::AddRepository,
            argv: in_box(
                t,
                &name,
                s(&[
                    "sh",
                    "-c",
                    script,
                    "sh",
                    &repo.key_url,
                    &repo.keyring_path(),
                    &repo.source_line(),
                    &repo.name,
                ]),
            ),
        });
    }

    let packages: Vec<&str> = recipe
        .packages
        .base
        .iter()
        .chain(setup.into_iter().flat_map(|v| &v.packages))
        .map(String::as_str)
        .collect();
    let mut install = s(&[
        "sh",
        "-c",
        r#"set -e; export DEBIAN_FRONTEND=noninteractive; apt-get update --error-on=any; apt-get install -y --no-install-recommends "$@""#,
        "sh",
    ]);
    install.extend(packages.iter().map(|p| p.to_string()));
    steps.push(Step {
        kind: StepKind::InstallPackages,
        argv: in_box(t, &name, install),
    });

    let run_file = if t.installer.extension().is_some_and(|e| e == "zip") {
        steps.push(Step {
            kind: StepKind::ExtractInstaller,
            // Inside the box: the host may lack unzip (or be a Flatpak sandbox).
            argv: in_box_user(
                t,
                &name,
                // unzip -d does not create missing parents (~/.cache/vinci on a fresh system).
                s(&[
                    "sh",
                    "-c",
                    r#"set -e; mkdir -p "$2"; unzip -o -q "$1" -d "$2""#,
                    "sh",
                    &t.installer.to_string_lossy(),
                    &t.work_dir.to_string_lossy(),
                ]),
            ),
        });
        let stem = t
            .installer
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        t.work_dir.join(format!("{stem}.run"))
    } else {
        t.installer.to_path_buf()
    };

    let mut run = vec!["env".to_string()];
    run.extend(
        recipe
            .resolve
            .installer_env
            .iter()
            .map(|(k, v)| format!("{k}={v}")),
    );
    run.push(run_file.to_string_lossy().into_owned());
    run.extend(recipe.resolve.installer_args.iter().cloned());
    steps.push(Step {
        kind: StepKind::InstallResolve,
        argv: in_box(t, &name, run),
    });
    steps.push(Step {
        kind: StepKind::Verify,
        argv: in_box_user(
            t,
            &name,
            s(&[
                "test",
                "-x",
                &format!("{}/bin/resolve", recipe.resolve.install_dir),
            ]),
        ),
    });

    if !recipe.resolve.remove_libs.is_empty() {
        // Patterns are validated (relative, no "..", no whitespace); `$2..` stay unquoted
        // on purpose so the shell expands the globs, which never runs substitutions.
        let mut rm = s(&[
            "sh",
            "-c",
            r#"set -e; cd "$1"; shift; for p in "$@"; do rm -f -- $p; done"#,
            "sh",
            &recipe.resolve.install_dir,
        ]);
        rm.extend(recipe.resolve.remove_libs.iter().cloned());
        steps.push(Step {
            kind: StepKind::RemoveConflictingLibs,
            argv: in_box(t, &name, rm),
        });
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn target(vendor: Vendor, installer: &Path) -> Target<'_> {
        Target {
            edition: Edition::Free,
            vendor,
            distrobox: "distrobox",
            installer,
            work_dir: Path::new("/home/u/.cache/vinci/work"),
            snapshot: false,
        }
    }

    fn argv_of(steps: &[Step], kind: StepKind) -> &[String] {
        &steps
            .iter()
            .find(|s| s.kind == kind)
            .unwrap_or_else(|| panic!("no {kind:?} step"))
            .argv
    }

    fn kinds(steps: &[Step]) -> Vec<StepKind> {
        steps.iter().map(|s| s.kind).collect()
    }

    fn zip() -> PathBuf {
        PathBuf::from("/home/u/Downloads/DaVinci_Resolve_21.1.2_Linux.zip")
    }

    #[test]
    fn names_containers_by_edition() {
        let r = Recipe::bundled();
        assert_eq!(container_name(&r, Edition::Free), "vinci-free");
        assert_eq!(container_name(&r, Edition::Studio), "vinci-studio");
    }

    #[test]
    fn nvidia_plan() {
        let r = Recipe::bundled();
        let zip = zip();
        let plan = install_plan(&r, &target(Vendor::Nvidia, &zip));
        use StepKind::*;
        assert_eq!(
            kinds(&plan),
            [
                PullImage,
                RemoveExisting,
                Create,
                Initialize,
                InstallPackages,
                ExtractInstaller,
                InstallResolve,
                Verify,
                RemoveConflictingLibs
            ]
        );
        assert_eq!(
            argv_of(&plan, RemoveExisting),
            ["podman", "rm", "--force", "--ignore", "vinci-free"]
        );
        assert!(argv_of(&plan, Create).contains(&"--nvidia".to_string()));
        assert_eq!(
            &argv_of(&plan, Initialize)[5..],
            ["sudo", "-n", "true"],
            "waits for distrobox init before any sudo step"
        );
        assert_eq!(
            &argv_of(&plan, ExtractInstaller)[..6],
            ["distrobox", "enter", "--name", "vinci-free", "--", "sh"]
        );
        let install = argv_of(&plan, InstallPackages);
        assert!(
            install
                .iter()
                .any(|a| a.contains("apt-get update --error-on=any")),
            "apt-get update exits 0 on fetch failures unless told otherwise"
        );
        assert!(install.contains(&"ffmpeg".to_string()));
        assert!(!install.contains(&"rocm-opencl-runtime".to_string()));
        let run = argv_of(&plan, InstallResolve);
        assert!(run.contains(&"SKIP_PACKAGE_CHECK=1".to_string()));
        assert!(run.ends_with(&["-i".into(), "-y".into(), "-a".into()]));
        assert_eq!(
            &argv_of(&plan, Verify)[5..],
            ["test", "-x", "/opt/resolve/bin/resolve"]
        );
        assert!(
            run.contains(&"/home/u/.cache/vinci/work/DaVinci_Resolve_21.1.2_Linux.run".to_string())
        );
    }

    #[test]
    fn amd_plan_adds_rocm_repo_without_nvidia_flag() {
        let r = Recipe::bundled();
        let zip = zip();
        let plan = install_plan(&r, &target(Vendor::Amd, &zip));
        assert!(!argv_of(&plan, StepKind::Create).contains(&"--nvidia".to_string()));
        let repo = argv_of(&plan, StepKind::AddRepository);
        assert!(repo.contains(&"https://repo.radeon.com/rocm/rocm.gpg.key".to_string()));
        assert!(repo.contains(&"rocm".to_string()));
        assert!(
            argv_of(&plan, StepKind::InstallPackages).contains(&"rocm-opencl-runtime".to_string())
        );
    }

    #[test]
    fn intel_plan_installs_neo() {
        let r = Recipe::bundled();
        let zip = zip();
        let plan = install_plan(&r, &target(Vendor::Intel, &zip));
        let install = plan
            .iter()
            .find(|s| s.kind == StepKind::InstallPackages)
            .unwrap();
        assert!(install.argv.contains(&"intel-opencl-icd".to_string()));
    }

    #[test]
    fn run_file_skips_extraction() {
        let r = Recipe::bundled();
        let run = PathBuf::from("/home/u/Downloads/DaVinci_Resolve_Studio_21.1.2_Linux.run");
        let plan = install_plan(&r, &target(Vendor::Nvidia, &run));
        assert!(!kinds(&plan).contains(&StepKind::ExtractInstaller));
        let install = plan
            .iter()
            .find(|s| s.kind == StepKind::InstallResolve)
            .unwrap();
        assert!(install.argv.contains(&run.to_string_lossy().into_owned()));
    }

    #[test]
    fn container_commands_run_as_root_inside_the_box() {
        let r = Recipe::bundled();
        let zip = zip();
        for step in install_plan(&r, &target(Vendor::Amd, &zip)) {
            if matches!(
                step.kind,
                StepKind::AddRepository | StepKind::InstallPackages | StepKind::InstallResolve
            ) {
                assert_eq!(
                    &step.argv[..6],
                    ["distrobox", "enter", "--name", "vinci-free", "--", "sudo"]
                );
            }
        }
    }

    /// Runs the real extraction snippet into a work dir whose parents don't exist yet,
    /// which is the situation on a fresh system (found by the end-to-end run).
    #[test]
    fn extract_creates_missing_work_dir() {
        if std::process::Command::new("unzip")
            .arg("-v")
            .output()
            .is_err()
        {
            return;
        }
        let base = std::env::temp_dir().join(format!("rli-extract-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let zip = base.join("DaVinci_Resolve_21.1.2_Linux.zip");
        let made = std::process::Command::new("python3")
            .args(["-c", "import sys,zipfile; z=zipfile.ZipFile(sys.argv[1],'w'); z.writestr('DaVinci_Resolve_21.1.2_Linux.run','#!/bin/sh'); z.close()"])
            .arg(&zip)
            .status()
            .unwrap()
            .success();
        assert!(made);
        let work = base.join("home/.cache/vinci/work");
        let r = Recipe::bundled();
        let plan = install_plan(
            &r,
            &Target {
                work_dir: &work,
                ..target(Vendor::Nvidia, &zip)
            },
        );
        let argv = &argv_of(&plan, StepKind::ExtractInstaller)[5..];
        let ok = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .status()
            .unwrap()
            .success();
        let extracted = work.join("DaVinci_Resolve_21.1.2_Linux.run").exists();
        std::fs::remove_dir_all(&base).unwrap();
        assert!(ok && extracted);
    }

    /// Runs the generated shell snippets for real against a temp dir, to prove the
    /// lib-removal glob works and nothing else is touched.
    #[test]
    fn remove_libs_script_only_deletes_matching_files() {
        let r = Recipe::bundled();
        let zip = zip();
        let plan = install_plan(&r, &target(Vendor::Nvidia, &zip));
        let step = plan
            .iter()
            .find(|s| s.kind == StepKind::RemoveConflictingLibs)
            .unwrap();
        // Strip "distrobox enter --name X -- sudo" and swap the install dir for a temp one.
        let mut argv = step.argv[6..].to_vec();
        let dir = std::env::temp_dir().join(format!("rli-libs-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("libs")).unwrap();
        for f in [
            "libglib-2.0.so.0",
            "libgio-2.0.so.0.7",
            "libkeep.so",
            "libgmodule-2.0.so",
        ] {
            std::fs::write(dir.join("libs").join(f), "").unwrap();
        }
        argv[4] = dir.to_string_lossy().into_owned();
        let ok = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .status()
            .unwrap()
            .success();
        let mut left: Vec<_> = std::fs::read_dir(dir.join("libs"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(ok);
        assert_eq!(left, ["libkeep.so"]);
    }
}
