use anyhow::Context;
use cargo::{
    core::{
        compiler::{CompileKind, CompileTarget, RustcTargetData}, registry::PackageRegistry, resolver::{CliFeatures, ForceAllTargets, HasDevUnits, ResolveBehavior}, Manifest, Shell, SourceId, Summary
    }, ops::write_pkg_lockfile, sources::SourceConfigMap, util::{context::Definition, ConfigValue, Filesystem}, GlobalContext
};
use crates_index::{Crate, DependencyKind, Version};
use dashmap::{DashMap, DashSet};
use parking_lot::Mutex;
use progress::Progress;
use rayon::iter::{IntoParallelRefIterator, ParallelBridge, ParallelIterator};
use serde_spanned::Spanned;
use std::{
    cell::LazyCell,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    env,
    fs::File,
    io::{BufReader, BufWriter},
    ops::Deref,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        LazyLock,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};
use url::Url;

use crate::synth_workspace::synth_workspace;

mod index;
mod progress;
mod synth_workspace;

static CWD: LazyLock<PathBuf> = LazyLock::new(|| {
    env::current_dir().expect("couldn't get the current directory of the process")
});
static CARGO_HOMES_PATH: LazyLock<PathBuf> = LazyLock::new(|| CWD.join("cargo-homes"));
static INDEX_PATH: LazyLock<PathBuf> = LazyLock::new(|| CWD.join("index"));
static LOCK_FILES_PATH: LazyLock<PathBuf> = LazyLock::new(|| CWD.join("lock-files"));

const SEARCH_CRATE: &str = "nu-ansi-term";
const SEARCH_REQ: LazyLock<semver::VersionReq> =
    LazyLock::new(|| semver::VersionReq::parse("^0.50").expect("valid version req"));
const SEARCH_MSRV: semver::Version = semver::Version::new(1, 62, 1);

const RESOLVE_BEHAVIOR: ResolveBehavior = ResolveBehavior::V2;
static COMPILE_KIND: LazyLock<CompileKind> = LazyLock::new(|| {
    CompileKind::Target(CompileTarget::new("x86_64-pc-windows-msvc").expect("valid compile target"))
});

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
                                        "D:/Projects/nu-ansi-term-compat/".to_string(),
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
        .take(1000)
        .par_bridge()
        .try_for_each(|(crate_name, semver, version)| {
            step();
            GLOBAL_CONTEXT.with(|gctx| {
                let gctx = gctx.as_ref().map_err(|err| anyhow::anyhow!("{err}"))?;

                let workspace = synth_workspace(crate_name, version, &gctx)?;
                let resolve = match cargo::ops::load_pkg_lockfile(&workspace)? {
                    Some(resolve) => resolve,
                    None => {
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

                        match resolve {
                            Err(err) => {
                                let err =
                                    ResolveError::from_str(crate_name.clone(), semver.clone(), err)
                                        .map_err(|err| anyhow::Error::msg(err))?;
                                let mut resolve_errors = resolve_errors.lock();
                                resolve_errors.push(err);
                                return Ok(());
                            }
                            Ok(mut resolve) => {
                                write_pkg_lockfile(&workspace, &mut resolve)?;
                                resolve
                            }
                        }
                    }
                };

                // let uses_nu_ansi_term = resolve.iter().find(|package_id| package_id.name().as_str() == "nu-ansi-term");
                // println!("{} uses nu-ansi-term? {}", crate_.name(), uses_nu_ansi_term.is_some());

                anyhow::Result::<_>::Ok(())
            }).with_context(|| format!("error while resolving {}@{}", crate_name, semver))?;
            anyhow::Result::<_>::Ok(())
        })?;
    drop((step, warn));
    progress.finish(
        "Resolved",
        format!(
            "{} crate dependencies, {} unresolvable",
            index.len(),
            resolve_errors.lock().len()
        ),
    );

    Ok(())
}

struct ResolveError {
    crate_name: String,
    version: semver::Version,
    kind: ResolveErrorKind,
}

enum ResolveErrorKind {
    DependencyFullyYanked,
}

impl ResolveError {
    pub fn from_str(
        crate_name: String,
        version: semver::Version,
        value: impl ToString,
    ) -> Result<ResolveError, String> {
        let value = value.to_string();
        if value.contains("failed to select a version for the requirement")
            && value.contains("is yanked")
        {
            return Ok(ResolveError {
                crate_name,
                version,
                kind: ResolveErrorKind::DependencyFullyYanked,
            });
        }

        Err(value)
    }
}
