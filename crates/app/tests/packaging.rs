//! Keeps the packaging files consistent with each other and with the code: one app ID
//! everywhere, the permissions the app relies on, and valid desktop/AppStream files.

use std::path::Path;
use std::process::Command;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const APP_ID: &str = "io.github.dionatansimioni.Vinci";

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(ROOT).join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn app_id_is_the_same_everywhere() {
    assert!(read("crates/app/src/main.rs").contains(&format!("\"{APP_ID}\"")));
    let desktop = read(&format!("data/{APP_ID}.desktop"));
    assert!(desktop.contains(&format!("Icon={APP_ID}\n")));
    assert!(desktop.contains("Exec=vinci\n"));
    let meta = read(&format!("data/{APP_ID}.metainfo.xml"));
    assert!(meta.contains(&format!("<id>{APP_ID}</id>")));
    assert!(meta.contains(&format!(
        "<launchable type=\"desktop-id\">{APP_ID}.desktop</launchable>"
    )));
    assert!(Path::new(ROOT)
        .join(format!("data/icons/{APP_ID}.svg"))
        .exists());
    let manifest = read(&format!("build-aux/flatpak/{APP_ID}.yml"));
    assert!(manifest.starts_with(&format!("id: {APP_ID}\n")));
    assert!(manifest.contains("command: vinci\n"));
}

#[test]
fn metainfo_states_it_is_unofficial() {
    let meta = read(&format!("data/{APP_ID}.metainfo.xml"));
    assert!(meta.contains("not affiliated with Blackmagic Design"));
    assert!(meta.contains("sem afiliação com a Blackmagic Design"));
}

#[test]
fn flatpak_has_the_permissions_the_code_needs() {
    let manifest = read(&format!("build-aux/flatpak/{APP_ID}.yml"));
    for arg in [
        // host::ExecMode::FlatpakSpawn
        "--talk-name=org.freedesktop.Flatpak",
        // installer in ~/Downloads, launcher in the host's ~/.local/share
        "--filesystem=home",
        // remote::Http
        "--share=network",
        "--device=dri",
    ] {
        assert!(
            manifest.contains(&format!("  - {arg}\n")),
            "missing finish-arg {arg}"
        );
    }
}

/// Runs the freedesktop validators when they are installed (CI installs them).
#[test]
fn official_validators_accept_our_files() {
    let desktop = Path::new(ROOT).join(format!("data/{APP_ID}.desktop"));
    if let Ok(out) = Command::new("desktop-file-validate").arg(&desktop).output() {
        let msg = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success() && msg.trim().is_empty(), "{msg}");
    }
    let meta = Path::new(ROOT).join(format!("data/{APP_ID}.metainfo.xml"));
    if let Ok(out) = Command::new("appstreamcli")
        .args(["validate", "--no-net"])
        .arg(&meta)
        .output()
    {
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}
