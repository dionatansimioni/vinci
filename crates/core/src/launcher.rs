//! The host-side menu entry that starts Resolve inside its container, plus the
//! `.drp`/`.drt` file associations.
//!
//! We write our own `.desktop` instead of `distrobox-export` so the GPU environment
//! (PRIME offload, HSA override, Qt platform) is baked into `Exec`.

use crate::gpu::{self, Gpu, Support, Vendor};
use crate::recipe::Recipe;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const DESKTOP_ID: &str = "io.github.dionatansimioni.Vinci.Resolve";
pub const ICON_NAME: &str = "io.github.dionatansimioni.Vinci.Resolve";
pub const MIME_XML_NAME: &str = "io.github.dionatansimioni.Vinci.Resolve.xml";

/// Environment for Resolve: recipe defaults, vendor extras, then what this machine needs.
pub fn environment(recipe: &Recipe, gpus: &[Gpu], chosen: &Gpu) -> BTreeMap<String, String> {
    let mut env = recipe.launcher.env.clone();
    let vendor = match chosen.vendor {
        Vendor::Nvidia => Some(&recipe.gpu.nvidia),
        Vendor::Amd => Some(&recipe.gpu.amd),
        Vendor::Intel => Some(&recipe.gpu.intel),
        Vendor::Other(_) => None,
    };
    if let Some(v) = vendor {
        env.extend(v.env.clone());
    }
    if let Support::Experimental {
        hsa_override: Some(gfx),
    } = chosen.support()
    {
        env.insert("HSA_OVERRIDE_GFX_VERSION".into(), gfx.into());
    }
    if gpu::is_hybrid(gpus) {
        match chosen.vendor {
            Vendor::Nvidia => {
                env.insert("__NV_PRIME_RENDER_OFFLOAD".into(), "1".into());
                env.insert("__GLX_VENDOR_LIBRARY_NAME".into(), "nvidia".into());
                env.insert("__VK_LAYER_NV_optimus".into(), "NVIDIA_only".into());
            }
            _ => {
                if let Some(slot) = &chosen.pci_slot {
                    env.insert("DRI_PRIME".into(), dri_prime_tag(slot));
                }
            }
        }
    }
    env
}

/// `0000:03:00.0` → `pci-0000_03_00_0`, the form Mesa accepts in `DRI_PRIME`.
fn dri_prime_tag(slot: &str) -> String {
    format!("pci-{}", slot.replace([':', '.'], "_"))
}

/// Quotes one argument for a desktop entry `Exec` key (Desktop Entry Spec, "Exec key").
fn quote_exec_arg(arg: &str) -> String {
    let reserved = |c: char| " \t\n\"'\\><~|&;$*?#()`".contains(c);
    if !arg.is_empty() && !arg.chars().any(reserved) {
        return arg.replace('%', "%%");
    }
    let mut q = String::from("\"");
    for c in arg.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                q.push('\\');
                q.push(c);
            }
            '%' => q.push_str("%%"),
            _ => q.push(c),
        }
    }
    q.push('"');
    // The .desktop parser unescapes backslashes once before the Exec rules apply.
    q.replace('\\', "\\\\")
}

pub struct Entry<'a> {
    pub distrobox: &'a str,
    pub container: &'a str,
    /// e.g. "DaVinci Resolve" or "DaVinci Resolve Studio"
    pub name: &'a str,
    pub env: &'a BTreeMap<String, String>,
}

/// The `.desktop` file contents.
pub fn desktop_file(recipe: &Recipe, e: &Entry) -> String {
    let mut argv: Vec<String> = vec![
        e.distrobox.into(),
        "enter".into(),
        "--name".into(),
        e.container.into(),
        "--".into(),
        "env".into(),
    ];
    argv.extend(e.env.iter().map(|(k, v)| format!("{k}={v}")));
    argv.push(format!("{}/bin/resolve", recipe.resolve.install_dir));
    let exec: Vec<String> = argv.iter().map(|a| quote_exec_arg(a)).collect();
    let mime: String = recipe
        .launcher
        .mime_types
        .iter()
        .map(|m| format!("{m};"))
        .collect();
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={name}\n\
         GenericName=Video Editor\n\
         Comment=Installed by Vinci (unofficial)\n\
         Exec={exec} %f\n\
         Icon={ICON_NAME}\n\
         Terminal=false\n\
         Categories=AudioVideo;Video;AudioVideoEditing;\n\
         MimeType={mime}\n\
         StartupWMClass=resolve\n\
         X-Vinci-Container={container}\n",
        name = e.name,
        exec = exec.join(" "),
        container = e.container,
    )
}

/// shared-mime-info package declaring Resolve project and timeline files.
pub fn mime_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="application/x-resolveproj">
    <comment>DaVinci Resolve project</comment>
    <glob pattern="*.drp"/>
  </mime-type>
  <mime-type type="application/x-resolvetimeline">
    <comment>DaVinci Resolve timeline</comment>
    <glob pattern="*.drt"/>
  </mime-type>
</mime-info>
"#
    .to_string()
}

/// Where each generated file goes under `$XDG_DATA_HOME` (usually `~/.local/share`).
pub struct Paths {
    pub desktop: PathBuf,
    pub mime: PathBuf,
    pub icon: PathBuf,
}

