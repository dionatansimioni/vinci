//! Updating Vinci itself (AppImage only: Flatpak updates come from the store). The latest
//! GitHub release is checked on startup; the new AppImage is downloaded next to the running
//! one, checked against the SHA-256 GitHub publishes for it, and swapped in.

use crate::remote::Fetch;
use crate::version::Version;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const LATEST_API: &str = "https://api.github.com/repos/dionatansimioni/vinci/releases/latest";
/// Where users download by hand, when updating in place isn't possible.
pub const RELEASES_PAGE: &str = "https://github.com/dionatansimioni/vinci/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The AppImage asset, when the release has one with a checksum (needed to update in
    /// place; without it the user is sent to [`RELEASES_PAGE`]).
    pub appimage: Option<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub sha256: String,
}

/// Reads the GitHub "latest release" JSON.
pub fn parse_release(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let version: Version = v["tag_name"].as_str()?.strip_prefix('v')?.parse().ok()?;
    let name = format!("Vinci-{version}-x86_64.AppImage");
    let appimage = v["assets"].as_array().and_then(|assets| {
        let a = assets.iter().find(|a| a["name"] == name.as_str())?;
        let url = a["browser_download_url"].as_str()?;
        let sha256 = a["digest"].as_str()?.strip_prefix("sha256:")?;
        let valid = url.starts_with("https://github.com/")
            && sha256.len() == 64
            && sha256.chars().all(|c| c.is_ascii_hexdigit());
        valid.then(|| Asset {
            name: name.clone(),
            url: url.to_string(),
            sha256: sha256.to_ascii_lowercase(),
        })
    });
    Some(Release { version, appimage })
}

/// The latest release, when it is newer than `current`. Offline or any error: `None`.
pub fn check(fetch: &dyn Fetch, current: &Version) -> Option<Release> {
    let release = parse_release(&fetch.get(LATEST_API)?)?;
    (release.version > *current).then_some(release)
}

/// Whether the AppImage can be replaced where it is (its folder is writable by us).
pub fn can_replace(appimage: &Path) -> bool {
    appimage.parent().is_some_and(|dir| {
        rustix::fs::access(dir, rustix::fs::Access::WRITE_OK).is_ok()
            && rustix::fs::access(appimage, rustix::fs::Access::WRITE_OK).is_ok()
    })
}

