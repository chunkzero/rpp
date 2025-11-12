use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct FileMeta {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    pub path: String,
    pub size: u64,
}

impl FileMeta {
    pub fn from_path(path: &Path) -> crate::Result<FileMeta> {
        let path = path.canonicalize()?;
        let meta = path.metadata()?;

        Ok(Self {
            name: path
                .file_name()
                .ok_or(crate::Error::FileMeta("Invalid file name".into()))?
                .to_string_lossy()
                .to_string(),
            extension: path
                .extension()
                .map(|extension| extension.to_string_lossy().to_string()),
            path: path.to_string_lossy().to_string(),
            size: meta.len(),
        })
    }
}
