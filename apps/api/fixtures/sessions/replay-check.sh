#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
for fixture in two-players-v4.nfr two-players-fight-v2.nfr; do
  cargo run --quiet --bin nightfall-replay -- --source file --file "fixtures/sessions/$fixture"
done
