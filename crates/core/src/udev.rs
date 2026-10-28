//! udev rules for Blackmagic USB devices (Studio dongle, Speed Editor, panels).
//!
//! Resolve's installer writes them inside the container, where the host's udev never
//! sees them, so Vinci copies them to the host's /etc/udev/rules.d as root. The
//! container is writable by the user, so a rule from it must never be able to run a
//! program as root: every line goes through a strict allow-list first.

use crate::host::{argv, Runner};
use crate::recipe::Recipe;
use std::path::{Path, PathBuf};

pub const HOST_DIR: &str = "/etc/udev/rules.d";

/// Keys a rule may compare against.
const MATCH_KEYS: &[&str] = &[
    "ACTION",
    "SUBSYSTEM",
    "SUBSYSTEMS",
    "KERNEL",
    "KERNELS",
    "DRIVER",
    "DRIVERS",
    "TAG",
    "TAGS",
];
/// Keys a rule may assign. Notably absent: RUN, PROGRAM, IMPORT, OWNER, NAME, SYMLINK,
/// and GROUP (groups like "resolve" don't exist on the host).
const ASSIGN_KEYS: &[&str] = &["MODE", "TAG"];
/// Blackmagic Design's USB vendor ID. Permission changes must be limited to it: Resolve's
/// own 75-davincipanel.rules opens *every* hidraw device (keyboards, FIDO keys) with 0777.
pub const BLACKMAGIC_VENDOR: &str = "1edb";

/// Our scoped replacement for that broad rule: hidraw access for Blackmagic devices only.
pub const SCOPED_HIDRAW_RULE: (&str, &str) = (
    "70-vinci-blackmagic-hidraw.rules",
    "# Written by Vinci: hidraw access for Blackmagic panels and keyboards only.\nSUBSYSTEM==\"hidraw\", ATTRS{idVendor}==\"1edb\", MODE=\"0666\"\n",
);

/// `ATTRS{idVendor}` → ("ATTRS", Some("idVendor")).
fn split_key(key: &str) -> (&str, Option<&str>) {
    match key.split_once('{') {
        Some((k, rest)) => (k, rest.strip_suffix('}')),
        None => (key, None),
    }
}

fn token_ok(tok: &str) -> bool {
    let tok = tok.trim();
    let ops = ["==", "!=", "+=", ":=", "="];
    let Some((op, (key, value))) = ops
        .iter()
        .find_map(|op| tok.split_once(op).map(|kv| (*op, kv)))
    else {
        return false;
    };
    let value = value.trim();
    if !(value.len() >= 2 && value.starts_with('"') && value.ends_with('"')) {
        return false;
    }
    let inner = &value[1..value.len() - 1];
    if inner.contains('`') || inner.contains('"') {
        return false;
    }
    let (name, arg) = split_key(key.trim());
    let arg_ok = arg.is_none_or(|a| {
        a.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._".contains(c))
    });
    if !arg_ok {
        return false;
    }
    let matching = op == "==" || op == "!=";
    match name {
        "ATTR" | "ATTRS" => matching && arg.is_some(),
        "ENV" => arg.is_some(),
        _ if matching => MATCH_KEYS.contains(&name),
        _ => ASSIGN_KEYS.contains(&name) && !inner.contains('$') && !inner.contains('%'),
    }
}

/// True if every line is a comment, blank, or a rule made only of allowed tokens, and
/// every permission change is limited to Blackmagic's vendor ID.
pub fn is_safe(rules: &str) -> bool {
    let vendor_match = format!("{{idVendor}}==\"{BLACKMAGIC_VENDOR}\"");
    rules.lines().all(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return true;
        }
        let tokens: Vec<&str> = line.split(',').map(str::trim).collect();
        let changes_mode = tokens.iter().any(|t| t.starts_with("MODE"));
        let scoped = tokens.iter().any(|t| {
            (t.starts_with("ATTRS{") || t.starts_with("ATTR{")) && t.ends_with(&vendor_match)
        });
        tokens.iter().all(|t| token_ok(t)) && (!changes_mode || scoped)
    })
}

