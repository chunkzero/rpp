//! Rolldown bundling into one ESM file.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use rolldown::plugin::{
    HookLoadArgs, HookLoadOutput, HookLoadReturn, HookResolveIdArgs, HookResolveIdOutput,
    HookResolveIdReturn, HookUsage, Plugin, PluginContext, PluginContextResolveOptions,
    SharedLoadPluginContext,
};
use rolldown::{
    Bundler, BundlerOptions, ChunkFilenamesOutputOption, CodeSplittingMode, InputItem, ModuleType,
    OutputFormat, Platform, ResolveOptions, SourceMapPathTransform, SourceMapType,
};
use rolldown_common::{Output, ResolvedExternal};
use rolldown_sourcemap::{JSONSourceMap, SourceMap};
use twox_hash::XxHash3_64;

use crate::error::{Error, Result};

// Not `\0`-prefixed: Rolldown emits no source map entries for such modules.
const VIRTUAL_PREFIX: &str = "rpp-virtual:";
const NODE_MODULES: &str = "node_modules";
const SOURCE_MAP_COMMENT: &str = "//# sourceMappingURL=";

/// What to bundle.
#[derive(Debug, Clone, Default)]
pub struct BundleRequest {
    /// Directory every bundled file must be inside (after resolving symlinks).
    pub root: PathBuf,
    /// The entry specifier: a key of `virtual_modules`, or a path relative to `root`.
    pub entry: String,
    /// Modules that exist only in memory, keyed by the exact import specifier
    /// (e.g. `#rpp`, `rpp:entry`). They take precedence over filesystem resolution,
    /// are parsed as TypeScript, and may import each other or files under `root`
    /// (relative imports from a virtual module resolve against `root`).
    pub virtual_modules: BTreeMap<String, String>,
    /// Directories outside `root` that may also be bundled, keyed by an exact import
    /// specifier (e.g. `#plugins/window`) that resolves to that package's entry file.
    /// Files inside a package directory may import each other relatively; source maps
    /// show them as `<specifier>/<path relative to the package directory>`.
    pub packages: BTreeMap<String, BundlePackage>,
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
        source_map: chunk.source_map,
        inputs: built.inputs,
        input_hashes: built.input_hashes,
    })
}

/// Behaviour that differs between [`bundle`] and [`crate::pack`].
#[derive(Debug, Default)]
pub(crate) struct Settings {
    /// Specifiers left as imports; one ending in `/` matches every specifier under it.
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
    pub source_map: String,
}

pub(crate) struct Built {
    pub chunks: Vec<Chunk>,
    pub inputs: Vec<PathBuf>,
    pub input_hashes: Vec<(PathBuf, u64)>,
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
    let mut packages = Vec::with_capacity(request.packages.len());
    for (specifier, package) in &request.packages {
        let dir = package.dir.canonicalize().map_err(|e| {
            Error::Bundle(format!(
                "cannot resolve package `{specifier}` at {}: {e}",
                package.dir.display()
            ))
        })?;
        packages.push(Package {
            specifier: specifier.clone(),
            dir,
            entry: package.entry.clone(),
        });
    }
    let packages = Arc::new(packages);
    let diagnostics = Arc::new(Mutex::new(Vec::new()));
    let loaded = Arc::new(Mutex::new(BTreeMap::new()));
    let plugin = VirtualModules {
        root: root.clone(),
        packages: Arc::clone(&packages),
        modules: request.virtual_modules.clone(),
        externals: settings.externals.clone(),
        aliases: settings.aliases.clone(),
        allow_node_modules: settings.allow_node_modules,
        diagnostics: Arc::clone(&diagnostics),
        loaded: Arc::clone(&loaded),
    };

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
    let allow_node_modules = settings.allow_node_modules;
    let options = BundlerOptions {
        input: Some(input),
        cwd: Some(root.clone()),
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
        resolve: settings
            .main_fields
            .clone()
            .map(|main_fields| ResolveOptions {
                main_fields: Some(main_fields),
                ..Default::default()
            }),
        sourcemap_path_transform: Some(SourceMapPathTransform::new(Arc::new({
            let root = root.clone();
            let packages = Arc::clone(&packages);
            move |sources, _| {
                let root = root.clone();
                let packages = Arc::clone(&packages);
                Box::pin(async move {
                    Ok(sources
                        .iter()
                        .map(|s| display_source(&root, &packages, allow_node_modules, s))
                        .collect())
                })
            }
        }))),
        ..Default::default()
    };

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
    let output = output.expect("output exists when there are no errors");

    let mut chunks = Vec::new();
    for asset in &output.assets {
        let Output::Chunk(chunk) = asset else {
            continue;
        };
        let source_map = chunk
            .map
            .as_ref()
            .ok_or_else(|| Error::Bundle("bundling produced no source map".to_string()))?
            .to_json_string();
        chunks.push(Chunk {
            file_name: chunk.filename.to_string(),
            code: chunk.code.clone(),
            source_map,
        });
    }

