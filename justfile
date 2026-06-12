# rpp task runner — https://github.com/casey/just
# `just` with no arguments lists available recipes.

_default:
    @just --list

# Type-check the whole workspace
check:
    cargo check --workspace --all-targets

# Run all tests (workspace)
test *ARGS:
    cargo test --workspace {{ARGS}}

# Run tests for a single crate, e.g. `just test-crate rpp`
test-crate crate *ARGS:
    cargo test -p {{crate}} {{ARGS}}

# Format all code
fmt:
    cargo fmt --all

# Check formatting without modifying
fmt-check:
    cargo fmt --all -- --check

# Clippy with warnings denied
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Build release binaries
build:
    cargo build --release --workspace

# Run the rpp CLI, e.g. `just rpp build`
rpp *ARGS:
    cargo run -p rpp-cli -- {{ARGS}}

# Build the example WASM guest plugin (requires the wasm32-wasip2 target)
wasm-example:
    cd examples/plugins/grayscale-wasm && cargo build --release --target wasm32-wasip2
    cp examples/plugins/grayscale-wasm/target/wasm32-wasip2/release/grayscale_wasm.wasm \
       examples/plugins/grayscale-wasm/plugin.wasm

# Everything CI would run: format check, lints, tests
ci: fmt-check lint test

# Remove build artifacts and example caches
clean:
    cargo clean
    rm -rf examples/pack/.rpp examples/pack/dist