pub fn paths(data_home: &Path) -> Paths {
    Paths {
        desktop: data_home.join(format!("applications/{DESKTOP_ID}.desktop")),
        mime: data_home.join(format!("mime/packages/{MIME_XML_NAME}")),
        icon: data_home.join(format!("icons/hicolor/256x256/apps/{ICON_NAME}.png")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(vendor: Vendor, device_id: u16, driver: &str, slot: &str) -> Gpu {
        Gpu {
            vendor,
            device_id,
            driver: Some(driver.into()),
            pci_slot: Some(slot.into()),
            render_node: None,
            amd_gfx_target: None,
        }
    }

    #[test]
    fn single_nvidia_gets_only_recipe_env() {
        let r = Recipe::bundled();
        let nv = g(Vendor::Nvidia, 0x2571, "nvidia", "0000:06:10.0");
        let env = environment(&r, std::slice::from_ref(&nv), &nv);
        assert_eq!(env.get("QT_QPA_PLATFORM").map(String::as_str), Some("xcb"));
        assert!(!env.contains_key("__NV_PRIME_RENDER_OFFLOAD"));
    }

    #[test]
    fn hybrid_nvidia_laptop_uses_prime_offload() {
        let r = Recipe::bundled();
        let igpu = g(Vendor::Intel, 0x46a6, "i915", "0000:00:02.0");
        let nv = g(Vendor::Nvidia, 0x25a2, "nvidia", "0000:01:00.0");
        let env = environment(&r, &[igpu, nv.clone()], &nv);
        assert_eq!(env["__NV_PRIME_RENDER_OFFLOAD"], "1");
        assert_eq!(env["__GLX_VENDOR_LIBRARY_NAME"], "nvidia");
    }

    #[test]
    fn hybrid_amd_uses_dri_prime_and_hsa_override() {
        let r = Recipe::bundled();
        let igpu = g(Vendor::Intel, 0x46a6, "i915", "0000:00:02.0");
        let amd = Gpu {
            amd_gfx_target: Some(100100),
            ..g(Vendor::Amd, 0x731f, "amdgpu", "0000:03:00.0")
        };
        let env = environment(&r, &[igpu, amd.clone()], &amd);
        assert_eq!(env["DRI_PRIME"], "pci-0000_03_00_0");
        assert_eq!(env["HSA_OVERRIDE_GFX_VERSION"], "10.3.0");
    }

    #[test]
    fn quotes_exec_arguments_per_spec() {
        assert_eq!(quote_exec_arg("plain"), "plain");
        assert_eq!(quote_exec_arg("50%"), "50%%");
        assert_eq!(quote_exec_arg("/home/my user/x"), "\"/home/my user/x\"");
        assert_eq!(quote_exec_arg("a$b"), "\"a\\\\$b\"");
    }

    #[test]
    fn desktop_file_launches_resolve_in_container() {
        let r = Recipe::bundled();
        let mut env = BTreeMap::new();
        env.insert("QT_QPA_PLATFORM".to_string(), "xcb".to_string());
        let text = desktop_file(
            &r,
            &Entry {
                distrobox: "/home/u/.local/share/vinci/distrobox/bin/distrobox",
                container: "vinci-free",
                name: "DaVinci Resolve",
                env: &env,
            },
        );
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.contains(
            "Exec=/home/u/.local/share/vinci/distrobox/bin/distrobox enter --name vinci-free -- env QT_QPA_PLATFORM=xcb /opt/resolve/bin/resolve %f\n"
        ));
        assert!(
            text.contains("MimeType=application/x-resolveproj;application/x-resolvetimeline;\n")
        );
        assert!(text.contains("Name=DaVinci Resolve\n"));
    }

    #[test]
    fn desktop_file_validates_with_desktop_file_validate() {
        // Only runs where desktop-file-utils is installed; otherwise the check is skipped.
        let Ok(out) = std::process::Command::new("desktop-file-validate")
            .arg("--help")
            .output()
        else {
            return;
        };
        let _ = out;
        let r = Recipe::bundled();
        let env = environment(
            &r,
            &[],
            &g(Vendor::Nvidia, 0x2571, "nvidia", "0000:06:10.0"),
        );
        let text = desktop_file(
            &r,
            &Entry {
                distrobox: "/home/my user/bin/distrobox",
                container: "vinci-free",
                name: "DaVinci Resolve",
                env: &env,
            },
        );
        let dir = std::env::temp_dir().join(format!("rli-desktop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("{DESKTOP_ID}.desktop"));
        std::fs::write(&file, text).unwrap();
        let out = std::process::Command::new("desktop-file-validate")
            .arg(&file)
            .output()
            .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let msg = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success() && !msg.contains("error"), "{msg}");
    }

    #[test]
    fn mime_xml_declares_globs() {
        let x = mime_xml();
        assert!(x.contains("*.drp") && x.contains("*.drt"));
        let p = paths(Path::new("/home/u/.local/share"));
        assert!(p
            .mime
            .ends_with("mime/packages/io.github.dionatansimioni.Vinci.Resolve.xml"));
        assert!(p
            .desktop
            .ends_with("applications/io.github.dionatansimioni.Vinci.Resolve.desktop"));
    }
}
