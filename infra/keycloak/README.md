# Keycloak (identity provider)

Decision D1 in `docs/plans/phase-0b-connected-slice.md` sets Keycloak as the OIDC provider. The
API never sees a password. The API only validates JWTs (issuer `OIDC_ISSUER`, audience
`OIDC_AUDIENCE`). The Unreal client logs in with the OAuth 2.0 **device authorization grant**
(RFC 8628), because the client cannot embed a browser.

All of this setup is **development only**: `start-dev` mode, plain HTTP, and default credentials.

## Start it

```bash
docker compose up -d keycloak      # also starts postgres; waits until healthy (~30-60 s)
```

Keycloak keeps its data in a `keycloak` database in the same Postgres container.
`infra/postgres/init-keycloak.sql` makes this database. Postgres runs this script **only when
it first makes its data volume**. If you have a `pgdata` volume, do one of these steps:

- Remove the volume with `docker compose down -v`. This command removes all local data.
- Make the database one time:

```bash
docker compose exec postgres psql -U nightfall -c 'CREATE DATABASE keycloak'
```

Keycloak imports the realm from `realm-nightfall.json` when it starts (`--import-realm`). The import ignores a
realm that exists. Thus, after you change the file, run `docker compose down -v` to import the realm again.
As an alternative, remove the realm in the admin console.

## Admin console

<http://localhost:8080>: the user is `admin` and the password is `admin`. To change these values, set `KEYCLOAK_ADMIN` /
`KEYCLOAK_ADMIN_PASSWORD` in your shell or in `.env`. Then change to the `nightfall` realm.

## What the realm contains

| Thing | Setting |
|-------|---------|
| Realm | `nightfall`, issuer `http://localhost:8080/realms/nightfall` |
| `nightfall-client` | Public. Device grant on. PKCE `S256` is necessary. Standard flow and direct grants off. An audience mapper adds `aud: nightfall-api`. `offline_access` is an optional scope. |
| `nightfall-api` | Confidential, bearer-only (it exists only to be the audience). The dev secret is in the JSON. |
| Realm roles | `player`, `gm` |
| User | `testplayer` / `testplayer` (**dev only**), role `player` |
| Lifetimes | Access token 15 min. Refresh 30 days. You can use offline tokens. |

## How the device flow works for the game client

1. The client POSTs to the device endpoint (with a PKCE challenge). The client gets a `device_code`,
   a short `user_code` and a `verification_uri`.
2. The client shows the URL and the code to the player. The player opens the URL in a browser, signs in and
   approves.
3. At the same time, the client polls the token endpoint each `interval` seconds (5) with the
   `device_code` and the PKCE verifier. The client gets `authorization_pending` until the player approves.
   Then the client gets an access token (15 min) and a refresh token. For an offline refresh token that
   stays valid after a session stops, add the `offline_access` scope.
4. The client sends the access token to the API as `Authorization: Bearer ...` / gRPC metadata.

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

[`device-flow-demo.sh`](device-flow-demo.sh) does all of the steps above. It also does step 2 automatically:

- It gets the verification URL with a cookie jar.
- It posts the login form.
- It posts the "Yes" button of the grant page.

We used this script to make sure that this setup operates correctly.

```bash
infra/keycloak/device-flow-demo.sh                       # openid profile email
SCOPE="openid offline_access" infra/keycloak/device-flow-demo.sh   # also an offline refresh token
OUTPUT=access_token infra/keycloak/device-flow-demo.sh   # print only the raw access token
```

The API test `tests/keycloak_device_flow.rs` uses `OUTPUT=access_token` to get a real token. Then the test
calls authenticated RPCs with this token. The test runs when `KEYCLOAK_URL` is set
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

`sub` is different for each import, because Keycloak makes the user id. `aud` includes `nightfall-api`.
The API must examine for membership, not for equality, because Keycloak also adds `account`. Token
response: `expires_in` 900, `refresh_expires_in` 2592000.

## Changing the realm

1. Change `realm-nightfall.json`.
2. Run `docker compose down -v && docker compose up -d keycloak`.

It is better to do the changes in the admin console first. Then export the realm and remove the unnecessary parts of the export.