    let mut hashes = loaded.lock().expect("loaded lock").clone();
    hashes.retain(|path, _| path.is_absolute() && path.is_file());
    let mut manifests = BTreeSet::new();
    for path in hashes.keys() {
        let boundary = std::iter::once(&root)
            .chain(packages.iter().map(|p| &p.dir))
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
    let input_hashes: Vec<(PathBuf, u64)> = hashes.into_iter().collect();
    let inputs = input_hashes.iter().map(|(path, _)| path.clone()).collect();

    Ok(Built {
        chunks,
        inputs,
        input_hashes,
    })
}

/// Turns a source path relative to `root` (as Rolldown reports it) into the form
/// listed in the source map.
fn display_source(
    root: &Path,
    packages: &[Package],
    allow_node_modules: bool,
    source: &str,
) -> String {
    if let Some(at) = source.find(VIRTUAL_PREFIX) {
        return source[at + VIRTUAL_PREFIX.len()..].to_string();
    }
    let absolute = normalize(&root.join(source));
    if allow_node_modules {
        let components: Vec<_> = absolute.components().collect();
        if let Some(at) = components
            .iter()
            .rposition(|c| c.as_os_str() == NODE_MODULES)
        {
            let tail: PathBuf = components[at..].iter().collect();
            return tail.to_string_lossy().replace('\\', "/");
        }
    }
    for package in packages {
        if let Ok(relative) = absolute.strip_prefix(&package.dir) {
            let relative = relative.to_string_lossy().replace('\\', "/");
            return format!("{}/{relative}", package.specifier);
        }
    }
    source.replace('\\', "/")
}

/// Resolves `.` and `..` components without touching the filesystem.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// A [`BundlePackage`] with its directory canonicalized.
#[derive(Debug)]
struct Package {
    specifier: String,
    dir: PathBuf,
    entry: String,
}

/// Serves `virtual_modules` and package entries, and rejects imports that leave `root`
/// and the package directories.
#[derive(Debug)]
struct VirtualModules {
    root: PathBuf,
    packages: Arc<Vec<Package>>,
    modules: BTreeMap<String, String>,
    externals: Vec<String>,
    aliases: BTreeMap<String, String>,
    allow_node_modules: bool,
    diagnostics: Arc<Mutex<Vec<String>>>,
    loaded: Arc<Mutex<BTreeMap<PathBuf, u64>>>,
}

impl VirtualModules {
    fn reject(&self, specifier: &str, importer: &str, reason: &str) -> HookResolveIdOutput {
        let message = format!("cannot import `{specifier}` from {importer}: {reason}");
        self.diagnostics
            .lock()
            .expect("diagnostics lock")
            .push(message);
        HookResolveIdOutput {
            id: specifier.into(),
            external: Some(ResolvedExternal::Bool(true)),
            ..Default::default()
        }
    }

    fn describe_importer(&self, importer: Option<&str>) -> String {
        match importer {
            None => "the entry point".to_string(),
            Some(id) => match id.strip_prefix(VIRTUAL_PREFIX) {
                Some(specifier) => specifier.to_string(),
                None => relative_to(&self.root, Path::new(id)),
            },
        }
    }

    fn is_allowed(&self, id: &str) -> bool {
        Path::new(id)
            .canonicalize()
            .is_ok_and(|path| self.is_allowed_path(&path))
    }

    fn is_allowed_path(&self, path: &Path) -> bool {
        path.starts_with(&self.root)
            || self.packages.iter().any(|p| path.starts_with(&p.dir))
            || (self.allow_node_modules && path.components().any(|c| c.as_os_str() == NODE_MODULES))
    }

    fn is_external(&self, specifier: &str) -> bool {
        self.externals
            .iter()
            .any(|e| specifier == e || (e.ends_with('/') && specifier.starts_with(e.as_str())))
    }

