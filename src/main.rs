use anyhow::Context;
use cargo::{
    core::{
        compiler::{CompileKind, CompileTarget, RustcTargetData}, registry::PackageRegistry, resolver::{CliFeatures, ForceAllTargets, HasDevUnits}, Manifest, Shell, SourceId, Summary
    }, sources::SourceConfigMap, util::{context::Definition, ConfigValue}, GlobalContext
};
use count_crates::count_crates;
use crates_index::{Crate, DependencyKind, Version};
use dashmap::{DashMap, DashSet};
use progress::Progress;
use rayon::iter::{IntoParallelRefIterator, ParallelBridge, ParallelIterator};
use serde_spanned::Spanned;
use std::{
    cell::LazyCell, collections::{BTreeMap, HashMap, HashSet, VecDeque}, env, fs::File, io::{BufReader, BufWriter}, ops::Deref, path::{Path, PathBuf}, sync::{atomic::{AtomicUsize, Ordering}, LazyLock}, thread
};
use url::Url;

use crate::synth_workspace::synth_workspace;

mod count_crates;
mod progress;
mod synth_workspace;

const SEARCH_CRATE: &str = "nu-ansi-term";
const SEARCH_REQ: LazyLock<semver::VersionReq> =
    LazyLock::new(|| semver::VersionReq::parse("^0.50").expect("valid version req"));
const SEARCH_MSRV: semver::Version = semver::Version::new(1, 62, 1);

static COMPILE_KIND: LazyLock<CompileKind> = LazyLock::new(|| {
    CompileKind::Target(CompileTarget::new("x86_64-pc-windows-msvc").expect("valid compile target"))
});

static NEXT_THREAD_ID: AtomicUsize = AtomicUsize::new(1);
thread_local! {
    static GLOBAL_CONTEXT: LazyCell<anyhow::Result<GlobalContext>> = LazyCell::new(|| {
        let id = NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed);
        let cargo_home = format!("{}/cargo-homes/worker-{:02}", env!("CARGO_MANIFEST_DIR"), id);
        
        let shell = Shell::new();
        let cwd = env::current_dir().context("couldn't get the current directory of the process")?;
        
        let mut gctx = GlobalContext::new(shell, cwd, cargo_home.into());

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
        gctx.configure(0, false, None, true, false, true, &None, &[], &[])?;
        Ok(gctx)
    });
}

