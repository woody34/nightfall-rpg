Test-only RSA keys for JWT verification tests (generated with `openssl genrsa -traditional 2048`).
They sign nothing outside the test suite and must never be trusted by a running server.
`jwks-a.json` publishes key A (plus an encryption key that must be ignored, as Keycloak's
JWKS has one); `jwks-ab.json` adds key B, simulating a key rotation.
