# rpp task runner — https://github.com/casey/just
# `just` with no arguments lists available recipes.

_default:
    @just --list

# Type-check the whole workspace
check:
    cargo check --locked --workspace --all-targets

# Type-check one crate, e.g. `just check-crate rpp --no-default-features`
check-crate crate *ARGS:
    cargo check --locked -p {{crate}} --all-targets {{ARGS}}

# Run all tests (workspace)
test *ARGS:
    cargo test --locked --workspace {{ARGS}}

# Run tests for a single crate, e.g. `just test-crate rpp`
test-crate crate *ARGS:
    cargo test --locked -p {{crate}} {{ARGS}}

# Format all code
fmt:
    cargo fmt --all

# Check formatting without modifying
fmt-check:
    cargo fmt --all -- --check

# Clippy with warnings denied
lint:
    cargo clippy --locked --workspace --all-targets -- -D warnings

# Lint one crate, with optional Cargo feature flags
lint-crate crate *ARGS:
    cargo clippy --locked -p {{crate}} --all-targets {{ARGS}} -- -D warnings

# Check the core library's supported configurations independently of the CLI
check-features:
    cargo check --locked -p rpp --all-targets --no-default-features
    cargo check --locked -p rpp --all-targets
    cargo check --locked -p rpp --all-targets --features wasm,tracing

# Fail before tests can skip WASM coverage when the guest target is missing
require-wasm:
    @test -d "$(rustc --print sysroot)/lib/rustlib/wasm32-wasip2/lib" || { echo "Missing wasm32-wasip2 target; run mise install." >&2; exit 1; }

# Exercise the component host and its CLI integrations
verify-wasm: require-wasm
    cargo test --locked -p rpp-wasm --test integration
    cargo test --locked -p rpp-cli --test example_wasm_plugin --test window_host_e2e

# Build release binaries
build:
    cargo build --locked --release --workspace

# Run the rpp CLI, e.g. `just rpp build`
rpp *ARGS:
    cargo run --locked -p rpp-cli -- {{ARGS}}

# Full verification before publishing substantial changes
ci: require-wasm fmt-check lint check-features test

# Remove build artifacts and example caches
clean:
    cargo clean
    rm -rf examples/pack/.rpp examples/pack/dist

# Build the grayscale-wasm example component
example-wasm: require-wasm
    cargo build --locked --release --target wasm32-wasip2 --manifest-path examples/plugins/grayscale-wasm/guest/Cargo.toml
    cp examples/plugins/grayscale-wasm/guest/target/wasm32-wasip2/release/grayscale_wasm_guest.wasm examples/plugins/grayscale-wasm/grayscale.wasm
