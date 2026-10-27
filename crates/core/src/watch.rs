//! Watch folders (SPEC §9): what to convert on each pass, where results go, and the
//! record of already-handled files that survives restarts. Also the autostart entry
//! that starts the background service with the desktop session.

use crate::convert;
use crate::ffmpeg::{ExportPreset, ImportPreset};
use crate::settings::Settings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Results go into a subfolder, so they are never picked up again as input.
pub const IMPORT_OUTPUT: &str = "Ready for Resolve";
pub const EXPORT_OUTPUT: &str = "MP4";
/// A file must be unchanged this long before we touch it (still being copied otherwise).
pub const SETTLE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Import(ImportPreset),
    Export(ExportPreset),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub input: PathBuf,
    pub out_dir: PathBuf,
    pub kind: Kind,
}

/// Files already handled (converted or failed), keyed by path + size + mtime so a file
/// replaced with new content is converted again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    entries: BTreeSet<(PathBuf, u64, u64)>,
}

fn fingerprint(p: &Path) -> Option<(PathBuf, u64, u64)> {
    let m = std::fs::metadata(p).ok()?;
    let mtime = m
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some((p.to_path_buf(), m.len(), mtime))
}

impl Done {
    pub fn default_path() -> PathBuf {
        crate::state::host_xdg_dir("XDG_STATE_HOME", ".local/state").join("vinci/watch.json")
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, serde_json::to_vec(self)?)
    }

    pub fn contains(&self, p: &Path) -> bool {
        fingerprint(p).is_some_and(|f| self.entries.contains(&f))
    }

    pub fn mark(&mut self, p: &Path) {
        if let Some(f) = fingerprint(p) {
            self.entries.insert(f);
        }
    }

    /// Forgets files that no longer exist, so the record doesn't grow forever.
    pub fn prune(&mut self) {
        self.entries.retain(|(p, _, _)| p.exists());
    }
}

/// The conversions to run now, given the settings and what was already done.
pub fn jobs(settings: &Settings, import: ImportPreset, done: &Done, now: SystemTime) -> Vec<Job> {
    let mut out = Vec::new();
    let mut add = |dir: &Option<PathBuf>, sub: &str, kind: Kind| {
        let Some(dir) = dir else { return };
        let handled: BTreeSet<PathBuf> = BTreeSet::new();
        for input in convert::pending(dir, &handled, SETTLE, now) {
            if !done.contains(&input) {
                out.push(Job {
                    input,
                    out_dir: dir.join(sub),
                    kind,
                });
            }
        }
    };
    add(&settings.watch_import, IMPORT_OUTPUT, Kind::Import(import));
    add(
        &settings.watch_export,
        EXPORT_OUTPUT,
        Kind::Export(ExportPreset::Web),
    );
    out
}

pub const AUTOSTART_ID: &str = "io.github.dionatansimioni.Vinci.Watch";

/// How to start Vinci's watch mode from the desktop session, for each way it can be
/// installed.
pub fn service_command(in_flatpak: bool, appimage: Option<&str>, exe: &Path) -> Vec<String> {
    if in_flatpak {
        [
            "flatpak",
            "run",
            "--command=vinci",
            "io.github.dionatansimioni.Vinci",
            "--watch",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    } else if let Some(img) = appimage {
        vec![img.to_string(), "--watch".into()]
    } else {
        vec![exe.to_string_lossy().into_owned(), "--watch".into()]
    }
}

pub fn autostart_path() -> PathBuf {
    crate::state::host_xdg_dir("XDG_CONFIG_HOME", ".config")
        .join(format!("autostart/{AUTOSTART_ID}.desktop"))
}

pub fn autostart_entry(command: &[String]) -> String {
    let exec: Vec<String> = command
        .iter()
        .map(|a| {
            if a.chars().any(|c| " \"'\\$`".contains(c)) {
                format!("\"{}\"", a.replace('\\', "\\\\\\\\").replace('"', "\\\\\""))
            } else {
                a.clone()
            }
        })
        .collect();
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Vinci (background conversions)\n\
         Comment=Converts files placed in Vinci's watch folders\n\
         Exec={}\n\
         Icon=io.github.dionatansimioni.Vinci\n\
         NoDisplay=true\n\
         X-GNOME-Autostart-enabled=true\n",
        exec.join(" ")
    )
}

