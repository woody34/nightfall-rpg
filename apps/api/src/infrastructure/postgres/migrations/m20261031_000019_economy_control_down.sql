DO $$ BEGIN
    IF EXISTS(SELECT 1 FROM economy_leases) OR EXISTS(SELECT 1 FROM economy_deliveries)
       OR EXISTS(SELECT 1 FROM economy_counters) OR EXISTS(SELECT 1 FROM economy_counter_facts) THEN
        RAISE EXCEPTION 'economy control rollback refused: rows exist';
    END IF;
END $$;
DROP TABLE economy_counter_facts;
DROP TABLE economy_counters;
ALTER TABLE item_instances DROP CONSTRAINT item_instances_delivery_fk;
DROP TABLE economy_deliveries;
DROP TABLE economy_leases;
