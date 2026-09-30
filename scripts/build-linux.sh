#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
export CARGO_HOME="$PWD/.tools/linux-cargo"
export RUSTUP_HOME="$PWD/.tools/linux-rustup"
export CARGO_TARGET_DIR="$PWD/target/linux"
export PATH="$CARGO_HOME/bin:$PATH"
cargo test -p steam-patch-core -p steam-patch-cli
cargo build --release -p steam-patch-cli
