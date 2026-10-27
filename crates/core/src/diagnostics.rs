//! The "Copy diagnostic report" text. Nothing is ever sent anywhere: the user pastes it
//! into an issue if they want help. Paths under $HOME are anonymized.

use crate::distro::Distro;
use crate::gpu::Gpu;
use crate::hardware::Report;
use crate::state::State;
use crate::tools::HostTools;
use std::fmt::Write;
use std::path::Path;

pub struct Input<'a> {
    pub app_version: &'a str,
    pub recipe_revision: u32,
    pub distro: Option<&'a Distro>,
    pub gpus: &'a [Gpu],
    pub report: &'a Report,
    pub ram_bytes: Option<u64>,
    pub free_disk_bytes: Option<u64>,
    pub tools: Option<&'a HostTools>,
    pub state: &'a State,
    /// Summary and details of the last failure, if any.
    pub error: Option<(&'a str, &'a str)>,
    pub home: &'a Path,
}

fn gib(b: u64) -> String {
    format!("{:.1} GiB", b as f64 / (1u64 << 30) as f64)
}

pub fn report(i: &Input) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "### Vinci diagnostic report");
    let _ = writeln!(
        s,
        "- Vinci: {} (recipe rev {})",
        i.app_version, i.recipe_revision
    );
    let _ = writeln!(
        s,
        "- Distro: {}",
        i.distro.map_or("unknown".into(), |d| format!(
            "{} (id={}, like={})",
            d.pretty_name,
            d.id,
            d.id_like.join(",")
        ))
    );
    let _ = writeln!(
        s,
        "- RAM: {} · free disk: {}",
        i.ram_bytes.map_or("?".into(), gib),
        i.free_disk_bytes.map_or("?".into(), gib)
    );
    for g in i.gpus {
        let _ = writeln!(
            s,
            "- GPU: {} [{:04x}] driver={} support={:?}{}",
            g.display_name(),
            g.device_id,
            g.driver.as_deref().unwrap_or("none"),
            g.support(),
            g.amd_gfx_target
                .map_or(String::new(), |t| format!(" gfx={t}")),
        );
    }
    if let Some(c) = &i.report.chosen_gpu {
        let _ = writeln!(s, "- Chosen GPU: {}", c.display_name());
    }
    if let Some(t) = i.tools {
        let _ = writeln!(
            s,
            "- podman: {} · distrobox: {:?}",
            t.podman
                .as_ref()
                .map_or("missing".into(), |v| v.to_string()),
            t.distrobox
        );
    }
    match &i.state.installation {
        Some(inst) => {
            let _ = writeln!(
                s,
                "- Installed: {:?} {} in container {}",
                inst.edition, inst.version, inst.container
            );
        }
        None => {
            let _ = writeln!(s, "- Installed: nothing");
        }
    }
    if !i.report.findings.is_empty() {
        let _ = writeln!(
            s,
            "- Check findings: {:?}",
            i.report
                .findings
                .iter()
                .map(|f| format!("{f:?}"))
                .collect::<Vec<_>>()
        );
    }
    if let Some((summary, details)) = i.error {
        let _ = writeln!(
            s,
            "\n#### Last error\n{summary}\n```\n{}\n```",
            details.trim_end()
        );
    }
    anonymize(&s, i.home)
}

/// Replaces the home directory (which usually contains the user's name) with `~`.
pub fn anonymize(text: &str, home: &Path) -> String {
    let home = home.to_string_lossy();
    if home.len() <= 1 {
        return text.to_string();
    }
    text.replace(home.as_ref(), "~")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::Vendor;
    use crate::hardware;

    #[test]
    fn report_has_the_essentials_and_hides_home() {
        let gpus = vec![Gpu {
            vendor: Vendor::Nvidia,
            device_id: 0x2571,
            driver: Some("nvidia".into()),
            pci_slot: None,
            render_node: None,
            amd_gfx_target: None,
        }];
        let sys = hardware::System {
            gpus: gpus.clone(),
            ram_bytes: Some(32 << 30),
            free_disk_bytes: Some(100 << 30),
        };
        let report = hardware::check(&sys);
        let distro = Distro::parse("ID=fedora\nPRETTY_NAME=\"Fedora Linux 43\"\n");
        let text = super::report(&Input {
            app_version: "0.1.0",
            recipe_revision: 1,
            distro: Some(&distro),
            gpus: &gpus,
            report: &report,
            ram_bytes: sys.ram_bytes,
            free_disk_bytes: sys.free_disk_bytes,
            tools: None,
            state: &State::default(),
            error: Some((
                "This step failed",
                "E: cannot open /home/maria/Downloads/x.zip",
            )),
            home: Path::new("/home/maria"),
        });
        assert!(text.contains("Vinci: 0.1.0 (recipe rev 1)"));
        assert!(text.contains("Fedora Linux 43"));
        assert!(text.contains("RAM: 32.0 GiB"));
        assert!(text.contains("driver=nvidia support=Supported"));
        assert!(text.contains("Installed: nothing"));
        assert!(text.contains("~/Downloads/x.zip"));
        assert!(!text.contains("maria"), "user name leaked: {text}");
    }

    #[test]
    fn anonymize_ignores_root_home() {
        assert_eq!(anonymize("/usr/bin", Path::new("/")), "/usr/bin");
    }
}
