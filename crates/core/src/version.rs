//! Resolve version numbers (`20.2.1`, `19.1.4.11`), ordered numerically.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Version {
    parts: Vec<u32>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid version string: {0:?}")]
pub struct ParseVersionError(pub String);

impl Version {
    pub fn new(parts: &[u32]) -> Self {
        Self {
            parts: parts.to_vec(),
        }
    }

    pub fn major(&self) -> u32 {
        self.parts[0]
    }

    /// Component at `i`, treating missing trailing components as zero.
    fn part(&self, i: usize) -> u32 {
        self.parts.get(i).copied().unwrap_or(0)
    }
}

impl FromStr for Version {
    type Err = ParseVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseVersionError(s.to_string());
        if s.is_empty() {
            return Err(err());
        }
        let parts = s
            .split('.')
            .map(|p| p.parse::<u32>().map_err(|_| err()))
            .collect::<Result<Vec<_>, _>>()?;
        if parts.len() > 4 {
            return Err(err());
        }
        Ok(Self { parts })
    }
}

impl TryFrom<String> for Version {
    type Error = ParseVersionError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Version> for String {
    fn from(v: Version) -> String {
        v.to_string()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s: Vec<String> = self.parts.iter().map(u32::to_string).collect();
        f.write_str(&s.join("."))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let n = self.parts.len().max(other.parts.len());
        (0..n)
            .map(|i| self.part(i).cmp(&other.part(i)))
            .find(|o| o.is_ne())
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        s.parse().unwrap()
    }

    #[test]
    fn parses_and_displays() {
        assert_eq!(v("20.2.1").to_string(), "20.2.1");
        assert_eq!(v("19.1.4.11").major(), 19);
    }

    #[test]
    fn rejects_garbage() {
        assert!("".parse::<Version>().is_err());
        assert!("20.x".parse::<Version>().is_err());
        assert!("1.2.3.4.5".parse::<Version>().is_err());
    }

    #[test]
    fn orders_numerically_not_lexically() {
        assert!(v("20.10") > v("20.9"));
        assert!(v("20.2.1") > v("20.2"));
        assert_eq!(v("20.2"), v("20.2"));
        assert_eq!(v("20.2.0").cmp(&v("20.2")), std::cmp::Ordering::Equal);
    }

    #[test]
    fn serde_roundtrip() {
        let json = serde_json::to_string(&v("20.2.1")).unwrap();
        assert_eq!(json, "\"20.2.1\"");
        assert_eq!(serde_json::from_str::<Version>(&json).unwrap(), v("20.2.1"));
    }
}
