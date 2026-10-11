#!/usr/bin/env bash
# Offline deterministic domain recording; no API, DB, broker, or in-zone mutation endpoint.
set -euo pipefail
cd "$(dirname "$0")/../../../.."
NIGHTFALL_V5_RECORDING_OUT="$PWD/apps/api/fixtures/sessions/phase3-casts-v5.nfr" \
  cargo test -p nightfall-api --test zone_phase3_runtime \
  schema_three_coverage_counts_live_lifecycles_once_and_verifies_the_suffix -- --exact
