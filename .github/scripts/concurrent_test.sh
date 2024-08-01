#!/bin/bash
set -e

export TERM=xterm-256color

# Statements waiting to be executed
statements=(
    "LOOM_LOG=debug \
LOOM_LOCATION=1 \
LOOM_CHECKPOINT_INTERVAL=1000 \
LOOM_CHECKPOINT_FILE=loom.json \
RUSTFLAGS=\"--cfg loom -Znext-solver\" \
cargo +nightly test --no-default-features --features nightly,loom -p tests --release -- --nocapture"
)

# loop echo and executing statements
for statement in "${statements[@]}"; do
    echo "$(tput setaf 3)$statement$(tput sgr0)"
    eval $statement
    echo
done
