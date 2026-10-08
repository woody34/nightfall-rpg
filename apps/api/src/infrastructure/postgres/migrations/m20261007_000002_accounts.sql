-- Story 1.3: one row per identity-provider subject, created on first authenticated request.
-- id is the IdP `sub` (Keycloak user id), so it is not generated here.
CREATE TABLE accounts (
    id            uuid        PRIMARY KEY,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_login_at timestamptz NOT NULL DEFAULT now()
);
