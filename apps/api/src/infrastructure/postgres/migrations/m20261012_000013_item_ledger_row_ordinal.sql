-- D76: operation_ordinal is the canonical row position 0..n-1 of the owner's rows for one
-- instance in one tick (D69 order), so the Consume aggregate is not ordinal 0. D94: the shape is
-- [non-terminal rows..., Consume aggregate, at most one terminal row (after = null)], so the
-- aggregate holds n-1, or n-2 when the n-1 row is terminal.
ALTER TABLE item_ledger DROP CONSTRAINT item_ledger_consume_ordinal;
-- Still exactly one Consume aggregate per character/instance/tick; it is that group's last row.
COMMENT ON INDEX item_ledger_consume_batch_key IS
    'One Consume aggregate per (zone, epoch, tick, character, instance); D76: it holds the group''s highest operation_ordinal.';
-- Checked at commit (or SET CONSTRAINTS ... IMMEDIATE) once the whole tick's rows are appended:
-- per (zone, epoch, tick, character, instance) the ordinals are unique and exactly 0..n-1, and a
-- Consume row holds n-1, or n-2 followed by a terminal row (D94). It fires on INSERT only, so append-only pre-D76 history is never
-- re-validated or rewritten.
CREATE FUNCTION item_ledger_row_ordinal() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    total bigint;
    distinct_ordinals bigint;
    lowest integer;
    highest integer;
    consume_ordinal integer;
    terminal_last boolean;
BEGIN
    SELECT count(*), count(DISTINCT operation_ordinal), min(operation_ordinal), max(operation_ordinal),
           max(operation_ordinal) FILTER (WHERE op = 'consume')
      INTO total, distinct_ordinals, lowest, highest, consume_ordinal
      FROM item_ledger
     WHERE zone_id = NEW.zone_id AND epoch = NEW.epoch AND tick = NEW.tick
       AND character_id = NEW.character_id AND instance_id = NEW.instance_id;
    SELECT EXISTS(
        SELECT 1 FROM item_ledger
         WHERE zone_id = NEW.zone_id AND epoch = NEW.epoch AND tick = NEW.tick
           AND character_id = NEW.character_id AND instance_id = NEW.instance_id
           AND operation_ordinal = highest AND op <> 'consume'
           AND jsonb_typeof(delta->'after') = 'null')
      INTO terminal_last;
    IF distinct_ordinals <> total OR lowest <> 0 OR highest <> total - 1
       OR (consume_ordinal IS NOT NULL AND consume_ordinal <> highest
           AND NOT (consume_ordinal = highest - 1 AND terminal_last)) THEN
        RAISE EXCEPTION 'item ledger ordinals must be the canonical row positions 0..n-1 per instance and tick (D76/D94)'
            USING ERRCODE = '23514', CONSTRAINT = 'item_ledger_row_ordinal';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER item_ledger_row_ordinal AFTER INSERT ON item_ledger
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION item_ledger_row_ordinal();
