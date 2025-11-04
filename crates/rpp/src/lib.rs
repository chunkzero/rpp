use thiserror::Error;

pub mod build;
pub mod config;
pub mod lua;
mod util;
// #[cfg(feature = "tracing")]

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Lua(#[from] mlua::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
