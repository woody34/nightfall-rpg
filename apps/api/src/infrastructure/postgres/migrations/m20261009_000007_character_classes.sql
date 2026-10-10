-- Phase 2 expands identity and the actor-owned progression ledger. Unknown or wrong-race
-- Phase 1 profiles abort migration; existing XP/resources/level/positions remain untouched.
ALTER TABLE characters
    ADD COLUMN base_class_id integer,
    ADD COLUMN current_class_id integer,
    ADD COLUMN active_class_slot smallint NOT NULL DEFAULT 0,
    ADD COLUMN sex text NOT NULL DEFAULT 'male',
    ADD COLUMN hair_style integer NOT NULL DEFAULT 0,
    ADD COLUMN hair_color integer NOT NULL DEFAULT 0,
    ADD COLUMN face integer NOT NULL DEFAULT 0,
    ADD COLUMN sp bigint NOT NULL DEFAULT 0,
    ADD COLUMN cp integer NOT NULL DEFAULT 0,
    ADD COLUMN token_tier_1_count integer NOT NULL DEFAULT 0,
    ADD COLUMN token_tier_2_count integer NOT NULL DEFAULT 0,
    ADD COLUMN milestone_claimed_mask smallint NOT NULL DEFAULT 0;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM characters WHERE NOT (
            (race = 'human' AND class_profile IN ('human_fighter', 'human_mystic')) OR
            (race = 'elf' AND class_profile IN ('elven_fighter', 'elven_mystic')) OR
            (race = 'dark_elf' AND class_profile IN ('dark_fighter', 'dark_mystic')) OR
            (race = 'orc' AND class_profile IN ('orc_fighter', 'orc_mystic')) OR
            (race = 'dwarf' AND class_profile = 'dwarven_fighter')
        )
    ) THEN
        RAISE EXCEPTION 'cannot migrate unknown or wrong-race character class_profile';
    END IF;
END $$;

UPDATE characters SET base_class_id = CASE class_profile
    WHEN 'human_fighter' THEN 0 WHEN 'human_mystic' THEN 10
    WHEN 'elven_fighter' THEN 18 WHEN 'elven_mystic' THEN 25
    WHEN 'dark_fighter' THEN 31 WHEN 'dark_mystic' THEN 38
    WHEN 'orc_fighter' THEN 44 WHEN 'orc_mystic' THEN 49
    WHEN 'dwarven_fighter' THEN 53 END;
UPDATE characters SET current_class_id = base_class_id;

ALTER TABLE characters
    ALTER COLUMN base_class_id SET NOT NULL,
    ALTER COLUMN current_class_id SET NOT NULL,
    ADD CONSTRAINT characters_base_class_race CHECK (
        (race = 'human' AND base_class_id IN (0,10)) OR
        (race = 'elf' AND base_class_id IN (18,25)) OR
        (race = 'dark_elf' AND base_class_id IN (31,38)) OR
        (race = 'orc' AND base_class_id IN (44,49)) OR
        (race = 'dwarf' AND base_class_id = 53)),
    ADD CONSTRAINT characters_current_class_lineage CHECK (
        (base_class_id = 0 AND current_class_id BETWEEN 0 AND 9) OR
        (base_class_id = 10 AND current_class_id BETWEEN 10 AND 17) OR
        (base_class_id = 18 AND current_class_id BETWEEN 18 AND 24) OR
        (base_class_id = 25 AND current_class_id BETWEEN 25 AND 30) OR
        (base_class_id = 31 AND current_class_id BETWEEN 31 AND 37) OR
        (base_class_id = 38 AND current_class_id BETWEEN 38 AND 43) OR
        (base_class_id = 44 AND current_class_id BETWEEN 44 AND 48) OR
        (base_class_id = 49 AND current_class_id BETWEEN 49 AND 52) OR
        (base_class_id = 53 AND current_class_id BETWEEN 53 AND 57)),
    ADD CONSTRAINT characters_active_slot CHECK (active_class_slot BETWEEN 0 AND 3),
    ADD CONSTRAINT characters_sex CHECK (sex IN ('male','female')),
    ADD CONSTRAINT characters_appearance CHECK (hair_style = 0 AND hair_color = 0 AND face = 0),
    ADD CONSTRAINT characters_sp_nonneg CHECK (sp >= 0),
    ADD CONSTRAINT characters_cp_nonneg CHECK (cp >= 0),
    ADD CONSTRAINT characters_token_tier_1_nonneg CHECK (token_tier_1_count >= 0),
    ADD CONSTRAINT characters_token_tier_2_nonneg CHECK (token_tier_2_count >= 0),
    ADD CONSTRAINT characters_milestone_mask CHECK (milestone_claimed_mask BETWEEN 0 AND 3);

