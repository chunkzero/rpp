//! Glob matching used to map files to processors and for generator queries.

use glob::{MatchOptions, Pattern};

/// A compiled glob matcher over a set of patterns.
///
/// Matching is performed against forward-slash relative paths. `**` matches
/// across path separators; `*` and `?` do not.
#[derive(Debug, Clone)]
pub(crate) struct GlobSet {
    patterns: Vec<Pattern>,
}

const OPTIONS: MatchOptions = MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};

impl GlobSet {
    /// Compile a list of glob patterns.
    ///
    /// Invalid patterns are returned as an error string identifying the pattern.
    pub(crate) fn new<I, S>(patterns: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut compiled = Vec::new();
        for p in patterns {
            let p = p.as_ref();
            let pat = Pattern::new(p).map_err(|e| format!("invalid glob `{p}`: {e}"))?;
            compiled.push(pat);
        }
        Ok(Self { patterns: compiled })
    }

    /// Returns true if `path` matches any pattern in the set.
    pub(crate) fn is_match(&self, path: &str) -> bool {
        self.patterns.iter().any(|p| p.matches_with(path, OPTIONS))
    }

    /// Returns true if the set contains no patterns.
    pub(crate) fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

/// Match a single glob pattern against a path (forward-slash relative).
pub(crate) fn matches(pattern: &str, path: &str) -> bool {
    match Pattern::new(pattern) {
        Ok(p) => p.matches_with(path, OPTIONS),
        Err(_) => false,
    }
}
