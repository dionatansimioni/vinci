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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repair {
    /// Rewrite menu entry, icon and MIME types.
    RecreateLauncher,
    /// Reinstall from the kept installer (or ask for one).
    Reinstall,
    /// Nothing automatic; explain (e.g. reboot after a driver update).
    Explain,
}

impl Problem {
    pub fn repair(&self) -> Repair {
        match self {
            Self::LauncherMissing => Repair::RecreateLauncher,
            Self::ContainerMissing | Self::ResolveMissing => Repair::Reinstall,
            Self::GpuNotVisible => Repair::Explain,
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
                &["distrobox", "enter", "--name", "vinci-free", "--", "sh"],
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
}
