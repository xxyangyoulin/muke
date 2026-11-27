#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --check
cargo fmt --manifest-path examples/standalone-plugin/Cargo.toml --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
while IFS= read -r -d '' file; do
  node --input-type=module --check < "$file"
done < <(find frontend plugins examples/standalone-plugin/frontend -name '*.js' -print0)
cargo test --locked --test rust_export -- --ignored
python3 tests/rust/core.py
python3 tests/rust/contract.py --report /tmp/muke-rust-baseline.json
python3 tests/rust/plugins.py
python3 tests/rust/wallpapers.py
python3 tests/rust/external.py
python3 tests/rust/llm.py
