-- Story 1.4: play tickets, the per-account session generation, and idempotency generalised to
-- every mutating operation (plan Revision 1, items 7, 8, 9).

-- Single-use admission tickets for the /ws upgrade. Only the SHA-256 of the ticket is stored,
-- so a database read does not yield a usable ticket. `generation` is the account's session
-- generation at issue time; consuming a ticket whose generation is older than the account's
-- current one is refused (a newer ticket superseded it).
CREATE TABLE play_tickets (
    ticket_hash  bytea       PRIMARY KEY CHECK (octet_length(ticket_hash) = 32),
    account_id   uuid        NOT NULL,
    character_id uuid        NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
    generation   integer     NOT NULL CHECK (generation > 0),
    created_at   timestamptz NOT NULL DEFAULT now(),
    expires_at   timestamptz NOT NULL,
    consumed_at  timestamptz
);

CREATE INDEX play_tickets_character_id_idx ON play_tickets (character_id);
-- Cleanup job (Phase 9) deletes expired tickets.
CREATE INDEX play_tickets_expires_at_idx ON play_tickets (expires_at);

-- One row per account that has ever been issued a ticket. Every issue bumps `generation`,
-- which fences older tickets and (Epic 4) older sockets of the same account.
CREATE TABLE account_sessions (
    account_id uuid        PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    generation integer     NOT NULL CHECK (generation > 0),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- Idempotency keys become generic: scoped to the calling account and the operation, with the
-- stored response. Existing rows all belong to CreateCharacter.
--
-- There is no deployed database yet, so the expand and contract steps of this change ship in
-- one migration (database-guidelines.md section 2); a live system would split them.
ALTER TABLE idempotency_keys
    ADD COLUMN account_id uuid,
    ADD COLUMN operation  text,
    ADD COLUMN response   jsonb;

UPDATE idempotency_keys k
   SET account_id = c.account_id,
       operation  = 'create_character',
       response   = jsonb_build_object('character_id', k.character_id)
  FROM characters c
 WHERE c.id = k.character_id;

-- A key whose character is gone cannot be replayed anyway.
DELETE FROM idempotency_keys WHERE account_id IS NULL;

ALTER TABLE idempotency_keys
    ALTER COLUMN account_id SET NOT NULL,
    ALTER COLUMN operation  SET NOT NULL,
    ALTER COLUMN response   SET NOT NULL,
    DROP CONSTRAINT idempotency_keys_pkey,
    ADD CONSTRAINT idempotency_keys_pkey PRIMARY KEY (account_id, operation, key),
    ADD CONSTRAINT idempotency_keys_operation_format CHECK (operation ~ '^[a-z][a-z_]*$'),
    DROP COLUMN character_id;
