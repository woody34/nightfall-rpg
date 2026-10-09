-- Story E4.1: progression state on characters (plan §3.3). hp/mp NULL means "full on load":
-- the stat engine, not the database, knows the maxima. revision is the optimistic-concurrency
-- fence for checkpoints: every applied checkpoint bumps it by one.
ALTER TABLE characters
    ADD COLUMN xp            bigint  NOT NULL DEFAULT 0,
    ADD COLUMN hp            integer,
    ADD COLUMN mp            integer,
    ADD COLUMN class_profile text,
    ADD COLUMN alive         boolean NOT NULL DEFAULT true,
    ADD COLUMN revision      bigint  NOT NULL DEFAULT 0;

-- Existing rows start as the race's fighter profile (packages/data/classes/<id>.toml).
UPDATE characters
   SET class_profile = CASE race
        WHEN 'human'    THEN 'human_fighter'
        WHEN 'elf'      THEN 'elven_fighter'
        WHEN 'dark_elf' THEN 'dark_fighter'
        WHEN 'orc'      THEN 'orc_fighter'
        WHEN 'dwarf'    THEN 'dwarven_fighter'
    END;

ALTER TABLE characters
    ALTER COLUMN class_profile SET NOT NULL,
    ADD CONSTRAINT characters_xp_nonneg       CHECK (xp >= 0),
    ADD CONSTRAINT characters_hp_nonneg       CHECK (hp IS NULL OR hp >= 0),
    ADD CONSTRAINT characters_mp_nonneg       CHECK (mp IS NULL OR mp >= 0),
    ADD CONSTRAINT characters_revision_nonneg CHECK (revision >= 0),
    ADD CONSTRAINT characters_class_profile_format CHECK (class_profile ~ '^[a-z][a-z_]*$'),
    DROP CONSTRAINT characters_level_check,
    ADD CONSTRAINT characters_level_range CHECK (level BETWEEN 1 AND 85);
