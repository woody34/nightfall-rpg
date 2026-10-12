//! Explicit V6 construction. Historical constructors never acquire an item sidecar.
use super::{SnapshotError, StateDigestVersion, ZoneBounds, ZoneSeed, ZoneState};
use crate::domain::zone::{
    ItemInstanceRange, Phase3AdmissionPolicy, Phase3Rules, Phase4Rules, Phase4Snapshot, StatRules,
};
use std::sync::Arc;

impl ZoneState {
    /// Construct a V6 epoch from validated, explicitly supplied immutable bundles and a
    /// recorded allocator reservation. Service startup supplies the compiled hash bundle.
    #[allow(clippy::too_many_arguments)] // Independent epoch inputs must remain explicit.
    pub fn new_phase4(
        seed: ZoneSeed,
        bounds: ZoneBounds,
        time_origin_ms: i64,
        stat_rules: Arc<StatRules>,
        classes: Arc<crate::domain::class::ClassRegistry>,
        skills: Phase3Rules,
        items: Arc<Phase4Rules>,
        allocator: ItemInstanceRange,
    ) -> Result<Self, SnapshotError> {
        if skills.runtime_revision() != 3 {
            return Err(SnapshotError::Rules("V6 requires the V6 skill capability bundle".into()));
        }
        items
            .validate_phase3(&skills)
            .map_err(SnapshotError::Rules)?;
        let canonical = Phase4Snapshot::new(&items, allocator);
        let mut state =
            Self::new_skill_epoch(seed, bounds, time_origin_ms, stat_rules, classes, skills)?;
        state.phase4 = Some(crate::domain::zone::phase4::Phase4Zone {
            canonical,
            rules: items,
        });
        state.meta.schema_version = 10;
        state.meta.digest_version = StateDigestVersion::BinaryV6;
        state.meta.phase3_admission_policy = Some(Phase3AdmissionPolicy::version_two());
        state.snapshot().validate_version()?;
        Ok(state)
    }

    /// Recorded item policy; environment switches cannot alter an existing epoch.
    pub fn phase4_enabled(&self) -> bool {
        self.meta.schema_version == 10 && self.meta.digest_version == StateDigestVersion::BinaryV6
    }
}
