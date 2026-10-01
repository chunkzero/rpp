//! Rolldown bundling into one ESM file.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use rolldown::plugin::{
    HookLoadArgs, HookLoadOutput, HookLoadReturn, HookResolveIdArgs, HookResolveIdOutput,
    HookResolveIdReturn, HookUsage, Plugin, PluginContext, PluginContextResolveOptions,
    SharedLoadPluginContext,
};
use rolldown::{
    Bundler, BundlerOptions, CodeSplittingMode, InputItem, ModuleType, OutputFormat, Platform,
    SourceMapPathTransform, SourceMapType,
};
use rolldown_common::{Output, ResolvedExternal};

use crate::error::{Error, Result};

// Not `\0`-prefixed: Rolldown emits no source map entries for such modules.
const VIRTUAL_PREFIX: &str = "rpp-virtual:";

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
    let loaded = Arc::new(Mutex::new(BTreeSet::new()));
    let plugin = VirtualModules {
        root: root.clone(),
        packages: Arc::clone(&packages),
        modules: request.virtual_modules.clone(),
        diagnostics: Arc::clone(&diagnostics),
        loaded: Arc::clone(&loaded),
    };

    let entry = if request.virtual_modules.contains_key(&request.entry) {
        request.entry.clone()
    } else {
        root.join(&request.entry).to_string_lossy().into_owned()
    };
    let options = BundlerOptions {
        input: Some(vec![InputItem {
            name: Some("bundle".to_string()),
            import: entry,
        }]),
        cwd: Some(root.clone()),
        dir: Some(root.to_string_lossy().into_owned()),
        format: Some(OutputFormat::Esm),
        platform: Some(Platform::Neutral),
        sourcemap: Some(SourceMapType::Hidden),
        code_splitting: Some(CodeSplittingMode::Bool(false)),
        sourcemap_path_transform: Some(SourceMapPathTransform::new(Arc::new({
            let root = root.clone();
            move |sources, _| {
                let root = root.clone();
                let packages = Arc::clone(&packages);
                Box::pin(async move {
                    Ok(sources
                        .iter()
                        .map(|s| display_source(&root, &packages, s))
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

    let mut chunks = output.assets.iter().filter_map(|asset| match asset {
        Output::Chunk(chunk) => Some(chunk),
        Output::Asset(_) => None,
    });
    let (Some(chunk), None) = (chunks.next(), chunks.next()) else {
        return Err(Error::Bundle(
            "bundling produced more than one chunk".to_string(),
        ));
    };
    let source_map = chunk
        .map
        .as_ref()
        .ok_or_else(|| Error::Bundle("bundling produced no source map".to_string()))?
        .to_json_string();

    let inputs: Vec<PathBuf> = loaded
        .lock()
        .expect("loaded lock")
        .iter()
        .filter(|path| path.is_absolute() && path.is_file())
        .cloned()
        .collect();

    Ok(Bundle {
        code: chunk.code.clone(),
        source_map,
        inputs,
    })
}

/// Turns a source path relative to `root` (as Rolldown reports it) into the form
/// listed in the source map.
fn display_source(root: &Path, packages: &[Package], source: &str) -> String {
    if let Some(at) = source.find(VIRTUAL_PREFIX) {
        return source[at + VIRTUAL_PREFIX.len()..].to_string();
    }
    let absolute = normalize(&root.join(source));
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
    diagnostics: Arc<Mutex<Vec<String>>>,
    loaded: Arc<Mutex<BTreeSet<PathBuf>>>,
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
        Path::new(id).canonicalize().is_ok_and(|path| {
            path.starts_with(&self.root) || self.packages.iter().any(|p| path.starts_with(&p.dir))
        })
    }
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
        let importer = self.describe_importer(args.importer);
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
            self.loaded
                .lock()
                .expect("loaded lock")
                .insert(PathBuf::from(args.id));
            return Ok(None);
        };
        Ok(self.modules.get(specifier).map(|code| HookLoadOutput {
            code: code.as_str().into(),
            module_type: Some(ModuleType::Ts),
            ..Default::default()
        }))
    }
}
