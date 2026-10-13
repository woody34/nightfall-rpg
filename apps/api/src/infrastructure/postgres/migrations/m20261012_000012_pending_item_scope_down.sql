DO $$ BEGIN
    IF EXISTS(SELECT request_id FROM pending_item_requests GROUP BY request_id HAVING count(*)>1) THEN
        RAISE EXCEPTION 'pending scope rollback would lose legal account/operation key identities';
    END IF;
END $$;
DROP TRIGGER phase4_pending_scope ON pending_item_requests;
DROP FUNCTION phase4_pending_scope();
ALTER TABLE pending_item_requests DROP CONSTRAINT pending_item_requests_pkey;
ALTER TABLE pending_item_requests DROP CONSTRAINT pending_item_requests_owner;
ALTER TABLE pending_item_requests DROP CONSTRAINT pending_item_requests_operation;
ALTER TABLE pending_item_requests DROP CONSTRAINT pending_item_requests_kind;
ALTER TABLE pending_item_requests ADD CONSTRAINT pending_item_requests_kind CHECK(kind IN ('starter','backfill','grant'));
ALTER TABLE pending_item_requests DROP COLUMN account_id;
ALTER TABLE pending_item_requests DROP COLUMN operation;
ALTER TABLE pending_item_requests ADD PRIMARY KEY(request_id);
ALTER TABLE characters DROP CONSTRAINT characters_item_owner_key;
