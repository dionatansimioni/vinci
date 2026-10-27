//! User-facing text for core results. Kept apart from the widgets so it can be tested.

use crate::i18n::tr;
use rli_core::gpu::{Gpu, UnsupportedReason};
use rli_core::hardware::{Finding, Severity};

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