fn safe_name(name: &str) -> bool {
    name.ends_with(".rules")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// Copies the recipe's rule files out of the container into `staging` (under $HOME,
/// which the container shares). Returns the staged files that passed validation, and
/// the names that were rejected.
pub fn stage(
    runner: &dyn Runner,
    recipe: &Recipe,
    distrobox: &str,
    container: &str,
    staging: &Path,
) -> std::io::Result<(Vec<PathBuf>, Vec<String>)> {
    let _ = std::fs::remove_dir_all(staging);
    std::fs::create_dir_all(staging)?;
    let script = r#"set -e; dest="$1"; shift; for p in "$@"; do for f in /$p; do [ -f "$f" ] && cp "$f" "$dest/"; done; done; true"#;
    let mut cmd = argv(&[
        distrobox,
        "enter",
        "--name",
        container,
        "--",
        "sh",
        "-c",
        script,
        "sh",
        &staging.to_string_lossy(),
    ]);
    cmd.extend(recipe.resolve.udev_rules.iter().cloned());
    runner.run(&cmd)?;

    let (name, text) = SCOPED_HIDRAW_RULE;
    std::fs::write(staging.join(name), text)?;
    classify(staging)
}

/// Splits the files in `dir` into rules safe to install and names that were rejected.
fn classify(dir: &Path) -> std::io::Result<(Vec<PathBuf>, Vec<String>)> {
    let mut ok = Vec::new();
    let mut rejected = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(e.path()).unwrap_or_default();
        if safe_name(&name) && is_safe(&text) {
            ok.push(e.path());
        } else {
            rejected.push(name);
        }
    }
    Ok((ok, rejected))
}

/// Root command that installs the staged rules and makes udev apply them now.
pub fn install_command(files: &[PathBuf]) -> Vec<String> {
    let mut v = argv(&[
        "sh",
        "-c",
        r#"set -e; install -m 0644 "$@" /etc/udev/rules.d/; udevadm control --reload; udevadm trigger --subsystem-match=usb --subsystem-match=hidraw"#,
        "sh",
    ]);
    v.extend(files.iter().map(|f| f.to_string_lossy().into_owned()));
    v
}

/// Root command that removes rules Vinci installed (by name, inside /etc/udev/rules.d).
pub fn remove_command(installed: &[PathBuf]) -> Vec<String> {
    let mut v = argv(&[
        "sh",
        "-c",
        r#"for f in "$@"; do rm -f "/etc/udev/rules.d/$(basename "$f")"; done; udevadm control --reload"#,
        "sh",
    ]);
    v.extend(installed.iter().map(|f| f.to_string_lossy().into_owned()));
    v
}

