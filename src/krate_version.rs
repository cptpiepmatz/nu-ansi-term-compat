use std::{hash::Hash, ops::Deref};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KrateVersion {
    crate_version: crates_index::Version,
    semver_version: semver::Version,
}

impl KrateVersion {
    pub fn as_key(&self) -> impl Ord + Eq + Hash + use<'_> {
        (self.crate_version.name(), self.crate_version.version())
    }

    pub fn semver(&self) -> &semver::Version {
        &self.semver_version
    }
}

impl Deref for KrateVersion {
    type Target = crates_index::Version;

    fn deref(&self) -> &Self::Target {
        &self.crate_version
    }
}

impl From<crates_index::Version> for KrateVersion {
    fn from(version: crates_index::Version) -> KrateVersion {
        Self {
            semver_version: version.version().parse().expect("valid semver version"),
            crate_version: version,
        }
    }
}

impl PartialEq for KrateVersion {
    fn eq(&self, other: &Self) -> bool {
        self.as_key() == other.as_key()
    }
}

impl Eq for KrateVersion {}

impl PartialOrd for KrateVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.as_key().partial_cmp(&other.as_key())
    }
}

impl Ord for KrateVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_key().cmp(&other.as_key())
    }
}

impl Hash for KrateVersion {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_key().hash(state);
    }
}