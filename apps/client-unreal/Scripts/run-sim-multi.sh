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
REPLAY_NOTE=""
for scenario in "${SCENARIOS[@]}"; do
  n="$(basename "$scenario" .nfs)"
  c="${CODE[$n]}"
  status=PASS; note=""
  if ((c == 124)); then status=FAIL; note="timeout after ${SIM_TIMEOUT}s"
  elif ((c != 0)); then status=FAIL; note="bot exit $c"; fi
  [[ -f "$ARTIFACTS/$n/$n.xml" ]] || { status=FAIL; note="${note:+$note; }no JUnit report"; }
  [[ "$status" == PASS ]] || FAILED=$((FAILED + 1))
  echo "$status $n (${SECS[$n]}s, group=$GROUP)${note:+ - $note}"
done

# One merged JUnit file; absent or unparseable reports become failed testcases.
python3 - "$ARTIFACTS" "$GROUP" "${SCENARIOS[@]}" <<'PY'
import os, sys, xml.etree.ElementTree as ET
art, group, *scen = sys.argv[1:]
root = ET.Element("testsuites", name=group)
tot = {"tests": 0, "failures": 0, "errors": 0}
for s in scen:
    n = os.path.basename(s)[:-4]
    path = os.path.join(art, n, n + ".xml")
    suites = []
    try:
        r = ET.parse(path).getroot()
        suites = [r] if r.tag == "testsuite" else list(r.iter("testsuite"))
        if not suites: raise ValueError("no testsuite element")
    except Exception as e:
        ts = ET.Element("testsuite", name=n, tests="1", failures="1", errors="0")
        tc = ET.SubElement(ts, "testcase", classname=n, name="report")
        ET.SubElement(tc, "failure", message="missing or unreadable JUnit report: %s" % e)
        suites = [ts]
    for ts in suites:
        ts.set("group", group)
        root.append(ts)
        for k in tot: tot[k] += int(ts.get(k, "0") or 0)
for k, v in tot.items(): root.set(k, str(v))
ET.ElementTree(root).write(os.path.join(art, "group.xml"), encoding="utf-8", xml_declaration=True)
PY

if ((REPLAY)); then
  shopt -s nullglob
  nfrs=("$ARTIFACTS"/*.nfr "$ARTIFACTS"/*/*.nfr)
  shopt -u nullglob
  if ((${#nfrs[@]} == 0)); then
    echo "FAIL replay: no session recordings"; FAILED=$((FAILED + 1))
  else
    for nfr in "${nfrs[@]}"; do
      if sim_replay "$nfr" "${nfr%.nfr}.replay.log"; then echo "PASS replay $(basename "$nfr")"
      else echo "FAIL replay $(basename "$nfr") diverged"; FAILED=$((FAILED + 1)); fi
    done
  fi
fi

((timed_out)) && echo "sim: group timeout ${SIM_TIMEOUT}s hit"
echo "sim: group $GROUP, ${#SCENARIOS[@]} process(es), $FAILED failure(s); merged report $ARTIFACTS/group.xml"
((FAILED == 0))
