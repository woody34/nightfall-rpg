-- Preserve the compiled retained-effect DTO exactly; 000008 remains immutable.
ALTER TABLE character_effects
    ADD COLUMN target_incarnation bigint NOT NULL DEFAULT 0,
    ADD COLUMN removed_on_damage boolean NOT NULL DEFAULT false,
    ADD COLUMN removed_on_action_except_move boolean NOT NULL DEFAULT false,
    ADD COLUMN irreplaceable boolean NOT NULL DEFAULT false,
    ADD CONSTRAINT character_effects_target_incarnation CHECK (target_incarnation BETWEEN 0 AND 4294967295);
