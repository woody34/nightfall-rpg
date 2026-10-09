#!/usr/bin/env bash
# Runs bot scenarios (.nfs) headless against the real API, one after another.
#
#   Scripts/run-sim.sh [--api attach|start] [--artifacts DIR] [--no-replay] [--timeout SECS] SCENARIO|GLOB...
#
#   --api attach   (default) use the API already answering :3000/health, e.g. `moon run api:dev`
#   --api start    docker compose up -d, then start the API with AUTH_DEV_TOKENS=1 (stopped on exit)
#   --artifacts    where Saved/Sim output and recordings go (default Saved/SimArtifacts/<timestamp>)
#   --no-replay    skip the server replay check of each recording
#   --timeout      per-scenario wall clock limit, default 600
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

sim_resolve_env
sim_ensure_build
sim_api_prepare "$API_MODE"
mkdir -p "$SIM_SAVED_DIR"

FAILED=0
SUMMARY=()
for scenario in "${SCENARIOS[@]}"; do
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

  status=PASS; note=""
  if ((code == 124 || code == 137)); then status=FAIL; note="timeout after ${SIM_TIMEOUT}s"
  elif ((code != 0)); then status=FAIL; note="bot exit $code"; fi
  [[ -f "$dest/$name.xml" ]] || { status=FAIL; note="${note:+$note; }no JUnit report"; }

  replay="skipped"
  if ((REPLAY)); then
    sim_export_recording "$dest" "$name" || true
    shopt -s nullglob; nfrs=("$dest"/*.nfr); shopt -u nullglob
    if ((${#nfrs[@]} == 0)); then
      replay="none"; status=FAIL; note="${note:+$note; }no session recording"
    else
      replay="ok"
      for nfr in "${nfrs[@]}"; do
        if ! sim_replay "$nfr" "${nfr%.nfr}.replay.log"; then
          replay="DIVERGED($(basename "$nfr"))"; status=FAIL; note="${note:+$note; }replay check failed"
        fi
      done
    fi
  fi
  shopt -s nullglob; nfrs=("$dest"/*.nfr); shopt -u nullglob
  for nfr in "${nfrs[@]}"; do
    if ! python3 "$HERE/sim-trace.py" "$nfr" "$dest/$name.xml"; then
      status=FAIL; note="${note:+$note; }trace generation failed"
    fi
  done
  if [[ "$status" != PASS ]]; then
    FAILED=$((FAILED + 1))
    if ((VIDEO)); then
      # Retry reports/logs live separately. Video problems remain diagnostic and cannot change
      # the verdict or hide the original failed report already copied to dest.
      if ! bash "$HERE/sim-video.sh" "$scenario" "$dest/video" >"$dest/video.log" 2>&1; then
        note="${note:+$note; }video unavailable (see video.log)"
      fi
    fi
  fi
  line="$status $name (${secs}s, replay=$replay)${note:+ - $note}"
  SUMMARY+=("$line")
  echo "$line"
done

echo "sim: ${#SCENARIOS[@]} scenario(s), $FAILED failed; artifacts in $ARTIFACTS"
((FAILED == 0))
