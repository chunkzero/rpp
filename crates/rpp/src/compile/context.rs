use std::path::{Path, PathBuf};

pub trait BuildContext {
    fn path(&self) -> &str;
    fn mtime(&self) -> u64;
    fn size(&self) -> u64;
    fn hash(&self) -> u64;
    fn set_hash(&mut self, hash: u64);
    fn dependencies(&self) -> &[String];
    fn set_dependencies(&mut self, deps: Vec<String>);
    fn outputs(&self) -> &[String];
    fn set_outputs(&mut self, outputs: Vec<String>);
}

pub struct SimpleBuildContext {
    pub path: String,
    pub mtime: u64,
    pub size: u64,
    pub hash: u64,
    pub dependencies: Vec<String>,
    pub outputs: Vec<String>,
}

impl BuildContext for SimpleBuildContext {
    fn path(&self) -> &str {
        &self.path
    }

    fn mtime(&self) -> u64 {
        self.mtime
    }

    fn size(&self) -> u64 {
        self.size
    }

    fn hash(&self) -> u64 {
        self.hash
    }

    fn set_hash(&mut self, hash: u64) {
        self.hash = hash;
    }

    fn dependencies(&self) -> &[String] {
        &self.dependencies
    }

    fn set_dependencies(&mut self, deps: Vec<String>) {
        self.dependencies = deps;
    }

    fn outputs(&self) -> &[String] {
        &self.outputs
    }

    fn set_outputs(&mut self, outputs: Vec<String>) {
        self.outputs = outputs;
    }
}

pub trait FileProcessContext<'a> {
    fn path(&self) -> &'a Path;

    fn build_ctx(&self) -> &'a dyn BuildContext;

    fn depend_on(&self, path: PathBuf) -> crate::Result<()>;

    fn write_output(&self, path: PathBuf, output: &[u8]) -> crate::Result<()>;

    fn queue_process(&self, path: PathBuf) -> crate::Result<()>;
}
