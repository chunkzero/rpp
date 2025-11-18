use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use crate::{compile::context::BuildContext, pack::Pack};

/// Represents a file; can be promoted from a path (only carries raw path data) to bytes, then to strings;
#[derive(Debug)]
pub enum ProcessorFile {
    String(PathBuf, String),
    Bytes(PathBuf, Vec<u8>),
    Path(PathBuf),
}

impl ProcessorFile {
    #[inline(always)]
    pub fn is_string(&self) -> bool {
        matches!(self, Self::String(_, _))
    }

    #[inline(always)]
    pub fn is_bytes(&self) -> bool {
        matches!(self, Self::Bytes(_, _))
    }

    #[inline(always)]
    pub fn is_path(&self) -> bool {
        matches!(self, Self::Path(_))
    }

    pub fn to_string(self) -> crate::Result<ProcessorFile> {
        Ok(match self {
            ProcessorFile::String(_, _) => self,
            ProcessorFile::Bytes(path, bytes) => Self::String(
                path,
                String::from_utf8(bytes).map_err(|_| {
                    crate::Error::Custom("Bytes were not a valid UTF-8 string".into())
                })?,
            ),
            ProcessorFile::Path(path) => {
                let bytes = std::fs::read(&path)?;
                Self::String(
                    path,
                    String::from_utf8(bytes).map_err(|_| {
                        crate::Error::Custom("Bytes were not a valid UTF-8 string".into())
                    })?,
                )
            }
        })
    }

    pub fn to_bytes(self) -> crate::Result<ProcessorFile> {
        Ok(match self {
            ProcessorFile::String(path, value) => Self::Bytes(path, value.into_bytes()),
            ProcessorFile::Bytes(_, _) => self,
            ProcessorFile::Path(path) => {
                let bytes = std::fs::read(&path)?;
                Self::Bytes(path, bytes)
            }
        })
    }

    pub fn path(&self) -> &Path {
        match self {
            ProcessorFile::String(path, _) => &path,
            ProcessorFile::Bytes(path, _) => &path,
            ProcessorFile::Path(path) => &path,
        }
    }
}

#[derive(Debug)]
pub struct FileProcessContext<'a> {
    pub build_context: &'a BuildContext,
    pub pack: Pack,

    pub file: ProcessorFile,
    dependencies: BTreeSet<PathBuf>,
}

impl<'a> FileProcessContext<'a> {
    pub fn new(build_context: &'a BuildContext, pack: Pack, file: ProcessorFile) -> Self {
        Self {
            build_context,
            pack,
            file,
            dependencies: Default::default(),
        }
    }

    pub fn depend_on(&mut self, path: PathBuf) {
        self.dependencies.insert(path);
    }
}

pub trait Processor {
    fn description(&self) -> String;

    fn process(&self, context: &mut FileProcessContext) -> crate::Result<()>;
}
