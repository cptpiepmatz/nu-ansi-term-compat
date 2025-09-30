use std::{collections::{BTreeMap, HashMap}, num::NonZeroU32, path::Path, sync::atomic::AtomicBool};

use anyhow::Context;
use dashmap::mapref::entry;
use gix::{
    Progress, Repository,
    progress::Discard,
    remote::{Direction, fetch::Shallow},
};
use ignore::{DirEntry, WalkBuilder};
use rayon::iter::{ParallelBridge, ParallelIterator};

use crate::INDEX_PATH;

pub fn ensure_index() -> anyhow::Result<()> {
    let url = crates_index::git::URL;

    let path = INDEX_PATH.as_path();
    if gix::open(path).is_ok() {
        return Ok(());
    };

    let prepare_clone = gix::prepare_clone(url, path)?;
    let (mut prepare_checkout, _) = prepare_clone
        .with_shallow(Shallow::DepthAtRemote(
            const { NonZeroU32::new(1).unwrap() },
        ))
        .fetch_then_checkout(Discard, &AtomicBool::new(false))?;
    prepare_checkout.main_worktree(Discard, &AtomicBool::new(false))?;

    Ok(())
}

fn walk_index() -> impl Iterator<Item = Result<DirEntry, anyhow::Error>> {
    WalkBuilder::new(INDEX_PATH.as_path())
        .add_custom_ignore_filename("/.github")
        .build()
        .map(|entry| entry.map_err(anyhow::Error::from))
        .filter(|entry| match entry.as_ref() {
            Err(_) => true,
            Ok(entry) if entry.depth() < 2 => false,
            Ok(entry) => match entry.file_type() {
                None => false,
                Some(file_type) => file_type.is_file(),
            },
        })
}

pub fn count_index() -> anyhow::Result<usize> {
    walk_index().try_fold(0, |accum, entry| {
        entry?;
        Ok(accum + 1)
    })
}

pub fn parse_index(
    step: impl Fn() + Sync,
) -> anyhow::Result<HashMap<String, BTreeMap<semver::Version, crates_index::Version>>> {
    walk_index()
    .par_bridge()
        .map(|entry| {
            step();
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let versions = serde_jsonlines::json_lines(entry.path())?
                .map(|res| res.map_err(anyhow::Error::from))
                .collect::<anyhow::Result<Vec<crates_index::Version>>>()?;
            let versions = versions
                .into_iter()
                .map(|version| Ok((semver::Version::parse(version.version())?, version)))
                .collect::<anyhow::Result<BTreeMap<semver::Version, crates_index::Version>>>()?;
            anyhow::Result::<_>::Ok((name, versions))
        })
        .collect()
}
