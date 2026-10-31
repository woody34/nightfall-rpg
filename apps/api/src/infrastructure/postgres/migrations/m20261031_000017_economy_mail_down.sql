DO $$ BEGIN
    IF EXISTS(SELECT 1 FROM mail_messages) THEN
        RAISE EXCEPTION 'economy mail rollback refused: mail rows exist';
    END IF;
END $$;
ALTER TABLE item_instances DROP CONSTRAINT item_instances_mail_fk;
DROP TABLE mail_messages;
