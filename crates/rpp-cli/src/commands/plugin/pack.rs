//! `rpp plugin pack`: bundle a plugin into a self-contained registry archive.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use flate2::write::GzEncoder;
use flate2::{read::GzDecoder, Compression};
use rpp::js::SDK_FILES;
use rpp::manifest::PluginManifest;
use rpp_js::{BundleRequest, PackRequest};
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
    /// The archive path.
    pub file: PathBuf,
    /// Lowercase hex SHA-256 of the archive.
    pub sha256: String,
}

/// Pack the plugin in `dir` into `<out>/<name>-<version>.rpp.tgz` and
/// `<out>/<name>-<version>.rpp.tgz.sha256`.
pub fn pack(dir: &Path, out: &Path) -> Result<Packed> {
    let manifest_path = dir.join("rpp.json");
    let text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest = PluginManifest::parse_json(&text, &manifest_path)?;
    let Some(range) = &manifest.rpp else {
        bail!(
            "{} must set `rpp` to the rpp version range the plugin supports",
            manifest_path.display()
        );
    };
    let mut raw: Value = serde_json::from_str(&text)?;

    let mut entries = BTreeMap::from([("plugin".to_string(), manifest.entry.clone())]);
    if let Some(config) = &manifest.config {
        entries.insert("config".to_string(), config.clone());
    }
    let packed = rpp_js::pack(&PackRequest {
        root: dir.to_path_buf(),
        entries,
        self_specifier: Some(format!("#plugins/{}", manifest.id)),
    })?;

    let object = raw.as_object_mut().expect("manifest is an object");
    object.remove("dependencies");
    object.insert("entry".into(), "dist/plugin.js".into());
    if manifest.config.is_some() {
        object.insert("config".into(), "dist/config.js".into());
    }

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    files.insert(
        "rpp.json".into(),
        format!("{}\n", serde_json::to_string_pretty(&raw)?).into_bytes(),
    );
    for (path, contents) in packed.files.into_iter().chain(packed.declarations) {
        files.insert(path, contents.into_bytes());
    }
    for component in manifest.components.values() {
        let path = dir.join(&component.module);
        let bytes =
            fs::read(&path).with_context(|| format!("reading component {}", path.display()))?;
        files.insert(component.module.trim_start_matches("./").to_string(), bytes);
    }

    let archive = archive(&files)?;
    self_check(&archive)?;

    let sha256 = format!("{:x}", Sha256::digest(&archive));
    let name = format!("{}-{}.rpp.tgz", manifest.id, manifest.version);
    fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let file = out.join(&name);
    fs::write(&file, &archive).with_context(|| format!("writing {}", file.display()))?;
    let checksum = out.join(format!("{name}.sha256"));
    fs::write(&checksum, format!("{sha256}  {name}\n"))
        .with_context(|| format!("writing {}", checksum.display()))?;

    Ok(Packed {
        name: manifest.id,
        version: manifest.version.to_string(),
        rpp: range.to_string(),
        description: manifest.description,
        file,
        sha256,
    })
}

/// A gzipped tar with sorted entries at the archive root and no timestamps or owners.
fn archive(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, contents) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append_data(&mut header, path, contents.as_slice())?;
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    std::io::Write::write_all(&mut encoder, &builder.into_inner()?)?;
    Ok(encoder.finish()?)
}

/// Unpacks the archive and bundles its entries the way an installing rpp does.
fn self_check(archive: &[u8]) -> Result<()> {
    let dir = tempfile::tempdir()?;
    tar::Archive::new(GzDecoder::new(archive))
        .unpack(dir.path())
        .context("unpacking the archive")?;
    let manifest = PluginManifest::load(dir.path()).context("checking the packed manifest")?;
    let mut entries = vec![manifest.entry];
    entries.extend(manifest.config);
    for entry in entries {
        let request = BundleRequest {
            root: dir.path().to_path_buf(),
            entry: entry.clone(),
            virtual_modules: SDK_FILES
                .iter()
                .map(|(name, source)| {
                    let specifier = match *name {
                        "index.ts" => "#rpp".to_string(),
                        other => format!("#rpp/{}", other.trim_end_matches(".ts")),
                    };
                    (specifier, (*source).to_string())
                })
                .collect(),
            ..Default::default()
        };
        rpp_js::bundle(&request)
            .with_context(|| format!("the packed `{entry}` is not self-contained"))?;
    }
    Ok(())
}
