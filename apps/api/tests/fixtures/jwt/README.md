Test-only RSA keys for the JWT verification tests (made with `openssl genrsa -traditional 2048`).
These keys sign nothing outside the test suite. A server that runs must never trust these keys.
`jwks-a.json` publishes key A. It also has an encryption key that the tests must ignore, because the Keycloak
JWKS has one. `jwks-ab.json` adds key B, to simulate a key rotation.
