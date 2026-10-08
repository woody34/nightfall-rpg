# Keycloak (identity provider)

Decision D1 in `docs/plans/phase-0b-connected-slice.md`: Keycloak is the OIDC provider. The
API never sees a password; it only validates JWTs (issuer `OIDC_ISSUER`, audience
`OIDC_AUDIENCE`). The Unreal client logs in with the OAuth 2.0 **device authorization grant**
(RFC 8628) because it cannot embed a browser.

Everything here is **development only**: `start-dev` mode, plain HTTP, default credentials.

## Start it

```bash
docker compose up -d keycloak      # also starts postgres; waits until healthy (~30-60 s)
```

Keycloak stores its data in a `keycloak` database in the same Postgres container. The
database is created by `infra/postgres/init-keycloak.sql`, which Postgres runs **only when
its data volume is first created**. If you already have a `pgdata` volume, either wipe it
(`docker compose down -v`, destroys all local data) or create the database once:

```bash
docker compose exec postgres psql -U nightfall -c 'CREATE DATABASE keycloak'
```

The realm is imported on boot from `realm-nightfall.json` (`--import-realm`). Import skips a
realm that already exists, so after editing the file run `docker compose down -v` (or delete
the realm in the admin console) to re-import.

## Admin console

<http://localhost:8080> — user `admin`, password `admin` (override with `KEYCLOAK_ADMIN` /
`KEYCLOAK_ADMIN_PASSWORD` in your shell or `.env`). Switch to the `nightfall` realm.

## What the realm contains

| Thing | Setting |
|-------|---------|
| Realm | `nightfall`, issuer `http://localhost:8080/realms/nightfall` |
| `nightfall-client` | public; device grant on; PKCE `S256` required; standard flow and direct grants off; audience mapper adds `aud: nightfall-api`; `offline_access` is an optional scope |
| `nightfall-api` | confidential, bearer-only (exists to be the audience); dev secret in the JSON |
| Realm roles | `player`, `gm` |
| User | `testplayer` / `testplayer` (**dev only**), role `player` |
| Lifetimes | access token 15 min; refresh 30 days; offline tokens allowed |

## How the device flow works for the game client

1. Client POSTs to the device endpoint (with a PKCE challenge) and gets a `device_code`,
   a short `user_code` and a `verification_uri`.
2. Client shows the URL and code to the player, who opens it in any browser, signs in and
   approves.
3. Meanwhile the client polls the token endpoint every `interval` seconds (5) with the
   `device_code` and PKCE verifier. It gets `authorization_pending` until the player approves,
   then an access token (15 min) and refresh token. Add the `offline_access` scope for an
   offline refresh token that survives sessions.
4. Client sends the access token to the API as `Authorization: Bearer ...` / gRPC metadata.

## curl walkthrough

```bash
R=http://localhost:8080/realms/nightfall

# PKCE S256 pair (required by the client)
VERIFIER=$(openssl rand -base64 48 | tr -d '=+/\n' | head -c 64)
CHALLENGE=$(printf %s "$VERIFIER" | openssl dgst -sha256 -binary | openssl base64 -A | tr '+/' '-_' | tr -d '=')

# 1. Device authorization request
DEV=$(curl -s -X POST $R/protocol/openid-connect/auth/device \
  -d client_id=nightfall-client -d scope=openid \
  -d code_challenge=$CHALLENGE -d code_challenge_method=S256)
echo "$DEV" | jq .
DC=$(jq -r .device_code <<<"$DEV")

# 2. Open the verification URL in a browser, sign in as testplayer / testplayer, approve.
jq -r .verification_uri_complete <<<"$DEV"

# 3. Poll (returns {"error":"authorization_pending"} until step 2 is done)
TOK=$(curl -s -X POST $R/protocol/openid-connect/token \
  -d grant_type=urn:ietf:params:oauth:grant-type:device_code \
  -d client_id=nightfall-client -d device_code=$DC -d code_verifier=$VERIFIER)

# 4. Decode the access token payload (jq's @base64d tolerates missing padding)
jq -r '.access_token | split(".")[1] | gsub("-";"+") | gsub("_";"/") | @base64d | fromjson' <<<"$TOK"
```

### Fully scripted (no browser)

[`device-flow-demo.sh`](device-flow-demo.sh) does all of the above and automates step 2 by
fetching the verification URL with a cookie jar, posting the login form, and posting the
grant page's "Yes" button. It is what was used to verify this setup.

```bash
infra/keycloak/device-flow-demo.sh                       # openid profile email
SCOPE="openid offline_access" infra/keycloak/device-flow-demo.sh   # also an offline refresh token
OUTPUT=access_token infra/keycloak/device-flow-demo.sh   # print only the raw access token
```

The API's `tests/keycloak_device_flow.rs` uses `OUTPUT=access_token` to get a real token and
calls authenticated RPCs with it; it runs when `KEYCLOAK_URL` is set
(`KEYCLOAK_URL=http://localhost:8080 moon run api:test`).

### Expected token claims (abridged)

```json
{
  "iss": "http://localhost:8080/realms/nightfall",
  "aud": ["nightfall-api", "account"],
  "sub": "fb71069a-b366-48df-9a4f-7093b3ae7da7",
  "azp": "nightfall-client",
  "realm_access": {
    "roles": ["offline_access", "default-roles-nightfall", "uma_authorization", "player"]
  },
  "scope": "openid profile email",
  "preferred_username": "testplayer",
  "email": "testplayer@example.test"
}
```

`sub` differs per import (Keycloak generates the user id). `aud` contains `nightfall-api`
(the API should check for membership, not equality: Keycloak also adds `account`). Token
response: `expires_in` 900, `refresh_expires_in` 2592000.

## Changing the realm

Edit `realm-nightfall.json`, then `docker compose down -v && docker compose up -d keycloak`.
Prefer making changes in the admin console first and exporting, then trimming the export.
