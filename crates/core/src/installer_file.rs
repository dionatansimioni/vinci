//! Recognizes Blackmagic's Linux installer files and picks the best one in a folder.
//!
//! Known names: `DaVinci_Resolve_20.2.1_Linux.zip`,
//! `DaVinci_Resolve_Studio_20.2.1_Linux.run`.

use crate::version::Version;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Edition {
    Free,
    Studio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Zip,
    Run,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallerFile {
    pub path: PathBuf,
    pub edition: Edition,
    pub version: Version,
    pub kind: FileKind,
}

/// Parses an installer filename. Returns `None` for anything that isn't one.
pub fn parse_filename(name: &str) -> Option<(Edition, Version, FileKind)> {
    let (stem, kind) = if let Some(s) = name.strip_suffix(".zip") {
        (s, FileKind::Zip)
    } else {
        (name.strip_suffix(".run")?, FileKind::Run)
    };
    let rest = stem
        .strip_prefix("DaVinci_Resolve_")?
        .strip_suffix("_Linux")?;
    let (edition, ver) = match rest.strip_prefix("Studio_") {
        Some(v) => (Edition::Studio, v),
        None => (Edition::Free, rest),
    };
    Some((edition, ver.parse().ok()?, kind))
}

/// Suffixes browsers use for in-progress downloads (Firefox, Chromium, Epiphany, wget).
const PARTIAL_SUFFIXES: &[&str] = &[".part", ".crdownload", ".download", ".tmp"];

/// Installers in `dir` that look fully downloaded: non-empty and with no in-progress
/// sibling (Firefox keeps an empty `name.zip` next to `name.zip.part` while downloading).
/// Callers should still confirm the size is stable before using one.
pub fn ready_in_dir(dir: &Path) -> Vec<InstallerFile> {
    find_in_dir(dir)
        .into_iter()
        .filter(|f| {
            let size = std::fs::metadata(&f.path).map(|m| m.len()).unwrap_or(0);
            let busy = PARTIAL_SUFFIXES.iter().any(|sfx| {
                let mut p = f.path.clone().into_os_string();
                p.push(sfx);
                Path::new(&p).exists()
            });
            size > 0 && !busy
        })
        .collect()
}

/// Finds installer files directly inside `dir`, newest version first.
/// A `.run` wins over a `.zip` of the same version since it needs no extraction.
pub fn find_in_dir(dir: &Path) -> Vec<InstallerFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<InstallerFile> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let (edition, version, kind) = parse_filename(&name)?;
            Some(InstallerFile {
                path: e.path(),
                edition,
                version,
                kind,
            })
        })
        .collect();
    found.sort_by(|a, b| {
        b.version
            .cmp(&a.version)
            .then_with(|| (a.kind == FileKind::Zip).cmp(&(b.kind == FileKind::Zip)))
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_free_and_studio() {
        let (e, v, k) = parse_filename("DaVinci_Resolve_20.2.1_Linux.zip").unwrap();
        assert_eq!(
            (e, v.to_string(), k),
            (Edition::Free, "20.2.1".into(), FileKind::Zip)
        );
        let (e, v, k) = parse_filename("DaVinci_Resolve_Studio_19.1.4_Linux.run").unwrap();
        assert_eq!(
            (e, v.to_string(), k),
            (Edition::Studio, "19.1.4".into(), FileKind::Run)
        );
    }

    #[test]
    fn ignores_unrelated_files() {
        for name in [
            "DaVinci_Resolve_20.2.1_Windows.zip",
            "DaVinci_Resolve_20.2.1_Linux.tar.gz",
            "DaVinci_Resolve_Studio_abc_Linux.zip",
            "notes.txt",
        ] {
            assert!(parse_filename(name).is_none(), "{name}");
        }
    }

    #[test]
    fn finds_newest_and_prefers_run() {
        let dir = std::env::temp_dir().join(format!("rli-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for n in [
            "DaVinci_Resolve_19.1_Linux.zip",
            "DaVinci_Resolve_Studio_20.2_Linux.zip",
            "DaVinci_Resolve_Studio_20.2_Linux.run",
            "random.zip",
        ] {
            std::fs::write(dir.join(n), b"").unwrap();
        }
        let found = find_in_dir(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].edition, Edition::Studio);
        assert_eq!(found[0].kind, FileKind::Run);
        assert_eq!(found[2].version.to_string(), "19.1");
    }

    #[test]
    fn ignores_downloads_still_in_progress() {
        let dir = std::env::temp_dir().join(format!("rli-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let w = |n: &str, data: &[u8]| std::fs::write(dir.join(n), data).unwrap();
        w("DaVinci_Resolve_21.1.2_Linux.zip", b""); // Firefox placeholder
        w("DaVinci_Resolve_21.1.2_Linux.zip.part", b"partial");
        w("DaVinci_Resolve_Studio_21.1.2_Linux.zip", b"data");
        w(
            "DaVinci_Resolve_Studio_21.1.2_Linux.zip.crdownload",
            b"partial",
        );
        w("DaVinci_Resolve_21.1.1_Linux.zip", b"done");
        let ready = ready_in_dir(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].version.to_string(), "21.1.1");
    }

    #[test]
    fn missing_dir_is_empty() {
        assert!(find_in_dir(Path::new("/nonexistent/rli")).is_empty());
    }
}
