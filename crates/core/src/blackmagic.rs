//! Blackmagic's public "latest stable version" endpoint, used only to tell the user an
//! update exists. Any failure here is non-fatal by design (offline-first).

use crate::version::Version;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Response {
    linux: Release,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Release {
    major: u32,
    minor: u32,
    release_num: u32,
}

/// Parses the endpoint's JSON into a version like `21.1.2`. The build number is ignored
/// so it compares equal to the version in the installer filename.
pub fn parse_latest(json: &str) -> Option<Version> {
    let r: Response = serde_json::from_str(json).ok()?;
    Some(Version::new(&[
        r.linux.major,
        r.linux.minor,
        r.linux.release_num,
    ]))
}

/// Whether `latest` is newer than what is installed.
pub fn update_available(installed: &Version, latest: &Version) -> bool {
    latest > installed
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from the live endpoint on 2026-10-27.
    const LIVE: &str = r#"{"linux":{"releaseId":"be08842994c94d999da4ab052d0a2b2a","downloadId":"8b9041dd175d4245b5a19a03a7700e77","major":21,"minor":1,"releaseNum":2,"build":10}}"#;

    #[test]
    fn parses_live_response() {
        assert_eq!(parse_latest(LIVE).unwrap().to_string(), "21.1.2");
    }

    #[test]
    fn tolerates_extra_and_missing_optional_fields() {
        let j = r#"{"linux":{"major":22,"minor":0,"releaseNum":0,"new":"x"},"mac":{}}"#;
        assert_eq!(parse_latest(j).unwrap().to_string(), "22.0.0");
    }

    #[test]
    fn garbage_is_none_not_a_crash() {
        for j in ["", "{}", "<html>", r#"{"linux":{"major":"x"}}"#] {
            assert!(parse_latest(j).is_none(), "{j}");
        }
    }

    #[test]
    fn compares_with_installed_version() {
        let v = |s: &str| s.parse::<Version>().unwrap();
        assert!(update_available(&v("21.1.1"), &v("21.1.2")));
        assert!(!update_available(&v("21.1.2"), &v("21.1.2")));
        assert!(!update_available(&v("21.1.2"), &v("21.1")));
    }
}
