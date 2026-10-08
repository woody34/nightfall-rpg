-- Characters, idempotency keys, and the transactional outbox.
-- Conventions (docs/engineering/database-guidelines.md): timestamptz everywhere, uuid v7 keys,
-- smallint for bounded stats, explicit constraints, named unique constraints so the application
-- can match on them, no SELECT * in application code.

CREATE TABLE characters (
    id              uuid        PRIMARY KEY,
    account_id      uuid        NOT NULL,
    name            text        NOT NULL,
    name_normalized text        NOT NULL,
    race            text        NOT NULL CHECK (race IN ('human', 'elf', 'dark_elf', 'orc', 'dwarf')),
    level           integer     NOT NULL DEFAULT 1 CHECK (level BETWEEN 1 AND 80),
    str             smallint    NOT NULL CHECK (str BETWEEN 1 AND 99),
    dex             smallint    NOT NULL CHECK (dex BETWEEN 1 AND 99),
    con             smallint    NOT NULL CHECK (con BETWEEN 1 AND 99),
    "int"           smallint    NOT NULL CHECK ("int" BETWEEN 1 AND 99),
    wit             smallint    NOT NULL CHECK (wit BETWEEN 1 AND 99),
    men             smallint    NOT NULL CHECK (men BETWEEN 1 AND 99),
    pos_x           real        NOT NULL DEFAULT 0,
    pos_y           real        NOT NULL DEFAULT 0,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT characters_name_normalized_key UNIQUE (name_normalized),
    CONSTRAINT characters_name_len CHECK (char_length(name) BETWEEN 3 AND 16)
);

CREATE INDEX characters_account_id_idx ON characters (account_id);

-- One row per client idempotency key. Rows older than the retention window are deleted by a
-- scheduled job (Phase 9); the index on created_at makes that cheap.
CREATE TABLE idempotency_keys (
    key          uuid        PRIMARY KEY,
    fingerprint  text        NOT NULL,
    character_id uuid        NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idempotency_keys_created_at_idx ON idempotency_keys (created_at);

-- Transactional outbox: events are staged in the same transaction as the state change and
-- relayed to NATS by a background publisher. published_at IS NULL == pending.
CREATE TABLE outbox (
    id           bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    subject      text        NOT NULL,
    payload      jsonb       NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    published_at timestamptz
);

CREATE INDEX outbox_pending_idx ON outbox (id) WHERE published_at IS NULL;
