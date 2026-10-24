#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
export CARGO_HOME="$PWD/.tools/linux-cargo"
export RUSTUP_HOME="$PWD/.tools/linux-rustup"
export CARGO_TARGET_DIR="$PWD/target/linux"
export PATH="$CARGO_HOME/bin:$PATH"
cargo test -p patchhub-core -p patchhub-cli
cargo build --release -p patchhub-cli -p patchhub-installer-worker --bin patchhub-core --bin patchhub-worker
