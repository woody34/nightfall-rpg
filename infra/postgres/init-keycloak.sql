-- Runs once, on first initialisation of an empty Postgres volume.
-- Existing dev volumes need: docker compose exec postgres psql -U nightfall -c 'CREATE DATABASE keycloak'
-- (or `docker compose down -v` to start clean).
SELECT 'CREATE DATABASE keycloak'
WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'keycloak')\gexec
