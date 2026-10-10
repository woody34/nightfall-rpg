#!/usr/bin/env bash
# Runs bot scenarios (.nfs) headless against the real API, one after another.
#
#   Scripts/run-sim.sh [--api attach|start] [--artifacts DIR] [--no-replay] [--timeout SECS] SCENARIO|GLOB...
#
#   --api attach   (default) use the API already answering :3000/health, e.g. `moon run api:dev`
#   --api start    docker compose up -d, then start the API with AUTH_DEV_TOKENS=1 (stopped on exit)
#   --artifacts    must be empty (including hidden files); retains prior evidence on rejection.
#                  where Saved/Sim output and recordings go (default Saved/SimArtifacts/<timestamp>)
#   --no-replay    skip the server replay check of each recording
#   --timeout      per-scenario wall clock limit, default 600
#   --video        render one diagnostic retry only after a headless failure (optional prerequisites)
#
# Per scenario the bot is run as
#   Nightfall -game -nullrhi -nosound -unattended -BotScenario=<path>
# and must exit 0 (pass) / 1 (fail); it writes Saved/Sim/<scenario>.{xml,log} and the session
# recording (.nfr). Each recording is then replayed by nightfall-replay, which must match byte for
# byte. Exit status is non-zero if any scenario fails (bot, missing recording or replay divergence).
# See sim-lib.sh for environment overrides. Docs: apps/client-unreal/README.md "Running simulations".
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=sim-lib.sh
. "$HERE/sim-lib.sh"

API_MODE=attach; ARTIFACTS=""; REPLAY=1; VIDEO=0
ARGS=()
while (($#)); do
  case "$1" in
    --api) API_MODE="${2:?--api needs attach|start}"; shift 2 ;;
    --artifacts) ARTIFACTS="${2:?--artifacts needs a directory}"; shift 2 ;;
    --no-replay) REPLAY=0; shift ;;
    --video) VIDEO=1; shift ;;
    --timeout) SIM_TIMEOUT="${2:?--timeout needs seconds}"; shift 2 ;;
    -h|--help) sed -n '2,/^set -/p' "$0" | sed '$d'; exit 0 ;;
    --) shift; ARGS+=("$@"); break ;;
    -*) sim_die "unknown option $1" ;;
    *) ARGS+=("$1"); shift ;;
  esac
