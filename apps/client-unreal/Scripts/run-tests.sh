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
# Usage: Scripts/run-tests.sh [test filter, default "Nightfall"]
set -euo pipefail
: "${UE_ROOT:?UE_ROOT must point at the Unreal install}"
HERE="$(cd "$(dirname "$0")" && pwd)"
PROJECT="$(cd "$HERE/.." && pwd)/Nightfall.uproject"
REPO="$(cd "$HERE/../../.." && pwd)"
FILTER="${1:-Nightfall}"
ISSUER="${OIDC_ISSUER:-http://localhost:8080/realms/nightfall}"

EXTRA=()
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
  -TestExit="Automation Test Queue Empty" -unattended -nullrhi -nosplash -nosound -log -stdout "${EXTRA[@]}"
