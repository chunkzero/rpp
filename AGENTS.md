# AGENTS.md

This file provides guidelines for agentic coding agents working in the RPP (Resource Pack Processor) repository.

## Project Overview

RPP is a Rust-based toolchain for building and testing Minecraft resource packs. It consists of:
- `crates/rpp`: Core library with compile and Lua plugin systems
- `crates/rpp-cli`: CLI application with build, init, plugin, and serve commands

## Build Commands

```bash
# Build the entire workspace
cargo build

# Build release version
cargo build --release

# Build specific crate
cargo build -p rpp
cargo build -p rpp-cli

# Check code without building
cargo check

# Run the CLI
cargo run -p rpp-cli -- <args>
```

## Test Commands

```bash
# Run all tests
cargo test

# Run tests for specific crate
cargo test -p rpp
cargo test -p rpp-cli

# Run a specific test by name
cargo test <test_name>

# Run tests with output
cargo test -- --nocapture

# Run tests in release mode
cargo test --release
```

## Lint/Format Commands

```bash
# Format code
cargo fmt

# Check formatting without modifying
cargo fmt -- --check

# Run Clippy lints
cargo clippy
cargo clippy -- -D warnings

# Run Clippy on all targets
cargo clippy --all-targets --all-features
```

## Code Style Guidelines

### Imports

Order imports as follows:
1. Standard library (`std::`, `core::`, `alloc::`)
2. External crates
3. Internal crate imports (`crate::`)
4. Super module imports (`super::`)

Group imports with a blank line between each section. Use absolute paths within the crate.

```rust
use std::{fs::File, io::BufReader, path::PathBuf, time::SystemTime};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::compile::worker;
```

### Formatting

- Use standard Rust formatting via `cargo fmt`
- Max line length: 100 characters (default)
- 4 spaces for indentation
- No trailing whitespace
- Final newline at end of files

### Types and Naming

- **Types**: PascalCase (`CompileError`, `PackCompiler`)
- **Functions/Methods**: snake_case (`build()`, `submit_job()`)
- **Variables**: snake_case (`cache_dir`, `to_process`)
- **Constants**: SCREAMING_SNAKE_CASE
- **Modules**: snake_case (`compile`, `lua`)
- **Traits**: PascalCase with descriptive names
- **Generic parameters**: Single uppercase letters (`T`, `K`, `V`)

### Error Handling

Use `thiserror` for defining error types:

```rust
#[derive(Debug, Error)]
pub enum CompileError {
    #[error("Failed to read cache file {path}: {source}")]
    CacheRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
```

- Use `#[from]` for automatic error conversion
- Use `#[source]` for underlying errors
- Provide descriptive error messages with context
- Main error type in each crate should be named `Error`
- Result type alias: `pub type Result<T> = std::result::Result<T, Error>;`

### Module Structure

```rust
// lib.rs or mod.rs pattern
pub mod compile;
mod rpp;
pub(crate) mod util;

#[cfg(feature = "lua")]
pub mod lua;

pub use rpp::*;
```

- Use `pub(crate)` for internal visibility
- Use feature gates (`#[cfg(feature = "...")]`) for optional functionality
- Re-export commonly used items at crate root

### Documentation

- Document all public APIs with `///`
- Use `//` for internal comments sparingly
- Document panics, errors, and safety requirements

### Unsafe Code

Minimize unsafe code. When necessary:
- Document safety invariants
- Keep unsafe blocks as small as possible
- Mark unsafe functions with `unsafe fn` and document requirements

### Features

Available features in `rpp` crate:
- `lua`: Lua scripting support (enabled by default)
- `watcher`: File watching support (enabled by default)
- `tracing`: Tracing integration

Always use `dep:` prefix when referring to optional dependencies in features.

### Workspace Structure

- Use workspace inheritance for shared metadata
- Keep crate versions synchronized via `workspace.package`
- Edition 2021

## Common Patterns

### Builder Pattern

```rust
impl PackCompiler {
    pub fn builder() -> builder::PackCompilerBuilder {
        Default::default()
    }
}
```

### Static Lazy Initialization

```rust
use once_cell::sync::Lazy;
use regex::Regex;

pub(crate) static ID_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap()
});
```

### CLI with Clap

```rust
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[arg(short, long = "config", default_value = "rpp.toml")]
    pub config_path: String,
    #[command(subcommand)]
    pub command: Command,
}
```

## Important Notes

- This project uses `mlua` with vendored LuaJIT
- The CLI binary is named `rpp` (from `rpp-cli` crate)
- Cache files use `bincode` with big-endian fixed-int encoding
- The project supports custom ignore files (`.rppignore`)