fn main() -> anyhow::Result<()> {
    let mut progress = Progress::new();

    progress.spinner("Cloning", "crates.io registry");
    let git_index = crates_index::GitIndex::new_cargo_default()?;
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
    let index: HashMap<&str, BTreeMap<semver::Version, (&Crate, &Version)>> = crates
        .par_iter()
        .map(|crate_| {
            step();
            let versions = crate_
                .versions()
                .par_iter()
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

    let (step, warn) = progress.bar(index.len(), "Resolving", "crate dependencies");
    index
        .iter()
        .flat_map(|(crate_name, versions)| {
            match versions
                .iter()
                .rev()
                .find(|(_, (_, version))| !version.is_yanked()) {
                    Some(version) => Some(version),
                    None => {step(); None}
                }
        })
        .par_bridge()
        .try_for_each(|(semver, (crate_, version))| {
            step();
            GLOBAL_CONTEXT.with(|gctx| {
                let gctx = gctx.as_ref().map_err(|err| anyhow::anyhow!("{err}"))?;

                let workspace = synth_workspace(crate_, version, &gctx)?;
                let mut registry =
                    PackageRegistry::new_with_source_config(&gctx, SourceConfigMap::new(&gctx)?)?;
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
                if let Err(err) = resolve {
                    warn(err.to_string());
                }

                anyhow::Result::<_>::Ok(())
            })?;
            anyhow::Result::<_>::Ok(())
        })?;
    drop((step, warn));
    progress.finish("Resolved", format!("{} crate dependencies", index.len()));

    // let (step, warn) = progress.bar(total_crate_count, "Indexing", "reverse dependencies");
    // let reverse_index: DashMap<&str, DashMap<&semver::Version, DashSet<(&str, &semver::Version)>>> =
    //     DashMap::new();
    // index.par_iter().try_for_each(|(name, versions)| {
    //     step();
    //     for (semver, (crate_, version)) in versions {
    //         if version.is_yanked() {
    //             continue;
    //         }

    //         for dependency in version.dependencies() {
    //             if let DependencyKind::Dev = dependency.kind() {
    //                 continue;
    //             }

    //             // if dependency.is_optional() {
    //             //     // maybe too aggressive
    //             //     continue;
    //             // }

    //             let Some(dependency_versions) = index.get(dependency.crate_name()) else {
    //                 // warn(format!(
    //                 //     "could not find dependency of {name}@{semver} in crates index: {}",
    //                 //     dependency.crate_name()
    //                 // ));
    //                 continue;
    //             };
    //             let Ok(req) = semver::VersionReq::parse(dependency.requirement()) else {
    //                 // warn(format!(
    //                 //     "could not parse dependency req of {name}@{semver}: {}@{}",
    //                 //     dependency.crate_name(),
    //                 //     dependency.requirement()
    //                 // ));
    //                 continue;
    //             };

    //             let Some((dependency_semver, (dependency_crate, dependency_version))) =
    //                 dependency_versions.iter().rev().find(
    //                     |(dependency_semver, (_, dependency_version))| {
    //                         req.matches(dependency_semver) && !dependency_version.is_yanked()
    //                     },
    //                 )
    //             else {
    //                 // warn(format!(
    //                 //     "could not find required dependency version of {name}@{semver}: {}@{}",
    //                 //     dependency.crate_name(),
    //                 //     req
    //                 // ));
    //                 continue;
    //             };

    //             reverse_index
    //                 .entry(dependency_crate.name())
    //                 .or_default()
    //                 .entry(dependency_semver)
    //                 .or_default()
    //                 .insert((name, semver));
    //         }
    //     }

    //     anyhow::Result::<_>::Ok(())
    // })?;
    // drop((step, warn));
    // progress.finish("Indexed", "reverse dependencies");

    // progress.spinner(
    //     "Walking",
    //     format!("reverse dependencies for {SEARCH_CRATE}"),
    // );
    // let mut reverse_dependencies = HashSet::<(&str, &semver::Version)>::new();
    // let mut queue = VecDeque::<(&str, &semver::Version)>::new();
    // if let Some(versions) = index.get(SEARCH_CRATE) {
    //     for (semver, (crate_, _)) in versions
    //         .iter()
    //         .filter(|(semver, _)| SEARCH_REQ.matches(semver))
    //     {
    //         queue.push_back((crate_.name(), semver));
    //     }
    // }
    // while let Some((crate_name, semver)) = queue.pop_front() {
    //     if let Some(versions) = reverse_index.get(crate_name) {
    //         if let Some(version) = versions.get(semver) {
    //             let dependents = version.value();
    //             for dependent in dependents.iter() {
    //                 let dependent = *dependent.key();
    //                 if reverse_dependencies.insert(dependent) {
    //                     queue.push_back(dependent);
    //                 }
    //             }
    //         }
    //     }
    // }
    // progress.finish(
    //     "Walked",
    //     format!(
    //         "reverse dependencies for {SEARCH_CRATE}, found {} entries",
    //         reverse_dependencies.len()
    //     ),
    // );

    // let (step, _) = progress.bar(
    //     reverse_dependencies.len(),
    //     "Filtering",
    //     "reverse dependencies for latest versions",
    // );
    // let filtered_reverse_dependencies: HashSet<(&str, &semver::Version)> = reverse_dependencies
    //     .par_iter()
    //     .filter(|(crate_name, crate_semver)| {
    //         step();
    //         let Some(versions) = index.get(crate_name) else {
    //             return false;
    //         };
    //         let Some((latest_semver, _)) = versions
    //             .iter()
    //             .rev()
    //             .find(|(_, (_, version))| !version.is_yanked())
    //         else {
    //             return false;
    //         };
    //         latest_semver.eq(crate_semver)
    //     })
    //     .cloned()
    //     .collect();
    // drop(step);
    // progress.finish(
    //     "Filtered",
    //     format!(
    //         "reverse dependencies to latest version, reduced to {} entries",
    //         filtered_reverse_dependencies.len()
    //     ),
    // );

    // let (step, _) = progress.bar(
    //     filtered_reverse_dependencies.len(),
    //     "Filtering",
    //     format!("reverse dependencies for msrv {SEARCH_MSRV}"),
    // );
    // let filtered_reverse_dependencies: HashSet<(&str, &semver::Version)> =
    //     filtered_reverse_dependencies
    //         .iter()
    //         .filter(|(crate_name, crate_semver)| {
    //             step();

    //             let Some(versions) = index.get(crate_name) else {
    //                 return false;
    //             };

    //             let Some((crate_, version)) = versions.get(crate_semver) else {
    //                 return false;
    //             };

    //             let Some(crate_msrv) = version.rust_version() else {
    //                 return true;
    //             };
    //             let Ok(crate_msrv) = semver::VersionReq::parse(crate_msrv) else {
    //                 return true;
    //             };

    //             crate_msrv.matches(&SEARCH_MSRV)
    //         })
    //         .cloned()
    //         .collect();
    // drop(step);
    // progress.finish(
    //     "Filtered",
    //     format!(
    //         "reverse dependencies for msrv {SEARCH_MSRV}, reduced to {} entries",
    //         filtered_reverse_dependencies.len()
    //     ),
    // );

    // let (step, _) = progress.bar(
    //     filtered_reverse_dependencies.len(),
    //     "Filtering",
    //     "reverse dependencies for used libraries",
    // );
    // let filtered_reverse_dependencies: HashSet<(&str, &semver::Version)> =
    //     filtered_reverse_dependencies
    //         .iter()
    //         .filter(|(crate_name, crate_semver)| {
    //             step();
    //             reverse_index.get(crate_name).is_none()
    //         })
    //         .cloned()
    //         .collect();
    // drop(step);
    // progress.finish(
    //     "Filtered",
    //     format!(
    //         "reverse dependencies for used libraries, reduced to {} entries",
    //         filtered_reverse_dependencies.len()
    //     ),
    // );

    // let file = File::create("reverse_dependencies.json").unwrap();
    // let writer = BufWriter::new(file);
    // serde_json::to_writer(writer, &filtered_reverse_dependencies).unwrap();

    Ok(())
}
