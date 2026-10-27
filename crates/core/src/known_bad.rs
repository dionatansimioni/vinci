//! Remote list of Resolve versions known to break in our container setup.
//!
//! Published as `data/known-bad.json` in the repository and fetched read-only.

use crate::installer_file::Edition;
use crate::version::Version;
use serde::Deserialize;

pub const URL: &str =
    "https://raw.githubusercontent.com/dionatansimioni/vinci/main/data/known-bad.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Applies {
    Both,
    Free,
    Studio,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Entry {
    pub version: Version,
    #[serde(default = "both")]
    pub edition: Applies,
    pub reason: String,
    /// Last version known to work, offered instead.
    pub fallback: Option<Version>,
}

fn both() -> Applies {
    Applies::Both
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct KnownBad {
    pub versions: Vec<Entry>,
}

impl KnownBad {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    pub fn lookup(&self, version: &Version, edition: Edition) -> Option<&Entry> {
        self.versions.iter().find(|e| {
            &e.version == version
                && match e.edition {
                    Applies::Both => true,
                    Applies::Free => edition == Edition::Free,
                    Applies::Studio => edition == Edition::Studio,
                }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "versions": [
            { "version": "20.3", "reason": "crash on start with ROCm", "fallback": "20.2.1" },
            { "version": "20.2.2", "edition": "studio", "reason": "activation broken" }
        ]
    }"#;

    #[test]
    fn finds_matching_entries() {
        let kb = KnownBad::parse(SAMPLE).unwrap();
        let v = |s: &str| s.parse::<Version>().unwrap();
        let hit = kb.lookup(&v("20.3"), Edition::Free).unwrap();
        assert_eq!(hit.fallback, Some(v("20.2.1")));
        assert!(kb.lookup(&v("20.2.2"), Edition::Studio).is_some());
        assert!(kb.lookup(&v("20.2.2"), Edition::Free).is_none());
        assert!(kb.lookup(&v("20.2.1"), Edition::Free).is_none());
    }

    #[test]
    fn repository_file_is_valid() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/known-bad.json");
        KnownBad::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
    }
}
