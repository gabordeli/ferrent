#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo fmt --all --check
./ci/check-docs-links.sh
# The daemon's default feature is libtorrent; the fast gate checks the
# mock path, the bridge has its own gate (ci/check-libtorrent.sh).
cargo clippy --workspace --all-targets --no-default-features -- -D warnings
cargo test --workspace --no-default-features
cargo build -p ferrent-gui -p ferrent-cli
