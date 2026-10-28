//! Self-check run when the app opens (SPEC §12.8): is the install still intact after
//! system updates? Each problem maps to the repair that fixes it.

use crate::gpu::Vendor;
use crate::host::{argv, Runner};
use crate::launcher;
use crate::recipe::Recipe;
use crate::state::Installation;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The container is gone (e.g. `podman system reset`).
    ContainerMissing,
    /// The container exists but Resolve isn't in it any more.
    ResolveMissing,
    /// The menu entry was deleted.
    LauncherMissing,
    /// The GPU isn't visible inside the container (driver updated, device moved…).
    GpuNotVisible,
    /// The container predates the current recipe: a package or repository it now asks
    /// for (e.g. a newer GPU driver) is missing.
    PackagesOutdated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repair {
    /// Rewrite menu entry, icon and MIME types.
    RecreateLauncher,
    /// Reinstall from the kept installer (or ask for one).
    Reinstall,
    /// Nothing automatic; explain (e.g. reboot after a driver update).
    Explain,
    /// Install what the recipe asks for into the existing container
    /// (`container::package_steps`). Resolve and its settings stay as they are.
    UpdatePackages,
}

impl Problem {
    pub fn repair(&self) -> Repair {
        match self {
            Self::LauncherMissing => Repair::RecreateLauncher,
            Self::ContainerMissing | Self::ResolveMissing => Repair::Reinstall,
            Self::GpuNotVisible => Repair::Explain,
            Self::PackagesOutdated => Repair::UpdatePackages,
        }
    }
}

/// Command that proves the GPU is usable inside the container, per vendor.
fn gpu_probe(vendor: Vendor) -> Option<Vec<String>> {
    match vendor {
        Vendor::Nvidia => Some(argv(&["nvidia-smi", "-L"])),
        // clinfo lists OpenCL platforms; ROCm and NEO both register one.
        Vendor::Amd | Vendor::Intel => Some(argv(&["sh", "-c", "clinfo -l | grep -q Platform"])),
        Vendor::Other(_) => None,
    }
}

/// Succeeds when the container has the vendor repository and every package installed.
fn packages_probe(recipe: &Recipe, vendor: Option<Vendor>) -> Vec<String> {
    let (packages, repo) = match vendor {
        Some(v) => crate::container::requirements(recipe, v),
        None => (recipe.packages.base.clone(), None),
    };
    let list = repo
        .map(|r| format!("/etc/apt/sources.list.d/{}.list", r.name))
        .unwrap_or_default();
    // Unknown packages print nothing on stdout, so count the installed ones instead of
    // looking for bad lines.
    let mut a = argv(&[
        "sh",
        "-c",
        r#"list=$1; shift; if [ -n "$list" ] && [ ! -f "$list" ]; then exit 1; fi; n=$(dpkg-query -W -f='${db:Status-Abbrev}\n' "$@" 2>/dev/null | grep -c '^ii'); [ "$n" -eq $# ]"#,
        "sh",
        &list,
    ]);
    a.extend(packages);
    a
}

