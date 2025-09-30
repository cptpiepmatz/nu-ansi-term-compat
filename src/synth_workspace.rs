use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use cargo::{
    GlobalContext,
    core::{
        Dependency, Manifest, Package, PackageId, SourceId, Summary, Workspace, WorkspaceConfig,
        WorkspaceRootConfig, dependency::DepKind, manifest::ManifestMetadata,
    },
    util::{Filesystem, interning::InternedString},
};
use cargo_util_schemas::manifest::RustVersion;
use crates_index::{Crate, DependencyKind, Version};
use toml::Spanned;

pub fn synth_workspace<'gctx>(
    crate_name: &str,
    version: &Version,
    gctx: &'gctx GlobalContext,
) -> anyhow::Result<Workspace<'gctx>> {
    Workspace::ephemeral(synth_package(crate_name, version, gctx)?, gctx, None, true)
}

fn synth_package<'gctx>(
    crate_name: &str,
    version: &Version,
    gctx: &'gctx GlobalContext,
) -> anyhow::Result<Package> {
    Ok(Package::new(
        synth_manifest(crate_name, version, gctx)?,
        &synth_manifest_path(crate_name, version, gctx),
    ))
}

fn synth_manifest_path<'gctx>(
    crate_name: &str,
    version: &Version,
    gctx: &'gctx GlobalContext,
) -> PathBuf {
    crate::LOCK_FILES_PATH
        .join(crate_name)
        .join(version.version())
        .join("Cargo.toml")
}

fn synth_manifest<'gctx>(
    crate_name: &str,
    version: &Version,
    gctx: &'gctx GlobalContext,
) -> anyhow::Result<Manifest> {
    Ok(Manifest::new(
        Default::default(),
        Spanned::new(0..0, Default::default()).into(),
        Default::default(),
        Default::default(),
        synth_summary(crate_name, version, gctx)?,
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        synth_manifest_metadata(crate_name, version, gctx)?,
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        WorkspaceConfig::Member { root: None },
        Default::default(),
        Default::default(),
        synth_rust_version(crate_name, version, gctx)?,
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
    ))
}

fn synth_summary<'gctx>(
    crate_name: &str,
    version: &Version,
    gctx: &'gctx GlobalContext,
) -> anyhow::Result<Summary> {
    Summary::new(
        synth_package_id(crate_name, version, gctx)?,
        synth_dependencies(crate_name, version, gctx)?,
        &synth_features(crate_name, version, gctx),
        version.links(),
        synth_rust_version(crate_name, version, gctx)?,
    )
}

fn synth_package_id<'gctx>(
    crate_name: &str,
    version: &Version,
    gctx: &'gctx GlobalContext,
) -> anyhow::Result<PackageId> {
    PackageId::try_new(
        crate_name,
        version.version(),
        synth_source_id(crate_name, version, gctx)?,
    )
}

fn synth_source_id<'gctx>(
    crate_name: &str,
    version: &Version,
    gctx: &'gctx GlobalContext,
) -> anyhow::Result<SourceId> {
    SourceId::crates_io(gctx)
}

fn synth_dependencies<'gctx>(
    crate_name: &str,
    version: &Version,
    gctx: &'gctx GlobalContext,
) -> anyhow::Result<Vec<Dependency>> {
    version
        .dependencies()
        .iter()
        .map(|dependency| {
            let mut out = Dependency::parse(
                dependency.crate_name(),
                Some(dependency.requirement()),
                synth_source_id(crate_name, version, gctx)?,
            )?;

            out.set_default_features(dependency.has_default_features());
            out.set_explicit_name_in_toml(dependency.name());
            out.set_features(dependency.features());
            out.set_kind(match dependency.kind() {
                DependencyKind::Normal => DepKind::Normal,
                DependencyKind::Dev => DepKind::Development,
                DependencyKind::Build => DepKind::Build,
            });
            out.set_optional(dependency.is_optional());
            out.set_platform(
                dependency
                    .target()
                    .map(|target| target.parse())
                    .transpose()?,
            );

            Ok(out)
        })
        .collect()
}

fn synth_features<'gctx>(
    crate_name: &str,
    version: &Version,
    gctx: &'gctx GlobalContext,
) -> BTreeMap<InternedString, Vec<InternedString>> {
    version
        .features()
        .iter()
        .map(|(key, vals)| {
            let key = InternedString::new(key);
            let vals = vals.iter().map(|val| InternedString::new(val)).collect();
            (key, vals)
        })
        .collect()
}

// fn synth_workspace_config<'gctx>(
//     crate_name: &str,
//     version: &Version,
//     gctx: &'gctx GlobalContext,
// ) -> WorkspaceConfig {
//     let root_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
//         .join("lock-files")
//         .join(crate_name.name())
//         .join(version.version());
//     WorkspaceConfig::Root(WorkspaceRootConfig::new(
//         &root_dir, &None, &None, &None, &None, &None,
//     ))
// }

fn synth_rust_version<'gctx>(
    crate_name: &str,
    version: &Version,
    gctx: &'gctx GlobalContext,
) -> anyhow::Result<Option<RustVersion>> {
    version
        .rust_version()
        .map(|rv| rv.parse())
        .transpose()
        .map_err(anyhow::Error::from)
}

fn synth_manifest_metadata<'gctx>(
    crate_name: &str,
    version: &Version,
    gctx: &'gctx GlobalContext,
) -> anyhow::Result<ManifestMetadata> {
    Ok(ManifestMetadata {
        authors: Default::default(),
        keywords: Default::default(),
        categories: Default::default(),
        license: Default::default(),
        license_file: Default::default(),
        description: Default::default(),
        readme: Default::default(),
        homepage: Default::default(),
        repository: Default::default(),
        documentation: Default::default(),
        badges: Default::default(),
        links: Default::default(),
        rust_version: synth_rust_version(crate_name, version, gctx)?,
    })
}
