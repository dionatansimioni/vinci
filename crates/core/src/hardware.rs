//! Pre-install compatibility check. Blocks only what certainly won't work; warns otherwise.

use crate::gpu::{self, Gpu, Support, UnsupportedReason};

const GIB: u64 = 1024 * 1024 * 1024;
/// Container image + Resolve install + downloaded/extracted installer.
pub const MIN_FREE_DISK: u64 = 12 * GIB;
pub const RECOMMENDED_FREE_DISK: u64 = 40 * GIB;
pub const RECOMMENDED_RAM: u64 = 16 * GIB;
pub const MIN_RAM: u64 = 8 * GIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Blocker,
}

/// What was found. The UI turns each variant into a translated message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    NoUsableGpu,
    GpuNeedsDriver {
        gpu: Gpu,
    },
    GpuExperimental {
        gpu: Gpu,
        hsa_override: Option<&'static str>,
    },
    GpuUnknown {
        gpu: Gpu,
    },
    GpuUnsupported {
        gpu: Gpu,
        reason: UnsupportedReason,
    },
    Hybrid {
        chosen: Gpu,
    },
    LowRam {
        bytes: u64,
    },
    VeryLowRam {
        bytes: u64,
    },
    LowDisk {
        bytes: u64,
    },
    InsufficientDisk {
        bytes: u64,
    },
}

