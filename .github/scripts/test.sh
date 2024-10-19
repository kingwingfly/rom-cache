#!/bin/bash
set -e

export TERM=xterm-256color

# Statements waiting to be executed
statements=(
    "cargo clippy --no-default-features -- -D warnings"

    "cargo +nightly run --example example --no-default-features --features nightly"

    "cargo +nightly test --no-default-features --features nightly"

    "cargo +nightly miri test --no-default-features --features nightly -- --nocapture"

    ".github/scripts/concurrent_test.sh"

    "cargo doc --no-deps --no-default-features"
)

# loop echo and executing statements
for statement in "${statements[@]}"; do
    echo "$(tput setaf 3)$statement$(tput sgr0)"
    eval $statement
    echo
done
