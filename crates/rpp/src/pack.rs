use std::{collections::BTreeSet, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug)]
pub struct PackMeta {
    pub pack: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlays: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<Value>,
}

#[derive(Debug)]
pub struct PackDiff {
    pub invalidated: BTreeSet<PathBuf>,
    pub deletions: BTreeSet<PathBuf>,
    pub creations: BTreeSet<PathBuf>,
}

#[derive(Debug)]
pub struct Pack {
    pub meta: PackMeta,
    pub root: PathBuf,
    sources: BTreeSet<PathBuf>,
    invalidated_sources: BTreeSet<PathBuf>,
}

impl Pack {
    pub fn load_from_dir(root: PathBuf) -> crate::Result<Self> {
        let meta = root.join("pack.jsonc");

        let meta = std::fs::read_to_string(meta)?;

        let meta = jsonc_parser::parse_to_serde_value(&meta, &Default::default())?
            .map(|value| serde_json::from_value::<PackMeta>(value))
            .unwrap()?;

        let walk = ignore::WalkBuilder::new(&root)
            .standard_filters(false)
            .parents(false)
            .ignore(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .add_custom_ignore_filename(".rppignore")
            .build();

        let mut sources = BTreeSet::new();

        for entry in walk {
            let entry = entry.map_err(crate::Error::from)?;
            let file_type = entry
                .file_type()
                .ok_or_else(|| crate::Error::Custom("No file type".into()))?;

            if file_type.is_dir() {
                continue;
            }

            let path = entry
                .path()
                .strip_prefix(&root)
                .map_err(|_| crate::Error::Custom("Failure stripping path".into()))?;

            sources.insert(path.to_path_buf());
        }

        Ok(Self {
            root,
            meta,
            sources,
            invalidated_sources: BTreeSet::new(),
        })
    }

    pub fn diff(&mut self) -> crate::Result<PackDiff> {
        let invalidated = std::mem::take(&mut self.invalidated_sources);

        let walk = ignore::WalkBuilder::new(&self.root)
            .standard_filters(false)
            .parents(false)
            .ignore(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .add_custom_ignore_filename(".rppignore")
            .build();

        let mut current_sources = BTreeSet::new();

        for entry in walk {
            let entry = entry.map_err(crate::Error::from)?;
            let file_type = entry
                .file_type()
                .ok_or_else(|| crate::Error::Custom("No file type".into()))?;

            if file_type.is_dir() {
                continue;
            }

            let path = entry
                .path()
                .strip_prefix(&self.root)
                .map_err(|_| crate::Error::Custom("Failure stripping path".into()))?;

            current_sources.insert(path.to_path_buf());
        }

        let deletions = self.sources.difference(&current_sources).cloned().collect();

        let creations = current_sources.difference(&self.sources).cloned().collect();

        self.sources = current_sources;

        Ok(PackDiff {
            invalidated,
            deletions,
            creations,
        })
    }
}
