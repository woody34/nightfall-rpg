#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
for fixture in two-players-v4.nfr two-players-fight-v2.nfr phase3-casts-v5.nfr phase3-gameplay-v5.nfr; do
  cargo run --quiet --bin nightfall-replay -- check --file "fixtures/sessions/$fixture"
done
# V6 (T11 capture): the embedded V6 bundle composition takes about 75 s in a debug build, so
# this fixture uses the release binary.
for fixture in t11-v6-zone1-epoch23.nfr; do
  cargo run --quiet --release --bin nightfall-replay -- check --file "fixtures/sessions/$fixture"
done
