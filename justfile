# rpp task runner — https://github.com/casey/just
# `just` with no arguments lists available recipes.

set shell := ["bash", "-euo", "pipefail", "-c"]

_default:
    @just --list

# Type-check the whole workspace
check:
    cargo check --locked --workspace --all-targets

# Type-check one crate, e.g. `just check-crate rpp --no-default-features`
check-crate crate *ARGS:
    cargo check --locked -p {{ crate }} --all-targets {{ ARGS }}

# Run all tests (workspace)
test *ARGS:
    cargo test --locked --workspace {{ ARGS }}

# Run tests for a single crate, e.g. `just test-crate rpp`
test-crate crate *ARGS:
    cargo test --locked -p {{ crate }} {{ ARGS }}

# Format sources, configuration, and documentation
fmt: fmt-rust fmt-jvm fmt-lua fmt-config
    just --unstable --fmt

# Check formatting without modifying
fmt-check: (fmt-rust "--check") fmt-jvm-check fmt-lua-check fmt-config-check
    just --unstable --fmt --check

# Format Rust, including guest crates outside the workspace; accepts --check
fmt-rust *ARGS:
    cargo fmt --all -- {{ ARGS }}
    cargo fmt --manifest-path crates/rpp-wasm/tests/fixtures/math-component/Cargo.toml -- {{ ARGS }}
    cargo fmt --manifest-path crates/rpp-cli/tests/fixtures/window-host-component/Cargo.toml -- {{ ARGS }}
    cargo fmt --manifest-path crates/rpp-cli/tests/fixtures/js-component/Cargo.toml -- {{ ARGS }}
    cargo fmt --manifest-path examples/plugins/grayscale-wasm/guest/Cargo.toml -- {{ ARGS }}

# Format Java and Gradle Kotlin scripts
fmt-jvm:
    ktlint --format "integrations/jvm/**/*.kts" "!**/build/**" "!**/.gradle/**"
    git ls-files -z --cached --others --exclude-standard -- '*.java' | xargs -0 google-java-format --aosp --replace

# Check Java and Gradle Kotlin script formatting
fmt-jvm-check:
    ktlint "integrations/jvm/**/*.kts" "!**/build/**" "!**/.gradle/**"
    git ls-files -z --cached --others --exclude-standard -- '*.java' | xargs -0 google-java-format --aosp --dry-run --set-exit-if-changed

# Format Lua plugins and examples
fmt-lua:
    stylua --verify examples

# Check Lua formatting
fmt-lua-check:
    stylua --check examples

# Format configuration and documentation
fmt-config:
    oxfmt --write .

# Check configuration and documentation formatting
fmt-config-check:
    oxfmt --check .

# Clippy with warnings denied
lint:
    cargo clippy --locked --workspace --all-targets -- -D warnings

# Lint one crate, with optional Cargo feature flags
lint-crate crate *ARGS:
    cargo clippy --locked -p {{ crate }} --all-targets {{ ARGS }} -- -D warnings

# Check the core library's supported configurations independently of the CLI
check-features:
    cargo check --locked -p rpp --all-targets --no-default-features
    cargo check --locked -p rpp --all-targets
    cargo check --locked -p rpp --all-targets --features wasm,tracing
    cargo check --locked -p rpp --all-targets --no-default-features --features js

# Fail before tests can skip WASM coverage when the guest target is missing
require-wasm:
    @test -d "$(rustc --print sysroot)/lib/rustlib/wasm32-wasip2/lib" || { echo "Missing wasm32-wasip2 target; run mise install." >&2; exit 1; }

# Exercise the component host and its CLI integrations
verify-wasm: require-wasm
    cargo test --locked -p rpp-wasm --test integration
    cargo test --locked -p rpp-cli --test example_wasm_plugin --test window_host_e2e --test js_component_e2e

# Build release binaries
build:
    cargo build --locked --release --workspace

# Run the rpp CLI, e.g. `just rpp build`
rpp *ARGS:
    cargo run --locked -p rpp-cli -- {{ ARGS }}

# Run the JVM Gradle wrapper with mise's JDKs, e.g. `just jvm :test`
[positional-arguments]
jvm *ARGS:
    integrations/jvm/gradlew -p integrations/jvm "$@" --no-daemon

# Test the JVM client against an actual CLI and compile the Minestom example
verify-jvm:
    cargo build --locked -p rpp-cli
    RPP_BIN="{{ justfile_directory() }}/target/debug/rpp" just jvm :test --rerun :examples:minestom:classes

# Verify Rust independently of JVM checks
verify-rust: require-wasm lint check-features test

# Full verification before publishing substantial changes
ci: fmt-check verify-rust verify-jvm

# Remove build artifacts and example caches
clean:
    cargo clean
    rm -rf examples/pack/.rpp examples/pack/dist

# Build the grayscale-wasm example component
example-wasm: require-wasm
    cargo build --locked --release --target wasm32-wasip2 --manifest-path examples/plugins/grayscale-wasm/guest/Cargo.toml
    cp examples/plugins/grayscale-wasm/guest/target/wasm32-wasip2/release/grayscale_wasm_guest.wasm examples/plugins/grayscale-wasm/grayscale.wasm

# Test the publish-plugin action's helper script
test-actions:
    python3 -m unittest discover -s .github/actions/publish-plugin -p 'test_*.py'
