use anyhow::Context;
use cargo::{
    GlobalContext,
    core::{
        Shell,
        registry::PackageRegistry,
        resolver::{CliFeatures, HasDevUnits, ResolveBehavior},
    },
    sources::SourceConfigMap,
    util::{ConfigValue, context::Definition},
};
use parking_lot::Mutex;
use progress::Progress;
use rayon::iter::{ParallelBridge, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::{
    cell::LazyCell,
    collections::HashMap,
    env,
    fs::File,
    io::BufWriter,
    ops::Deref,
    path::PathBuf,
    sync::{
        LazyLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::synth_workspace::synth_workspace;

mod index;
mod progress;
mod synth_workspace;

const SEARCH_CRATE: &str = "nu-ansi-term";
const RESOLVE_BEHAVIOR: ResolveBehavior = ResolveBehavior::V2;

type LazyPath = LazyLock<PathBuf>;
static CWD: LazyPath =
    LazyPath::new(|| env::current_dir().expect("couldn't get the current directory"));
static CARGO_HOMES_PATH: LazyPath = LazyPath::new(|| CWD.join("cargo-homes"));
static INDEX_PATH: LazyPath = LazyPath::new(|| CWD.join("index"));
static LOCK_FILES_PATH: LazyPath = LazyPath::new(|| CWD.join("lock-files"));
static DEPENDENTS_PATH: LazyPath = LazyPath::new(|| CWD.join("dependents.json"));
static UNRESOLVABLE_PATH: LazyPath = LazyPath::new(|| CWD.join("unresolvable.json"));

static NEXT_THREAD_ID: AtomicUsize = AtomicUsize::new(1);
thread_local! {
    static GLOBAL_CONTEXT: LazyCell<anyhow::Result<GlobalContext>> = LazyCell::new(|| {
        let id = NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed);
        let cargo_home = format!("{}/worker-{:02}", CARGO_HOMES_PATH.display(), id);
        let mut gctx = GlobalContext::new(Shell::new(), CWD.clone(), cargo_home.into());

        let def = Definition::Path(file!().into());
        gctx.set_values(HashMap::from_iter([
            (
                "source".to_string(),
                ConfigValue::Table(
                    HashMap::from_iter([
                        (
                            "crates-io".to_string(),
                            ConfigValue::Table(
                                HashMap::from_iter([(
                                    "replace-with".to_string(),
                                    ConfigValue::String("prepared".to_string(), def.clone()),
                                )]),
                                def.clone(),
                            ),
                        ),
                        (
                            "prepared".to_string(),
                            ConfigValue::Table(
                                HashMap::from_iter([(
                                    "local-registry".to_string(),
                                    ConfigValue::String(
                                        INDEX_PATH
                                          .parent()
                                          .expect("not root")
                                          .to_string_lossy()
                                          .to_string(),
                                        def.clone()
                                    )
                                )]),
                                def.clone()
                            ),
                        ),
                    ]),
                    def.clone(),
                ),
            ),
        ]))?;
        gctx.configure(0, true, None, false, false, true, &None, &[], &[])?;
        Ok(gctx)
    });
}

fn main() -> anyhow::Result<()> {
    let mut progress = Progress::new();

    progress.spinner("Cloning", "crates.io registry");
    index::ensure_index()?;
    progress.finish("Cloned", "crates.io registry");

    progress.spinner("Counting", "total number of crates");
    let total_crate_count = index::count_index()?;
    progress.finish("Counted", format!("a total of {total_crate_count} crates"));

    let (step, _) = progress.bar(total_crate_count, "Parsing", "crates registry");
    let index = index::parse_index(step)?;
    progress.finish("Parsed", "crates registry");

    let (step, warn) = progress.bar(index.len(), "Resolving", "crate dependencies");
    let resolve_errors = Mutex::<Vec<ResolveError>>::default();
    let dependents = Mutex::<Vec<(&str, &semver::Version)>>::default();
    index
        .iter()
        .flat_map(|(crate_name, versions)| {
            match versions
                .iter()
                .rev()
                .find(|(_, version)| !version.is_yanked())
            {
                Some((semver, version)) => Some((crate_name, semver, version)),
                None => {
                    step();
                    None
                }
            }
        })
        .par_bridge()
        .try_for_each(|(crate_name, semver, version)| {
            step();
            GLOBAL_CONTEXT
                .with(|gctx| {
                    let gctx = gctx.as_ref().map_err(|err| anyhow::anyhow!("{err}"))?;

                    let workspace = synth_workspace(crate_name, version, &gctx)?;
                    let mut registry = PackageRegistry::new_with_source_config(
                        &gctx,
                        SourceConfigMap::new(&gctx)?,
                    )?;
                    registry.lock_patches();
                    let resolve = cargo::ops::resolve_with_previous(
                        &mut registry,
                        &workspace,
                        &CliFeatures {
                            features: Default::default(),
                            all_features: true,
                            uses_default_features: true,
                        },
                        HasDevUnits::No,
                        None,
                        None,
                        &[],
                        false,
                    );

                    let resolve = match resolve {
                        Ok(resolve) => resolve,
                        Err(err) => {
                            let err =
                                ResolveError::from_str(crate_name.clone(), semver.clone(), err)
                                    .map_err(|err| anyhow::Error::msg(err))?;
                            let mut resolve_errors = resolve_errors.lock();
                            resolve_errors.push(err);
                            return Ok(());
                        }
                    };

                    if resolve
                        .iter()
                        .find(|package_id| package_id.name().as_str() == SEARCH_CRATE)
                        .is_some()
                    {
                        dependents.lock().push((crate_name, semver));
                    }

                    anyhow::Result::<_>::Ok(())
                })
                .with_context(|| format!("error while resolving {}@{}", crate_name, semver))?;
            anyhow::Result::<_>::Ok(())
        })?;
    drop((step, warn));
    progress.finish(
        "Resolved",
        format!(
            "{} crate dependencies, {} dependents, {} unresolvable",
            index.len(),
            dependents.lock().len(),
            resolve_errors.lock().len()
        ),
    );

    progress.spinner("Writing", "dependents");
    let file = File::create(DEPENDENTS_PATH.as_path())?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, dependents.lock().deref())?;
    progress.finish("Writing", "dependents");

    progress.spinner("Writing", "unresolvable crates");
    let file = File::create(UNRESOLVABLE_PATH.as_path())?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, resolve_errors.lock().deref())?;
    progress.finish("Writing", "unresolvable crates");

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResolveError {
    crate_name: String,
    version: semver::Version,
    kind: ResolveErrorKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum ResolveErrorKind {
    DependencyFullyYanked,
    UnavailableDependency,
    CyclicDependency,
    AllPossibleVersionsConflictWithPreviouslySelected,
    NoMatchingPackageFound,
    CandidateVersionsFoundDidntMatch,
    FeatureConflict,
    IndexEntryIsInvalid,
}

impl ResolveError {
    pub fn from_str(
        crate_name: String,
        version: semver::Version,
        value: impl ToString,
    ) -> Result<ResolveError, String> {
        let value = value.to_string();
        let kind = 'kind: {
            if value.contains("failed to select a version for")
                && value.contains("which could resolve this conflict")
            {
                break 'kind Some(ResolveErrorKind::FeatureConflict);
            }

            if value.contains("candidate versions found which didn't match") {
                break 'kind Some(ResolveErrorKind::CandidateVersionsFoundDidntMatch);
            }

            if value.contains("all possible versions conflict with previously selected packages") {
                break 'kind Some(
                    ResolveErrorKind::AllPossibleVersionsConflictWithPreviouslySelected,
                );
            }

            if value.contains("failed to select a version for the requirement") {
                if value.contains("is yanked") {
                    break 'kind Some(ResolveErrorKind::DependencyFullyYanked);
                }

                if value.contains("is unavailable") {
                    break 'kind Some(ResolveErrorKind::UnavailableDependency);
                }
            }

            if value.contains("cyclic package dependency") {
                break 'kind Some(ResolveErrorKind::CyclicDependency);
            }

            if value.contains("no matching package named") {
                break 'kind Some(ResolveErrorKind::NoMatchingPackageFound);
            }

            if value.contains("index entry is invalid") {
                break 'kind Some(ResolveErrorKind::IndexEntryIsInvalid);
            }

            None
        };
        match kind {
            None => Err(value),
            Some(kind) => Ok(ResolveError {
                crate_name,
                version,
                kind,
            }),
        }
    }
}
