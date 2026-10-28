//! Recipes: the declarative description of images, packages, repositories and URLs.
//!
//! A copy is embedded at build time. A newer one may be fetched from GitHub, but it is
//! only used when its minisign signature checks out, its schema matches, it passes
//! validation, and its revision is higher than the embedded one.

use crate::version::Version;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Recipe format this build understands.
pub const SCHEMA: u32 = 1;
pub const BUNDLED: &str = include_str!("../../../data/recipe.toml");
pub const REMOTE_URL: &str =
    "https://raw.githubusercontent.com/dionatansimioni/vinci/main/data/recipe.toml";
pub const REMOTE_SIG_URL: &str =
    "https://raw.githubusercontent.com/dionatansimioni/vinci/main/data/recipe.toml.minisig";
/// minisign public key that signs remote recipes. Remote recipes stay disabled until set.
pub const PUBLIC_KEY: Option<&str> = None;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema: u32,
    pub revision: u32,
    pub container: Container,
    pub distrobox: Distrobox,
    pub packages: Packages,
    pub gpu: GpuSetups,
    pub resolve: ResolveSetup,
    pub launcher: Launcher,
    pub blackmagic: Blackmagic,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Container {
    pub image: String,
    #[serde(default)]
    pub mirrors: Vec<String>,
    pub name_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Distrobox {
    pub pinned: Version,
    pub min_host: Version,
    pub max_host: Version,
    pub install_url: String,
}

impl Distrobox {
    /// Whether a distrobox already on the host can be used instead of our pinned copy.
    pub fn host_compatible(&self, host: &Version) -> bool {
        host >= &self.min_host && host < &self.max_host
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packages {
    pub base: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuSetups {
    pub nvidia: VendorSetup,
    pub amd: VendorSetup,
    pub intel: VendorSetup,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VendorSetup {
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub apt_repo: Option<AptRepo>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub experimental: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AptRepo {
    pub name: String,
    pub key_url: String,
    pub url: String,
    pub suite: String,
    pub components: Vec<String>,
}

impl AptRepo {
    pub fn keyring_path(&self) -> String {
        format!("/etc/apt/keyrings/{}.asc", self.name)
    }

    /// One-line apt source; only built from validated fields.
    pub fn source_line(&self) -> String {
        format!(
            "deb [arch=amd64 signed-by={}] {} {} {}",
            self.keyring_path(),
            self.url,
            self.suite,
            self.components.join(" ")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveSetup {
    pub install_dir: String,
    pub installer_args: Vec<String>,
    #[serde(default)]
    pub remove_libs: Vec<String>,
    #[serde(default)]
    pub udev_rules: Vec<String>,
    #[serde(default)]
    pub installer_env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launcher {
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub mime_types: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blackmagic {
    pub latest_free: String,
    pub latest_studio: String,
    pub download_page: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RecipeError {
    #[error("recipe is not valid TOML: {0}")]
    Parse(String),
    #[error("recipe schema {found} is not supported (expected {SCHEMA})")]
    Schema { found: u32 },
    #[error("invalid recipe field {field}: {value:?}")]
    Invalid { field: &'static str, value: String },
    #[error("recipe signature is invalid")]
    Signature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Bundled,
    Remote,
}

impl Recipe {
    /// Parses and validates. Remote input goes through here, so be strict.
    pub fn parse(text: &str) -> Result<Self, RecipeError> {
        // Check the schema first so a future format gets a clear error, not a field error.
        #[derive(Deserialize)]
        struct Head {
            schema: u32,
        }
        let head: Head = toml::from_str(text).map_err(|e| RecipeError::Parse(e.to_string()))?;
        if head.schema != SCHEMA {
            return Err(RecipeError::Schema { found: head.schema });
        }
        let recipe: Self = toml::from_str(text).map_err(|e| RecipeError::Parse(e.to_string()))?;
        recipe.validate()?;
        Ok(recipe)
    }

    pub fn bundled() -> Self {
        Self::parse(BUNDLED).expect("embedded recipe is valid (checked by tests)")
    }

    fn validate(&self) -> Result<(), RecipeError> {
        let bad = |field, value: &str| {
            Err(RecipeError::Invalid {
                field,
                value: value.to_string(),
            })
        };
        for image in std::iter::once(&self.container.image).chain(&self.container.mirrors) {
            if !is_image_ref(image) {
                return bad("container.image", image);
            }
        }
        if !is_token(&self.container.name_prefix) {
            return bad("container.name_prefix", &self.container.name_prefix);
        }
        let urls = [
            &self.distrobox.install_url,
            &self.blackmagic.latest_free,
            &self.blackmagic.latest_studio,
            &self.blackmagic.download_page,
        ];
        for url in urls {
            if !is_https_url(url) {
                return bad("url", url);
            }
        }
        let vendors = [&self.gpu.nvidia, &self.gpu.amd, &self.gpu.intel];
        let packages = self
            .packages
            .base
            .iter()
            .chain(vendors.iter().flat_map(|v| &v.packages));
        for p in packages {
            if !is_package_name(p) {
                return bad("package", p);
            }
        }
        for repo in vendors.iter().filter_map(|v| v.apt_repo.as_ref()) {
            if !is_token(&repo.name) {
                return bad("apt_repo.name", &repo.name);
            }
            if !is_https_url(&repo.url) || !is_https_url(&repo.key_url) {
                return bad("apt_repo.url", &repo.url);
            }
            if let Some(t) = std::iter::once(&repo.suite)
                .chain(&repo.components)
                .find(|t| !is_token(t))
            {
                return bad("apt_repo.suite", t);
            }
        }
        if !self.resolve.install_dir.starts_with('/') || has_unsafe_path(&self.resolve.install_dir)
        {
            return bad("resolve.install_dir", &self.resolve.install_dir);
        }
        for pattern in &self.resolve.udev_rules {
            if pattern.starts_with('/') || has_unsafe_path(pattern) || !pattern.ends_with(".rules")
            {
                return bad("resolve.udev_rules", pattern);
            }
        }
        for arg in &self.resolve.installer_args {
            if !is_flag(arg) {
                return bad("resolve.installer_args", arg);
            }
        }
        for lib in &self.resolve.remove_libs {
            if lib.starts_with('/') || has_unsafe_path(lib) || lib.is_empty() {
                return bad("resolve.remove_libs", lib);
            }
        }
        if let Some(icon) = &self.launcher.icon {
            if icon.starts_with('/') || has_unsafe_path(icon) || icon.is_empty() {
                return bad("launcher.icon", icon);
            }
        }
        let envs = vendors
            .iter()
            .flat_map(|v| &v.env)
            .chain(&self.resolve.installer_env)
            .chain(&self.launcher.env);
        for (k, v) in envs {
            if !is_env_key(k) || v.chars().any(char::is_control) {
                return bad("env", k);
            }
        }
        for m in &self.launcher.mime_types {
            if !m.contains('/')
                || !m
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "/.+-_".contains(c))
            {
                return bad("launcher.mime_types", m);
            }
        }
        Ok(())
    }
}

/// Picks the recipe to use. The remote one wins only when everything checks out;
/// any failure falls back to the bundled recipe and reports why.
pub fn choose(
    bundled: Recipe,
    remote: Option<(&str, &str)>,
    public_key: Option<&str>,
) -> (Recipe, Source, Option<RecipeError>) {
    let (Some((text, sig)), Some(key)) = (remote, public_key) else {
        return (bundled, Source::Bundled, None);
    };
    match verify(text, sig, key).and_then(|()| Recipe::parse(text)) {
        Ok(r) if r.revision > bundled.revision => (r, Source::Remote, None),
        Ok(_) => (bundled, Source::Bundled, None),
        Err(e) => (bundled, Source::Bundled, Some(e)),
    }
}

/// Verifies a minisign signature (`.minisig` file contents) over `text`.
pub fn verify(text: &str, signature: &str, public_key: &str) -> Result<(), RecipeError> {
    let pk =
        minisign_verify::PublicKey::from_base64(public_key).map_err(|_| RecipeError::Signature)?;
    let sig = minisign_verify::Signature::decode(signature).map_err(|_| RecipeError::Signature)?;
    pk.verify(text.as_bytes(), &sig, false)
        .map_err(|_| RecipeError::Signature)
}

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// A bare command-line switch such as `-i` or `--noconfirm` (no values, no shell syntax).
fn is_flag(s: &str) -> bool {
    s.len() > 1
        && s.starts_with('-')
        && s[1..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn is_package_name(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+-.".contains(c))
}

/// Variables that could make a process load or run attacker-chosen code.
const DANGEROUS_ENV: &[&str] = &[
    "PATH",
    "BASH_ENV",
    "ENV",
    "IFS",
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "PERL5LIB",
    "PERL5OPT",
    "NODE_OPTIONS",
    "GCONV_PATH",
];

fn is_env_key(s: &str) -> bool {
    !s.starts_with("LD_")
        && !DANGEROUS_ENV.contains(&s)
        && s.chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn is_https_url(s: &str) -> bool {
    s.strip_prefix("https://").is_some_and(|rest| {
        !rest.is_empty() && !rest.chars().any(|c| c.is_whitespace() || c.is_control())
    })
}

fn is_image_ref(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@".contains(c))
}

fn has_unsafe_path(s: &str) -> bool {
    s.split('/').any(|seg| seg == "..") || s.chars().any(|c| c.is_control() || c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(edit: impl Fn(&str) -> String) -> Result<Recipe, RecipeError> {
        Recipe::parse(&edit(BUNDLED))
    }

    #[test]
    fn bundled_recipe_is_valid() {
        let r = Recipe::bundled();
        assert_eq!(r.schema, SCHEMA);
        assert!(r.container.image.contains("ubuntu:24.04"));
        assert!(r.gpu.intel.experimental);
        assert_eq!(
            r.gpu.amd.apt_repo.as_ref().unwrap().source_line(),
            "deb [arch=amd64 signed-by=/etc/apt/keyrings/rocm.asc] https://repo.radeon.com/rocm/apt/7.2.4 noble main"
        );
    }

    #[test]
    fn distrobox_compatibility_window() {
        let d = Recipe::bundled().distrobox;
        let v = |s: &str| s.parse::<Version>().unwrap();
        assert!(d.host_compatible(&v("1.8.2.5")));
        assert!(d.host_compatible(&v("1.9")));
        assert!(!d.host_compatible(&v("1.7.2")));
        assert!(!d.host_compatible(&v("2.0.0")));
    }

    #[test]
    fn rejects_future_schema() {
        let e = with(|s| s.replacen("schema = 1", "schema = 2", 1)).unwrap_err();
        assert_eq!(e, RecipeError::Schema { found: 2 });
    }

    #[test]
    fn rejects_unknown_fields() {
        let e = with(|s| s.replacen("[container]", "[container]\nrun = \"rm -rf /\"", 1));
        assert!(matches!(e, Err(RecipeError::Parse(_))));
    }

    #[test]
    fn rejects_injection_attempts() {
        let cases: &[(&str, &str)] = &[
            ("\"ffmpeg\",", "\"ffmpeg; rm -rf ~\","),
            ("\"libs/libglib-2.0.so*\"", "\"../../etc/passwd\""),
            ("\"libs/libglib-2.0.so*\"", "\"/etc/passwd\""),
            (
                "suite = \"noble\"",
                "suite = \"noble main\\ndeb http://evil\"",
            ),
            (
                "url = \"https://repo.radeon.com/rocm/apt/7.2.4\"",
                "url = \"http://repo.radeon.com\"",
            ),
            (
                "SKIP_PACKAGE_CHECK = \"1\"",
                "LD_PRELOAD = \"/tmp/evil.so\"",
            ),
            ("SKIP_PACKAGE_CHECK = \"1\"", "PATH = \"/tmp/evil\""),
            ("SKIP_PACKAGE_CHECK = \"1\"", "lower = \"1\""),
            ("\"-a\"]", "\"-C /tmp/x\"]"),
            ("\"-a\"]", "\"a\"]"),
            ("\"graphics/DV_Resolve.png\"", "\"../../etc/shadow\""),
            (
                "QT_QPA_PLATFORM = \"xcb\"",
                "QT_QPA_PLATFORM = \"xcb\\nEVIL=1\"",
            ),
            (
                "name_prefix = \"vinci\"",
                "name_prefix = \"vinci --privileged\"",
            ),
        ];
        for (from, to) in cases {
            assert!(BUNDLED.contains(from), "fixture text missing: {from}");
            let r = with(|s| s.replacen(from, to, 1));
            assert!(r.is_err(), "accepted {to}");
        }
    }

    mod signing {
        use super::*;
        use minisign::KeyPair;
        use std::io::Cursor;

        fn sign(text: &str, kp: &KeyPair) -> String {
            minisign::sign(
                Some(&kp.pk),
                &kp.sk,
                Cursor::new(text.as_bytes()),
                None,
                None,
            )
            .unwrap()
            .to_string()
        }

        fn remote_text(revision: u32) -> String {
            BUNDLED.replacen(
                &format!("revision = {}", Recipe::bundled().revision),
                &format!("revision = {revision}"),
                1,
            )
        }

        #[test]
        fn accepts_newer_signed_recipe() {
            let kp = KeyPair::generate_unencrypted_keypair().unwrap();
            let text = remote_text(Recipe::bundled().revision + 5);
            let sig = sign(&text, &kp);
            let (r, src, err) = choose(
                Recipe::bundled(),
                Some((&text, &sig)),
                Some(&kp.pk.to_base64()),
            );
            assert_eq!(
                (r.revision, src, err),
                (Recipe::bundled().revision + 5, Source::Remote, None)
            );
        }

        #[test]
        fn rejects_tampered_or_foreign_signature() {
            let kp = KeyPair::generate_unencrypted_keypair().unwrap();
            let other = KeyPair::generate_unencrypted_keypair().unwrap();
            let text = remote_text(Recipe::bundled().revision + 5);
            let sig = sign(&text, &kp);
            let tampered = text.replace("ffmpeg", "ffmpeg-evil");
            let (r, src, err) = choose(
                Recipe::bundled(),
                Some((&tampered, &sig)),
                Some(&kp.pk.to_base64()),
            );
            assert_eq!(
                (r.revision, src, err),
                (
                    Recipe::bundled().revision,
                    Source::Bundled,
                    Some(RecipeError::Signature)
                )
            );
            let (_, src, err) = choose(
                Recipe::bundled(),
                Some((&text, &sig)),
                Some(&other.pk.to_base64()),
            );
            assert_eq!((src, err), (Source::Bundled, Some(RecipeError::Signature)));
        }

        #[test]
        fn ignores_older_or_equal_revision_and_missing_key() {
            let kp = KeyPair::generate_unencrypted_keypair().unwrap();
            let text = remote_text(Recipe::bundled().revision);
            let sig = sign(&text, &kp);
            let (_, src, err) = choose(
                Recipe::bundled(),
                Some((&text, &sig)),
                Some(&kp.pk.to_base64()),
            );
            assert_eq!((src, err), (Source::Bundled, None));
            let (_, src, _) = choose(Recipe::bundled(), Some((&text, &sig)), None);
            assert_eq!(src, Source::Bundled);
        }
    }
}
