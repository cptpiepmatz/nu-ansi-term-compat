use count_crates::count_crates;
use crate_version::CrateVersion;
use crates_index::{Crate, Version};
use dashmap::{DashMap, DashSet};
use progress::Progress;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::File,
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
    sync::LazyLock,
};

use crate::krate::Krate;

mod count_crates;
mod crate_version;
mod krate;
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

    let step = progress.bar(total_crate_count, "Parsing", "crates registry");
    let krates: HashMap<String, Krate> = git_index
        .crates_parallel()
        .map(|krate| {
            step();
            krate
                .map_err(anyhow::Error::new)
                .map(|krate| (krate.name().to_string(), krate.into()))
        })
        .collect::<anyhow::Result<_>>()?;
    drop(step);
    progress.finish("Parsed", "crates registry");

    let reverse_index: DashMap<String, DashSet<CrateVersion>> = 'make_reverse_index: {
        'try_file: {
            let Ok(file) = File::open(REVERSE_INDEX_PATH.as_path()) else {
                break 'try_file;
            };
            progress.spinner("Loading", "reverse index cache");
            let reader = BufReader::new(file);
            let Result::Ok(reverse_index) = serde_json::from_reader(reader) else {
                progress.warning("could not deserialize reverse index cache");
                break 'try_file;
            };

            let reverse_index: DashMap<String, DashSet<CrateVersion>> = reverse_index;
            progress.finish(
                "Loaded",
                format!("reverse index cache with {} entries", reverse_index.len()),
            );
            break 'make_reverse_index reverse_index;
        }

        let reverse_index = DashMap::new();
        let step = progress.bar(total_crate_count, "Indexing", "dependents");
        git_index.crates_parallel().try_for_each(|crate_| {
            let crate_ = crate_?;
            let version = crate_.most_recent_version();
            for dependency in version.dependencies() {
                let key = dependency.crate_name().to_owned();
                reverse_index
                    .entry(key)
                    .or_insert_with(DashSet::new)
                    .insert(version.clone().into());
            }

            step();
            anyhow::Result::<()>::Ok(())
        })?;
        drop(step);
        progress.finish(
            "Indexed",
            format!("{} reverse dependencies", reverse_index.len()),
        );

        progress.spinner("Writing", "reverse index cache");
        let writer = BufWriter::new(File::create(REVERSE_INDEX_PATH.as_path())?);
        serde_json::to_writer(writer, &reverse_index)?;
        progress.finish("Written", "reverse index cache");

        reverse_index
    };

    let reverse_dependencies = {
        progress.spinner("Walking", "reverse dependencies");

        let reverse_dependencies: DashSet<CrateVersion> = DashSet::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        visited.insert(SEARCH_CRATE.to_string());
        queue.push_back(SEARCH_CRATE.to_string());

        while let Some(name) = queue.pop_front() {
            if let Some(dependents) = reverse_index.get(&name) {
                for dependent in dependents.value().iter() {
                    let dependent = dependent.key();
                    if reverse_dependencies.insert(dependent.clone().into()) {
                        let name = dependent.as_inner().name();
                        if visited.insert(name.to_string()) {
                            queue.push_back(name.to_string());
                        }
                    }
                }
            }
        }

        progress.finish(
            "Walked",
            format!("reverse dependencies, found {}", reverse_dependencies.len()),
        );

        reverse_dependencies
    };

    Ok(())
}
