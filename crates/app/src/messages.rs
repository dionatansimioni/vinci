//! User-facing text for core results. Kept apart from the widgets so it can be tested.

use crate::i18n::tr;
use rli_core::container::StepKind;
use rli_core::flow;
use rli_core::gpu::{Gpu, UnsupportedReason};
use rli_core::hardware::{Finding, Severity};
use rli_core::installer_file::{Edition, InstallerFile};
use rli_core::tools::Action;

pub fn gpu_label(g: &Gpu) -> String {
    g.display_name()
}

fn gib(bytes: u64) -> String {
    format!("{:.0} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

/// Title and explanation for one finding of the hardware check.
pub fn finding(f: &Finding) -> (String, String) {
    match f {
        Finding::NoUsableGpu => (
            tr("No compatible GPU found"),
            tr("DaVinci Resolve needs a GPU with CUDA, ROCm or OpenCL support."),
        ),
        Finding::GpuNeedsDriver { gpu } => (
            format!("{}: {}", gpu_label(gpu), tr("proprietary driver required")),
            tr("Install the NVIDIA driver from your distribution, restart, then open Vinci again."),
        ),
        Finding::GpuExperimental { gpu, .. } => (
            format!("{}: {}", gpu_label(gpu), tr("experimental support")),
            tr("This GPU is not officially supported by ROCm. Vinci applies a known workaround, but Resolve may be unstable."),
        ),
        Finding::GpuUnknown { gpu } => (
            format!("{}: {}", gpu_label(gpu), tr("unknown compatibility")),
            tr("Vinci could not tell whether this GPU works with Resolve. You can try anyway."),
        ),
        Finding::GpuUnsupported { gpu, reason } => (
            format!("{}: {}", gpu_label(gpu), tr("not supported")),
            match reason {
                UnsupportedReason::AmdPrePolaris => {
                    tr("AMD GPUs older than Vega are not supported by current ROCm.")
                }
                UnsupportedReason::IntelPreGen12 => {
                    tr("Intel GPUs older than 12th generation (Tiger Lake) are not supported.")
                }
                UnsupportedReason::NoComputeDevice => tr("This display adapter has no compute support."),
            },
        ),
        Finding::Hybrid { chosen } => (
            tr("Multiple GPUs detected"),
            format!("{} {}", tr("Resolve will run on"), gpu_label(chosen)),
        ),
        Finding::LowRam { bytes } => (
            format!("{} ({})", tr("Low memory"), gib(*bytes)),
            tr("Blackmagic recommends 16 GiB or more. Large projects may be slow."),
        ),
        Finding::VeryLowRam { bytes } => (
            format!("{} ({})", tr("Very low memory"), gib(*bytes)),
            tr("Resolve may close unexpectedly with less than 8 GiB."),
        ),
        Finding::LowDisk { bytes } => (
            format!("{} ({})", tr("Little free space"), gib(*bytes)),
            tr("Installing works, but caches and renders need room. 40 GiB or more is recommended."),
        ),
        Finding::InsufficientDisk { bytes } => (
            format!("{} ({})", tr("Not enough free space"), gib(*bytes)),
            tr("At least 12 GiB of free space is required."),
        ),
    }
}

pub fn edition_name(e: Edition) -> &'static str {
    match e {
        Edition::Free => "DaVinci Resolve",
        Edition::Studio => "DaVinci Resolve Studio",
    }
}

pub fn installer_title(f: &InstallerFile) -> String {
    format!("{} {}", edition_name(f.edition), f.version)
}

/// What the progress view says while a step runs.
pub fn step(kind: StepKind) -> String {
    match kind {
        StepKind::PullImage => tr("Downloading the base system"),
        StepKind::Snapshot => tr("Saving a restore point of the current version"),
        StepKind::RemoveExisting => tr("Removing the previous installation"),
        StepKind::Create => tr("Creating the container"),
        StepKind::Initialize => tr("Setting up the container"),
        StepKind::AddRepository => tr("Adding the GPU driver repository"),
        StepKind::InstallPackages => tr("Installing libraries and GPU support"),
        StepKind::ExtractInstaller => tr("Extracting the installer"),
        StepKind::InstallResolve => tr("Installing DaVinci Resolve"),
        StepKind::Verify => tr("Checking the installation"),
        StepKind::RemoveConflictingLibs => tr("Applying compatibility fixes"),
        StepKind::InstallPodman => tr("Installing podman"),
        StepKind::InstallDistrobox => tr("Installing distrobox"),
    }
}

pub fn host_action(a: &Action) -> String {
    match a {
        Action::InstallPodman { .. } => tr("Podman (container engine, asks for your password)"),
        Action::InstallDistrobox { .. } => {
            tr("Distrobox (installed in your user folder, no password)")
        }
        Action::ManualPodman { hint } => hint.clone(),
    }
}

/// One-line summary of why an install failed.
pub fn failure(e: &flow::Error) -> String {
    match e {
        flow::Error::Step(rli_core::installer::Error::StepFailed { kind, .. }) => {
            format!("{} {}", tr("This step failed:"), step(*kind))
        }
        flow::Error::Step(rli_core::installer::Error::Spawn { .. }) => {
            tr("A required program could not be started.")
        }
        flow::Error::Step(rli_core::installer::Error::Cancelled { .. }) => {
            tr("The installation was cancelled.")
        }
        flow::Error::Integrate(_) => {
            tr("Resolve was installed, but the menu entry could not be created.")
        }
        flow::Error::State(_) => {
            tr("Resolve was installed, but Vinci could not save its settings.")
        }
    }
}

/// Title and explanation for a problem found by the self-check.
pub fn problem(p: &rli_core::health::Problem) -> (String, String) {
    use rli_core::health::Problem;
    match p {
        Problem::ContainerMissing => (
            tr("The Resolve container is missing"),
            tr("It may have been removed by a system cleanup. Reinstall to get it back; your projects are safe."),
        ),
        Problem::ResolveMissing => (
            tr("DaVinci Resolve is missing from its container"),
            tr("Reinstall to fix it; your projects are safe."),
        ),
        Problem::LauncherMissing => (
            tr("The menu entry is missing"),
            tr("Vinci can create it again."),
        ),
        Problem::GpuNotVisible => (
            tr("The GPU is not visible to Resolve"),
            tr("This usually happens after a driver update. Restart the computer; if it persists, copy the diagnostic report and ask for help."),
        ),
        Problem::PackagesOutdated => (
            tr("The Resolve container needs an update"),
            tr("This version of Vinci installs newer GPU and video drivers in it. Your projects and Resolve's settings are kept."),
        ),
    }
}

pub fn severity_icon(s: Severity) -> &'static str {
    match s {
        Severity::Info => "dialog-information-symbolic",
        Severity::Warning => "dialog-warning-symbolic",
        Severity::Blocker => "dialog-error-symbolic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rli_core::gpu::Vendor;

    fn gpu() -> Gpu {
        Gpu {
            vendor: Vendor::Amd,
            device_id: 0x731f,
            driver: Some("amdgpu".into()),
            pci_slot: None,
            render_node: None,
            amd_gfx_target: None,
        }
    }

    #[test]
    fn every_problem_and_cancel_have_text() {
        use rli_core::health::Problem;
        for p in [
            Problem::ContainerMissing,
            Problem::ResolveMissing,
            Problem::LauncherMissing,
            Problem::GpuNotVisible,
            Problem::PackagesOutdated,
        ] {
            let (title, body) = problem(&p);
            assert!(!title.is_empty() && !body.is_empty(), "{p:?}");
        }
        let cancelled = flow::Error::Step(rli_core::installer::Error::Cancelled {
            kind: StepKind::InstallResolve,
        });
        assert!(!failure(&cancelled).is_empty());
    }

    #[test]
    fn every_finding_has_text() {
        let all = [
            Finding::NoUsableGpu,
            Finding::GpuNeedsDriver { gpu: gpu() },
            Finding::GpuExperimental {
                gpu: gpu(),
                hsa_override: Some("10.3.0"),
            },
            Finding::GpuUnknown { gpu: gpu() },
            Finding::GpuUnsupported {
                gpu: gpu(),
                reason: UnsupportedReason::AmdPrePolaris,
            },
            Finding::GpuUnsupported {
                gpu: gpu(),
                reason: UnsupportedReason::IntelPreGen12,
            },
            Finding::Hybrid { chosen: gpu() },
            Finding::LowRam { bytes: 12 << 30 },
            Finding::VeryLowRam { bytes: 4 << 30 },
            Finding::LowDisk { bytes: 20 << 30 },
            Finding::InsufficientDisk { bytes: 5 << 30 },
        ];
        for f in &all {
            let (title, body) = finding(f);
            assert!(!title.is_empty() && !body.is_empty(), "{f:?}");
        }
        assert_eq!(
            finding(&Finding::LowRam { bytes: 12 << 30 }).0,
            "Low memory (12 GiB)"
        );
        assert!(gpu_label(&gpu()).starts_with("AMD "));
    }
}
