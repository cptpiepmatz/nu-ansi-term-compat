use crates_index::{Crate, Version};
use semver::VersionReq;
use std::{collections::{BTreeMap, HashMap}, ops::Deref};

use crate::krate_version::KrateVersion;

pub struct Krate {
    krate: Crate,
    versions: BTreeMap<String, KrateVersion>,
    asked_versions: HashMap<VersionReq, String>,
}

impl From<Crate> for Krate {
    fn from(krate: Crate) -> Self {
        let versions = krate
            .versions()
            .into_iter()
            .map(|version| (version.name().to_string(), version.clone().into()))
            .collect();

        let asked_versions = HashMap::new();

        Self {
            krate,
            versions,
            asked_versions,
        }
    }
}

impl Deref for Krate {
    type Target = Crate;

    fn deref(&self) -> &Self::Target {
        &self.krate
    }
}

impl Krate {
    pub fn ask_version(&mut self, req: &VersionReq) -> Option<&KrateVersion> {
        if let Some(version) = self.asked_versions.get(req) {
            return self.versions.get(version);
        }

        for (key, version) in self.versions.iter().rev() {
            if req.matches(version.semver()) {
                self.asked_versions.insert(req.to_owned(), key.to_owned());
                return Some(version);
            }
        }

        None
    }
}
