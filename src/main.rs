use anyhow::Context;
use count_crates::count_crates;
use crates_index::{Crate, Version};
use dashmap::{DashMap, DashSet};
use krate_version::KrateVersion;
use progress::Progress;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs::File,
    io::{BufReader, BufWriter},
    ops::Deref,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use crate::krate::Krate;

mod count_crates;
mod krate;
mod krate_version;
mod progress;

const SEARCH_CRATE: &str = "nu-ansi-term";
static REGISTRY_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("registry"));
static REVERSE_INDEX_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("reverse_index.json"));

fn main() -> anyhow::Result<()> {
    let mut progress = Progress::new();

    progress.spinner("Cloning", "crates.io registry");
    let git_index =
        crates_index::GitIndex::with_path(REGISTRY_PATH.clone(), crates_index::git::URL)?;
    progress.finish("Cloned", "crates.io registry");

    progress.spinner("Counting", "total number of crates");
    let total_crate_count = count_crates(git_index.path())?;
    progress.finish("Counted", format!("a total of {total_crate_count} crates"));

    let (step, _) = progress.bar(total_crate_count, "Parsing", "crates registry");
    let crates: Vec<Crate> = git_index
        .crates_parallel()
        .map(|crate_| {
            step();
            crate_.map_err(anyhow::Error::new)
        })
        .collect::<anyhow::Result<_>>()?;
    drop(step);
    progress.finish("Parsed", "crates registry");

    let (step, _) = progress.bar(total_crate_count, "Indexing", "crate versions");
    let crate_version_index: HashMap<&str, BTreeMap<semver::Version, (&Crate, &Version)>> = crates
        .par_iter()
        .map(|crate_| {
            step();
            let versions = crate_
                .versions()
                .iter()
                .map(move |version| {
                    anyhow::Result::<_>::Ok((
                        semver::Version::parse(version.version()).with_context(|| {
                            format!("expected {:?} to be valid semver", version.version())
                        })?,
                        (crate_, version),
                    ))
                })
                .collect::<anyhow::Result<_>>()?;
            anyhow::Result::<_>::Ok((crate_.name(), versions))
        })
        .collect::<anyhow::Result<_>>()?;
    drop(step);
    progress.finish("Indexed", "crate versions");

    let (step, warn) = progress.bar(total_crate_count, "Indexing", "reverse dependencies");
    let reverse_index: DashMap<&str, DashMap<&semver::Version, DashSet<(&str, &semver::Version)>>> =
        DashMap::new();
    crate_version_index
        .par_iter()
        .try_for_each(|(name, versions)| {
            step();
            for (semver, (crate_, version)) in versions {
                for dependency in version.dependencies() {
                    let Some(dependency_versions) =
                        crate_version_index.get(dependency.crate_name())
                    else {
                        warn(format!(
                            "could not find dependency of {name}@{semver} in crates index: {}",
                            dependency.crate_name()
                        ));
                        continue;
                    };
                    let Ok(req) = semver::VersionReq::parse(dependency.requirement()) else {
                        warn(format!(
                            "could not parse dependency req of {name}@{semver}: {}@{}",
                            dependency.crate_name(),
                            dependency.requirement()
                        ));
                        continue;
                    };

                    let Some((dependency_semver, (dependency_crate, dependency_version))) =
                        dependency_versions
                            .iter()
                            .rev()
                            .find(|(dependency_semver, _)| req.matches(dependency_semver))
                    else {
                        warn(format!(
                            "could not find required dependency version of {name}@{semver}: {}@{}",
                            dependency.crate_name(),
                            req
                        ));
                        continue;
                    };

                    reverse_index
                        .entry(dependency_crate.name())
                        .or_default()
                        .entry(dependency_semver)
                        .or_default()
                        .insert((name, semver));
                }
            }

            anyhow::Result::<_>::Ok(())
        })?;
    drop((step, warn));
    progress.finish("Indexed", "reverse dependencies");

    Ok(())
}
