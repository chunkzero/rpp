use std::{fs::File, io::BufReader, path::PathBuf, time::SystemTime};

use crate::compile::cache::Cache;

pub mod cache;
pub mod context;
pub mod event;
mod worker;

pub struct PackCompiler {
    pool: worker::WorkerPool,

    pack: PathBuf,
    cache_dir: PathBuf,
    cache: Cache,
}

impl PackCompiler {
    pub fn new(
        event_handler_providers: Vec<worker::EventHandlerProvider>,
        pack: PathBuf,
        cache_dir: PathBuf,
    ) -> crate::Result<Self> {
        let cache_path = cache_dir.join("cache.rppstate");

        let cache: Cache = if cache_path.is_file() {
            let file = File::open(cache_path)?;
            bincode::decode_from_reader(BufReader::new(file), bincode_config())?
        } else {
            Cache::default()
        };

        Ok(Self {
            pool: worker::WorkerPool::new(
                event_handler_providers,
                std::thread::available_parallelism()
                    .map(|value| value.into())
                    .unwrap_or(1),
            ),
            pack,
            cache_dir,
            cache,
        })
    }

    pub fn clean(&self) -> std::io::Result<()> {
        if self.cache_dir.is_file() {
            std::fs::remove_file(&self.cache_dir)?;
        }
        if self.cache_dir.is_dir() {
            std::fs::remove_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }

    pub fn build(&mut self) -> crate::Result<()> {
        while let Ok(_) = self.pool.try_recv_result() {}

        let cache = &self.cache.sources;

        let walk = ignore::WalkBuilder::new(&self.pack)
            .standard_filters(false)
            .parents(false)
            .ignore(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .add_custom_ignore_filename(".rppignore")
            .build();

        let mut to_process = Vec::<(PathBuf, String, u64, u64)>::new();

        for entry in walk {
            let entry = entry?;

            match entry.file_type() {
                Some(file_type) => {
                    if !file_type.is_file() {
                        continue;
                    }
                }
                None => continue,
            }

            let path = entry.path().to_string_lossy();
            let metadata = entry.metadata()?;
            let mtime: u64 = mtime(&metadata);
            let size = metadata.len();

            if match cache.get(path.as_ref()) {
                Some(entry) => entry.fingerprint.size != size || entry.fingerprint.mtime != mtime,
                None => true,
            } {
                to_process.push((entry.path().into(), path.into(), mtime, size));
            };
        }

        if to_process.is_empty() {
            return Ok(());
        }

        let job_count = to_process.len();

        for (path, path_str, mtime, size) in to_process {
            self.pool
                .submit_job(worker::Job {
                    path,
                    path_str,
                    mtime,
                    size,
                })
                .map_err(|_| crate::Error::Custom("Error submitting job".into()));
        }

        for _ in 0..job_count {
            match self.pool.recv_result() {
                Ok(result) => {
                    self.cache.sources.insert(result.path, result.state);
                }
                Err(_) => {
                    return Err(crate::Error::Custom(
                        "Worker pool closed unexpectedly".into(),
                    ));
                }
            }
        }

        Ok(())
    }
}

#[inline(always)]
fn bincode_config(
) -> bincode::config::Configuration<bincode::config::BigEndian, bincode::config::Fixint> {
    bincode::config::standard()
        .with_big_endian()
        .with_fixed_int_encoding()
}

fn mtime(metadata: &std::fs::Metadata) -> u64 {
    if let Ok(time) = metadata.modified() {
        time.duration_since(SystemTime::UNIX_EPOCH)
            .map(|time| time.as_millis() as u64)
            .unwrap_or(0)
    } else {
        0
    }
}
