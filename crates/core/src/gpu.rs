//! GPU detection from sysfs and classification of what Resolve can use.
//!
//! Detection reads `<root>/sys/class/drm/card*/device/` so tests can point it at a
//! fake tree. Classification is pure and works on the parsed [`Gpu`] values.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Other(u16),
}

impl Vendor {
    pub fn from_pci_id(id: u16) -> Self {
        match id {
            0x10de => Self::Nvidia,
            0x1002 => Self::Amd,
            0x8086 => Self::Intel,
            other => Self::Other(other),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Nvidia => "NVIDIA",
            Self::Amd => "AMD",
            Self::Intel => "Intel",
            Self::Other(_) => "Other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu {
    pub vendor: Vendor,
    pub device_id: u16,
    /// Kernel driver bound to the device (`nvidia`, `nouveau`, `amdgpu`, `i915`, `xe`…).
    pub driver: Option<String>,
    pub pci_slot: Option<String>,
    /// e.g. `renderD128`
    pub render_node: Option<String>,
    /// AMD only: ROCm gfx target (e.g. `100300` for gfx1030), from the KFD topology.
    pub amd_gfx_target: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Support {
    Supported,
    /// Works only after the host installs the proprietary driver.
    NeedsDriver,
    /// May work with a workaround; `hsa_override` is the `HSA_OVERRIDE_GFX_VERSION` to set.
    Experimental {
        hsa_override: Option<&'static str>,
    },
    Unsupported(UnsupportedReason),
    /// Not enough information to decide; warn but allow.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedReason {
    AmdPrePolaris,
    IntelPreGen12,
    NoComputeDevice,
}

/// i915 device-ID high bytes of Gen12+ (Xe) parts: TGL, RKL, DG1, ADL, RPL, DG2/Arc,
/// MTL/ARL, LNL, BMG, PTL.
const INTEL_GEN12_PREFIXES: &[u8] = &[
    0x9A, 0x4C, 0x49, 0x46, 0xA7, 0x56, 0x4F, 0x7D, 0x64, 0xE2, 0xB0,
];
/// Pre-Gen12 high bytes: HSW, BDW, SKL, APL, GLK, KBL, CFL, WHL, CML, ICL, JSL, EHL.
const INTEL_OLD_PREFIXES: &[u8] = &[
    0x04, 0x0A, 0x0D, 0x16, 0x19, 0x5A, 0x31, 0x59, 0x3E, 0x87, 0x9B, 0x8A, 0x4E, 0x45,
];
/// Discrete Intel parts: DG1, DG2/Arc A, BMG/Arc B.
const INTEL_DISCRETE_PREFIXES: &[u8] = &[0x49, 0x56, 0x4F, 0xE2];

impl Gpu {
    pub fn support(&self) -> Support {
        match self.vendor {
            Vendor::Nvidia => match self.driver.as_deref() {
                Some("nvidia") => Support::Supported,
                _ => Support::NeedsDriver,
            },
            Vendor::Amd => match self.amd_gfx_target {
                None => Support::Unknown,
                Some(t) => classify_amd(t),
            },
            Vendor::Intel => {
                if self.driver.as_deref() == Some("xe") {
                    return Support::Supported;
                }
                let hi = (self.device_id >> 8) as u8;
                if INTEL_GEN12_PREFIXES.contains(&hi) {
                    Support::Supported
                } else if INTEL_OLD_PREFIXES.contains(&hi) {
                    Support::Unsupported(UnsupportedReason::IntelPreGen12)
                } else {
                    Support::Unknown
                }
            }
            Vendor::Other(_) => Support::Unsupported(UnsupportedReason::NoComputeDevice),
        }
    }

    pub fn is_discrete(&self) -> bool {
        match self.vendor {
            Vendor::Nvidia => true,
            Vendor::Intel => INTEL_DISCRETE_PREFIXES.contains(&((self.device_id >> 8) as u8)),
            // APUs report gfx targets like gfx90c/gfx1035/gfx1103; treat the rest as discrete.
            Vendor::Amd => !matches!(
                self.amd_gfx_target,
                Some(90012 | 100305 | 100306 | 110003 | 110500)
            ),
            Vendor::Other(_) => false,
        }
    }

    /// Whether this device can run Resolve at all (possibly after user action).
    pub fn is_usable(&self) -> bool {
        !matches!(self.support(), Support::Unsupported(_))
    }
}

/// Locations of the PCI ID database across distros (and the host, seen from a Flatpak).
const PCI_IDS: &[&str] = &[
    "/usr/share/hwdata/pci.ids",
    "/usr/share/misc/pci.ids",
    "/run/host/usr/share/hwdata/pci.ids",
    "/run/host/usr/share/misc/pci.ids",
];

/// Marketing name from a `pci.ids` database, preferring the bracketed part
/// (`GA106 [RTX A2000 12GB]` → `RTX A2000 12GB`).
pub fn model_name_from(pci_ids: &str, vendor: u16, device: u16) -> Option<String> {
    let vendor_hdr = format!("{vendor:04x}  ");
    let device_hdr = format!("\t{device:04x}  ");
    let name = pci_ids
        .lines()
        .skip_while(|l| !l.starts_with(&vendor_hdr))
        .skip(1)
        .take_while(|l| l.starts_with('\t') || l.starts_with('#') || l.is_empty())
        .find_map(|l| l.strip_prefix(&device_hdr))?
        .trim();
    let short = name
        .rsplit_once('[')
        .and_then(|(_, rest)| rest.strip_suffix(']'))
        .unwrap_or(name);
    Some(short.to_string())
}

impl Gpu {
    fn pci_vendor_id(&self) -> u16 {
        match self.vendor {
            Vendor::Nvidia => 0x10de,
            Vendor::Amd => 0x1002,
            Vendor::Intel => 0x8086,
            Vendor::Other(id) => id,
        }
    }

    /// Human-readable name such as "NVIDIA RTX A2000 12GB", or "NVIDIA 2571" when the
    /// PCI ID database is missing.
    pub fn display_name(&self) -> String {
        let model = PCI_IDS.iter().find_map(|p| {
            let text = std::fs::read_to_string(p).ok()?;
            model_name_from(&text, self.pci_vendor_id(), self.device_id)
        });
        match model {
            Some(m) => format!("{} {m}", self.vendor.name()),
            None => format!("{} {:04x}", self.vendor.name(), self.device_id),
        }
    }
}

fn classify_amd(target: u32) -> Support {
    let (major, minor) = (target / 10000, (target / 100) % 100);
    match (major, minor) {
        (11.., _) | (10, 3..) => Support::Supported,
        (10, _) => Support::Experimental {
            hsa_override: Some("10.3.0"),
        },
        (9, _) => Support::Experimental {
            hsa_override: Some("9.0.0"),
        },
        _ => Support::Unsupported(UnsupportedReason::AmdPrePolaris),
    }
}

/// Picks the GPU Resolve should run on: the best-supported discrete one, then integrated.
pub fn preferred(gpus: &[Gpu]) -> Option<&Gpu> {
    let rank = |g: &Gpu| {
        let support = match g.support() {
            Support::Supported => 0,
            Support::Experimental { .. } | Support::Unknown => 1,
            Support::NeedsDriver => 2,
            Support::Unsupported(_) => return None,
        };
        let vendor = match g.vendor {
            Vendor::Nvidia => 0,
            Vendor::Amd => 1,
            _ => 2,
        };
        Some((!g.is_discrete(), support, vendor))
    };
    gpus.iter()
        .filter_map(|g| rank(g).map(|r| (r, g)))
        .min_by_key(|(r, _)| *r)
        .map(|(_, g)| g)
}

/// True when more than one usable GPU is present (Optimus/PRIME laptops, multi-GPU desktops).
pub fn is_hybrid(gpus: &[Gpu]) -> bool {
    gpus.iter().filter(|g| g.is_usable()).count() > 1
}

/// Reads GPUs from `<root>/sys/class/drm`. Pass `/` on a real system.
pub fn detect(root: &Path) -> Vec<Gpu> {
    let drm = root.join("sys/class/drm");
    let Ok(entries) = std::fs::read_dir(&drm) else {
        return Vec::new();
    };
    let mut cards: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("card") && n[4..].chars().all(|c| c.is_ascii_digit()))
        .collect();
    cards.sort();
    let amd_targets = read_kfd_targets(root);
    let mut amd_index = 0;
    cards
        .iter()
        .filter_map(|card| {
            let dev = drm.join(card).join("device");
            let vendor = read_hex(&dev.join("vendor"))?;
            let device_id = read_hex(&dev.join("device"))?;
            let uevent = std::fs::read_to_string(dev.join("uevent")).unwrap_or_default();
            let field = |k: &str| {
                uevent
                    .lines()
                    .find_map(|l| l.strip_prefix(k))
                    .map(str::to_string)
            };
            let render_node = std::fs::read_dir(dev.join("drm")).ok().and_then(|it| {
                it.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .find(|n| n.starts_with("renderD"))
            });
            let vendor = Vendor::from_pci_id(vendor);
            // KFD nodes are enumerated in PCI order, matching card order for amdgpu.
            let amd_gfx_target = (vendor == Vendor::Amd).then(|| {
                let t = amd_targets.get(amd_index).copied();
                amd_index += 1;
                t
            });
            Some(Gpu {
                vendor,
                device_id,
                driver: field("DRIVER="),
                pci_slot: field("PCI_SLOT_NAME="),
                render_node,
                amd_gfx_target: amd_gfx_target.flatten(),
            })
        })
        .collect()
}

fn read_hex(path: &Path) -> Option<u16> {
    let s = std::fs::read_to_string(path).ok()?;
    u16::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

/// GPU nodes in the KFD topology (CPU nodes report `gfx_target_version 0`).
fn read_kfd_targets(root: &Path) -> Vec<u32> {
    let nodes = root.join("sys/class/kfd/kfd/topology/nodes");
    let Ok(entries) = std::fs::read_dir(&nodes) else {
        return Vec::new();
    };
    let mut ids: Vec<u32> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .collect();
    ids.sort();
    ids.iter()
        .filter_map(|id| {
            let props =
                std::fs::read_to_string(nodes.join(id.to_string()).join("properties")).ok()?;
            props
                .lines()
                .find_map(|l| l.strip_prefix("gfx_target_version "))
                .and_then(|v| v.trim().parse().ok())
                .filter(|&v: &u32| v != 0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(vendor: Vendor, device_id: u16, driver: &str) -> Gpu {
        Gpu {
            vendor,
            device_id,
            driver: Some(driver.into()),
            pci_slot: None,
            render_node: None,
            amd_gfx_target: None,
        }
    }

    fn amd(target: u32) -> Gpu {
        Gpu {
            amd_gfx_target: Some(target),
            ..gpu(Vendor::Amd, 0x73bf, "amdgpu")
        }
    }

    #[test]
    fn reads_model_names_from_pci_ids() {
        let db = "# comment\n10de  NVIDIA Corporation\n\t2571  GA106 [RTX A2000 12GB]\n\t\t1028 1611  subsystem\n\t2684  AD102 [GeForce RTX 4090]\n1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t2571  Not this one\n\t744c  Navi 31 [Radeon RX 7900 XT/7900 XTX/7900 GRE/7900M]\n8086  Intel Corporation\n\t1234  Plain Name\n";
        assert_eq!(
            model_name_from(db, 0x10de, 0x2571).as_deref(),
            Some("RTX A2000 12GB")
        );
        assert_eq!(
            model_name_from(db, 0x10de, 0x2684).as_deref(),
            Some("GeForce RTX 4090")
        );
        assert_eq!(
            model_name_from(db, 0x1002, 0x744c).as_deref(),
            Some("Radeon RX 7900 XT/7900 XTX/7900 GRE/7900M")
        );
        assert_eq!(
            model_name_from(db, 0x8086, 0x1234).as_deref(),
            Some("Plain Name")
        );
        assert_eq!(model_name_from(db, 0x10de, 0xffff), None);
        assert_eq!(model_name_from(db, 0x1af4, 0x2571), None);
    }

    #[test]
    fn nvidia_needs_proprietary_driver() {
        assert_eq!(
            gpu(Vendor::Nvidia, 0x2571, "nvidia").support(),
            Support::Supported
        );
        assert_eq!(
            gpu(Vendor::Nvidia, 0x2571, "nouveau").support(),
            Support::NeedsDriver
        );
    }

    #[test]
    fn amd_generations() {
        assert_eq!(amd(110000).support(), Support::Supported); // RDNA3
        assert_eq!(amd(100300).support(), Support::Supported); // RDNA2
        assert_eq!(
            amd(100100).support(),
            Support::Experimental {
                hsa_override: Some("10.3.0")
            }
        );
        assert_eq!(
            amd(90000).support(),
            Support::Experimental {
                hsa_override: Some("9.0.0")
            }
        );
        assert_eq!(
            amd(80003).support(),
            Support::Unsupported(UnsupportedReason::AmdPrePolaris)
        );
        assert_eq!(
            gpu(Vendor::Amd, 0x1234, "amdgpu").support(),
            Support::Unknown
        );
    }

    #[test]
    fn intel_generations() {
        assert_eq!(
            gpu(Vendor::Intel, 0x56a0, "i915").support(),
            Support::Supported
        ); // Arc A770
        assert_eq!(
            gpu(Vendor::Intel, 0x9a49, "i915").support(),
            Support::Supported
        ); // TGL
        assert_eq!(
            gpu(Vendor::Intel, 0x1234, "xe").support(),
            Support::Supported
        );
        assert_eq!(
            gpu(Vendor::Intel, 0x5917, "i915").support(),
            Support::Unsupported(UnsupportedReason::IntelPreGen12)
        );
        assert_eq!(
            gpu(Vendor::Intel, 0xffff, "i915").support(),
            Support::Unknown
        );
    }

    #[test]
    fn virtual_display_is_not_usable() {
        assert!(!gpu(Vendor::Other(0x1234), 0x1111, "bochs-drm").is_usable());
    }

    #[test]
    fn prefers_discrete_over_integrated() {
        let igpu = gpu(Vendor::Intel, 0x46a6, "i915");
        let nv = gpu(Vendor::Nvidia, 0x25a2, "nvidia");
        let list = [igpu.clone(), nv.clone()];
        assert_eq!(preferred(&list), Some(&nv));
        assert!(is_hybrid(&list));
        assert_eq!(preferred(std::slice::from_ref(&igpu)), Some(&igpu));
        assert!(!is_hybrid(&[igpu]));
    }

    #[test]
    fn preferred_skips_unsupported() {
        let vm = gpu(Vendor::Other(0x1234), 0x1111, "bochs-drm");
        assert_eq!(preferred(&[vm]), None);
    }

    #[test]
    fn detects_from_fake_sysfs() {
        let root = std::env::temp_dir().join(format!("rli-sysfs-{}", std::process::id()));
        let card = |n: u32, vendor: &str, device: &str, driver: &str, render: u32| {
            let dev = root.join(format!("sys/class/drm/card{n}/device"));
            std::fs::create_dir_all(dev.join(format!("drm/renderD{render}"))).unwrap();
            std::fs::write(dev.join("vendor"), format!("{vendor}\n")).unwrap();
            std::fs::write(dev.join("device"), format!("{device}\n")).unwrap();
            std::fs::write(
                dev.join("uevent"),
                format!("DRIVER={driver}\nPCI_SLOT_NAME=0000:0{n}:00.0\n"),
            )
            .unwrap();
        };
        card(0, "0x1234", "0x1111", "bochs-drm", 128);
        card(1, "0x1002", "0x73bf", "amdgpu", 129);
        std::fs::create_dir_all(root.join("sys/class/drm/card1-DP-1")).unwrap();
        for (id, target) in [(0, 0), (1, 100300)] {
            let node = root.join(format!("sys/class/kfd/kfd/topology/nodes/{id}"));
            std::fs::create_dir_all(&node).unwrap();
            std::fs::write(
                node.join("properties"),
                format!("cpu_cores_count 8\ngfx_target_version {target}\n"),
            )
            .unwrap();
        }
        let gpus = detect(&root);
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].vendor, Vendor::Other(0x1234));
        assert_eq!(gpus[1].vendor, Vendor::Amd);
        assert_eq!(gpus[1].driver.as_deref(), Some("amdgpu"));
        assert_eq!(gpus[1].render_node.as_deref(), Some("renderD129"));
        assert_eq!(gpus[1].amd_gfx_target, Some(100300));
        assert_eq!(gpus[1].support(), Support::Supported);
    }
}
