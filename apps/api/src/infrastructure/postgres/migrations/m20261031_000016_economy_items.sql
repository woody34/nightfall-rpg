-- Phase 5 (design §11.4 row 2): one item table for every durable location (R7). Rows in mail,
-- delivery and auction escrow have no character; warehouse and freight rows keep the character.
-- The mail/delivery FKs are added by 000017/000019 once their tables exist (partial FK columns).
ALTER TABLE item_instances ADD COLUMN owner_kind text NOT NULL DEFAULT 'character'
    CONSTRAINT item_instances_owner_kind CHECK (owner_kind IN ('character','warehouse','freight','mail','delivery','auction_bid'));
ALTER TABLE item_instances ADD COLUMN container_id uuid;
ALTER TABLE item_instances ADD COLUMN mail_id uuid;
ALTER TABLE item_instances ADD COLUMN delivery_id uuid;
ALTER TABLE item_instances ALTER COLUMN character_id DROP NOT NULL;
ALTER TABLE item_instances DROP CONSTRAINT item_instances_location;
ALTER TABLE item_instances ADD CONSTRAINT item_instances_location
    CHECK (location IN ('inventory','paperdoll','warehouse','freight','mail','delivery','auction_bid'));
ALTER TABLE item_instances ADD CONSTRAINT item_instances_character_location
    CHECK ((location IN ('inventory','paperdoll','warehouse','freight')) = (character_id IS NOT NULL));
ALTER TABLE item_instances ADD CONSTRAINT item_instances_container
    CHECK ((location IN ('mail','delivery','auction_bid')) = (container_id IS NOT NULL));
ALTER TABLE item_instances ADD CONSTRAINT item_instances_owner_location
    CHECK ((owner_kind = 'character' AND location IN ('inventory','paperdoll'))
        OR (owner_kind IN ('warehouse','freight','mail','delivery','auction_bid') AND owner_kind = location));
ALTER TABLE item_instances ADD CONSTRAINT item_instances_partial_fk_columns
    CHECK ((location = 'mail') = (mail_id IS NOT NULL) AND (location = 'delivery') = (delivery_id IS NOT NULL)
        AND (mail_id IS NULL OR mail_id = container_id) AND (delivery_id IS NULL OR delivery_id = container_id));
ALTER TABLE item_instances DROP CONSTRAINT item_instances_slot;
ALTER TABLE item_instances ADD CONSTRAINT item_instances_slot
    CHECK ((location = 'paperdoll' AND slot BETWEEN 0 AND 24 AND slot IS NOT NULL) OR (location <> 'paperdoll' AND slot IS NULL));
CREATE INDEX item_instances_container_idx ON item_instances (container_id,id) WHERE container_id IS NOT NULL;
ALTER TABLE characters ADD COLUMN phase5_checkpoint jsonb;
