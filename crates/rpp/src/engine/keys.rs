//! Cache key computation: global key, processor chains, and chain keys (spec §7).

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;

use crate::config::{BuildConfig, Config, LimitsConfig, PackConfig};
use crate::model::PluginFactory;
use crate::util::glob::GlobSet;
use crate::util::hash::{xxh3, HashWriter};

/// One step in a file's processor chain.
#[derive(Debug, Clone)]
pub(crate) struct ChainStep {
    /// Index of the owning plugin in the engine's factory list.
    pub(crate) plugin_index: usize,
    /// Owning plugin id, retained for diagnostics.
    pub(crate) plugin_id: String,
    /// The plugin's cache key.
    pub(crate) plugin_key: u64,
    /// The processor name.
    pub(crate) processor: String,
    pub(crate) cacheable: bool,
}

/// A precompiled processor: its owning plugin metadata plus a glob matcher.
pub(crate) struct CompiledProcessor {
    pub(crate) plugin_index: usize,
    pub(crate) plugin_id: String,
    pub(crate) plugin_key: u64,
    pub(crate) processor: String,
    pub(crate) priority: i32,
    /// Stable declaration order within the plugin.
    pub(crate) decl_order: usize,
    pub(crate) cacheable: bool,
    pub(crate) globs: GlobSet,
}

/// Compile all processors across the factory list into matchable units.
///
/// Returns an error string if any plugin declares an invalid glob.
pub(crate) fn compile_processors(
    factories: &[Arc<dyn PluginFactory>],
) -> Result<Vec<CompiledProcessor>, String> {
    let mut out = Vec::new();
    for (plugin_index, factory) in factories.iter().enumerate() {
        for (decl_order, def) in factory.processors().iter().enumerate() {
            let globs = GlobSet::new(&def.patterns)
                .map_err(|e| format!("plugin `{}`: {e}", factory.id()))?;
            out.push(CompiledProcessor {
                plugin_index,
                plugin_id: factory.id().to_string(),
                plugin_key: factory.processor_key(),
                processor: def.name.clone(),
                priority: def.priority,
                decl_order,
                cacheable: factory.cacheable_processors(),
                globs,
            });
        }
    }
    Ok(out)
}

/// Build the ordered processor chain for a given file path.
///
/// Ordering: priority ascending; ties broken by plugin order in the config,
/// then by declaration order within a plugin.
pub(crate) fn chain_for(processors: &[CompiledProcessor], path: &str) -> Vec<ChainStep> {
    let mut matched: Vec<&CompiledProcessor> = processors
        .iter()
        .filter(|p| !p.globs.is_empty() && p.globs.is_match(path))
        .collect();

    matched.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then(a.plugin_index.cmp(&b.plugin_index))
            .then(a.decl_order.cmp(&b.decl_order))
    });

    matched
        .into_iter()
        .map(|p| ChainStep {
            plugin_index: p.plugin_index,
            plugin_id: p.plugin_id.clone(),
            plugin_key: p.plugin_key,
            processor: p.processor.clone(),
            cacheable: p.cacheable,
        })
        .collect()
}

/// Compute the chain key: xxh3 over the ordered `(plugin_key, processor)` chain.
pub(crate) fn chain_key(chain: &[ChainStep]) -> u64 {
    let mut writer = HashWriter::new();
    writer.write_str("chain.v1");
    for step in chain {
        writer.write_u64(step.plugin_key);
        writer.write_str(&step.processor);
        writer.write_bool(step.cacheable);
    }
    writer.finish()
}

/// Compute the global cache key (rpp version + build-relevant config).
///
/// Plugin keys are not part of it: processor results are keyed per chain and generator
/// results per plugin, so a plugin change invalidates only what it affects.
pub(crate) fn global_key(config: &Config) -> u64 {
    let mut writer = HashWriter::new();
    writer.write_str("rpp.global.v3");
    writer.write_str(env!("CARGO_PKG_VERSION"));
    writer.write_u64(config_digest(config));

    writer.finish()
}

/// Build-relevant `[build]` fields for the global cache key. Outputs are
/// content-addressed, so the output path and squash settings are excluded.
#[derive(Serialize)]
struct BuildKeySection<'a> {
    source: String,
    limits: &'a LimitsConfig,
}

/// `[pack]` plus build-relevant fields for the global cache key.
#[derive(Serialize)]
struct GlobalKeyConfig<'a> {
    pack: &'a PackConfig,
    build: BuildKeySection<'a>,
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn build_key_section(build: &BuildConfig) -> BuildKeySection<'_> {
    BuildKeySection {
        source: path_key(&build.source),
        limits: &build.limits,
    }
}

/// Hash the build-relevant config sections using canonical JSON serialization.
fn config_digest(config: &Config) -> u64 {
    let payload = GlobalKeyConfig {
        pack: &config.pack,
        build: build_key_section(&config.build),
    };
    let bytes = serde_json::to_vec(&payload).expect("global key config serializes");
    xxh3(&bytes)
}
