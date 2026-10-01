//! Rolldown bundling into one ESM file.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::Result;

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
}

/// A bundled program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// One ES module, with the entry's exports.
    pub code: String,
    /// Source map v3 JSON for `code`. Real files appear as paths relative to `root`
    /// with `/` separators; virtual modules appear under their specifier.
    pub source_map: String,
    /// Every real file that was bundled, absolute, sorted and deduplicated.
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
    let _ = request;
    todo!()
}
