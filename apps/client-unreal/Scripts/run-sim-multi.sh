#!/usr/bin/env bash
# Runs N bot processes at once against one API, one scenario each, sharing a -SimGroup id.
# Clients coordinate only through the server (e.g. wait until proxies >= 2), never IPC.
#
#   Scripts/run-sim-multi.sh [--api attach|start] [--artifacts DIR] [--group ID] [--timeout SECS]
#                            [--no-replay] SCENARIO SCENARIO...
#
#   Every positional argument is one process (a .nfs path; the same scenario twice is refused
#   because outputs are keyed by scenario name). All are launched together with
#   -SimGroup=<ID> (default sim-<timestamp>-<pid>) and waited for.
#   --timeout   group wall clock limit, default 600; survivors are killed and counted as failed
#   --api, --artifacts, --no-replay  as in run-sim.sh
#
# Artifacts: DIR/<scenario>/... per process (stdout.log; outputs are attributed by name), plus
# DIR/group.xml, one merged JUnit file (a missing or unreadable per-process report becomes a failed
# testcase). Exit 0 only if every process exited 0, wrote a report, and every recording replays.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=sim-lib.sh
. "$HERE/sim-lib.sh"

API_MODE=attach; ARTIFACTS=""; REPLAY=1; GROUP=""
ARGS=()
while (($#)); do
  case "$1" in
    --api) API_MODE="${2:?--api needs attach|start}"; shift 2 ;;
    --artifacts) ARTIFACTS="${2:?--artifacts needs a directory}"; shift 2 ;;
    --group) GROUP="${2:?--group needs an id}"; shift 2 ;;
    --timeout) SIM_TIMEOUT="${2:?--timeout needs seconds}"; shift 2 ;;
    --no-replay) REPLAY=0; shift ;;
    -h|--help) sed -n '2,/^set -/p' "$0" | sed '$d'; exit 0 ;;
    -*) sim_die "unknown option $1" ;;
    *) ARGS+=("$1"); shift ;;
  esac
