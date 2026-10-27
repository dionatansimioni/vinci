//! Everything fetched from the network: latest Resolve version, the known-bad list and
//! signed recipe updates. All of it is optional (offline-first, SPEC §12.9): any failure
//! degrades to what the app already has.

use crate::blackmagic;
use crate::installer_file::Edition;
use crate::known_bad::{self, Entry, KnownBad};
use crate::recipe::{self, Recipe, RecipeError, Source};
use crate::version::Version;
use std::path::{Path, PathBuf};
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

/// Where the last verified remote recipe is kept for offline use.
pub fn recipe_cache_dir(home: &Path) -> PathBuf {
    home.join(".cache/vinci/recipe")
}

/// The recipe to use now: a newer signed remote one if available (fresh or cached),
/// otherwise the bundled one. Returns why a remote recipe was rejected, if it was.
pub fn effective_recipe(
    fetch: &dyn Fetch,
    cache_dir: &Path,
    public_key: Option<&str>,
) -> (Recipe, Source, Option<RecipeError>) {
    let bundled = Recipe::bundled();
    if public_key.is_none() {
        return (bundled, Source::Bundled, None);
    }
    let fresh = fetch
        .get(recipe::REMOTE_URL)
        .zip(fetch.get(recipe::REMOTE_SIG_URL));
    if let Some((text, sig)) = &fresh {
        let (r, src, err) = recipe::choose(bundled.clone(), Some((text, sig)), public_key);
        if src == Source::Remote {
            let _ = std::fs::create_dir_all(cache_dir)
                .and_then(|()| std::fs::write(cache_dir.join("recipe.toml"), text))
                .and_then(|()| std::fs::write(cache_dir.join("recipe.toml.minisig"), sig));
            return (r, src, None);
        }
        if err.is_some() {
            return (r, src, err);
        }
    }
    // Offline (or nothing newer online): fall back to the cached copy, re-verified.
    let cached = std::fs::read_to_string(cache_dir.join("recipe.toml"))
        .ok()
        .zip(std::fs::read_to_string(cache_dir.join("recipe.toml.minisig")).ok());
    match cached {
        Some((text, sig)) => recipe::choose(bundled, Some((&text, &sig)), public_key),
        None => (bundled, Source::Bundled, None),
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

    mod signed {
        use super::*;
        use minisign::KeyPair;
        use std::io::Cursor;

        fn signed(revision: u32, kp: &KeyPair) -> (String, String) {
            let text = recipe::BUNDLED.replacen(
                &format!("revision = {}", Recipe::bundled().revision),
                &format!("revision = {revision}"),
                1,
            );
            let sig = minisign::sign(
                Some(&kp.pk),
                &kp.sk,
                Cursor::new(text.as_bytes()),
                None,
                None,
            )
            .unwrap()
            .to_string();
            (text, sig)
        }

        fn cache(name: &str) -> PathBuf {
            std::env::temp_dir().join(format!("rli-remote-{}-{name}", std::process::id()))
        }

        #[test]
        fn caches_verified_recipe_and_uses_it_offline() {
            let kp = KeyPair::generate_unencrypted_keypair().unwrap();
            let pk = kp.pk.to_base64();
            let (text, sig) = signed(Recipe::bundled().revision + 5, &kp);
            let dir = cache("offline");
            let online = Map::default()
                .with(recipe::REMOTE_URL, &text)
                .with(recipe::REMOTE_SIG_URL, &sig);
            let (r, src, _) = effective_recipe(&online, &dir, Some(&pk));
            assert_eq!(
                (r.revision, src),
                (Recipe::bundled().revision + 5, Source::Remote)
            );

            let (r, src, err) = effective_recipe(&Map::default(), &dir, Some(&pk));
            let _ = std::fs::remove_dir_all(&dir);
            assert_eq!(
                (r.revision, src, err),
                (Recipe::bundled().revision + 5, Source::Remote, None)
            );
        }

        #[test]
        fn tampered_cache_is_rejected() {
            let kp = KeyPair::generate_unencrypted_keypair().unwrap();
            let (text, sig) = signed(Recipe::bundled().revision + 5, &kp);
            let dir = cache("tampered");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("recipe.toml"), text.replace("ffmpeg", "evil")).unwrap();
            std::fs::write(dir.join("recipe.toml.minisig"), sig).unwrap();
            let (r, src, err) = effective_recipe(&Map::default(), &dir, Some(&kp.pk.to_base64()));
            let _ = std::fs::remove_dir_all(&dir);
            assert_eq!(
                (r.revision, src, err),
                (
                    Recipe::bundled().revision,
                    Source::Bundled,
                    Some(RecipeError::Signature)
                )
            );
        }

        #[test]
        fn no_key_means_bundled_and_no_network() {
            struct Panics;
            impl Fetch for Panics {
                fn get(&self, _: &str) -> Option<String> {
                    panic!("must not fetch without a key")
                }
            }
            let (_, src, _) = effective_recipe(&Panics, &cache("nokey"), None);
            assert_eq!(src, Source::Bundled);
        }
    }

    /// Hits the real Blackmagic endpoint. Run with `--ignored` when online.
    #[test]
    #[ignore]
    fn real_network_latest_version() {
        let info = check_update(&Http::new(), &Recipe::bundled(), Edition::Free, None);
        assert!(info.latest.is_some());
    }
}
