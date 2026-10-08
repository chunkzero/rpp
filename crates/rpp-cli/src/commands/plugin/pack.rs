//! `rpp plugin pack`: bundle a plugin into a self-contained registry archive.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rpp::js::{jsx_modules, JSX_IMPORT_SOURCE, SDK_CONFIG, SDK_INDEX};
use rpp::manifest::PluginManifest;
use rpp_fetch::registry::PACKAGE_MANIFEST;
use rpp_js::{BundleRequest, PackOutput, PackRequest};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// A written archive.
#[derive(Debug, Serialize)]
pub struct Packed {
    /// The plugin name.
    pub name: String,
    /// The plugin version.
    pub version: String,
    /// The `rpp` version range from the manifest.
    pub rpp: String,
    /// The manifest description.
    pub description: Option<String>,
    /// The archive's file name.
    pub file: String,
    /// The archive's path, inside the `--out` directory.
    #[serde(skip)]
    pub path: PathBuf,
    /// Lowercase hex SHA-256 of the archive.
    pub sha256: String,
}

/// Pack the plugin in `dir` into `<out>/<name>-<version>.rpp.tgz` and
/// `<out>/<name>-<version>.rpp.tgz.sha256`.
pub fn pack(dir: &Path, out: &Path) -> Result<Packed> {
    let manifest_path = dir.join(PACKAGE_MANIFEST);
    let text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest = PluginManifest::parse_json(&text, &manifest_path)?;
    let Some(range) = &manifest.rpp else {
        bail!(
            "{} must set `rpp` to the rpp version range the plugin supports",
            manifest_path.display()
        );
    };

    let bundled = rpp_js::pack(&PackRequest {
        root: dir.to_path_buf(),
        plugin: manifest.entry.clone(),
        config: manifest.config.clone(),
        exports: manifest.exports.clone(),
        self_specifier: Some(format!("plugin:{}", manifest.id)),
        jsx_import_source: Some(JSX_IMPORT_SOURCE.to_string()),
    })?;
    let files = collect_files(dir, &text, &manifest, bundled)?;
    let archive = rpp_archive::pack(&files)?;
    self_check(&archive)?;

    let file = format!("{}-{}.rpp.tgz", manifest.id, manifest.version);
    let (path, sha256) = write_outputs(out, &file, &archive)?;
    Ok(Packed {
        name: manifest.id,
        version: manifest.version.to_string(),
        rpp: range.to_string(),
        description: manifest.description,
        file,
        path,
        sha256,
    })
}

/// The archive's files: the packed manifest, the bundle and every component module.
fn collect_files(
    dir: &Path,
    text: &str,
    manifest: &PluginManifest,
    bundled: PackOutput,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    files.insert(
        PACKAGE_MANIFEST.to_string(),
        packed_manifest(text, &bundled)?,
    );
    for (path, contents) in bundled.files.into_iter().chain(bundled.declarations) {
        files.insert(path, contents.into_bytes());
    }
    for component in manifest.components.values() {
        let path = dir.join(&component.module);
        let bytes =
            fs::read(&path).with_context(|| format!("reading component {}", path.display()))?;
        files.insert(component.module.trim_start_matches("./").to_string(), bytes);
    }
    Ok(files)
}

/// The manifest `text` with its entries pointing at the bundle and without
/// `dependencies`.
fn packed_manifest(text: &str, bundled: &PackOutput) -> Result<Vec<u8>> {
    let mut raw: Value = serde_json::from_str(text)?;
    let object = raw
        .as_object_mut()
        .context("the plugin manifest is not a JSON object")?;
    object.remove("dependencies");
    object.insert("entry".into(), bundled.plugin.clone().into());
    if let Some(config) = &bundled.config {
        object.insert("config".into(), config.clone().into());
    }
    if !bundled.exports.is_empty() {
        let exports = bundled
            .exports
            .iter()
            .map(|(subpath, path)| (format!("./{subpath}"), Value::from(path.clone())))
            .collect();
        object.insert("exports".into(), Value::Object(exports));
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&raw)?).into_bytes())
}

/// Write `archive` to `<out>/<file>` and its checksum to `<out>/<file>.sha256`,
/// returning the archive's path and SHA-256.
fn write_outputs(out: &Path, file: &str, archive: &[u8]) -> Result<(PathBuf, String)> {
    let sha256 = format!("{:x}", Sha256::digest(archive));
    fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let path = out.join(file);
    fs::write(&path, archive).with_context(|| format!("writing {}", path.display()))?;
    let checksum = out.join(format!("{file}.sha256"));
    fs::write(&checksum, format!("{sha256}  {file}\n"))
        .with_context(|| format!("writing {}", checksum.display()))?;
    Ok((path, sha256))
}

/// Unpacks the archive as installs do and bundles each entry with only the SDK modules
/// its loader provides: `rpp` for the plugin, `rpp:config` for the config, both for exports
/// (pack sources and `rpp.config.ts` import them), and `rpp:jsx`.
fn self_check(archive: &[u8]) -> Result<()> {
    let dir = tempfile::tempdir()?;
    rpp_archive::unpack(archive, dir.path()).context("unpacking the archive")?;
    let manifest = PluginManifest::load(dir.path()).context("checking the packed manifest")?;
    let sdk = ("rpp", SDK_INDEX);
    let config_sdk = ("rpp:config", SDK_CONFIG);
    let mut entries = vec![(manifest.entry, vec![sdk])];
    if let Some(config) = manifest.config {
        entries.push((config, vec![config_sdk]));
    }
    entries.extend(
        manifest
            .exports
            .into_values()
            .map(|module| (module, vec![sdk, config_sdk])),
    );
    for (entry, sdk_modules) in entries {
        let request = BundleRequest {
            root: dir.path().to_path_buf(),
            entry: entry.clone(),
            virtual_modules: sdk_modules
                .into_iter()
                .map(|(specifier, source)| (specifier.to_string(), source.to_string()))
                .chain(jsx_modules())
                .collect(),
            jsx_import_source: Some(JSX_IMPORT_SOURCE.to_string()),
            ..Default::default()
        };
        rpp_js::bundle(&request)
            .with_context(|| format!("the packed `{entry}` is not self-contained"))?;
    }
    Ok(())
}
