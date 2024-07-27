#!/bin/bash
set -e

export TERM=xterm-256color

# Statements waiting to be executed
statements=(
    "cargo clippy --no-default-features -- -D warnings"

    "cargo test --no-default-features"

    "cargo +nightly test --no-default-features --features nightly"

    "cargo +nightly miri test --no-default-features -- --nocapture"
    "cargo +nightly miri test --no-default-features --features nightly -- --nocapture"

    "LOOM_LOG=debug \
LOOM_LOCATION=1 \
LOOM_CHECKPOINT_INTERVAL=1 \
LOOM_CHECKPOINT_FILE=loom.json \
RUSTFLAGS=\"--cfg loom\" \
cargo test --no-default-features loom_test --release"

    "cargo doc --no-deps --no-default-features"
)

# loop echo and executing statements
for statement in "${statements[@]}"; do
    echo "$(tput setaf 3)$statement$(tput sgr0)"
    eval $statement
    echo
done
