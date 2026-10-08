//! Rolldown bundling into one ESM file.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rolldown::plugin::Plugin;
use rolldown::{
    Bundler, BundlerOptions, ChunkFilenamesOutputOption, CodeSplittingMode, InputItem,
    OutputFormat, Platform, ResolveOptions, SourceMapPathTransform, SourceMapType, TsConfig,
};
use rolldown_common::{BundlerTransformOptions, Either, JsxOptions, Output};
use twox_hash::XxHash3_64;

use crate::error::{Error, Result};

mod paths;
mod plugin;

use paths::display_source;
use plugin::VirtualModules;

// Not `\0`-prefixed: Rolldown emits no source map entries for such modules.
const VIRTUAL_PREFIX: &str = "rpp-virtual:";
const NODE_MODULES: &str = "node_modules";

/// What to bundle.
#[derive(Debug, Clone, Default)]
pub struct BundleRequest {
    /// Directory every bundled file must be inside (after resolving symlinks).
    pub root: PathBuf,
    /// The entry specifier: a key of `virtual_modules`, or a path relative to `root`.
    pub entry: String,
    /// Modules that exist only in memory, keyed by the exact import specifier
    /// (e.g. `rpp`, `rpp:internal/entry`). They take precedence over filesystem resolution,
    /// are parsed as TypeScript, and may import each other or files under `root`
    /// (relative imports from a virtual module resolve against `root`).
    pub virtual_modules: BTreeMap<String, String>,
    /// Directories outside `root` that may also be bundled, keyed by an exact import
    /// specifier (e.g. `plugin:window`) that resolves to that package's entry file.
    /// Files inside a package directory may import each other relatively; source maps
    /// show them as `<specifier>/<path relative to the package directory>`, unless the
    /// directory is `root`.
    pub packages: BTreeMap<String, BundlePackage>,
    /// Compile JSX with the automatic runtime imported from `<source>/jsx-runtime`. JSX
    /// pragma comments, which would replace it, are rejected.
    pub jsx_import_source: Option<String>,
}

/// A directory bundled in addition to `root`; see [`BundleRequest::packages`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BundlePackage {
    /// The package directory.
    pub dir: PathBuf,
    /// The file the specifier resolves to, relative to `dir`.
    pub entry: String,
}

/// A bundled program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// One ES module, with the entry's exports.
    pub code: String,
    /// Source map v3 JSON for `code`. Real files appear as paths relative to `root`
    /// with `/` separators; virtual modules appear under their specifier.
    pub source_map: String,
    /// Every real file the bundler loaded, including modules tree-shaken out of `code`;
    /// absolute, sorted and deduplicated.
    pub inputs: Vec<PathBuf>,
    /// The xxh3-64 hash of the bytes the bundler consumed for each of `inputs`, in the
    /// same order. `package.json` files that import resolution consulted (the nearest
    /// one above each loaded file, up to its root or package directory) are inputs too.
    pub input_hashes: Vec<(PathBuf, u64)>,
}

/// Bundle `request.entry` and its static and dynamic imports into one ESM chunk.
///
/// TypeScript is stripped (type-only imports are erased), `package.json` `imports`
/// (`#name`) and `node_modules` resolve as in Node's ESM resolver, and output is
/// deterministic for the same inputs. Nothing is written to disk.
///
/// # Errors
///
/// [`crate::Error::Bundle`] for syntax errors, unresolved imports, `node:` or other
/// built-in imports, files outside `root`, or output with more than one chunk.
pub fn bundle(request: &BundleRequest) -> Result<Bundle> {
    let entry = InputItem {
        name: Some("bundle".to_string()),
        import: request.entry.clone(),
    };
    let built = build(request, &Settings::default(), vec![entry])?;
    let Ok([chunk]) = <[Chunk; 1]>::try_from(built.chunks) else {
        return Err(Error::Bundle(
            "bundling produced more than one chunk".to_string(),
        ));
    };
    Ok(Bundle {
        code: chunk.code,
        // A module with nothing to run (e.g. only types) produces no map.
        source_map: chunk.source_map.unwrap_or_else(|| {
            r#"{"version":3,"sources":[],"names":[],"mappings":""}"#.to_string()
        }),
        inputs: built.inputs,
        input_hashes: built.input_hashes,
    })
}

