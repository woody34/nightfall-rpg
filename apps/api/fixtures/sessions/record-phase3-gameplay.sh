#!/usr/bin/env bash
# Offline domain fixture: admitted corpse snapshot, live offers, CP, PvP, town and NPC XP.
set -euo pipefail
cd "$(dirname "$0")/../../../.."
NIGHTFALL_V5_GAMEPLAY_OUT="$PWD/apps/api/fixtures/sessions/phase3-gameplay-v5.nfr" \
  cargo test -p nightfall-api --lib \
  domain::zone::state::runtime::live_damage::tests::gameplay_recording_exercises_offers_auto_cp_pk_and_npc_xp -- --exact
