use crates_index::{Crate, Version};
use std::collections::HashMap;

pub struct Krate {
    krate: Crate,
    versions: HashMap<String, Version>,
    asked_versions: HashMap<String, Version>,
}

impl From<Crate> for Krate {
    fn from(krate: Crate) -> Self {
        let versions = krate
            .versions()
            .into_iter()
            .map(|version| (version.name().to_string(), version.clone()))
            .collect();

        let asked_versions = HashMap::new();

        Self {
            krate,
            versions,
            asked_versions,
        }
    }
}

impl Krate {
    
}
