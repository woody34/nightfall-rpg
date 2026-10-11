-- Phase 3 paused timer checkpoints (architecture R4/R10, design v3 section 5).
-- NULL policy identifies a legacy row. No wall-clock input and no duplicate SP column.
ALTER TABLE characters
    ADD COLUMN phase3_timer_policy_version integer,
    ADD COLUMN karma bigint NOT NULL DEFAULT 0,
    ADD COLUMN pk_count bigint NOT NULL DEFAULT 0,
    ADD COLUMN pvp_count bigint NOT NULL DEFAULT 0,
    ADD COLUMN flag_remaining_ticks bigint NOT NULL DEFAULT 0,
    ADD COLUMN death_penalty_level smallint NOT NULL DEFAULT 0,
    ADD COLUMN recoverable_death_xp_loss bigint NOT NULL DEFAULT 0,
    ADD COLUMN death_incarnation bigint,
    ADD COLUMN pre_death_xp bigint,
    ADD COLUMN hp_fraction_q bigint NOT NULL DEFAULT 0,
    ADD COLUMN mp_fraction_q bigint NOT NULL DEFAULT 0,
    ADD COLUMN cp_fraction_q bigint NOT NULL DEFAULT 0,
    ADD COLUMN modifier_install_order jsonb NOT NULL DEFAULT '[]'::jsonb,
    ADD CONSTRAINT characters_phase3_timer_policy CHECK (phase3_timer_policy_version IS NULL OR phase3_timer_policy_version = 1),
    ADD CONSTRAINT characters_karma_range CHECK (karma BETWEEN 0 AND 4294967295),
    ADD CONSTRAINT characters_pk_count_range CHECK (pk_count BETWEEN 0 AND 4294967295),
    ADD CONSTRAINT characters_pvp_count_range CHECK (pvp_count BETWEEN 0 AND 4294967295),
    ADD CONSTRAINT characters_flag_remaining_nonneg CHECK (flag_remaining_ticks >= 0),
    ADD CONSTRAINT characters_death_penalty_range CHECK (death_penalty_level BETWEEN 0 AND 5),
    ADD CONSTRAINT characters_recoverable_death_xp_nonneg CHECK (recoverable_death_xp_loss >= 0),
    ADD CONSTRAINT characters_death_record CHECK (
        (death_incarnation IS NULL AND pre_death_xp IS NULL AND recoverable_death_xp_loss = 0) OR
        (death_incarnation BETWEEN 0 AND 4294967295 AND pre_death_xp >= 0
         AND death_incarnation IS NOT NULL AND pre_death_xp IS NOT NULL
         AND recoverable_death_xp_loss <= pre_death_xp)),
    ADD CONSTRAINT characters_resource_fractions CHECK (
        hp_fraction_q BETWEEN 0 AND 999999 AND mp_fraction_q BETWEEN 0 AND 999999
        AND cp_fraction_q BETWEEN 0 AND 999999),
    ADD CONSTRAINT characters_modifier_install_order_array CHECK (jsonb_typeof(modifier_install_order) = 'array');

CREATE TABLE character_skill_reuse (
    character_id uuid NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    skill_id bigint NOT NULL,
    remaining_ticks bigint NOT NULL,
    PRIMARY KEY (character_id, skill_id),
    CONSTRAINT character_skill_reuse_skill CHECK (skill_id BETWEEN 1 AND 4294967295),
    CONSTRAINT character_skill_reuse_remainder CHECK (remaining_ticks > 50)
);

-- Each retained application occupies one row; handlers preserve XML order and typed
-- payload/periodic phase as canonical JSON. Runtime validation is required before restore.
-- No live EffectId is persisted: installation order refers to retained_ordinal.
CREATE TABLE character_effects (
    character_id uuid NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    retained_ordinal bigint NOT NULL,
    skill_id bigint NOT NULL,
    level bigint NOT NULL,
    remaining_ticks bigint NOT NULL,
    caster_kind text NOT NULL,
    caster_id uuid,
    caster_incarnation bigint,
    pool text NOT NULL,
    abnormal_type text NOT NULL,
    abnormal_level bigint NOT NULL,
    stay_after_death boolean NOT NULL,
    stay_on_subclass_change boolean NOT NULL,
    handlers jsonb NOT NULL,
    PRIMARY KEY (character_id, retained_ordinal),
    CONSTRAINT character_effects_ordinal CHECK (retained_ordinal BETWEEN 0 AND 4294967295),
    CONSTRAINT character_effects_skill CHECK (skill_id BETWEEN 1 AND 4294967295 AND skill_id <> 5076),
    CONSTRAINT character_effects_level CHECK (level BETWEEN 1 AND 4294967295),
    CONSTRAINT character_effects_remainder CHECK (remaining_ticks > 100),
    CONSTRAINT character_effects_caster_kind CHECK (caster_kind IN ('player', 'npc', 'system')),
    CONSTRAINT character_effects_caster_binding CHECK (
        (caster_kind = 'system' AND caster_id IS NULL AND caster_incarnation IS NULL) OR
        (caster_kind IN ('player', 'npc') AND caster_id IS NOT NULL AND caster_incarnation IS NOT NULL
         AND caster_incarnation BETWEEN 0 AND 4294967295)),
    CONSTRAINT character_effects_pool CHECK (pool IN ('buff', 'song_dance', 'debuff')),
    CONSTRAINT character_effects_abnormal_level CHECK (abnormal_level BETWEEN 0 AND 4294967295),
    CONSTRAINT character_effects_handlers_array CHECK (jsonb_typeof(handlers) = 'array')
);
