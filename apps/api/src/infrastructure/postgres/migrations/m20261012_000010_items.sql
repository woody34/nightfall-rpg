-- Phase 4 projection; IDs use durable zone ranges (design I1), not UUIDs/sequences.
ALTER TABLE characters ADD COLUMN phase4_checkpoint jsonb;
CREATE TABLE item_instances (
    id bigint PRIMARY KEY CONSTRAINT item_instances_id_positive CHECK (id > 0),
    character_id uuid NOT NULL REFERENCES characters(id),
    location text NOT NULL CONSTRAINT item_instances_location CHECK (location IN ('inventory','paperdoll')),
    slot smallint,
    template_id integer NOT NULL CONSTRAINT item_instances_template_positive CHECK (template_id > 0),
    count bigint NOT NULL CONSTRAINT item_instances_count_positive CHECK (count > 0),
    enchant bigint NOT NULL DEFAULT 0 CONSTRAINT item_instances_enchant_u32 CHECK (enchant BETWEEN 0 AND 4294967295),
    augment_id bigint CONSTRAINT item_instances_augment_u32 CHECK (augment_id BETWEEN 0 AND 4294967295),
    charge_bits smallint NOT NULL DEFAULT 0 CONSTRAINT item_instances_charge_bits CHECK (charge_bits BETWEEN 0 AND 7),
    bound boolean NOT NULL DEFAULT false CONSTRAINT item_instances_unbound CHECK (NOT bound),
    created_tick bigint NOT NULL CONSTRAINT item_instances_tick CHECK (created_tick >= 0),
    revision bigint NOT NULL CONSTRAINT item_instances_revision CHECK (revision >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT item_instances_slot CHECK ((location = 'paperdoll' AND slot BETWEEN 0 AND 24 AND slot IS NOT NULL) OR (location = 'inventory' AND slot IS NULL))
);
CREATE INDEX item_instances_character_idx ON item_instances (character_id,id);
CREATE UNIQUE INDEX item_instances_paperdoll_slot_key ON item_instances (character_id,slot) WHERE location = 'paperdoll';

-- A bigint range allocator row is globally serialized, including concurrent epochs.
CREATE TABLE item_id_ranges (
    zone_id integer NOT NULL CONSTRAINT item_id_ranges_zone CHECK (zone_id >= 0),
    epoch bigint NOT NULL CONSTRAINT item_id_ranges_epoch CHECK (epoch >= 0),
    range_start bigint NOT NULL,
    range_end_exclusive bigint NOT NULL,
    next_reserved bigint NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (zone_id,epoch,range_start),
    CONSTRAINT item_id_ranges_bounds CHECK (range_start >= 4294967296 AND range_start <= next_reserved AND next_reserved <= range_end_exclusive AND range_start < range_end_exclusive),
    -- Exclusive end is itself a bigint, so values reaching 2^63 cannot be stored.
    EXCLUDE USING gist (int8range(range_start,range_end_exclusive,'[)') WITH &&)
);

CREATE TABLE pending_item_requests (
    request_id uuid PRIMARY KEY,
    character_id uuid NOT NULL REFERENCES characters(id),
    kind text NOT NULL CONSTRAINT pending_item_requests_kind CHECK (kind IN ('starter','backfill','grant')),
    request_fingerprint text NOT NULL,
    manifest_version integer NOT NULL CONSTRAINT pending_item_requests_version CHECK (manifest_version > 0),
    manifest_hash text NOT NULL CONSTRAINT pending_item_requests_hash CHECK (manifest_hash ~ '^[0-9a-f]{64}$'),
    payload jsonb NOT NULL,
    status text NOT NULL DEFAULT 'pending' CONSTRAINT pending_item_requests_status CHECK (status IN ('pending','materialized')),
    materialized_revision bigint,
    receipt_ref uuid,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pending_item_requests_completion CHECK ((status = 'pending' AND materialized_revision IS NULL AND receipt_ref IS NULL) OR (status = 'materialized' AND materialized_revision >= 0 AND materialized_revision IS NOT NULL AND receipt_ref IS NOT NULL))
);
CREATE INDEX pending_item_requests_character_idx ON pending_item_requests (character_id,request_id);
CREATE UNIQUE INDEX pending_item_requests_kit_key ON pending_item_requests (character_id) WHERE kind IN ('starter','backfill');
CREATE INDEX pending_item_requests_pending_idx ON pending_item_requests (character_id,request_id) WHERE status = 'pending';

-- Frozen typed successes survive later inventory mutations and disconnect.
CREATE TABLE item_mutation_receipts (
    account_id uuid NOT NULL REFERENCES accounts(id),
    operation text NOT NULL CONSTRAINT item_mutation_receipts_operation CHECK (operation IN ('equip_item','unequip_item','use_item','destroy_item','enchant_item','toggle_auto_shot','refine_item','cancel_refine','exchange_special_ability','crystallize_item','grant_item','starter','backfill')),
    key uuid NOT NULL,
    character_id uuid NOT NULL REFERENCES characters(id),
    fingerprint text NOT NULL,
    response jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id,operation,key),
    FOREIGN KEY (account_id,operation,key) REFERENCES idempotency_keys(account_id,operation,key)
);
CREATE INDEX item_mutation_receipts_character_idx ON item_mutation_receipts (character_id);
CREATE TABLE item_ledger (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    character_id uuid NOT NULL REFERENCES characters(id),
    zone_id integer NOT NULL CONSTRAINT item_ledger_zone CHECK (zone_id >= 0),
    epoch bigint NOT NULL CONSTRAINT item_ledger_epoch CHECK (epoch >= 0),
    tick bigint NOT NULL CONSTRAINT item_ledger_tick CHECK (tick >= 0),
    revision bigint NOT NULL CONSTRAINT item_ledger_revision CHECK (revision >= 0),
    op text NOT NULL CONSTRAINT item_ledger_op CHECK (op IN ('grant','starter','backfill','equip','unequip','consume','enchant','destroy','crystallize','refine','cancel_refine','exchange','use','toggle_auto_shot')),
    instance_id bigint NOT NULL CONSTRAINT item_ledger_instance CHECK (instance_id > 0),
    operation_ordinal integer NOT NULL CONSTRAINT item_ledger_ordinal CHECK (operation_ordinal >= 0),
    CONSTRAINT item_ledger_consume_ordinal CHECK (op <> 'consume' OR operation_ordinal = 0),
    delta jsonb NOT NULL,
    receipt_ref uuid,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT item_ledger_logical_key UNIQUE (zone_id,epoch,tick,character_id,instance_id,op,operation_ordinal)
);
CREATE INDEX item_ledger_character_idx ON item_ledger (character_id,id);
CREATE INDEX item_ledger_instance_idx ON item_ledger (instance_id,id);
-- A consume batch has ordinal 0 and exactly one row per character/instance/tick.
CREATE UNIQUE INDEX item_ledger_consume_batch_key ON item_ledger (zone_id,epoch,tick,character_id,instance_id) WHERE op = 'consume';

-- Ledger history survives item deletion and cannot be rewritten by an adapter.
CREATE FUNCTION forbid_item_ledger_rewrite() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'item ledger is append-only' USING ERRCODE = '23514', CONSTRAINT = 'item_ledger_append_only';
END $$;
CREATE TRIGGER item_ledger_append_only BEFORE UPDATE OR DELETE ON item_ledger
FOR EACH ROW EXECUTE FUNCTION forbid_item_ledger_rewrite();