    /// The module's code with the source map it points to, when that map is readable
    /// from an allowed directory. Sources in the map become absolute paths.
    fn attach_input_map(&self, path: &Path, code: String) -> HookLoadOutput {
        let plain = |code: String| HookLoadOutput {
            code: code.into(),
            ..Default::default()
        };
        let Some(url) = code
            .trim_end()
            .lines()
            .next_back()
            .and_then(|line| line.strip_prefix(SOURCE_MAP_COMMENT))
            .filter(|url| !url.starts_with("data:"))
        else {
            return plain(code);
        };
        let Some(map_path) = path
            .parent()
            .and_then(|dir| dir.join(url.trim()).canonicalize().ok())
            .filter(|map_path| self.is_allowed_path(map_path))
        else {
            return plain(code);
        };
        let Ok(bytes) = fs::read(&map_path) else {
            return plain(code);
        };
        let map = std::str::from_utf8(&bytes)
            .ok()
            .zip(map_path.parent())
            .and_then(|(json, dir)| absolute_sources(json, dir));
        let Some(map) = map else {
            return plain(code);
        };
        self.loaded
            .lock()
            .expect("loaded lock")
            .insert(map_path, XxHash3_64::oneshot(&bytes));
        HookLoadOutput {
            code: code.into(),
            map: Some(map),
            ..Default::default()
        }
    }
}

/// Parses a source map, making each source an absolute normalized path.
fn absolute_sources(json: &str, dir: &Path) -> Option<SourceMap> {
    let mut map: JSONSourceMap = serde_json::from_str(json).ok()?;
    let base = match map.source_root.take() {
        Some(source_root) => dir.join(source_root),
        None => dir.to_path_buf(),
    };
    for source in &mut map.sources {
        *source = normalize(&base.join(&*source))
            .to_string_lossy()
            .into_owned();
    }
    SourceMap::from_json(map).ok()
}

fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

impl Plugin for VirtualModules {
    fn name(&self) -> std::borrow::Cow<'static, str> {
        "rpp-js:virtual-modules".into()
    }

    fn register_hook_usage(&self) -> HookUsage {
        HookUsage::ResolveId | HookUsage::Load
    }

    async fn resolve_id(
        &self,
        ctx: &PluginContext,
        args: &HookResolveIdArgs<'_>,
    ) -> HookResolveIdReturn {
        let specifier = args.specifier;
        if self.modules.contains_key(specifier) {
            return Ok(Some(HookResolveIdOutput::from_id(format!(
                "{VIRTUAL_PREFIX}{specifier}"
            ))));
        }
        if self.is_external(specifier) {
            return Ok(Some(HookResolveIdOutput {
                id: specifier.into(),
                external: Some(ResolvedExternal::Bool(true)),
                ..Default::default()
            }));
        }
        let importer = self.describe_importer(args.importer);
        if let Some(entry) = self.aliases.get(specifier) {
            return Ok(Some(match self.root.join(entry).canonicalize() {
                Ok(path) if path.starts_with(&self.root) => {
                    HookResolveIdOutput::from_id(path.to_string_lossy().into_owned())
                }
                _ => self.reject(
                    specifier,
                    &importer,
                    "the target is missing or outside the root",
                ),
            }));
        }
        if let Some(package) = self.packages.iter().find(|p| p.specifier == specifier) {
            return Ok(Some(
                match package.dir.join(&package.entry).canonicalize() {
                    Ok(path) if path.starts_with(&package.dir) => {
                        HookResolveIdOutput::from_id(path.to_string_lossy().into_owned())
                    }
                    _ => self.reject(
                        specifier,
                        &importer,
                        "the package entry is missing or outside the package directory",
                    ),
                },
            ));
        }
        if specifier.starts_with("node:") {
            return Ok(Some(self.reject(
                specifier,
                &importer,
                "Node built-ins are not available",
            )));
        }

        let from = match args.importer {
            Some(id) if id.starts_with(VIRTUAL_PREFIX) => {
                Some(self.root.join("<virtual>").to_string_lossy().into_owned())
            }
            other => other.map(str::to_string),
        };
        let options = PluginContextResolveOptions {
            import_kind: args.kind,
            is_entry: args.is_entry,
            ..Default::default()
        };
        let resolved = match ctx
            .resolve(specifier, from.as_deref(), Some(options))
            .await?
        {
            Ok(resolved) => resolved,
            Err(error) => {
                return Ok(Some(self.reject(specifier, &importer, &format!("{error}"))));
            }
        };
        if !matches!(resolved.external, ResolvedExternal::Bool(false)) {
            return Ok(Some(self.reject(
                specifier,
                &importer,
                "external imports are not supported",
            )));
        }
        if !self.is_allowed(resolved.id.as_str()) {
            return Ok(Some(self.reject(
                specifier,
                &importer,
                "the file is outside the root and package directories",
            )));
        }
        Ok(Some(HookResolveIdOutput::from_resolved_id(resolved)))
    }

    async fn load(&self, _ctx: SharedLoadPluginContext, args: &HookLoadArgs<'_>) -> HookLoadReturn {
        let Some(specifier) = args.id.strip_prefix(VIRTUAL_PREFIX) else {
            // Returning the bytes read here keeps the recorded hash in step with the build.
            let Ok(bytes) = fs::read(args.id) else {
                return Ok(None);
            };
            let Ok(code) = String::from_utf8(bytes) else {
                return Ok(None);
            };
            let path = PathBuf::from(args.id);
            self.loaded
                .lock()
                .expect("loaded lock")
                .insert(path.clone(), XxHash3_64::oneshot(code.as_bytes()));
            return Ok(Some(self.attach_input_map(&path, code)));
        };
        Ok(self.modules.get(specifier).map(|code| HookLoadOutput {
            code: code.as_str().into(),
            module_type: Some(ModuleType::Ts),
            ..Default::default()
        }))
    }
}