pub fn check(
    runner: &dyn Runner,
    recipe: &Recipe,
    distrobox: &str,
    inst: &Installation,
    vendor: Option<Vendor>,
    data_home: &Path,
) -> Vec<Problem> {
    let ok = |a: Vec<String>| runner.run(&a).is_ok_and(|o| o.success);
    let in_box = |cmd: Vec<String>| {
        let mut a = argv(&[distrobox, "enter", "--name", &inst.container, "--"]);
        a.extend(cmd);
        a
    };
    let mut problems = Vec::new();
    if !launcher::paths(data_home).desktop.exists() {
        problems.push(Problem::LauncherMissing);
    }
    if !ok(argv(&["podman", "container", "exists", &inst.container])) {
        problems.push(Problem::ContainerMissing);
        return problems;
    }
    let resolve = format!("{}/bin/resolve", recipe.resolve.install_dir);
    if !ok(in_box(argv(&["test", "-x", &resolve]))) {
        problems.push(Problem::ResolveMissing);
        return problems;
    }
    if !ok(in_box(packages_probe(recipe, vendor))) {
        problems.push(Problem::PackagesOutdated);
    }
    if let Some(probe) = vendor.and_then(gpu_probe) {
        if !ok(in_box(probe)) {
            problems.push(Problem::GpuNotVisible);
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::fake::FakeRunner;
    use crate::installer_file::Edition;

    fn inst() -> Installation {
        Installation {
            container: "vinci-free".into(),
            edition: Edition::Free,
            version: "21.1.2".parse().unwrap(),
        }
    }

    fn data_home_with_launcher(name: &str, present: bool) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rli-health-{}-{name}", std::process::id()));
        let p = launcher::paths(&d).desktop;
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        if present {
            std::fs::write(&p, "x").unwrap();
        }
        d
    }

    fn run(r: &FakeRunner, name: &str, launcher: bool, vendor: Vendor) -> Vec<Problem> {
        let d = data_home_with_launcher(name, launcher);
        let p = check(
            r,
            &Recipe::bundled(),
            "distrobox",
            &inst(),
            Some(vendor),
            &d,
        );
        std::fs::remove_dir_all(&d).unwrap();
        p
    }

    #[test]
    fn healthy_install_has_no_problems() {
        let r = FakeRunner::default().respond(&[], "", true);
        assert!(run(&r, "ok", true, Vendor::Nvidia).is_empty());
        let calls = r.calls();
        assert!(calls
            .iter()
            .any(|c| c.ends_with(&["nvidia-smi".into(), "-L".into()])));
    }

    #[test]
    fn missing_container_short_circuits() {
        let r = FakeRunner::default()
            .respond(&["podman", "container", "exists"], "", false)
            .respond(&[], "", true);
        assert_eq!(
            run(&r, "nocontainer", true, Vendor::Nvidia),
            [Problem::ContainerMissing]
        );
        assert_eq!(
            r.calls().len(),
            1,
            "nothing else probed once the container is gone"
        );
    }

    #[test]
    fn reports_launcher_and_gpu_problems() {
        let r = FakeRunner::default()
            .respond(
                &[
                    "distrobox",
                    "enter",
                    "--name",
                    "vinci-free",
                    "--",
                    "sh",
                    "-c",
                    "clinfo -l | grep -q Platform",
                ],
                "",
                false,
            )
            .respond(&[], "", true);
        let p = run(&r, "gpu", false, Vendor::Amd);
        assert_eq!(p, [Problem::LauncherMissing, Problem::GpuNotVisible]);
        assert_eq!(p[0].repair(), Repair::RecreateLauncher);
        assert_eq!(p[1].repair(), Repair::Explain);
    }

    #[test]
    fn missing_resolve_means_reinstall() {
        let r = FakeRunner::default()
            .respond(
                &["distrobox", "enter", "--name", "vinci-free", "--", "test"],
                "",
                false,
            )
            .respond(&[], "", true);
        let p = run(&r, "noresolve", true, Vendor::Nvidia);
        assert_eq!(p, [Problem::ResolveMissing]);
        assert_eq!(p[0].repair(), Repair::Reinstall);
    }

    #[test]
    fn container_from_an_older_recipe_needs_its_packages_updated() {
        let r = FakeRunner::default()
            .respond(
                &[
                    "distrobox",
                    "enter",
                    "--name",
                    "vinci-free",
                    "--",
                    "sh",
                    "-c",
                ],
                "",
                false,
            )
            .respond(&[], "", true);
        let p = run(&r, "packages", true, Vendor::Intel);
        assert!(p.contains(&Problem::PackagesOutdated), "{p:?}");
        assert_eq!(Problem::PackagesOutdated.repair(), Repair::UpdatePackages);
        let probe = r
            .calls()
            .into_iter()
            .find(|c| c.iter().any(|a| a.contains("dpkg-query")))
            .unwrap();
        assert!(probe.contains(&"/etc/apt/sources.list.d/intel-graphics.list".to_string()));
        assert!(probe.contains(&"intel-opencl-icd".to_string()));
        assert!(probe.contains(&"ffmpeg".to_string()), "base packages too");
    }

    /// The probe script against the real dpkg of this machine (Debian/Ubuntu only).
    #[test]
    fn packages_probe_script_works_with_real_dpkg() {
        if std::process::Command::new("dpkg-query")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let script = packages_probe(&Recipe::bundled(), None)[2].clone();
        let run = |args: &[&str]| {
            std::process::Command::new("sh")
                .args(["-c", &script, "sh"])
                .args(args)
                .status()
                .unwrap()
                .success()
        };
        assert!(run(&["", "dpkg"]));
        assert!(!run(&["", "dpkg", "surely-not-a-package-xyz"]));
        assert!(!run(&["/nonexistent/repo.list", "dpkg"]));
    }
}