/// Behaviour that differs between [`bundle`] and [`crate::pack`].
#[derive(Debug, Default)]
pub(crate) struct Settings {
    /// Specifiers left as imports; one ending in `/` or `:` matches every specifier under it.
    pub externals: Vec<String>,
    /// Specifiers that resolve to a file relative to `root`.
    pub aliases: BTreeMap<String, String>,
    /// Allow files under any `node_modules` directory, even outside `root`.
    pub allow_node_modules: bool,
    /// Package fields tried when a package has no `exports`; `None` keeps the default.
    pub main_fields: Option<Vec<String>>,
    /// Emit shared chunks instead of requiring a single chunk.
    pub split: bool,
}

pub(crate) struct Chunk {
    pub file_name: String,
    pub code: String,
    /// `None` for a facade chunk that only re-exports other chunks.
    pub source_map: Option<String>,
}

pub(crate) struct Built {
    pub chunks: Vec<Chunk>,
    pub inputs: Vec<PathBuf>,
    pub input_hashes: Vec<(PathBuf, u64)>,
}

/// A [`BundlePackage`] with its directory canonicalized.
#[derive(Debug)]
struct Package {
    specifier: String,
    dir: PathBuf,
    entry: String,
}

/// Runs one Rolldown build; entries in `inputs` are keys of `virtual_modules` or paths
/// relative to `root`.
pub(crate) fn build(
    request: &BundleRequest,
    settings: &Settings,
    inputs: Vec<InputItem>,
) -> Result<Built> {
    let root = request.root.canonicalize().map_err(|e| {
        Error::Bundle(format!(
            "cannot resolve root {}: {e}",
            request.root.display()
        ))
    })?;
    let packages = Arc::new(canonical_packages(request)?);
    let diagnostics = Arc::new(Mutex::new(Vec::new()));
    let loaded = Arc::new(Mutex::new(BTreeMap::new()));
    let plugin = VirtualModules {
        root: root.clone(),
        packages: Arc::clone(&packages),
        modules: request.virtual_modules.clone(),
        externals: settings.externals.clone(),
        aliases: settings.aliases.clone(),
        allow_node_modules: settings.allow_node_modules,
        jsx_import_source: request.jsx_import_source.clone(),
        diagnostics: Arc::clone(&diagnostics),
        loaded: Arc::clone(&loaded),
    };
    let options = bundler_options(request, settings, &root, &packages, inputs);

    let assets = run_rolldown(options, plugin, &diagnostics)?;
    let chunks = collect_chunks(&assets)?;
    let hashes = loaded.lock().expect("loaded lock").clone();
    let input_hashes = input_hashes(&root, &packages, hashes);
    let inputs = input_hashes.iter().map(|(path, _)| path.clone()).collect();

    Ok(Built {
        chunks,
        inputs,
        input_hashes,
    })
}

fn canonical_packages(request: &BundleRequest) -> Result<Vec<Package>> {
    request
        .packages
        .iter()
        .map(|(specifier, package)| {
            let dir = package.dir.canonicalize().map_err(|e| {
                Error::Bundle(format!(
                    "cannot resolve package `{specifier}` at {}: {e}",
                    package.dir.display()
                ))
            })?;
            Ok(Package {
                specifier: specifier.clone(),
                dir,
                entry: package.entry.clone(),
            })
        })
        .collect()
}

fn bundler_options(
    request: &BundleRequest,
    settings: &Settings,
    root: &Path,
    packages: &Arc<Vec<Package>>,
    inputs: Vec<InputItem>,
) -> BundlerOptions {
    let input = inputs
        .into_iter()
        .map(|item| InputItem {
            import: if request.virtual_modules.contains_key(&item.import) {
                item.import
            } else {
                root.join(&item.import).to_string_lossy().into_owned()
            },
            ..item
        })
        .collect();
    BundlerOptions {
        input: Some(input),
        cwd: Some(root.to_path_buf()),
        dir: Some(root.to_string_lossy().into_owned()),
        format: Some(OutputFormat::Esm),
        platform: Some(Platform::Neutral),
        sourcemap: Some(SourceMapType::Hidden),
        code_splitting: Some(CodeSplittingMode::Bool(settings.split)),
        entry_filenames: settings
            .split
            .then(|| ChunkFilenamesOutputOption::String("[name].js".to_string())),
        chunk_filenames: settings
            .split
            .then(|| ChunkFilenamesOutputOption::String("chunk-[hash].js".to_string())),
        tsconfig: Some(TsConfig::Auto(false)),
        transform: request
            .jsx_import_source
            .as_ref()
            .map(|source| BundlerTransformOptions {
                jsx: Some(Either::Right(JsxOptions {
                    runtime: Some("automatic".to_string()),
                    import_source: Some(source.clone()),
                    // Components may have side effects, so unused elements still run.
                    pure: Some(false),
                    ..Default::default()
                })),
                ..Default::default()
            }),
        resolve: settings
            .main_fields
            .clone()
            .map(|main_fields| ResolveOptions {
                main_fields: Some(main_fields),
                ..Default::default()
            }),
        sourcemap_path_transform: Some(source_path_transform(
            root,
            packages,
            settings.allow_node_modules,
        )),
        ..Default::default()
    }
}

