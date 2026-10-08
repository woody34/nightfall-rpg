#!/usr/bin/env bash
# Dev-only: runs the full device authorization grant against local Keycloak and
# automates the browser step by posting the login form with curl.
# Needs: curl, jq, openssl. See README.md for the manual, step-by-step version.
set -euo pipefail
R=${OIDC_ISSUER:-http://localhost:8080/realms/nightfall}
USER_NAME=${1:-testplayer}; PASS=${2:-testplayer}
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT; cd "$W"

# PKCE S256 (required by the client)
VERIFIER=$(openssl rand -base64 48 | tr -d '=+/\n' | head -c 64)
CHALLENGE=$(printf %s "$VERIFIER" | openssl dgst -sha256 -binary | openssl base64 -A | tr '+/' '-_' | tr -d '=')

# 1. Device authorization request
DEV=$(curl -sf -X POST "$R/protocol/openid-connect/auth/device" \
  -d client_id=nightfall-client -d "scope=${SCOPE:-openid}" \
  -d code_challenge="$CHALLENGE" -d code_challenge_method=S256)
echo "$DEV" | jq . >&2
DC=$(jq -r .device_code <<<"$DEV"); URI=$(jq -r .verification_uri_complete <<<"$DEV")

# 2. "Browser" step: open the verification URL, post credentials to the login form
curl -s -c jar -b jar -L "$URI" -o login.html
ACTION=$(grep -o 'action="[^"]*"' login.html | head -1 | sed 's/^action="//;s/"$//;s/&amp;/\&/g')
curl -s -c jar -b jar -L "$ACTION" --data-urlencode "username=$USER_NAME" \
  --data-urlencode "password=$PASS" -o after.html
# Keycloak may show a grant/consent page; accept it if so.
if grep -q 'name="accept"' after.html; then
  ACTION=$(grep -o 'action="[^"]*"' after.html | head -1 | sed 's/^action="//;s/"$//;s/&amp;/\&/g')
  case "$ACTION" in /*) ACTION="${R%%/realms/*}$ACTION";; esac
  CODE=$(grep -o 'name="code" value="[^"]*"' after.html | sed 's/.*value="//;s/"$//' | head -1)
  curl -s -c jar -b jar -L "$ACTION" -d code="$CODE" -d accept=Yes -o after.html
fi

# 3. Poll the token endpoint (what the game client does)
for _ in $(seq 10); do
  TOK=$(curl -s -X POST "$R/protocol/openid-connect/token" \
    -d grant_type=urn:ietf:params:oauth:grant-type:device_code \
    -d client_id=nightfall-client -d device_code="$DC" -d code_verifier="$VERIFIER")
  [ "$(jq -r .error <<<"$TOK")" != authorization_pending ] && break
  sleep 5
done
[ "$(jq -r .access_token <<<"$TOK")" != null ] || { echo "$TOK" >&2; exit 1; }
echo "$TOK" | jq '{token_type, expires_in, refresh_expires_in, scope, has_refresh: (.refresh_token != null)}' >&2
# 4. Decode the access token payload
jq -r '.access_token | split(".")[1] | gsub("-";"+") | gsub("_";"/") | @base64d | fromjson' <<<"$TOK"
