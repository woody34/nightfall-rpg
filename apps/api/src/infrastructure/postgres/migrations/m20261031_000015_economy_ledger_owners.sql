-- Phase 5 (design §11.4 row 1, plan §2.5): the item ledger names an owner of any kind (B13), the op
-- vocabulary gains the V7 values of §11.3 and the receipt scopes gain the 29 §11.3 scopes. The
-- logical key, the Consume batch key and the D76 row-ordinal trigger are re-keyed on the owner so
-- rule T1 (E26) holds per (owner, tick, instance). Insert-only triggers: pre-V7 history is never
-- re-validated or rewritten.
ALTER TABLE item_ledger ADD COLUMN owner_kind text NOT NULL DEFAULT 'character'
    CONSTRAINT item_ledger_owner_kind CHECK (owner_kind IN ('character','ground','refund','warehouse','freight','mail','delivery','auction_bid'));
ALTER TABLE item_ledger ADD COLUMN owner_id uuid;
ALTER TABLE item_ledger ALTER COLUMN character_id DROP NOT NULL;
ALTER TABLE item_ledger ADD CONSTRAINT item_ledger_owner_character
    CHECK ((owner_kind = 'character') = (character_id IS NOT NULL));
ALTER TABLE item_ledger ADD CONSTRAINT item_ledger_owner_id
    CHECK (owner_kind = 'character' OR owner_id IS NOT NULL);
ALTER TABLE item_ledger ADD COLUMN owner_key uuid GENERATED ALWAYS AS (COALESCE(character_id, owner_id)) STORED;
ALTER TABLE item_ledger DROP CONSTRAINT item_ledger_op;
ALTER TABLE item_ledger ADD CONSTRAINT item_ledger_op
    CHECK (op IN ('grant','starter','backfill','equip','unequip','consume','enchant','destroy','crystallize','refine','cancel_refine','exchange','use','toggle_auto_shot','loot','transfer_out','transfer_in','party_split','sweep','harvest','purchase','sale_proceeds','craft','cod_credit','auction_award','manor_reward'));
ALTER TABLE item_ledger DROP CONSTRAINT item_ledger_logical_key;
ALTER TABLE item_ledger ADD CONSTRAINT item_ledger_logical_key
    UNIQUE (zone_id,epoch,tick,owner_kind,owner_key,instance_id,op,operation_ordinal);
DROP INDEX item_ledger_consume_batch_key;
CREATE UNIQUE INDEX item_ledger_consume_batch_key ON item_ledger (zone_id,epoch,tick,owner_kind,owner_key,instance_id) WHERE op = 'consume';
COMMENT ON INDEX item_ledger_consume_batch_key IS
    'One Consume aggregate per (zone, epoch, tick, owner, instance); T1: it holds the group''s operation_ordinal n-1, or n-2 when one terminal row (transfer_out with after = null, or destroy) follows.';
CREATE INDEX item_ledger_owner_idx ON item_ledger (owner_kind,owner_key,id);
-- T1 (E26, amends D76/D94): per (zone, epoch, tick, owner, instance) the ordinals are exactly 0..n-1;
-- a Consume row holds n-1, or n-2 when exactly one terminal row (after = null: V7 transfer_out or
-- destroy, and the Phase 4 terminal rows D94 already settled) follows it. Pre-V7 rows have
-- owner_kind = 'character', so the same rule covers them.
DROP TRIGGER item_ledger_row_ordinal ON item_ledger;
DROP FUNCTION item_ledger_row_ordinal();
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
       AND owner_kind = NEW.owner_kind AND owner_key = NEW.owner_key AND instance_id = NEW.instance_id;
    SELECT EXISTS(
        SELECT 1 FROM item_ledger
         WHERE zone_id = NEW.zone_id AND epoch = NEW.epoch AND tick = NEW.tick
           AND owner_kind = NEW.owner_kind AND owner_key = NEW.owner_key AND instance_id = NEW.instance_id
           AND operation_ordinal = highest AND op <> 'consume'
           AND jsonb_typeof(delta->'after') = 'null')
      INTO terminal_last;
    IF distinct_ordinals <> total OR lowest <> 0 OR highest <> total - 1
       OR (consume_ordinal IS NOT NULL AND consume_ordinal <> highest
           AND NOT (consume_ordinal = highest - 1 AND terminal_last)) THEN
        RAISE EXCEPTION 'item ledger ordinals must be the canonical row positions 0..n-1 per owner, instance and tick (T1)'
            USING ERRCODE = '23514', CONSTRAINT = 'item_ledger_row_ordinal';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER item_ledger_row_ordinal AFTER INSERT ON item_ledger
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION item_ledger_row_ordinal();
ALTER TABLE item_mutation_receipts DROP CONSTRAINT item_mutation_receipts_operation;
ALTER TABLE item_mutation_receipts ADD CONSTRAINT item_mutation_receipts_operation
    CHECK (operation IN ('equip_item','unequip_item','use_item','destroy_item','enchant_item','toggle_auto_shot','refine_item','cancel_refine','exchange_special_ability','crystallize_item','grant_item','starter','backfill','pick_up_item','vendor_buy','vendor_sell','vendor_refund','multisell_choose','register_recipe','delete_recipe','craft_item','trade_confirm','store_buy','store_sell','private_manufacture','warehouse_deposit','warehouse_withdraw','freight_send','freight_withdraw','mail_send','mail_take','mail_reject','mail_cancel','mail_delete','manor_buy_seed','manor_sell_crop','auction_bid','auction_cancel_bid','party','trade_session','store_setup','mail_mark_read'));
