//! On-disk cache of bundler output, so unchanged plugins skip the bundler on load.

use std::path::{Path, PathBuf};

use rpp_js::{Bundle, BundleRequest};
use serde::{Deserialize, Serialize};

use crate::util::hash::{u64_hex, xxh3, HashWriter};

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct BundleCacheEntry {
    version: u32,
    request_key: u64,
    /// Absolute input paths with the hash of their content when bundled.
    inputs: Vec<(String, u64)>,
    code: String,
    source_map: String,
}

/// Return the bundle cached for plugin `id` when `request` and every file the previous bundle
/// read are unchanged; otherwise run `build` and store its result. With no `dir`, always build.
/// A missing or corrupt entry is a miss, and a failed write is ignored.
pub(crate) fn cached_bundle(
    dir: Option<&Path>,
    id: &str,
    request: &BundleRequest,
    build: impl FnOnce() -> Result<Bundle, String>,
) -> Result<Bundle, String> {
    let Some(dir) = dir else {
        return build();
    };
    let bundles = dir.join("bundles");
    let path = bundles.join(format!("{}.bin", u64_hex(xxh3(id.as_bytes()))));
    let request_key = request_key(request);

    if real_dirs(dir, &bundles) {
        if let Some(bundle) = load(&path, request_key) {
            return Ok(bundle);
        }
    }
    let bundle = build()?;
    store(&path, request_key, &bundle);
    Ok(bundle)
}

/// Whether `.rpp` (the parent of `dir`), `dir` and `bundles` are all real directories, so
/// a symlink cannot redirect cache reads or writes.
fn real_dirs(dir: &Path, bundles: &Path) -> bool {
    [dir.parent(), Some(dir), Some(bundles)]
        .into_iter()
        .flatten()
        .filter(|path| !path.as_os_str().is_empty())
        .all(|path| {
            std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
        })
}

fn load(path: &Path, request_key: u64) -> Option<Bundle> {
    let bytes = std::fs::read(path).ok()?;
    let (entry, _): (BundleCacheEntry, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    if entry.version != VERSION || entry.request_key != request_key {
        return None;
    }
    for (input, hash) in &entry.inputs {
        if xxh3(&std::fs::read(input).ok()?) != *hash {
            return None;
        }
    }
    Some(Bundle {
        code: entry.code,
        source_map: entry.source_map,
        inputs: entry
            .inputs
            .iter()
            .map(|(input, _)| PathBuf::from(input))
            .collect(),
        input_hashes: entry
            .inputs
            .into_iter()
            .map(|(input, hash)| (PathBuf::from(input), hash))
            .collect(),
    })
}

fn store(path: &Path, request_key: u64, bundle: &Bundle) {
    let mut inputs = Vec::with_capacity(bundle.input_hashes.len());
    for (input, hash) in &bundle.input_hashes {
        let Some(name) = input.to_str() else {
            return;
        };
        inputs.push((name.to_string(), *hash));
    }
    let entry = BundleCacheEntry {
        version: VERSION,
        request_key,
        inputs,
        code: bundle.code.clone(),
        source_map: bundle.source_map.clone(),
    };
    let Ok(bytes) = bincode::serde::encode_to_vec(&entry, bincode::config::standard()) else {
        return;
    };
    let Some(bundles) = path.parent() else {
        return;
    };
    let Some(dir) = bundles.parent() else {
        return;
    };
    if std::fs::create_dir_all(bundles).is_err() || !real_dirs(dir, bundles) {
        return;
    }
    let _ = crate::util::atomic::write(path, &bytes);
}

fn request_key(request: &BundleRequest) -> u64 {
    let mut writer = HashWriter::new();
    writer.write_str("rpp.js.bundle.v1");
    writer.write_str(env!("CARGO_PKG_VERSION"));
    writer.write_str(&request.root.to_string_lossy());
    writer.write_str(&request.entry);
    for (specifier, text) in &request.virtual_modules {
        writer.write_str(specifier);
        writer.write_str(text);
    }
    writer.write_str("packages");
    for (specifier, package) in &request.packages {
        writer.write_str(specifier);
        writer.write_str(&package.dir.to_string_lossy());
        writer.write_str(&package.entry);
    }
    writer.finish()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    struct Fixture {
        _tmp: tempfile::TempDir,
        cache: PathBuf,
        input: PathBuf,
        request: BundleRequest,
        builds: Cell<usize>,
    }

    impl Fixture {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let input = tmp.path().join("main.ts");
            std::fs::write(&input, "one").unwrap();
            Self {
                cache: tmp.path().join("cache"),
                request: BundleRequest {
                    root: tmp.path().to_path_buf(),
                    entry: "main.ts".into(),
                    ..Default::default()
                },
                input,
                _tmp: tmp,
                builds: Cell::new(0),
            }
        }

        fn get(&self, dir: Option<&Path>, request: &BundleRequest) -> Bundle {
            cached_bundle(dir, "plugin", request, || {
                self.builds.set(self.builds.get() + 1);
                Ok(Bundle {
                    code: std::fs::read_to_string(&self.input).unwrap(),
                    source_map: String::new(),
                    inputs: vec![self.input.clone()],
                    input_hashes: vec![(
                        self.input.clone(),
                        xxh3(std::fs::read(&self.input).unwrap().as_slice()),
                    )],
                })
            })
            .unwrap()
        }

        fn entry_path(&self) -> PathBuf {
            std::fs::read_dir(self.cache.join("bundles"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path()
        }
    }

    #[test]
    fn reuses_entry_when_inputs_unchanged() {
        let f = Fixture::new();
        f.get(Some(&f.cache), &f.request);
        let second = f.get(Some(&f.cache), &f.request);
        assert_eq!(f.builds.get(), 1);
        assert_eq!(second.code, "one");
        assert_eq!(second.inputs, vec![f.input.clone()]);
    }

    #[test]
    fn input_edit_rebuilds() {
        let f = Fixture::new();
        f.get(Some(&f.cache), &f.request);
        std::fs::write(&f.input, "two").unwrap();
        assert_eq!(f.get(Some(&f.cache), &f.request).code, "two");
        assert_eq!(f.builds.get(), 2);
    }

    #[test]
    fn request_change_rebuilds() {
        let f = Fixture::new();
        f.get(Some(&f.cache), &f.request);
        let mut changed = f.request.clone();
        changed
            .virtual_modules
            .insert("rpp:discovered".into(), "export default {};".into());
        f.get(Some(&f.cache), &changed);
        assert_eq!(f.builds.get(), 2);
    }

    #[test]
    fn corrupt_entry_is_miss() {
        let f = Fixture::new();
        f.get(Some(&f.cache), &f.request);
        std::fs::write(f.entry_path(), b"garbage").unwrap();
        f.get(Some(&f.cache), &f.request);
        assert_eq!(f.builds.get(), 2);
        f.get(Some(&f.cache), &f.request);
        assert_eq!(f.builds.get(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_bundles_dir_is_skipped() {
        let f = Fixture::new();
        let target = f._tmp.path().join("elsewhere");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&f.cache).unwrap();
        std::os::unix::fs::symlink(&target, f.cache.join("bundles")).unwrap();
        f.get(Some(&f.cache), &f.request);
        f.get(Some(&f.cache), &f.request);
        assert_eq!(f.builds.get(), 2);
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
    }

    #[test]
    fn none_dir_always_builds() {
        let f = Fixture::new();
        f.get(None, &f.request);
        f.get(None, &f.request);
        assert_eq!(f.builds.get(), 2);
        assert!(!f.cache.exists());
    }
}
