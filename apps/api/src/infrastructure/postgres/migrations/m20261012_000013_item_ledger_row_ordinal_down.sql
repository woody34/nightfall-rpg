DO $$ BEGIN
    IF EXISTS(SELECT 1 FROM item_ledger WHERE op = 'consume' AND operation_ordinal <> 0) THEN
        RAISE EXCEPTION 'row-ordinal rollback would invalidate D76 Consume aggregate ordinals';
    END IF;
END $$;
DROP TRIGGER item_ledger_row_ordinal ON item_ledger;
DROP FUNCTION item_ledger_row_ordinal();
COMMENT ON INDEX item_ledger_consume_batch_key IS NULL;
ALTER TABLE item_ledger ADD CONSTRAINT item_ledger_consume_ordinal CHECK (op <> 'consume' OR operation_ordinal = 0);
