-- Phase 5 (design §11.4 row 4, §10.5-§10.6): item auction state and the manor lists. Auction ids are
-- fixture-declared (bigint > 0); bidder columns are plain NULLable uuids (no FK: the history outlives
-- characters' presence in a zone). Manor amounts never exceed their start amount.
CREATE TABLE item_auctions (
    id bigint PRIMARY KEY CONSTRAINT item_auctions_id CHECK (id > 0),
    instance_npc integer NOT NULL CONSTRAINT item_auctions_instance CHECK (instance_npc > 0),
    item_index integer NOT NULL CONSTRAINT item_auctions_item_index CHECK (item_index >= 0),
    state smallint NOT NULL CONSTRAINT item_auctions_state CHECK (state BETWEEN 0 AND 2),
    extend_state smallint NOT NULL DEFAULT 0 CONSTRAINT item_auctions_extend_state CHECK (extend_state BETWEEN 0 AND 3),
    last_bidder uuid,
    highest_bidder uuid,
    highest_bid bigint CONSTRAINT item_auctions_highest_bid CHECK (highest_bid IS NULL OR highest_bid BETWEEN 1 AND 100000000000),
    start_at timestamptz NOT NULL,
    end_at timestamptz NOT NULL,
    revision bigint NOT NULL DEFAULT 0 CONSTRAINT item_auctions_revision CHECK (revision >= 0),
    CONSTRAINT item_auctions_window CHECK (start_at < end_at),
    CONSTRAINT item_auctions_highest_pair CHECK ((highest_bidder IS NULL) = (highest_bid IS NULL))
);
CREATE INDEX item_auctions_instance_idx ON item_auctions (instance_npc,start_at);
CREATE TABLE item_auction_bids (
    auction_id bigint NOT NULL REFERENCES item_auctions(id),
    character_id uuid NOT NULL REFERENCES characters(id),
    last_bid bigint NOT NULL CONSTRAINT item_auction_bids_last_bid CHECK (last_bid = -1 OR last_bid BETWEEN 1 AND 100000000000),
    revision bigint NOT NULL DEFAULT 0 CONSTRAINT item_auction_bids_revision CHECK (revision >= 0),
    PRIMARY KEY (auction_id,character_id)
);
CREATE TABLE manor_seed_production (
    castle_id smallint NOT NULL CONSTRAINT manor_seed_production_castle CHECK (castle_id BETWEEN 1 AND 9),
    seed_id integer NOT NULL CONSTRAINT manor_seed_production_seed CHECK (seed_id > 0),
    price bigint NOT NULL CONSTRAINT manor_seed_production_price CHECK (price >= 0),
    start_amount bigint NOT NULL CONSTRAINT manor_seed_production_start CHECK (start_amount >= 0),
    amount bigint NOT NULL,
    revision bigint NOT NULL DEFAULT 0 CONSTRAINT manor_seed_production_revision CHECK (revision >= 0),
    PRIMARY KEY (castle_id,seed_id),
    CONSTRAINT manor_seed_production_amount CHECK (amount BETWEEN 0 AND start_amount)
);
CREATE TABLE manor_crop_procure (
    castle_id smallint NOT NULL CONSTRAINT manor_crop_procure_castle CHECK (castle_id BETWEEN 1 AND 9),
    crop_id integer NOT NULL CONSTRAINT manor_crop_procure_crop CHECK (crop_id > 0),
    price bigint NOT NULL CONSTRAINT manor_crop_procure_price CHECK (price >= 0),
    start_amount bigint NOT NULL CONSTRAINT manor_crop_procure_start CHECK (start_amount >= 0),
    amount bigint NOT NULL,
    reward_type smallint NOT NULL CONSTRAINT manor_crop_procure_reward CHECK (reward_type BETWEEN 1 AND 2),
    revision bigint NOT NULL DEFAULT 0 CONSTRAINT manor_crop_procure_revision CHECK (revision >= 0),
    PRIMARY KEY (castle_id,crop_id),
    CONSTRAINT manor_crop_procure_amount CHECK (amount BETWEEN 0 AND start_amount)
);
