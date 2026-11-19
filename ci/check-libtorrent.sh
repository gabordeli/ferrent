#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo clippy -p ferrent-engine --features libtorrent --all-targets -- -D warnings
cargo test -p ferrent-engine --features libtorrent
