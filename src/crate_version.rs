use std::hash::Hash;
use crates_index::Version;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrateVersion(Version);

impl CrateVersion {
    pub fn as_key(&self) -> impl Ord + Eq + Hash + use<'_> {
        (self.0.name(), self.0.version())
    }

    pub fn as_inner(&self) -> &Version {
        &self.0
    } 
}

impl From<Version> for CrateVersion {
    fn from(version: Version) -> CrateVersion {
        Self(version)
    }
}

impl PartialEq for CrateVersion {
    fn eq(&self, other: &Self) -> bool {
        self.as_key() == other.as_key()
    }
}

impl Eq for CrateVersion {}

impl PartialOrd for CrateVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.as_key().partial_cmp(&other.as_key())
    }
}

impl Ord for CrateVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_key().cmp(&other.as_key())
    }
}

impl Hash for CrateVersion {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_key().hash(state);
    }
}