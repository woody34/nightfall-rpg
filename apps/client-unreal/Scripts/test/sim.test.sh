#!/usr/bin/env bash
# Plain-bash tests for run-sim.sh / run-sim-multi.sh against fake-bot.sh. No UE, API or Docker needed.
# Run: bash Scripts/test/sim.test.sh
set -uo pipefail
T="$(cd "$(dirname "$0")" && pwd)"
SCRIPTS="$(cd "$T/.." && pwd)"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
chmod +x "$T/fake-bot.sh" "$T/fake-replay.sh"

# Fake only the health command; no live API, sockets or Docker.
mkdir -p "$WORK/bin"
cat > "$WORK/bin/curl" <<'CURL'
#!/usr/bin/env bash
[[ "$*" != *localhost:1/* ]]
CURL
chmod +x "$WORK/bin/curl"
export PATH="$WORK/bin:$PATH"
PORT=3000

export SIM_BOT_BIN="$T/fake-bot.sh" SIM_REPLAY_CMD="$T/fake-replay.sh" SIM_API_URL="http://localhost:$PORT" SIM_SKIP_BUILD=1
export SIM_COVERAGE_CMD="bash $T/fake-replay.sh"
export SIM_TRACE_CMD="bash $T/fake-trace.sh"
export SIM_SAVED_DIR="$WORK/saved"

mk() { local f="$WORK/sc/$1.nfs"; mkdir -p "$WORK/sc"; shift; printf '# scenario\n%s\n' "$*" >"$f"; }
mk ok-a "fake result pass"
mk ok-b "fake result pass"
mk bad "fake result fail"
mk crash "fake result crash"
mk noxml "fake result noreport"
mk norec "fake result norecording"
mk div "fake result diverge"
mk hang "fake sleep 30"
mk grp "fake needgroup"
mk greenexit "fake result greenexit"
mk redreport "fake result redreport"

FAIL=0
check() { # name expected-exit actual-exit
  if [[ "$2" == "$3" ]]; then echo "ok   - $1"; else echo "FAIL - $1 (expected exit $2, got $3)"; FAIL=1; fi
}
contains() { # name file pattern
  if grep -q -- "$3" "$2"; then echo "ok   - $1"; else echo "FAIL - $1 (no '$3' in $2)"; FAIL=1; fi
}
failed_xml() {
  if python3 -c 'import sys,xml.etree.ElementTree as E; assert int(E.parse(sys.argv[1]).getroot().get("failures",0))>0' "$2"; then
    echo "ok   - $1"
  else echo "FAIL - $1 (JUnit did not record failure)"; FAIL=1; fi
}

run() { rm -rf "$WORK/art"; "$SCRIPTS/run-sim.sh" --artifacts "$WORK/art" "$@" >"$WORK/out" 2>&1; echo $?; }

rm -rf "$WORK/art" "$WORK/saved"
check "single passing scenario" 0 "$(run "$WORK/sc/ok-a.nfs")"
[[ -f "$WORK/art/ok-a/ok-a.xml" && -f "$WORK/art/ok-a/ok-a.log" && -f "$WORK/art/ok-a/ok-a.nfr" ]] \
  && echo "ok   - artifacts copied" || { echo "FAIL - artifacts copied"; FAIL=1; }
contains "one-line summary" "$WORK/out" "^PASS ok-a "

check "glob runs all, one failure fails the run" 1 "$(run "$WORK/sc/ok-*.nfs" "$WORK/sc/bad.nfs")"
contains "trace artifact reference" "$WORK/art/bad/bad.xml" "bad.trace.html"
contains "pass line for ok-b" "$WORK/out" "^PASS ok-b "
contains "fail line for bad" "$WORK/out" "^FAIL bad .*bot exit 1"
check "trace failure fails scenario" 1 "$(SIM_TRACE_CMD=false run "$WORK/sc/ok-a.nfs")"
contains "trace failure reported" "$WORK/out" "trace generation failed"
check "trace still generated with --no-replay" 0 "$(run --no-replay "$WORK/sc/ok-a.nfs")"
contains "trace is self-contained HTML" "$WORK/art/ok-a/ok-a.trace.html" "doctype html"
check "quoted glob" 0 "$(run "$WORK/sc/ok-*.nfs")"
check "crash fails" 1 "$(run "$WORK/sc/crash.nfs")"
contains "crash exit code reported" "$WORK/out" "bot exit 139"
failed_xml "crash synthesized failed testcase" "$WORK/art/crash/crash.xml"
check "failed report cannot pass with exit zero" 1 "$(run "$WORK/sc/redreport.nfs")"
check "green report cannot pass with exit one" 1 "$(run "$WORK/sc/greenexit.nfs")"
failed_xml "green report process failure added" "$WORK/art/greenexit/greenexit.xml"
check "missing report fails" 1 "$(run "$WORK/sc/noxml.nfs")"
contains "missing report named" "$WORK/out" "no JUnit report"
check "missing recording fails" 1 "$(run "$WORK/sc/norec.nfs")"
check "missing recording ok with --no-replay" 0 "$(run --no-replay "$WORK/sc/norec.nfs")"
check "replay divergence fails" 1 "$(run "$WORK/sc/div.nfs")"
contains "divergence reported" "$WORK/out" "replay=DIVERGED"
check "timeout fails" 1 "$(SIM_TIMEOUT=1 run "$WORK/sc/hang.nfs")"
contains "timeout reported" "$WORK/out" "timeout after 1s"
check "no match is a usage error" 2 "$(run "$WORK/sc/nothing-*.nfs")"
check "attach with no API is an error" 2 "$(SIM_API_URL=http://localhost:1 run "$WORK/sc/ok-a.nfs")"

multi() { rm -rf "$WORK/mart"; "$SCRIPTS/run-sim-multi.sh" --artifacts "$WORK/mart" --group g1 "$@" >"$WORK/mout" 2>&1; echo $?; }
rm -rf "$WORK/mart"
check "multi: two passing" 0 "$(multi "$WORK/sc/ok-a.nfs" "$WORK/sc/grp.nfs")"
contains "multi: group id reached the bot" "$WORK/mart/grp/grp.log" "group=g1"
contains "multi: merged JUnit has both suites" "$WORK/mart/group.xml" 'tests="2"'
check "multi: one failure fails the group" 1 "$(multi "$WORK/sc/ok-a.nfs" "$WORK/sc/bad.nfs")"
failed_xml "multi: failure counted in merged JUnit" "$WORK/mart/group.xml"
check "multi: missing report becomes failed testcase" 1 "$(multi "$WORK/sc/ok-a.nfs" "$WORK/sc/noxml.nfs")"
contains "multi: synthesized failure" "$WORK/mart/group.xml" "missing or unreadable"
start=$SECONDS
check "multi: group timeout kills the hung process" 1 "$(SIM_TIMEOUT=2 multi "$WORK/sc/ok-a.nfs" "$WORK/sc/hang.nfs")"
((SECONDS - start < 15)) && echo "ok   - multi: timeout bounded wall clock" || { echo "FAIL - multi: not bounded"; FAIL=1; }
check "multi: duplicate scenario refused" 2 "$(multi "$WORK/sc/ok-a.nfs" "$WORK/sc/ok-a.nfs")"
check "multi: needs two scenarios" 2 "$(multi "$WORK/sc/ok-a.nfs")"
check "multi: replay divergence fails" 1 "$(multi "$WORK/sc/ok-a.nfs" "$WORK/sc/div.nfs")"
failed_xml "multi: replay failure counted in merged JUnit" "$WORK/mart/group.xml"
check "multi: one missing client recording fails" 1 "$(multi "$WORK/sc/ok-a.nfs" "$WORK/sc/norec.nfs")"
failed_xml "multi: missing recording counted in merged JUnit" "$WORK/mart/group.xml"
check "multi: trace failure fails group" 1 "$(SIM_TRACE_CMD=false multi "$WORK/sc/ok-a.nfs" "$WORK/sc/ok-b.nfs")"
failed_xml "multi: trace failure counted in merged JUnit" "$WORK/mart/group.xml"
check "multi: failed report cannot pass with exit zero" 1 "$(multi "$WORK/sc/ok-a.nfs" "$WORK/sc/redreport.nfs")"
check "multi: process failure counted with green report" 1 "$(multi "$WORK/sc/ok-a.nfs" "$WORK/sc/greenexit.nfs")"
failed_xml "multi: green process report corrected" "$WORK/mart/group.xml"
# processes really run concurrently: two 3 s sleepers finish in well under 6 s
mk s1 "fake sleep 3"; mk s2 "fake sleep 3"
start=$SECONDS; multi "$WORK/sc/s1.nfs" "$WORK/sc/s2.nfs" >/dev/null
((SECONDS - start < 6)) && echo "ok   - multi: concurrent" || { echo "FAIL - multi: sequential"; FAIL=1; }

exit $FAIL