done
((${#ARGS[@]} >= 2)) || sim_die "need at least two scenarios (one per process); use run-sim.sh for one"
GROUP="${GROUP:-sim-$(date +%Y%m%d-%H%M%S)-$$}"
[[ -n "$ARTIFACTS" ]] || ARTIFACTS="$SIM_PROJECT_DIR/Saved/SimArtifacts/$GROUP"
mkdir -p "$ARTIFACTS"
ARTIFACTS="$(cd "$ARTIFACTS" && pwd)"
SIM_API_LOG="$ARTIFACTS/api.log"

mapfile -t SCENARIOS < <(
  for a in "${ARGS[@]}"; do sim_expand_scenarios "$a" | head -1; done)
((${#SCENARIOS[@]} == ${#ARGS[@]})) || sim_die "a scenario argument matched no file: ${ARGS[*]}"
declare -A NAMES=()
for s in "${SCENARIOS[@]}"; do
  n="$(basename "$s" .nfs)"
  [[ -z "${NAMES[$n]:-}" ]] || sim_die "scenario $n listed twice"
  NAMES[$n]=1
done

sim_resolve_env
sim_ensure_build
sim_api_prepare "$API_MODE"
mkdir -p "$SIM_SAVED_DIR"

marker="$(sim_new_marker)"
declare -A PID=() STARTED=()
for scenario in "${SCENARIOS[@]}"; do
  name="$(basename "$scenario" .nfs)"
  mkdir -p "$ARTIFACTS/$name"
  sim_bot_cmd "$scenario" "-SimGroup=$GROUP"
  (cd "$(sim_bot_cwd)" && exec "${SIM_BOT_CMD[@]}") >"$ARTIFACTS/$name/stdout.log" 2>&1 &
  PID[$name]=$!
  STARTED[$name]=$SECONDS
done

kill_all() { for n in "${!PID[@]}"; do kill -TERM -- "${PID[$n]}" 2>/dev/null || true; done; }
trap 'kill_all; sim_api_cleanup; exit 130' INT TERM

# Wait for all, bounded by the group timeout.
deadline=$((SECONDS + SIM_TIMEOUT))
declare -A CODE=() SECS=()
pending=${#SCENARIOS[@]}
timed_out=0
while ((pending > 0)); do
  for n in "${!PID[@]}"; do
    [[ -z "${CODE[$n]:-}" ]] || continue
    if ! kill -0 "${PID[$n]}" 2>/dev/null; then
      c=0; wait "${PID[$n]}" || c=$?
      CODE[$n]=$c; SECS[$n]=$((SECONDS - STARTED[$n])); pending=$((pending - 1))
    fi
  done
  ((pending == 0)) && break
  if ((SECONDS >= deadline)); then
    timed_out=1
    kill_all; sleep 3
    for n in "${!PID[@]}"; do
      [[ -n "${CODE[$n]:-}" ]] || { kill -KILL "${PID[$n]}" 2>/dev/null || true; wait "${PID[$n]}" 2>/dev/null || true; CODE[$n]=124; SECS[$n]=$((SECONDS - STARTED[$n])); }
    done
    break
  fi
  sleep 0.5
done

# Outputs from the shared Saved/Sim go to every process folder that owns them by name; recordings
# (named by the runner) and anything else new go to the group folder.
sim_collect "$marker" "$ARTIFACTS/_all"
rm -f "$marker"
for n in "${!PID[@]}"; do
  for f in "$ARTIFACTS/_all/$n".*; do [[ -e "$f" ]] && mv "$f" "$ARTIFACTS/$n/"; done
done
shopt -s nullglob
for f in "$ARTIFACTS"/_all/*; do mv "$f" "$ARTIFACTS/"; done
shopt -u nullglob
rmdir "$ARTIFACTS/_all" 2>/dev/null || true

FAILED=0
REPORTS=()
for scenario in "${SCENARIOS[@]}"; do
  n="$(basename "$scenario" .nfs)"
  dest="$ARTIFACTS/$n"
  c="${CODE[$n]}"
  status=PASS; note=""
  if ((c == 124)); then status=FAIL; note="timeout after ${SIM_TIMEOUT}s"
  elif ((c != 0)); then status=FAIL; note="bot exit $c"; fi
  if ! python3 "$HERE/sim-junit.py" check "$dest/$n.xml"; then
    status=FAIL; note="${note:+$note; }JUnit failure or missing or unreadable report"
  fi
  if ((REPLAY)); then
    sim_export_recording "$dest" "$n" || true
    shopt -s nullglob; nfrs=("$dest"/*.nfr); shopt -u nullglob
    if ((${#nfrs[@]} == 0)); then
      status=FAIL; note="${note:+$note; }no session recording for $n"
    else
      for nfr in "${nfrs[@]}"; do
        if ! sim_replay "$nfr" "${nfr%.nfr}.replay.log"; then
          status=FAIL; note="${note:+$note; }replay check failed ($(basename "$nfr"))"
        fi
      done
    fi
  fi
  shopt -s nullglob; nfrs=("$dest"/*.nfr); shopt -u nullglob
  for nfr in "${nfrs[@]}"; do
    if ! python3 "$HERE/sim-trace.py" "$nfr" "$dest/$n.xml"; then
      status=FAIL; note="${note:+$note; }trace generation failed"
    fi
  done
  if [[ "$status" != PASS ]]; then
    FAILED=$((FAILED + 1))
    python3 "$HERE/sim-junit.py" failure "$dest/$n.xml" "$n" "$note"
  fi
  REPORTS+=("$dest/$n.xml")
  echo "$status $n (${SECS[$n]}s, group=$GROUP)${note:+ - $note}"
done
# Finalize group JUnit only after process, per-session replay and trace verdicts are attached.
python3 "$HERE/sim-junit.py" merge "$ARTIFACTS/group.xml" "$GROUP" "${REPORTS[@]}"
((timed_out)) && echo "sim: group timeout ${SIM_TIMEOUT}s hit"
echo "sim: group $GROUP, ${#SCENARIOS[@]} process(es), $FAILED failure(s); merged report $ARTIFACTS/group.xml"
((FAILED == 0))
