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
# Usage: Scripts/run-tests.sh [--require-live-api] [test filter, default "Nightfall"]
set -euo pipefail
: "${UE_ROOT:?UE_ROOT must point at the Unreal install}"
HERE="$(cd "$(dirname "$0")" && pwd)"
PROJECT="$(cd "$HERE/.." && pwd)/Nightfall.uproject"
REPO="$(cd "$HERE/../../.." && pwd)"
REQUIRE_LIVE=0
[[ "${CI:-}" == "true" ]] && REQUIRE_LIVE=1
POSITIONAL=()
for arg in "$@"; do
  case "$arg" in
    --require-live-api) REQUIRE_LIVE=1 ;;
    -h|--help) sed -n '2,/^set -/p' "$0" | sed '$d'; exit 0 ;;
    *) POSITIONAL+=("$arg") ;;
  esac
done
FILTER="${POSITIONAL[0]:-Nightfall}"
ISSUER="${OIDC_ISSUER:-http://localhost:8080/realms/nightfall}"

EXTRA=()
if [[ "$REQUIRE_LIVE" == 1 ]]; then
  EXTRA+=("-RequireLiveApi")
  echo "run-tests: CI mode, live tests fail when the API is unreachable" >&2
fi
TOKEN="${NIGHTFALL_DEV_TOKEN:-}"
if [[ -z "$TOKEN" ]] && command -v jq >/dev/null && command -v openssl >/dev/null \
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
