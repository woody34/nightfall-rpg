#!/usr/bin/env bash
# Runs the Nightfall automation tests headless in the editor. Exit code is non-zero on failure.
# Nightfall.Net.SessionClient.Ping and Nightfall.Login.EndToEnd talk to the real API: start it
# first (`moon run api:dev`). EndToEnd skips itself when the API is down.
#
# EndToEnd logs in with the developer bypass (-DevTokenFile). The token is, in order:
#   $NIGHTFALL_DEV_TOKEN, if set;
#   a real access token for testplayer from the local Keycloak (infra/keycloak/device-flow-demo.sh),
#     when Keycloak answers and curl, jq and openssl are installed;
#   otherwise none, and the test uses a fresh `test:<uuid>` account (API needs AUTH_DEV_TOKENS=1).
# The token goes through a 0600 temp file (deleted on exit), not the command line, which UE logs.
#
# CI mode: --require-live-api passes -RequireLiveApi, which turns a live test's "API not reachable,
# skipped" warning into a failure, so a down server can never show green. Without it they skip.
# Also on when CI=true in the environment. The suite is ProductFilter-only, so "Nightfall" selects
# just our tests.
#
# $RUN_TESTS_EXTRA_ARGS (word-split) is appended to the editor command line, e.g. an -ini: override.
#
# offline / --offline selects all local tests and does not acquire auth credentials.
# Usage: Scripts/run-tests.sh [--offline|--require-live-api] [test filter, default "Nightfall"]
set -euo pipefail
: "${UE_ROOT:?UE_ROOT must point at the Unreal install}"
HERE="$(cd "$(dirname "$0")" && pwd)"
PROJECT="$(cd "$HERE/.." && pwd)/Nightfall.uproject"
REPO="$(cd "$HERE/../../.." && pwd)"
REQUIRE_LIVE=0
OFFLINE=0
[[ "${CI:-}" == "true" ]] && REQUIRE_LIVE=1
POSITIONAL=()
for arg in "$@"; do
  case "$arg" in
    --require-live-api) REQUIRE_LIVE=1 ;;
    offline|--offline) OFFLINE=1 ;;
    -h|--help) sed -n '2,/^set -/p' "$0" | sed '$d'; exit 0 ;;
    *) POSITIONAL+=("$arg") ;;
  esac
done
if [[ "$OFFLINE" == 1 && "$REQUIRE_LIVE" == 1 ]]; then echo "offline and required-live modes conflict" >&2; exit 2; fi
DEFAULT_FILTER=Nightfall
if [[ "$OFFLINE" == 1 ]]; then
  DEFAULT_FILTER="$(python3 - "$HERE/../Source/Nightfall/Tests" <<'PYFILTER'
from pathlib import Path
import re
import sys
names = set()
for path in Path(sys.argv[1]).glob('*.cpp'):
    names.update(re.findall(r'IMPLEMENT_(?:CUSTOM_)?SIMPLE_AUTOMATION_TEST\([^;]*?"(Nightfall\.[^"]+)"', path.read_text(), re.S))
# Live API tests and the optional vendor import have separate integration gates.
names = {name for name in names if not name.endswith(('.EndToEnd', '.LiveAdmission'))
         and name not in {'Nightfall.Net.SessionClient.Ping', 'Nightfall.Content.VendorArt'}}
if not names:
    raise SystemExit('offline test registry is empty')
print('+'.join('^' + name + '$' for name in sorted(names)))
PYFILTER
)"
fi
FILTER="${POSITIONAL[0]:-$DEFAULT_FILTER}"
ISSUER="${OIDC_ISSUER:-http://localhost:8080/realms/nightfall}"

EXTRA=()
if [[ "$REQUIRE_LIVE" == 1 ]]; then
  EXTRA+=("-RequireLiveApi")
  echo "run-tests: CI mode, live tests fail when the API is unreachable" >&2
fi
TOKEN=""
[[ "$OFFLINE" == 0 ]] && TOKEN="${NIGHTFALL_DEV_TOKEN:-}"
if [[ "$OFFLINE" == 0 && -z "$TOKEN" && "${AUTH_DEV_TOKENS:-}" == 1 ]]; then
  TOKEN="test:$(python3 -c 'import uuid; print(uuid.uuid4())')"
fi
if [[ "$OFFLINE" == 0 && -z "$TOKEN" ]] && command -v jq >/dev/null && command -v openssl >/dev/null \
  && curl -sf -o /dev/null "$ISSUER/.well-known/openid-configuration"; then
  TOKEN="$(OIDC_ISSUER="$ISSUER" OUTPUT=access_token "$REPO/infra/keycloak/device-flow-demo.sh" 2>/dev/null || true)"
  [[ -n "$TOKEN" ]] && echo "run-tests: using a Keycloak access token for testplayer" >&2
fi
if [[ -n "$TOKEN" ]]; then
  TOKEN_FILE="$(umask 077 && mktemp)"
  trap 'rm -f "$TOKEN_FILE"' EXIT
  printf %s "$TOKEN" >"$TOKEN_FILE"
  EXTRA+=("-DevTokenFile=$TOKEN_FILE")
fi

# This engine build resolves Engine/Content relative to the working directory.
cd "$UE_ROOT/Engine/Binaries/Linux"
./UnrealEditor "$PROJECT" -ExecCmds="Automation RunTests $FILTER; Quit" \
  -TestExit="Automation Test Queue Empty" -unattended -nullrhi -nosplash -nosound -log -stdout ${EXTRA[@]+"${EXTRA[@]}"} ${RUN_TESTS_EXTRA_ARGS:-}