done
((${#ARGS[@]})) || sim_die "no scenario given (see --help)"
[[ -n "$ARTIFACTS" ]] || ARTIFACTS="$SIM_PROJECT_DIR/Saved/SimArtifacts/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$ARTIFACTS"
ARTIFACTS="$(cd "$ARTIFACTS" && pwd)"
SIM_API_LOG="$ARTIFACTS/api.log"

mapfile -t SCENARIOS < <(sim_expand_scenarios "${ARGS[@]}")
((${#SCENARIOS[@]})) || sim_die "no scenario matches: ${ARGS[*]}"

sim_claim_artifacts "${SCENARIOS[@]}"

# Validate every unit before any API/Compose operation. Single-client sequences reset
# Phase2 ledgers per scenario; observer roles belong in the multi wrapper.
FIXTURES=()
for scenario in "${SCENARIOS[@]}"; do
  selected="$(sim_fixture_detect "$scenario")" || sim_die "invalid scenario fixture"
  sim_fixture_validate_mode "$selected" "$API_MODE"
  FIXTURES+=("$selected")
done
sim_resolve_env
sim_ensure_build
mkdir -p "$SIM_SAVED_DIR"

FAILED=0
SUMMARY=()
for scenario in "${SCENARIOS[@]}"; do
  selected="${FIXTURES[${#SUMMARY[@]}]}"
  sim_unit_prepare "$selected" "$API_MODE" "$scenario"
  name="$(basename "$scenario" .nfs)"
  dest="$ARTIFACTS/$name"
  marker="$(sim_new_marker)"
  mkdir -p "$dest"
  sim_bot_cmd "$scenario"
  start=$SECONDS
  code=0
  (cd "$(sim_bot_cwd)" && sim_with_timeout "$SIM_TIMEOUT" "${SIM_BOT_CMD[@]}") >"$dest/stdout.log" 2>&1 || code=$?
  secs=$((SECONDS - start))
  sim_collect "$marker" "$dest"
  rm -f "$marker"

  status=PASS; note=""; INFRA=()
  if ((code == 124 || code == 137)); then status=FAIL; note="timeout after ${SIM_TIMEOUT}s"
  elif ((code != 0)); then status=FAIL; note="bot exit $code"; fi
  [[ -f "$dest/$name.xml" ]] || { status=FAIL; note="${note:+$note; }no JUnit report"; }
  if ! python3 "$HERE/sim-junit.py" check "$dest/$name.xml"; then
    status=FAIL; note="${note:+$note; }JUnit failure or unreadable report"
  fi

  replay="skipped"
  if ((REPLAY)); then
    sim_export_recording "$dest" "$name" || true
    shopt -s nullglob; nfrs=("$dest"/*.nfr); shopt -u nullglob
    if ((${#nfrs[@]} == 0)); then
      INFRA+=(missing_recording)
      replay="none"; status=FAIL; note="${note:+$note; }no session recording"
    else
      replay="ok"
      for nfr in "${nfrs[@]}"; do
        if ! sim_replay "$nfr" "${nfr%.nfr}.replay.log"; then
          INFRA+=(replay_check)
          replay="DIVERGED($(basename "$nfr"))"; status=FAIL; note="${note:+$note; }replay check failed"
        fi
      done
    fi
  fi
  shopt -s nullglob; nfrs=("$dest"/*.nfr); shopt -u nullglob
  for nfr in "${nfrs[@]}"; do
    if ! python3 "$HERE/sim-trace.py" "$nfr" "$dest/$name.xml" || [[ ! -s "${nfr%.nfr}.trace.html" || ! -r "${nfr%.nfr}.trace.html" ]]; then
      INFRA+=(trace_generation)
      status=FAIL; note="${note:+$note; }trace generation failed"
    fi
  done
  if ! python3 "$HERE/sim-contract.py" merge --out "$dest/coverage.contract.json" --report "$dest/$name.xml" "$dest/$name.coverage.contract.json"; then
    INFRA+=(contract_coverage)
    status=FAIL; note="${note:+$note; }contract coverage missing or invalid"
  fi
  if ! sim_transition_coverage "$dest" "$REPLAY"; then
    INFRA+=(transition_coverage)
    status=FAIL; note="${note:+$note; }transition coverage missing or invalid"
  fi
  if ! python3 "$HERE/sim-gates.py" scenario --report "$dest/$name.xml" --out "$dest/pipeline-state.json" \
      --scenario "$name" --code "$code" "${INFRA[@]}"; then
    status=FAIL; note="${note:+$note; }bot report classification or infrastructure failure"
  fi
  if [[ "$status" != PASS ]]; then
    FAILED=$((FAILED + 1))
    python3 "$HERE/sim-junit.py" failure "$dest/$name.xml" "$name" "$note"
    if ((VIDEO)); then
      # Retry reports/logs live separately. Video problems remain diagnostic and cannot change
      # the verdict or hide the original failed report already copied to dest.
      if ! bash "$HERE/sim-video.sh" "$scenario" "$dest/video" >"$dest/video.log" 2>&1; then
        note="${note:+$note; }video unavailable (see video.log)"
      fi
    fi
  fi
  if ! sim_fixture_finish; then
    status=FAIL; note="${note:+$note; }fixture cleanup failed"
    FAILED=$((FAILED + 1)); FIXTURE_CLEANUP_FAILED=1
  fi
  line="$status $name (${secs}s, replay=$replay)${note:+ - $note}"
  SUMMARY+=("$line")
  echo "$line"
  [[ "${FIXTURE_CLEANUP_FAILED:-0}" != 1 ]] || break
done

FINAL_INFRA=()
if [[ "${FIXTURE_CLEANUP_FAILED:-0}" == 1 ]]; then FINAL_INFRA+=(fixture_cleanup); fi
if ! sim_suite_transitions "${SCENARIOS[@]}"; then
  FINAL_INFRA+=(suite_transition_coverage); FAILED=$((FAILED + 1))
fi
final_code=0; ((FAILED == 0)) || final_code=1
sim_finalize_verdict "$final_code" "${FINAL_INFRA[@]}" || sim_die "cannot finalize pipeline verdict"

echo "sim: ${#SCENARIOS[@]} scenario(s), $FAILED failed; artifacts in $ARTIFACTS"
((FAILED == 0))
