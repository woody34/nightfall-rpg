-- D76: operation_ordinal is the canonical row position 0..n-1 of the owner's rows for one
-- instance in one tick (D69 order), so the Consume aggregate holds the highest ordinal, not 0.
ALTER TABLE item_ledger DROP CONSTRAINT item_ledger_consume_ordinal;
-- Still exactly one Consume aggregate per character/instance/tick; it is that group's last row.
COMMENT ON INDEX item_ledger_consume_batch_key IS
    'One Consume aggregate per (zone, epoch, tick, character, instance); D76: it holds the group''s highest operation_ordinal.';
-- Checked at commit (or SET CONSTRAINTS ... IMMEDIATE) once the whole tick's rows are appended:
-- per (zone, epoch, tick, character, instance) the ordinals are unique and exactly 0..n-1, and a
-- Consume row holds n-1. It fires on INSERT only, so append-only pre-D76 history is never
-- re-validated or rewritten.
CREATE FUNCTION item_ledger_row_ordinal() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    total bigint;
    distinct_ordinals bigint;
    lowest integer;
    highest integer;
    consume_ordinal integer;
BEGIN
    SELECT count(*), count(DISTINCT operation_ordinal), min(operation_ordinal), max(operation_ordinal),
           max(operation_ordinal) FILTER (WHERE op = 'consume')
      INTO total, distinct_ordinals, lowest, highest, consume_ordinal
      FROM item_ledger
     WHERE zone_id = NEW.zone_id AND epoch = NEW.epoch AND tick = NEW.tick
       AND character_id = NEW.character_id AND instance_id = NEW.instance_id;
    IF distinct_ordinals <> total OR lowest <> 0 OR highest <> total - 1
       OR (consume_ordinal IS NOT NULL AND consume_ordinal <> highest) THEN
        RAISE EXCEPTION 'item ledger ordinals must be the canonical row positions 0..n-1 per instance and tick'
            USING ERRCODE = '23514', CONSTRAINT = 'item_ledger_row_ordinal';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER item_ledger_row_ordinal AFTER INSERT ON item_ledger
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION item_ledger_row_ordinal();
