-- Phase 5 (design §11.4 row 5, §10.0, §10.4, B8): the container leases (the only lock; keys compared
-- bytewise in COLLATE "C"; owner_id is the 39-digit zero-padded decimal of E42), pending deliveries
-- (one queue per target, delivered within one checkpoint cadence of reaching the head) and the
-- asynchronous economy counters with their fact-id dedupe.
CREATE TABLE economy_leases (
    owner_kind text COLLATE "C" NOT NULL CONSTRAINT economy_leases_owner_kind
        CHECK (owner_kind IN ('warehouse','freight','mailbox','mail','delivery','auction','manor')),
    owner_id text COLLATE "C" NOT NULL CONSTRAINT economy_leases_owner_id CHECK (owner_id ~ '^[0-9]{39}$'),
    zone_id integer NOT NULL CONSTRAINT economy_leases_zone CHECK (zone_id >= 0),
    epoch bigint NOT NULL CONSTRAINT economy_leases_epoch CHECK (epoch >= 0),
    account_id uuid REFERENCES accounts(id),
    operation text NOT NULL CONSTRAINT economy_leases_operation CHECK (operation <> ''),
    request_key uuid NOT NULL,
    acquired_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (owner_kind,owner_id)
);
CREATE INDEX economy_leases_zone_epoch_idx ON economy_leases (zone_id,epoch);
CREATE TABLE economy_deliveries (
    id uuid PRIMARY KEY,
    target_character uuid NOT NULL REFERENCES characters(id),
    kind text NOT NULL CONSTRAINT economy_deliveries_kind CHECK (kind IN ('cod_payment','mail_expiry_return','auction_award')),
    payload jsonb NOT NULL,
    status text NOT NULL DEFAULT 'pending' CONSTRAINT economy_deliveries_status CHECK (status IN ('pending','delivered')),
    delivered_zone integer CONSTRAINT economy_deliveries_zone CHECK (delivered_zone IS NULL OR delivered_zone >= 0),
    delivered_epoch bigint CONSTRAINT economy_deliveries_epoch CHECK (delivered_epoch IS NULL OR delivered_epoch >= 0),
    delivered_tick bigint CONSTRAINT economy_deliveries_tick CHECK (delivered_tick IS NULL OR delivered_tick >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    revision bigint NOT NULL DEFAULT 0 CONSTRAINT economy_deliveries_revision CHECK (revision >= 0),
    CONSTRAINT economy_deliveries_completion CHECK (
        (status = 'pending' AND delivered_zone IS NULL AND delivered_epoch IS NULL AND delivered_tick IS NULL)
        OR (status = 'delivered' AND delivered_zone IS NOT NULL AND delivered_epoch IS NOT NULL AND delivered_tick IS NOT NULL))
);
CREATE INDEX economy_deliveries_target_pending_idx ON economy_deliveries (target_character,id) WHERE status = 'pending';
-- Re-homed attachments reference their delivery; the history outlives nothing here (RESTRICT).
ALTER TABLE item_instances ADD CONSTRAINT item_instances_delivery_fk FOREIGN KEY (delivery_id) REFERENCES economy_deliveries(id) ON DELETE RESTRICT;
CREATE TABLE economy_counters (
    day date NOT NULL,
    counter text NOT NULL CONSTRAINT economy_counters_counter CHECK (counter <> ''),
    value numeric(39,0) NOT NULL,
    PRIMARY KEY (day,counter)
);
CREATE TABLE economy_counter_facts (
    fact_id uuid PRIMARY KEY,
    processed_at timestamptz NOT NULL DEFAULT now()
);
