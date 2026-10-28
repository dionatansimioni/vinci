//! Everything fetched from the network: the latest Resolve version and the known-bad
//! list. All of it is optional (offline-first, SPEC §12.9): any failure
//! degrades to what the app already has.

use crate::blackmagic;
use crate::installer_file::Edition;
use crate::known_bad::{self, Entry, KnownBad};
use crate::recipe::Recipe;
use crate::version::Version;
use std::time::Duration;

/// Fetches a URL as text. The real one uses HTTP; tests use a map.
pub trait Fetch {
    fn get(&self, url: &str) -> Option<String>;
}

pub struct Http {
    agent: ureq::Agent,
}

impl Http {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .user_agent(concat!(
                "Vinci/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/dionatansimioni/vinci)"
            ))
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetch for Http {
    fn get(&self, url: &str) -> Option<String> {
        self.agent
            .get(url)
            .call()
            .ok()?
            .body_mut()
            .with_config()
            .limit(1024 * 1024)
            .read_to_string()
            .ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub latest: Option<Version>,
    pub update_available: bool,
    /// Set when the latest version is on the known-bad list.
    pub latest_is_bad: Option<Entry>,
}

pub fn known_bad(fetch: &dyn Fetch) -> KnownBad {
    fetch
        .get(known_bad::URL)
        .and_then(|j| KnownBad::parse(&j).ok())
        .unwrap_or_default()
}

/// Checks Blackmagic for a newer release of `edition` than `installed`.
pub fn check_update(
    fetch: &dyn Fetch,
    recipe: &Recipe,
    edition: Edition,
    installed: Option<&Version>,
) -> UpdateInfo {
    let url = match edition {
        Edition::Free => &recipe.blackmagic.latest_free,
        Edition::Studio => &recipe.blackmagic.latest_studio,
    };
    let latest = fetch.get(url).and_then(|j| blackmagic::parse_latest(&j));
    let update_available = match (installed, &latest) {
        (Some(i), Some(l)) => blackmagic::update_available(i, l),
        _ => false,
    };
    let latest_is_bad = latest
        .as_ref()
        .and_then(|l| known_bad(fetch).lookup(l, edition).cloned());
    UpdateInfo {
        latest,
        update_available,
        latest_is_bad,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Map(HashMap<String, String>);

    impl Map {
        fn with(mut self, url: &str, body: &str) -> Self {
            self.0.insert(url.into(), body.into());
            self
        }
    }

    impl Fetch for Map {
        fn get(&self, url: &str) -> Option<String> {
            self.0.get(url).cloned()
        }
    }

    const LATEST: &str = r#"{"linux":{"major":21,"minor":2,"releaseNum":0,"build":5}}"#;

    #[test]
    fn reports_update_and_known_bad() {
        let r = Recipe::bundled();
        let fetch = Map::default().with(&r.blackmagic.latest_free, LATEST).with(
            known_bad::URL,
            r#"{"versions":[{"version":"21.2.0","reason":"broken","fallback":"21.1.2"}]}"#,
        );
        let installed: Version = "21.1.2".parse().unwrap();
        let info = check_update(&fetch, &r, Edition::Free, Some(&installed));
        assert_eq!(info.latest.unwrap().to_string(), "21.2.0");
        assert!(info.update_available);
        assert_eq!(
            info.latest_is_bad.unwrap().fallback.unwrap().to_string(),
            "21.1.2"
        );
    }

    #[test]
    fn offline_means_no_update_and_no_error() {
        let r = Recipe::bundled();
        let info = check_update(&Map::default(), &r, Edition::Studio, None);
        assert_eq!(
            info,
            UpdateInfo {
                latest: None,
                update_available: false,
                latest_is_bad: None
            }
        );
    }

    /// Hits the real Blackmagic endpoint. Run with `--ignored` when online.
    #[test]
    #[ignore]
    fn real_network_latest_version() {
        let info = check_update(&Http::new(), &Recipe::bundled(), Edition::Free, None);
        assert!(info.latest.is_some());
    }
}
