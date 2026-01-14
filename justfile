# Rung development tasks
# Install just: https://github.com/casey/just

# Default recipe - show available commands
default:
    @just --list

# Run all checks (what CI does)
ci: fmt-check clippy test doc

# Format code
fmt:
    cargo fmt --all

# Check formatting without modifying
fmt-check:
    cargo fmt --all -- --check

# Run clippy lints
clippy:
    cargo clippy --all-targets --all-features -- -D warnings

# Run tests
test:
    cargo test --all-features

# Run tests with output
test-verbose:
    cargo test --all-features -- --nocapture

# Build in release mode
build:
    cargo build --release

# Build and install locally
install:
    cargo install --path crates/rung-cli

# Generate documentation
doc:
    cargo doc --no-deps --all-features --open

# Check documentation builds without warnings
doc-check:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

# Clean build artifacts
clean:
    cargo clean

# Watch for changes and run tests
watch:
    cargo watch -x test

# Watch for changes and run clippy
watch-clippy:
    cargo watch -x clippy

# Run a specific crate's tests
test-crate crate:
    cargo test -p {{crate}}

# Check MSRV compatibility
msrv:
    cargo +1.83 check --all-targets --all-features

# Create a new release (updates version, creates tag)
release version:
    @echo "Updating version to {{version}}..."
    @sed -i '' 's/^version = ".*"/version = "{{version}}"/' Cargo.toml
    cargo check
    git add -A
    git commit -m "Release v{{version}}"
    git tag -a "v{{version}}" -m "Release v{{version}}"
    @echo "Created tag v{{version}}. Push with: git push && git push --tags"

# Development workflow: format, lint, test
dev: fmt clippy test
    @echo "✓ All checks passed!"
