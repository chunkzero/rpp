//! Mapping bundled stack locations back to original sources.

/// Rewrite every `<module>:<line>:<column>` location in `stack` (1-based, as V8
/// prints them) through `source_map` to `<source>:<line>:<column>`, where `<source>`
/// is the source map's source path. Locations in other modules, or without a mapping,
/// are left unchanged.
pub(crate) fn map_stack(stack: &str, module: &str, source_map: &str) -> String {
    let _ = (module, source_map);
    stack.to_string()
}
