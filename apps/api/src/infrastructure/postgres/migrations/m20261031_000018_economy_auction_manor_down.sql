DO $$ BEGIN
    IF EXISTS(SELECT 1 FROM item_auction_bids) OR EXISTS(SELECT 1 FROM item_auctions)
       OR EXISTS(SELECT 1 FROM manor_seed_production) OR EXISTS(SELECT 1 FROM manor_crop_procure) THEN
        RAISE EXCEPTION 'economy auction/manor rollback refused: rows exist';
    END IF;
END $$;
DROP TABLE manor_crop_procure;
DROP TABLE manor_seed_production;
DROP TABLE item_auction_bids;
DROP TABLE item_auctions;