-- Independent main/subclass progression, reserved for future village-master flows.
CREATE TABLE character_class_slots (
    character_id uuid NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    slot smallint NOT NULL,
    class_id integer NOT NULL,
    level integer NOT NULL,
    exp bigint NOT NULL,
    sp bigint NOT NULL,
    is_dual boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (character_id, slot),
    CONSTRAINT character_class_slots_slot CHECK (slot BETWEEN 0 AND 3),
    CONSTRAINT character_class_slots_class CHECK (class_id BETWEEN 0 AND 57 OR class_id BETWEEN 88 AND 118),
    CONSTRAINT character_class_slots_level CHECK (level BETWEEN 1 AND 85),
    CONSTRAINT character_class_slots_exp CHECK (exp >= 0),
    CONSTRAINT character_class_slots_sp CHECK (sp >= 0)
);
INSERT INTO character_class_slots(character_id,slot,class_id,level,exp,sp)
SELECT id,0,current_class_id,level,xp,sp FROM characters;

CREATE TABLE character_certifications (
    character_id uuid NOT NULL,
    subclass_slot smallint NOT NULL,
    ordinal smallint NOT NULL,
    skill_key text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(character_id,subclass_slot,ordinal),
    FOREIGN KEY(character_id,subclass_slot) REFERENCES character_class_slots(character_id,slot) ON DELETE CASCADE,
    CONSTRAINT character_certifications_slot CHECK (subclass_slot BETWEEN 1 AND 3),
    CONSTRAINT character_certifications_ordinal CHECK (ordinal BETWEEN 0 AND 3),
    CONSTRAINT character_certifications_skill CHECK (skill_key ~ '^[a-z][a-z0-9_.]*$')
);

-- Learned metadata is separate from Phase 3 skill effects; stable source keys, no invented skills.
CREATE TABLE character_learned_skills (
    character_id uuid NOT NULL,
    slot smallint NOT NULL,
    skill_key text NOT NULL,
    skill_level integer NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(character_id,slot,skill_key),
    FOREIGN KEY(character_id,slot) REFERENCES character_class_slots(character_id,slot) ON DELETE CASCADE,
    CONSTRAINT character_learned_skills_level CHECK (skill_level > 0),
    CONSTRAINT character_learned_skills_key CHECK (skill_key ~ '^[a-z][a-z0-9_.]*$')
);

-- Typed, bounded receipt references; full frozen responses use the existing idempotency table.
-- change_class successes are retained forever, irrespective of general key cleanup policy.
CREATE TABLE character_transfer_receipts (
    character_id uuid NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    ordinal smallint NOT NULL,
    account_id uuid NOT NULL,
    operation text NOT NULL DEFAULT 'change_class',
    key uuid NOT NULL,
    target_class_id integer NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(character_id,ordinal),
    UNIQUE(account_id,key),
    FOREIGN KEY(account_id,operation,key) REFERENCES idempotency_keys(account_id,operation,key) ON DELETE RESTRICT,
    CONSTRAINT character_transfer_receipts_ordinal CHECK (ordinal BETWEEN 0 AND 1),
    CONSTRAINT character_transfer_receipts_operation CHECK (operation = 'change_class'),
    CONSTRAINT character_transfer_receipts_target CHECK (target_class_id BETWEEN 1 AND 57)
);
CREATE INDEX character_transfer_receipts_key_idx ON character_transfer_receipts(account_id,operation,key);