/// Where each staged file ends up on the host.
pub fn host_paths(files: &[PathBuf]) -> Vec<PathBuf> {
    files
        .iter()
        .filter_map(|f| f.file_name())
        .map(|n| Path::new(HOST_DIR).join(n))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::fake::FakeRunner;

    /// The exact rules shipped by Resolve 21.1.2.
    const BLACKMAGIC: &str = r#"SUBSYSTEM=="usb", ATTRS{idVendor}=="1edb", MODE="0666"
"#;
    const KEYBOARD: &str = r#"SUBSYSTEMS=="usb", ENV{.LOCAL_ifNum}="$attr{bInterfaceNumber}"

# Editor Keyboard
SUBSYSTEM=="hidraw", KERNEL=="hidraw*", ATTRS{idVendor}=="1edb", ATTRS{idProduct}=="da0b", ENV{.LOCAL_ifNum}=="04", MODE="0666"
"#;

    #[test]
    fn accepts_blackmagic_rules() {
        assert!(is_safe(BLACKMAGIC));
        assert!(is_safe(KEYBOARD));
        assert!(is_safe(SCOPED_HIDRAW_RULE.1));
    }

    #[test]
    fn rejects_anything_that_runs_or_renames() {
        for evil in [
            r#"SUBSYSTEM=="usb", RUN+="/bin/sh -c 'id > /tmp/pwn'""#,
            r#"ACTION=="add", PROGRAM="/usr/bin/evil""#,
            r#"SUBSYSTEM=="usb", IMPORT{program}="/bin/evil""#,
            r#"KERNEL=="sda", NAME="evil""#,
            r#"KERNEL=="sda", SYMLINK+="evil""#,
            r#"KERNEL=="sda", OWNER="attacker""#,
            r#"SUBSYSTEM=="usb", MODE="$env{X}""#,
            r#"SUBSYSTEM=="usb" MODE="0666""#,
            r#"ATTRS{idVendor}="1edb""#,
            r#"FOO=="bar""#,
            // Resolve's own panel rule: every hidraw device, world-writable, unknown group.
            r#"KERNEL=="hidraw*", SUBSYSTEM=="hidraw", MODE="0777", GROUP="resolve""#,
            r#"SUBSYSTEM=="hidraw", MODE="0666""#,
            r#"SUBSYSTEM=="usb", ATTRS{idVendor}=="dead", MODE="0666""#,
        ] {
            assert!(!is_safe(evil), "accepted: {evil}");
        }
        assert!(
            !is_safe(&format!("{BLACKMAGIC}RUN+=\"/bin/x\"\n")),
            "one bad line rejects the file"
        );
    }

    #[test]
    fn stage_always_adds_the_scoped_hidraw_rule() {
        let staging = std::env::temp_dir().join(format!("rli-udev-{}", std::process::id()));
        let runner = FakeRunner::default().respond(&[], "", true);
        let (ok, rejected) = stage(
            &runner,
            &Recipe::bundled(),
            "distrobox",
            "vinci-studio",
            &staging,
        )
        .unwrap();
        let call = runner.calls()[0].clone();
        std::fs::remove_dir_all(&staging).unwrap();
        assert_eq!(ok, [staging.join(SCOPED_HIDRAW_RULE.0)]);
        assert!(rejected.is_empty());
        assert!(call.contains(&"opt/resolve/share/etc/udev/rules.d/*.rules".to_string()));
    }

    #[test]
    fn classify_keeps_only_safe_well_named_files() {
        let dir = std::env::temp_dir().join(format!("rli-udev-classify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("99-BlackmagicDevices.rules"), BLACKMAGIC).unwrap();
        std::fs::write(
            dir.join("75-davincipanel.rules"),
            "KERNEL==\"hidraw*\", SUBSYSTEM==\"hidraw\", MODE=\"0777\", GROUP=\"resolve\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("99-evil.rules"), "RUN+=\"/bin/x\"\n").unwrap();
        std::fs::write(dir.join("bad name.rules"), BLACKMAGIC).unwrap();
        let (ok, mut rejected) = classify(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        rejected.sort();
        assert_eq!(ok, [dir.join("99-BlackmagicDevices.rules")]);
        assert_eq!(
            rejected,
            ["75-davincipanel.rules", "99-evil.rules", "bad name.rules"]
        );
    }

    #[test]
    fn commands_target_etc_udev() {
        let files = [PathBuf::from(
            "/home/u/.cache/vinci/udev/99-BlackmagicDevices.rules",
        )];
        let install = install_command(&files);
        assert!(
            install[2].contains("/etc/udev/rules.d/") && install[2].contains("udevadm trigger")
        );
        assert_eq!(
            install.last().unwrap(),
            "/home/u/.cache/vinci/udev/99-BlackmagicDevices.rules"
        );
        assert_eq!(
            host_paths(&files),
            [PathBuf::from(
                "/etc/udev/rules.d/99-BlackmagicDevices.rules"
            )]
        );
        assert!(remove_command(&host_paths(&files))[2].contains("rm -f"));
    }
}