impl Finding {
    pub fn severity(&self) -> Severity {
        match self {
            Self::NoUsableGpu | Self::InsufficientDisk { .. } => Severity::Blocker,
            Self::Hybrid { .. } => Severity::Info,
            // Unsupported devices next to a usable one are just informative.
            Self::GpuUnsupported { .. } => Severity::Info,
            _ => Severity::Warning,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct System {
    pub gpus: Vec<Gpu>,
    pub ram_bytes: Option<u64>,
    pub free_disk_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub chosen_gpu: Option<Gpu>,
}

impl Report {
    pub fn is_blocked(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.severity() == Severity::Blocker)
    }
}

pub fn check(sys: &System) -> Report {
    let mut findings = Vec::new();
    let chosen = gpu::preferred(&sys.gpus).cloned();

    for g in &sys.gpus {
        match g.support() {
            Support::Supported => {}
            Support::NeedsDriver => findings.push(Finding::GpuNeedsDriver { gpu: g.clone() }),
            Support::Experimental { hsa_override } => findings.push(Finding::GpuExperimental {
                gpu: g.clone(),
                hsa_override,
            }),
            Support::Unknown => findings.push(Finding::GpuUnknown { gpu: g.clone() }),
            // Virtual display adapters are noise, not worth a line in the report.
            Support::Unsupported(UnsupportedReason::NoComputeDevice) => {}
            Support::Unsupported(reason) => findings.push(Finding::GpuUnsupported {
                gpu: g.clone(),
                reason,
            }),
        }
    }
    match &chosen {
        None => findings.push(Finding::NoUsableGpu),
        Some(c) if gpu::is_hybrid(&sys.gpus) => {
            findings.push(Finding::Hybrid { chosen: c.clone() })
        }
        Some(_) => {}
    }

    if let Some(ram) = sys.ram_bytes {
        if ram < MIN_RAM {
            findings.push(Finding::VeryLowRam { bytes: ram });
        } else if ram < RECOMMENDED_RAM {
            findings.push(Finding::LowRam { bytes: ram });
        }
    }
    if let Some(disk) = sys.free_disk_bytes {
        if disk < MIN_FREE_DISK {
            findings.push(Finding::InsufficientDisk { bytes: disk });
        } else if disk < RECOMMENDED_FREE_DISK {
            findings.push(Finding::LowDisk { bytes: disk });
        }
    }
    findings.sort_by_key(|f| std::cmp::Reverse(f.severity()));
    Report {
        findings,
        chosen_gpu: chosen,
    }
}

/// Parses `MemTotal` from `/proc/meminfo` into bytes.
pub fn parse_meminfo(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

pub fn read_ram() -> Option<u64> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

/// Free space available to unprivileged users on the filesystem holding `path`.
pub fn free_disk(path: &std::path::Path) -> Option<u64> {
    let st = rustix::fs::statvfs(path).ok()?;
    Some(st.f_bavail * st.f_frsize)
}

/// Gathers everything [`check`] needs from the running system.
pub fn probe(home: &std::path::Path) -> System {
    System {
        gpus: gpu::detect(std::path::Path::new("/")),
        ram_bytes: read_ram(),
        free_disk_bytes: free_disk(home),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::Vendor;

    fn g(vendor: Vendor, device_id: u16, driver: &str) -> Gpu {
        Gpu {
            vendor,
            device_id,
            driver: Some(driver.into()),
            pci_slot: None,
            render_node: None,
            amd_gfx_target: None,
        }
    }

    fn healthy() -> System {
        System {
            gpus: vec![g(Vendor::Nvidia, 0x2571, "nvidia")],
            ram_bytes: Some(32 * GIB),
            free_disk_bytes: Some(100 * GIB),
        }
    }

    #[test]
    fn healthy_system_is_clean() {
        let r = check(&healthy());
        assert!(r.findings.is_empty(), "{:?}", r.findings);
        assert!(!r.is_blocked());
        assert_eq!(r.chosen_gpu.unwrap().vendor, Vendor::Nvidia);
    }

    #[test]
    fn blocks_without_gpu_or_disk() {
        let mut sys = healthy();
        sys.gpus = vec![g(Vendor::Other(0x1234), 0x1111, "bochs-drm")];
        sys.free_disk_bytes = Some(5 * GIB);
        let r = check(&sys);
        assert!(r.is_blocked());
        assert_eq!(r.findings[0].severity(), Severity::Blocker);
        assert!(r.findings.contains(&Finding::NoUsableGpu));
        assert!(r
            .findings
            .contains(&Finding::InsufficientDisk { bytes: 5 * GIB }));
    }

    #[test]
    fn nouveau_warns_but_does_not_block() {
        let mut sys = healthy();
        sys.gpus = vec![g(Vendor::Nvidia, 0x2571, "nouveau")];
        let r = check(&sys);
        assert!(!r.is_blocked());
        assert!(matches!(r.findings[0], Finding::GpuNeedsDriver { .. }));
    }

    #[test]
    fn old_intel_next_to_nvidia_is_info_and_hybrid_reported() {
        let mut sys = healthy();
        sys.gpus.push(g(Vendor::Intel, 0x5917, "i915"));
        sys.gpus.push(g(Vendor::Intel, 0x46a6, "i915"));
        let r = check(&sys);
        assert!(!r.is_blocked());
        assert!(r
            .findings
            .iter()
            .any(|f| matches!(f, Finding::GpuUnsupported { .. })));
        assert!(r
            .findings
            .iter()
            .any(|f| matches!(f, Finding::Hybrid { .. })));
    }

    #[test]
    fn ram_and_disk_thresholds() {
        let mut sys = healthy();
        sys.ram_bytes = Some(12 * GIB);
        sys.free_disk_bytes = Some(20 * GIB);
        let r = check(&sys);
        assert!(r.findings.contains(&Finding::LowRam { bytes: 12 * GIB }));
        assert!(r.findings.contains(&Finding::LowDisk { bytes: 20 * GIB }));
        sys.ram_bytes = Some(4 * GIB);
        assert!(check(&sys)
            .findings
            .contains(&Finding::VeryLowRam { bytes: 4 * GIB }));
    }

    #[test]
    fn parses_meminfo() {
        let text = "MemTotal:       32768000 kB\nMemFree:  100 kB\n";
        assert_eq!(parse_meminfo(text), Some(32768000 * 1024));
        assert_eq!(parse_meminfo("garbage"), None);
    }

    #[test]
    fn probes_real_system_without_panicking() {
        let sys = probe(std::path::Path::new("/"));
        assert!(sys.ram_bytes.unwrap() > 0);
        assert!(sys.free_disk_bytes.is_some());
    }
}
