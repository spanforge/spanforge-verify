#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build --locked --release --bin linux-gate --bin spanforge-verify-fixture
mkdir -p target/linux-evidence
exec target/release/linux-gate --fixture target/release/spanforge-verify-fixture --repeat "${1:-100}" --evidence "target/linux-evidence/gate-$(date -u +%Y%m%dT%H%M%SZ).json"
