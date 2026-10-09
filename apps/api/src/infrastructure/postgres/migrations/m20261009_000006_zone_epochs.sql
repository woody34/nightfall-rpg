-- Epoch discovery must outlive JetStream retention. Never prune unresolved rows.
CREATE TABLE zone_epochs (
    zone_id integer NOT NULL CHECK (zone_id >= 0),
    epoch bigint NOT NULL CHECK (epoch >= 0),
    started_at timestamptz NOT NULL DEFAULT now(),
    first_seq bigint CHECK (first_seq > 0),
    last_checkpointed_tick bigint CHECK (last_checkpointed_tick >= 0),
    -- Written BEFORE attempting log admission; conservative upper bound after a crash.
    last_recorded_tick bigint CHECK (last_recorded_tick >= 0),
    closed_at timestamptz,
    PRIMARY KEY (zone_id, epoch)
);
CREATE INDEX zone_epochs_unclosed_idx ON zone_epochs (zone_id, epoch) WHERE closed_at IS NULL;
INSERT INTO zone_epochs (zone_id, epoch, started_at, first_seq)
SELECT zone_id, epoch, created_at, jetstream_first_seq FROM zone_snapshots;
-- Recovery baselines move forward; the first applied sequence remains epoch-wide.
ALTER TABLE zone_snapshots DROP CONSTRAINT zone_snapshots_check;
ALTER TABLE zone_snapshots ADD CONSTRAINT zone_snapshots_first_seq_positive CHECK (jetstream_first_seq > 0);
