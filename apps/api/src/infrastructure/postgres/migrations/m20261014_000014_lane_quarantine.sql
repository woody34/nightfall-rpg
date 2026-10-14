-- T11: a checkpoint lane that failed permanently while an unresolved epoch was recovered is
-- quarantined per owner so the zone can boot. Admission refuses the owner until an operator
-- resolves the row; the first recorded reason wins.
CREATE TABLE character_lane_quarantines (
    character_id uuid PRIMARY KEY REFERENCES characters(id) ON DELETE CASCADE,
    zone_id integer NOT NULL CONSTRAINT character_lane_quarantines_zone CHECK (zone_id >= 0),
    epoch bigint NOT NULL CONSTRAINT character_lane_quarantines_epoch CHECK (epoch >= 0),
    tick bigint NOT NULL CONSTRAINT character_lane_quarantines_tick CHECK (tick >= 0),
    reason text NOT NULL CONSTRAINT character_lane_quarantines_reason CHECK (reason <> ''),
    created_at timestamptz NOT NULL DEFAULT now()
);
