-- D41: a UUID is a key only within its authenticated account and operation scope.
ALTER TABLE pending_item_requests ADD COLUMN account_id uuid;
ALTER TABLE pending_item_requests ADD COLUMN operation text;
UPDATE pending_item_requests p SET account_id=c.account_id,operation=p.kind FROM characters c WHERE c.id=p.character_id;
ALTER TABLE pending_item_requests ALTER COLUMN account_id SET NOT NULL;
ALTER TABLE pending_item_requests ALTER COLUMN operation SET NOT NULL;
ALTER TABLE pending_item_requests DROP CONSTRAINT pending_item_requests_kind;
-- Direct Grant is recorded in MaterializePendingItems; it has no pending-completion row.
ALTER TABLE pending_item_requests ADD CONSTRAINT pending_item_requests_kind CHECK (kind IN ('starter','backfill'));
ALTER TABLE pending_item_requests ADD CONSTRAINT pending_item_requests_operation CHECK (operation=kind);
ALTER TABLE characters ADD CONSTRAINT characters_item_owner_key UNIQUE(id,account_id);
ALTER TABLE pending_item_requests ADD CONSTRAINT pending_item_requests_owner FOREIGN KEY(character_id,account_id) REFERENCES characters(id,account_id);
ALTER TABLE pending_item_requests DROP CONSTRAINT pending_item_requests_pkey;
ALTER TABLE pending_item_requests ADD PRIMARY KEY(account_id,operation,request_id);
-- Migration11 intentionally retains its frozen SQL. Fill omitted scope on its reruns;
-- explicit new-adapter scope is still checked by the owner FK and operation CHECK.
CREATE FUNCTION phase4_pending_scope() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.account_id IS NULL THEN SELECT account_id INTO NEW.account_id FROM characters WHERE id=NEW.character_id; END IF;
    IF NEW.operation IS NULL THEN NEW.operation=NEW.kind; END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER phase4_pending_scope BEFORE INSERT ON pending_item_requests FOR EACH ROW EXECUTE FUNCTION phase4_pending_scope();