/// Where the new version goes. A file named after its version (`Vinci-0.4.7-x86_64.AppImage`,
/// as downloaded) gets the new version's name; any other name (an AppImage manager's
/// `vinci.appimage`, a user's own choice) is kept, so menu entries pointing at it still work.
pub fn target_path(appimage: &Path, current: &Version, asset: &Asset) -> PathBuf {
    let versioned = format!("Vinci-{current}-x86_64.AppImage");
    if appimage
        .file_name()
        .is_some_and(|n| n == versioned.as_str())
    {
        appimage.with_file_name(&asset.name)
    } else {
        appimage.to_path_buf()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("download failed: {0}")]
    Download(String),
    #[error("the downloaded file does not match its checksum")]
    Checksum,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Downloads `asset`, verifies it and puts it at [`target_path`], removing the old file if
/// the name changed. Returns the new path. On any failure the running AppImage is untouched.
pub fn install(
    body: &mut dyn Read,
    appimage: &Path,
    current: &Version,
    asset: &Asset,
) -> Result<PathBuf, UpdateError> {
    let target = target_path(appimage, current, asset);
    let dir = appimage
        .parent()
        .ok_or_else(|| std::io::Error::other("AppImage has no folder"))?;
    let part = dir.join(format!(".{}.part", asset.name));
    let result = (|| {
        let mut file = std::fs::File::create(&part)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = body
                .read(&mut buf)
                .map_err(|e| UpdateError::Download(e.to_string()))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
        }
        let hex: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if hex != asset.sha256 {
            return Err(UpdateError::Checksum);
        }
        file.sync_all()?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))?;
        // Atomic: the running AppImage keeps its (now unlinked) file until it exits.
        std::fs::rename(&part, &target)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    if target != appimage {
        let _ = std::fs::remove_file(appimage);
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Map(HashMap<String, String>);
    impl Fetch for Map {
        fn get(&self, url: &str) -> Option<String> {
            self.0.get(url).cloned()
        }
    }

    const BODY: &[u8] = b"new appimage";

    fn sha(data: &[u8]) -> String {
        Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn release_json(tag: &str, digest: &str) -> String {
        let v = tag.trim_start_matches('v');
        format!(
            r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"assets":[
                {{"name":"Vinci-{v}-x86_64.AppImage.zsync","digest":"sha256:{z}",
                  "browser_download_url":"https://github.com/dionatansimioni/vinci/releases/download/{tag}/Vinci-{v}-x86_64.AppImage.zsync"}},
                {{"name":"Vinci-{v}-x86_64.AppImage","digest":"{digest}",
                  "browser_download_url":"https://github.com/dionatansimioni/vinci/releases/download/{tag}/Vinci-{v}-x86_64.AppImage"}}]}}"#,
            z = "0".repeat(64)
        )
    }

    fn v(s: &str) -> Version {
        s.parse().unwrap()
    }

    fn asset(version: &str) -> Asset {
        Asset {
            name: format!("Vinci-{version}-x86_64.AppImage"),
            url: String::new(),
            sha256: sha(BODY),
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vinci-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn finds_the_appimage_and_its_checksum() {
        let digest = format!("sha256:{}", sha(BODY));
        let r = parse_release(&release_json("v0.5.0", &digest)).unwrap();
        assert_eq!(r.version, v("0.5.0"));
        let a = r.appimage.unwrap();
        assert_eq!(a.name, "Vinci-0.5.0-x86_64.AppImage");
        assert!(a.url.ends_with("/v0.5.0/Vinci-0.5.0-x86_64.AppImage"));
        assert_eq!(a.sha256, sha(BODY));
        // No usable checksum: still announced, but only as a link to the releases page.
        let r = parse_release(&release_json("v0.5.0", "md5:abc")).unwrap();
        assert!(r.appimage.is_none());
    }

    #[test]
    fn only_newer_releases_count() {
        let json = release_json("v0.5.0", &format!("sha256:{}", sha(BODY)));
        let fetch = Map([(LATEST_API.to_string(), json)].into());
        assert!(check(&fetch, &v("0.4.8")).is_some());
        assert!(check(&fetch, &v("0.5.0")).is_none());
        assert!(check(&fetch, &v("0.6.0")).is_none());
        assert!(
            check(&Map(HashMap::new()), &v("0.4.8")).is_none(),
            "offline"
        );
        assert!(parse_release(r#"{"tag_name":"v9.0.0","prerelease":true}"#).is_none());
    }

    #[test]
    fn versioned_files_get_the_new_name_others_keep_theirs() {
        let a = asset("0.5.0");
        let cur = v("0.4.8");
        assert_eq!(
            target_path(Path::new("/d/Vinci-0.4.8-x86_64.AppImage"), &cur, &a),
            Path::new("/d/Vinci-0.5.0-x86_64.AppImage")
        );
        assert_eq!(
            target_path(Path::new("/home/u/AppImages/vinci.appimage"), &cur, &a),
            Path::new("/home/u/AppImages/vinci.appimage")
        );
    }

    #[test]
    fn install_replaces_the_appimage() {
        let dir = tmp("ok");
        let old = dir.join("Vinci-0.4.8-x86_64.AppImage");
        std::fs::write(&old, "old").unwrap();
        let new = install(&mut &BODY[..], &old, &v("0.4.8"), &asset("0.5.0")).unwrap();
        assert_eq!(new, dir.join("Vinci-0.5.0-x86_64.AppImage"));
        assert_eq!(std::fs::read(&new).unwrap(), BODY);
        assert!(!old.exists());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(new.metadata().unwrap().permissions().mode() & 0o777, 0o755);
        // Managed name: replaced in place.
        let managed = dir.join("vinci.appimage");
        std::fs::write(&managed, "old").unwrap();
        let same = install(&mut &BODY[..], &managed, &v("0.4.8"), &asset("0.5.0")).unwrap();
        assert_eq!(same, managed);
        assert_eq!(std::fs::read(&managed).unwrap(), BODY);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bad_download_leaves_the_old_file_alone() {
        let dir = tmp("bad");
        let old = dir.join("Vinci-0.4.8-x86_64.AppImage");
        std::fs::write(&old, "old").unwrap();
        let err = install(&mut &b"tampered"[..], &old, &v("0.4.8"), &asset("0.5.0"));
        assert!(matches!(err, Err(UpdateError::Checksum)));
        assert_eq!(std::fs::read(&old).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no leftovers");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Hits the real GitHub API and downloads the latest AppImage (~40 MB) over a fake
    /// older one. Run with `--ignored` when online.
    #[test]
    #[ignore]
    fn real_network_update() {
        let r = parse_release(&crate::remote::Http::new().get(LATEST_API).unwrap()).unwrap();
        let asset = r.appimage.expect("AppImage with checksum");
        let dir = tmp("real");
        let old = dir.join("Vinci-0.0.1-x86_64.AppImage");
        std::fs::write(&old, "old").unwrap();
        let mut body = crate::remote::download(&asset.url).unwrap();
        let new = install(&mut body, &old, &v("0.0.1"), &asset).unwrap();
        assert_eq!(new, dir.join(&asset.name));
        assert!(new.metadata().unwrap().len() > 1024 * 1024);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
