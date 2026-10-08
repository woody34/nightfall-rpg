-- Zone epoch index (Story 3.4, plan §8 #5): one row per zone epoch holding the epoch-start
-- snapshot (the same bytes as the JetStream `nightfall.zone.<zone>.<epoch>.snapshot` message)
-- and where the epoch lives in the NF_ZONES stream. Retention: an epoch's snapshot and log are
-- kept together for at least 7 days (the stream's max_age); rows are pruned by the same
-- scheduled job as other logs (Phase 9), never before the stream has aged the epoch out.

CREATE TABLE zone_snapshots (
    zone_id                 integer     NOT NULL CHECK (zone_id >= 0),
    epoch                   bigint      NOT NULL CHECK (epoch >= 0),
    snapshot                bytea       NOT NULL,
    jetstream_snapshot_seq  bigint      NOT NULL CHECK (jetstream_snapshot_seq > 0),
    jetstream_first_seq     bigint      CHECK (jetstream_first_seq > jetstream_snapshot_seq),
    time_origin_ms          bigint      NOT NULL,
    build_id                text        NOT NULL,
    config_hash             text        NOT NULL,
    schema_version          integer     NOT NULL CHECK (schema_version > 0),
    created_at              timestamptz NOT NULL DEFAULT now(),
    updated_at              timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT zone_snapshots_pkey PRIMARY KEY (zone_id, epoch)
);

CREATE INDEX zone_snapshots_created_at_idx ON zone_snapshots (created_at);