fn source_path_transform(
    root: &Path,
    packages: &Arc<Vec<Package>>,
    allow_node_modules: bool,
) -> SourceMapPathTransform {
    let root = root.to_path_buf();
    let packages = Arc::clone(packages);
    SourceMapPathTransform::new(Arc::new(move |sources, map_path| {
        let root = root.clone();
        let packages = Arc::clone(&packages);
        // Rolldown passes sources relative to the map's directory.
        let map_dir = Path::new(map_path)
            .parent()
            .map_or_else(|| root.clone(), |dir| root.join(dir));
        Box::pin(async move {
            Ok(sources
                .iter()
                .map(|s| display_source(&root, &map_dir, &packages, allow_node_modules, s))
                .collect())
        })
    }))
}

/// Generates the bundle, failing with every diagnostic the plugin and Rolldown reported.
fn run_rolldown(
    options: BundlerOptions,
    plugin: VirtualModules,
    diagnostics: &Mutex<Vec<String>>,
) -> Result<Vec<Output>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(Error::Io)?;
    let generated = runtime.block_on(async {
        let mut bundler = Bundler::with_plugins(options, vec![Plugin::new_shared(plugin)])?;
        let output = bundler.generate().await;
        let closed = bundler.close().await;
        output.and_then(|output| closed.map(|()| output))
    });

    let mut messages = std::mem::take(&mut *diagnostics.lock().expect("diagnostics lock"));
    let output = match generated {
        Ok(output) => Some(output),
        Err(errors) => {
            messages.extend(
                errors
                    .iter()
                    .map(|e| e.to_diagnostic().convert_to_string(false)),
            );
            None
        }
    };
    if !messages.is_empty() {
        messages.sort();
        messages.dedup();
        return Err(Error::Bundle(messages.join("\n")));
    }
    Ok(output
        .expect("output exists when there are no errors")
        .assets)
}

fn collect_chunks(assets: &[Output]) -> Result<Vec<Chunk>> {
    let mut chunks = Vec::new();
    for asset in assets {
        let Output::Chunk(chunk) = asset else {
            continue;
        };
        let source_map = chunk.map.as_ref().map(|map| map.to_json_string());
        chunks.push(Chunk {
            file_name: chunk.filename.to_string(),
            code: chunk.code.clone(),
            source_map,
        });
    }
    Ok(chunks)
}

/// The hash of every real file the bundler loaded, plus the `package.json` files that
/// import resolution may have consulted.
fn input_hashes(
    root: &Path,
    packages: &[Package],
    mut hashes: BTreeMap<PathBuf, u64>,
) -> Vec<(PathBuf, u64)> {
    hashes.retain(|path, _| path.is_absolute() && path.is_file());
    let mut manifests = BTreeSet::new();
    for path in hashes.keys() {
        let boundary = std::iter::once(root)
            .chain(packages.iter().map(|p| p.dir.as_path()))
            .find(|boundary| path.starts_with(boundary));
        let Some(boundary) = boundary else { continue };
        manifests.extend(
            path.ancestors()
                .skip(1)
                .take_while(|dir| dir.starts_with(boundary))
                .map(|dir| dir.join("package.json")),
        );
    }
    manifests.insert(root.join("package.json"));
    for manifest in manifests {
        if let Ok(bytes) = fs::read(&manifest) {
            hashes.insert(manifest, XxHash3_64::oneshot(&bytes));
        }
    }
    hashes.into_iter().collect()
}
