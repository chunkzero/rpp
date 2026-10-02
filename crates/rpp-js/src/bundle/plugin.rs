use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rolldown::plugin::{
    HookLoadArgs, HookLoadOutput, HookLoadReturn, HookResolveIdArgs, HookResolveIdOutput,
    HookResolveIdReturn, HookUsage, Plugin, PluginContext, PluginContextResolveOptions,
    SharedLoadPluginContext,
};
use rolldown::ModuleType;
use rolldown_common::ResolvedExternal;
use twox_hash::XxHash3_64;

use super::paths::{absolute_sources, relative_to};
use super::{Package, NODE_MODULES, VIRTUAL_PREFIX};

const SOURCE_MAP_COMMENT: &str = "//# sourceMappingURL=";

/// Serves `virtual_modules` and package entries, and rejects imports that leave `root`
/// and the package directories.
#[derive(Debug)]
pub(super) struct VirtualModules {
    pub(super) root: PathBuf,
    pub(super) packages: Arc<Vec<Package>>,
    pub(super) modules: BTreeMap<String, String>,
    pub(super) externals: Vec<String>,
    pub(super) aliases: BTreeMap<String, String>,
    pub(super) allow_node_modules: bool,
    pub(super) diagnostics: Arc<Mutex<Vec<String>>>,
    pub(super) loaded: Arc<Mutex<BTreeMap<PathBuf, u64>>>,
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

    /// Resolves `specifier` to `base/entry`, which must exist and stay inside `base`.
    fn resolve_inside(
        &self,
        base: &Path,
        entry: &str,
        specifier: &str,
        importer: &str,
        reason: &str,
    ) -> HookResolveIdOutput {
        match base.join(entry).canonicalize() {
            Ok(path) if path.starts_with(base) => {
                HookResolveIdOutput::from_id(path.to_string_lossy().into_owned())
            }
            _ => self.reject(specifier, importer, reason),
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

    /// Specifiers the plugin answers itself, without Rolldown's resolver.
    fn resolve_known(&self, specifier: &str, importer: &str) -> Option<HookResolveIdOutput> {
        if self.modules.contains_key(specifier) {
            return Some(HookResolveIdOutput::from_id(format!(
                "{VIRTUAL_PREFIX}{specifier}"
            )));
        }
        if self.is_external(specifier) {
            return Some(HookResolveIdOutput {
                id: specifier.into(),
                external: Some(ResolvedExternal::Bool(true)),
                ..Default::default()
            });
        }
        if let Some(entry) = self.aliases.get(specifier) {
            return Some(self.resolve_inside(
                &self.root,
                entry,
                specifier,
                importer,
                "the target is missing or outside the root",
            ));
        }
        if let Some(package) = self.packages.iter().find(|p| p.specifier == specifier) {
            return Some(self.resolve_inside(
                &package.dir,
                &package.entry,
                specifier,
                importer,
                "the package entry is missing or outside the package directory",
            ));
        }
        specifier
            .starts_with("node:")
            .then(|| self.reject(specifier, importer, "Node built-ins are not available"))
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
        let importer = self.describe_importer(args.importer);
        if let Some(output) = self.resolve_known(specifier, &importer) {
            return Ok(Some(output));
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
