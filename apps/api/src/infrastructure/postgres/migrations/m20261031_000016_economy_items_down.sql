DO $$ BEGIN
    IF EXISTS(SELECT 1 FROM item_instances WHERE location NOT IN ('inventory','paperdoll') OR owner_kind <> 'character')
       OR EXISTS(SELECT 1 FROM characters WHERE phase5_checkpoint IS NOT NULL) THEN
        RAISE EXCEPTION 'economy items rollback refused: V7 item rows or Phase 5 checkpoints exist';
    END IF;
END $$;
ALTER TABLE characters DROP COLUMN phase5_checkpoint;
DROP INDEX item_instances_container_idx;
ALTER TABLE item_instances DROP CONSTRAINT item_instances_slot;
ALTER TABLE item_instances ADD CONSTRAINT item_instances_slot
    CHECK ((location = 'paperdoll' AND slot BETWEEN 0 AND 24 AND slot IS NOT NULL) OR (location = 'inventory' AND slot IS NULL));
ALTER TABLE item_instances DROP CONSTRAINT item_instances_partial_fk_columns;
ALTER TABLE item_instances DROP CONSTRAINT item_instances_owner_location;
ALTER TABLE item_instances DROP CONSTRAINT item_instances_container;
ALTER TABLE item_instances DROP CONSTRAINT item_instances_character_location;
ALTER TABLE item_instances DROP CONSTRAINT item_instances_location;
ALTER TABLE item_instances ADD CONSTRAINT item_instances_location CHECK (location IN ('inventory','paperdoll'));
ALTER TABLE item_instances ALTER COLUMN character_id SET NOT NULL;
ALTER TABLE item_instances DROP COLUMN delivery_id;
ALTER TABLE item_instances DROP COLUMN mail_id;
ALTER TABLE item_instances DROP COLUMN container_id;
ALTER TABLE item_instances DROP COLUMN owner_kind;
