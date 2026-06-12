//! Cache key computation: global key, processor chains, and chain keys (spec §7).

use std::sync::Arc;

use crate::config::Config;
use crate::model::PluginFactory;
use crate::util::config_key;
use crate::util::glob::GlobSet;
use crate::util::hash::HashWriter;

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
                plugin_key: factory.cache_key(),
                processor: def.name.clone(),
                priority: def.priority,
                decl_order,
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
    }
    writer.finish()
}

/// Compute the global cache key (rpp version + build-relevant config + plugin keys).
pub(crate) fn global_key(config: &Config, factories: &[Arc<dyn PluginFactory>]) -> u64 {
    let mut writer = HashWriter::new();
    writer.write_str("rpp.global.v2");
    writer.write_str(env!("CARGO_PKG_VERSION"));
    writer.write_u64(config_key::config_digest(config));

    for factory in factories {
        writer.write_u64(factory.cache_key());
    }

    writer.finish()
}