/// Writes or removes the autostart entry to match the settings.
pub fn sync_autostart(settings: &Settings, command: &[String], path: &Path) -> std::io::Result<()> {
    let wanted = settings.background_service
        && (settings.watch_import.is_some() || settings.watch_export.is_some());
    if wanted {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, autostart_entry(command))
    } else {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rli-watch2-{}-{name}", std::process::id()))
    }

    #[test]
    fn finds_jobs_in_both_folders_and_skips_done_ones() {
        let base = tmp("jobs");
        let (imp, exp) = (base.join("Import"), base.join("Export"));
        std::fs::create_dir_all(imp.join(IMPORT_OUTPUT)).unwrap();
        std::fs::create_dir_all(&exp).unwrap();
        std::fs::write(imp.join("phone.mp4"), "a").unwrap();
        std::fs::write(imp.join("old.mov"), "b").unwrap();
        std::fs::write(imp.join(IMPORT_OUTPUT).join("phone_dnxhr.mov"), "c").unwrap();
        std::fs::write(exp.join("render.mov"), "d").unwrap();
        let settings = Settings {
            watch_import: Some(imp.clone()),
            watch_export: Some(exp.clone()),
            ..Settings::default()
        };
        let mut done = Done::default();
        done.mark(&imp.join("old.mov"));
        let later = SystemTime::now() + Duration::from_secs(60);
        let jobs = jobs(&settings, ImportPreset::DnxhrHq, &done, later);
        std::fs::remove_dir_all(&base).unwrap();
        assert_eq!(
            jobs,
            [
                Job {
                    input: imp.join("phone.mp4"),
                    out_dir: imp.join(IMPORT_OUTPUT),
                    kind: Kind::Import(ImportPreset::DnxhrHq)
                },
                Job {
                    input: exp.join("render.mov"),
                    out_dir: exp.join(EXPORT_OUTPUT),
                    kind: Kind::Export(ExportPreset::Web)
                },
            ]
        );
    }

    #[test]
    fn changed_file_is_converted_again_and_record_persists() {
        let base = tmp("done");
        std::fs::create_dir_all(&base).unwrap();
        let f = base.join("clip.mp4");
        std::fs::write(&f, "first").unwrap();
        let mut done = Done::default();
        done.mark(&f);
        assert!(done.contains(&f));
        let path = base.join("state/watch.json");
        done.save(&path).unwrap();
        assert!(Done::load(&path).contains(&f));
        std::fs::write(&f, "different length").unwrap();
        assert!(!done.contains(&f), "new content means a new conversion");
        std::fs::remove_file(&f).unwrap();
        done.prune();
        assert_eq!(done, Done::default());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn service_command_per_packaging() {
        let exe = Path::new("/usr/bin/vinci");
        assert_eq!(
            service_command(true, None, exe)[..3],
            ["flatpak", "run", "--command=vinci"]
        );
        assert_eq!(
            service_command(false, Some("/home/u/Apps/Vinci.AppImage"), exe),
            ["/home/u/Apps/Vinci.AppImage", "--watch"]
        );
        assert_eq!(
            service_command(false, None, exe),
            ["/usr/bin/vinci", "--watch"]
        );
    }

    #[test]
    fn autostart_follows_settings() {
        let path = tmp("autostart").join("autostart/x.desktop");
        let cmd = service_command(false, Some("/home/my user/Vinci.AppImage"), Path::new("/x"));
        let mut s = Settings {
            watch_import: Some("/v".into()),
            ..Settings::default()
        };
        sync_autostart(&s, &cmd, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("Exec=\"/home/my user/Vinci.AppImage\" --watch\n"),
            "{text}"
        );
        s.background_service = false;
        sync_autostart(&s, &cmd, &path).unwrap();
        assert!(!path.exists());
        s.background_service = true;
        s.watch_import = None;
        sync_autostart(&s, &cmd, &path).unwrap();
        assert!(!path.exists(), "no folders, no service");
        std::fs::remove_dir_all(tmp("autostart")).unwrap();
    }

    #[test]
    fn autostart_entry_validates() {
        if std::process::Command::new("desktop-file-validate")
            .arg("--help")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tmp("validate");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join(format!("{AUTOSTART_ID}.desktop"));
        std::fs::write(
            &f,
            autostart_entry(&service_command(
                false,
                Some("/home/my user/V.AppImage"),
                Path::new("/x"),
            )),
        )
        .unwrap();
        let out = std::process::Command::new("desktop-file-validate")
            .arg(&f)
            .output()
            .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let msg = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success() && !msg.contains("error"), "{msg}");
    }
}
