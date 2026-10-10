//! Pure class transfer and authoritative admission. No application or transport dependencies.
use super::*;
use crate::domain::character_progression::{FrozenTransferResult, SuccessfulTransferReceipt};
use crate::domain::class::{ClassId, ClassRegistry};
use crate::domain::zone::{profession_stats, transfer_resource, PlayerLoad, PlayerProgression};
use crate::domain::{AccountId, CharacterId, CharacterName};

impl ZoneState {
    pub(super) fn validate_class_entity(&self, e: &Entity) -> Result<(), SnapshotError> {
        if let Some(CombatState {
            role:
                CombatRole::Player {
                    class,
                    xp,
                    progression,
                },
            ..
        }) = &e.combat
        {
            match (&self.classes, progression) {
                (None, None) => {},
                (Some(registry), Some(p)) => {
                    p.class_state
                        .validate_for(
                            registry,
                            &p.identity,
                            crate::domain::CharacterId::from_uuid(e.id.as_uuid()),
                        )
                        .map_err(|_| SnapshotError::CombatMismatch)?;
                    let c = e.combat.as_ref().ok_or(SnapshotError::CombatMismatch)?;
                    let rules = self.rules.as_deref().ok_or(SnapshotError::CombatMismatch)?;
                    let (sheet, max_cp, speed, radius) = profession_stats(
                        rules,
                        registry,
                        &p.identity,
                        &p.class_state,
                        c.sheet.level(),
                    )
                    .map_err(|_| SnapshotError::CombatMismatch)?;
                    if crate::domain::character_progression::base_class_profile(
                        p.identity.base_class_id,
                    ) != Some(class.as_str())
                        || level_for_xp(rules, *xp) != c.sheet.level()
                        || *xp > xp_cap(rules).map_err(|_| SnapshotError::CombatMismatch)?
                        || crate::domain::CharacterName::new(e.name.clone()).is_err()
                        || e.targeting.dead != (c.hp == 0)
                        || sheet != c.sheet
                        || p.max_cp != max_cp
                        || p.class_state.cp > max_cp
                        || c.hp > sheet.max_hp()
                        || c.mp > sheet.max_mp()
                        || e.speed != speed
                        || c.collision_radius != radius
                    {
                        return Err(SnapshotError::CombatMismatch);
                    }
                    for r in &p.class_state.successful_transfer_receipts {
                        if r.result.character_id.as_uuid() != e.id.as_uuid()
                            || r.result.identity != p.identity
                            || r.result.current_class_id != r.target_class_id
                        {
                            return Err(SnapshotError::CombatMismatch);
                        }
                    }
                },
                _ => return Err(SnapshotError::CombatMismatch),
            }
        }
        Ok(())
    }

    /// Whether this epoch uses Phase 2 class rules.
    pub fn has_classes(&self) -> bool {
        self.classes.is_some()
    }

    /// Enables Phase 2 only for new epochs; restored legacy snapshots never call this.
    pub fn with_classes(mut self, classes: Arc<ClassRegistry>) -> Result<Self, SnapshotError> {
        self.meta.classes_rules_hash = Some(registry_hash(&classes)?);
        self.classes = Some(classes);
        self.meta.digest_version = StateDigestVersion::BinaryV3;
        Ok(self)
    }

    pub(super) fn loaded_player(
        &self,
        rules: &StatRules,
        load: &PlayerLoad,
        entity: EntityId,
    ) -> Result<CombatState, RejectReason> {
        let mut combat = player_combat(rules, load)?;
        match (&self.classes, &load.progression) {
            (None, None) => {},
            (Some(registry), Some(p)) => {
                p.class_state
                    .validate_for(registry, &p.identity, CharacterId::from_uuid(entity.as_uuid()))
                    .map_err(|_| RejectReason::InvalidLoad)?;
                if crate::domain::character_progression::base_class_profile(
                    p.identity.base_class_id,
                ) != Some(load.class.as_str())
                {
                    return Err(RejectReason::InvalidLoad);
                }
                let (sheet, max_cp, _, radius) =
                    profession_stats(rules, registry, &p.identity, &p.class_state, load.level)
                        .map_err(|_| RejectReason::InvalidLoad)?;
                let mut p = p.clone();
                p.max_cp = max_cp;
                p.class_state.cp = p.class_state.cp.min(max_cp);
                combat.hp = if load.alive {
                    load.hp.unwrap_or(sheet.max_hp()).clamp(1, sheet.max_hp())
                } else {
                    0
                };
                combat.mp = load.mp.unwrap_or(sheet.max_mp()).min(sheet.max_mp());
                combat.sheet = sheet;
                combat.collision_radius = radius;
                if let CombatRole::Player { progression, .. } = &mut combat.role {
                    *progression = Some(p);
                }
            },
            _ => return Err(RejectReason::InvalidLoad),
        }
        Ok(combat)
    }

    /// Validates actor identity and generation for both live reads and mutations.
    pub fn class_player(
        &self,
        entity: EntityId,
        account: AccountId,
        generation: SessionGeneration,
    ) -> Result<&PlayerProgression, RejectReason> {
        let e = self
            .entities
            .get(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        if e.generation != generation {
            return Err(RejectReason::StaleSession);
        }
        let Some(CombatRole::Player {
            progression: Some(p),
            ..
        }) = e.combat.as_ref().map(|c| &c.role)
        else {
            return Err(RejectReason::InvalidLoad);
        };
        if p.identity.account_id != account {
            return Err(RejectReason::NotPermitted);
        }
        Ok(p)
    }

    /// Stable, complete unmet requirements for an option, in validation order.
    pub fn transfer_unmet(&self, entity: EntityId, target: ClassId) -> Vec<RejectReason> {
        let mut unmet = Vec::new();
        let Some(e) = self.entities.get(&entity) else {
            return vec![RejectReason::UnknownEntity];
        };
        let Some(c) = &e.combat else {
            return vec![RejectReason::InvalidLoad];
        };
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &c.role
        else {
            return vec![RejectReason::InvalidLoad];
        };
        let Some(registry) = &self.classes else {
            return vec![RejectReason::InvalidLoad];
        };
        let Some(class) = registry.get(target) else {
            return vec![RejectReason::TransferIneligible];
        };
        if e.targeting.dead || c.hp == 0 {
            unmet.push(RejectReason::DeadActor);
        }
        if !(1..=2).contains(&class.tier)
            || class.parent != Some(p.class_state.current_class_id)
            || class.race != p.identity.race
            || class.base_class_id != p.identity.base_class_id
            || c.sheet.level() < class.min_level
        {
            unmet.push(RejectReason::TransferIneligible);
        }
        let engaged = c.auto_attack
            || c.chasing
            || c.swing.is_some()
            || c.ready_at > self.next_tick
            || self.hate.values().any(|h| h.get(entity).is_some())
            || self.entities.values().any(|other| {
                !other.targeting.dead
                    && other.combat.as_ref().is_some_and(|combat| {
                        (combat.auto_attack && other.targeting.target == Some(entity))
                            || combat.swing.is_some_and(|s| s.target == entity)
                    })
            });
        if engaged {
            unmet.push(RejectReason::InCombat);
        }
        if !e
            .pos
            .within(Vec2Fixed::from_tiles(126, 128), Fixed::from_tiles(3))
        {
            unmet.push(RejectReason::ClassMasterTooFar);
        }
        let token = if class.tier == 1 {
            p.class_state.token_tier_1_count
        } else {
            p.class_state.token_tier_2_count
        };
        let expected = if class.tier == 1 {
            "class_transfer_token_1"
        } else {
            "class_transfer_token_2"
        };
        if token == 0
            || !class.transfer.quest_hooks.is_empty()
            || class.transfer.requires.len() != 1
            || class
                .transfer
                .requires
                .iter()
                .any(|r| r.item != expected || r.count != 1)
        {
            unmet.push(RejectReason::TransferRequirement);
        }
        unmet
    }

    /// Ordered direct children, including locked third-tier metadata.
    pub fn class_options(&self, entity: EntityId) -> Vec<(ClassId, Vec<RejectReason>)> {
        let Some(CombatRole::Player {
            progression: Some(p),
            ..
        }) = self
            .entities
            .get(&entity)
            .and_then(|e| e.combat.as_ref())
            .map(|c| &c.role)
        else {
            return Vec::new();
        };
        self.classes
            .as_ref()
            .map(|r| {
                r.children(p.class_state.current_class_id)
                    .into_iter()
                    .map(|id| (id, self.transfer_unmet(entity, id)))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn change_class(
        &mut self,
        tick: Tick,
        entity: EntityId,
        account: AccountId,
        key: uuid::Uuid,
        target: ClassId,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        let e = self
            .entities
            .get(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let p = self.class_player(entity, account, e.generation)?;
        if let Some(receipt) = p.class_state.receipt(key) {
            return if receipt.target_class_id == target
                && receipt.result.character_id.as_uuid() == entity.as_uuid()
            {
                Ok(Vec::new())
            } else {
                Err(RejectReason::TransferConflict)
            };
        }
        if let Some(reason) = self.transfer_unmet(entity, target).first() {
            return Err(*reason);
        }
        let c = e.combat.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let CombatRole::Player { xp, .. } = &c.role else {
            return Err(RejectReason::NotAPlayer);
        };
        let mut p = p.clone();
        let old_class_id = p.class_state.current_class_id;
        p.class_state.current_class_id = target;
        let registry = self.classes.as_deref().ok_or(RejectReason::InvalidLoad)?;
        let class = registry
            .get(target)
            .ok_or(RejectReason::TransferIneligible)?;
        let balance = if class.tier == 1 {
            &mut p.class_state.token_tier_1_count
        } else {
            &mut p.class_state.token_tier_2_count
        };
        *balance = balance
            .checked_sub(1)
            .ok_or(RejectReason::TransferRequirement)?;
        let (sheet, max_cp, speed, radius) = profession_stats(
            self.rules.as_deref().ok_or(RejectReason::InvalidLoad)?,
            registry,
            &p.identity,
            &p.class_state,
            c.sheet.level(),
        )
        .map_err(|_| RejectReason::InvalidLoad)?;
        let hp = transfer_resource(c.hp, c.sheet.max_hp(), sheet.max_hp(), 1);
        let mp = transfer_resource(c.mp, c.sheet.max_mp(), sheet.max_mp(), 0);
        p.class_state.cp = transfer_resource(p.class_state.cp, p.max_cp, max_cp, 0);
        p.max_cp = max_cp;
        let grants = crate::domain::character_progression::auto_get_metadata(
            registry,
            target,
            sheet.level(),
        )
        .map_err(|_| RejectReason::InvalidLoad)?;
        let granted_skill_keys = p
            .class_state
            .merge_learned_skills_checked(registry, p.identity.race, grants)
            .map_err(|_| RejectReason::InvalidLoad)?;
        let result = FrozenTransferResult {
            character_id: CharacterId::from_uuid(entity.as_uuid()),
            identity: p.identity.clone(),
            name: CharacterName::new(&e.name).map_err(|_| RejectReason::InvalidLoad)?,
            current_class_id: target,
            level: sheet.level(),
            xp: *xp,
            sp: p.class_state.sp,
            stats: class.base_stats,
            position_millitiles: [e.pos.x.raw(), e.pos.y.raw()],
            hp,
            mp,
            cp: p.class_state.cp,
            max_hp: sheet.max_hp(),
            max_mp: sheet.max_mp(),
            max_cp,
            token_tier_1_count: p.class_state.token_tier_1_count,
            token_tier_2_count: p.class_state.token_tier_2_count,
            granted_skill_keys,
        };
        let receipt = SuccessfulTransferReceipt {
            key,
            target_class_id: target,
            result,
        };
        p.class_state
            .record_success(receipt.clone())
            .map_err(|_| RejectReason::TransferConflict)?;
        p.class_state
            .validate_for(registry, &p.identity, CharacterId::from_uuid(entity.as_uuid()))
            .map_err(|_| RejectReason::InvalidLoad)?;
        let e = self
            .entities
            .get_mut(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let c = e.combat.as_mut().ok_or(RejectReason::InvalidLoad)?;
        if let CombatRole::Player { progression, .. } = &mut c.role {
            *progression = Some(Box::new(p));
        }
        c.sheet = sheet;
        c.hp = hp;
        c.mp = mp;
        c.collision_radius = radius;
        e.speed = speed;
        Ok(vec![
            ZoneEvent::ClassChanged {
                tick,
                entity,
                class_id: target,
                generation: e.generation,
            },
            stats_changed(tick, entity, c),
            ZoneEvent::ClassTransfer {
                tick,
                entity,
                old_class_id,
                receipt: Box::new(receipt),
            },
        ])
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::many_single_char_names // Compact state/entity/account vectors match the zone test conventions.
)]
mod tests {
    use super::*;
    use crate::application::replay_log::{decode_snapshot, encode_snapshot, AppliedTickRecord};
    use crate::domain::character_progression::{
        CharacterAppearance, CharacterIdentity, ClassState,
    };
    use crate::domain::zone::{physical_damage, resource_max, PlayerLoad, Speed, StatKind};
    use crate::infrastructure::{
        class_data::{load_classes, ClassSource},
        rules_data::{load_rules, RulesSource},
    };
    fn fixture(level: u32) -> (ZoneState, EntityId, AccountId) {
        fixture_class(level, ClassId(0))
    }
    fn fixture_class(level: u32, base: ClassId) -> (ZoneState, EntityId, AccountId) {
        static RULES: std::sync::OnceLock<Arc<StatRules>> = std::sync::OnceLock::new();
        static CLASSES: std::sync::OnceLock<Arc<ClassRegistry>> = std::sync::OnceLock::new();
        let rules = RULES
            .get_or_init(|| load_rules(&RulesSource::embedded()).unwrap().rules)
            .clone();
        let registry = CLASSES
            .get_or_init(|| load_classes(&ClassSource::embedded()).unwrap().registry)
            .clone();
        let entity = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        let account = AccountId::from_uuid(uuid::Uuid::from_u128(2));
        let mut state = ZoneState::new(
            ZoneSeed {
                zone: ZoneId(1),
                epoch: 1,
            },
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
            0,
        )
        .with_rules(rules.clone())
        .with_classes(registry)
        .unwrap();
        let root = state.classes.as_ref().unwrap().get(base).unwrap();
        let race = root.race;
        let mut class_state = ClassState::new(base);
        class_state.token_tier_1_count = 1;
        class_state.token_tier_2_count = 1;
        let progression = PlayerProgression {
            identity: CharacterIdentity {
                account_id: account,
                race,
                base_class_id: base,
                appearance: CharacterAppearance::default(),
            },
            class_state,
            max_cp: 0,
        };
        let load = PlayerLoad {
            progression: Some(Box::new(progression)),
            level,
            xp: rules.xp_to_level(level).unwrap(),
            checkpoint_revision: Some(0),
            ..PlayerLoad::fresh(
                crate::domain::character_progression::base_class_profile(base).unwrap(),
            )
        };
        let t = state
            .run_tick(state.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity,
                name: "Tester".into(),
                pos: Vec2Fixed::from_tiles(126, 126),
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(load)),
            })]))
            .unwrap();
        assert!(t.dispositions.is_empty());
        (state, entity, account)
    }
    fn command(e: EntityId, a: AccountId, key: u128, target: u32) -> ZoneInput {
        ZoneInput {
            source: CommandSource::Session {
                entity: e,
                generation: SessionGeneration(1),
            },
            seq: None,
            command: ZoneCommand::ChangeClass {
                entity: e,
                account: a,
                request_key: uuid::Uuid::from_u128(key),
                target: ClassId(target),
            },
        }
    }
    fn apply(state: &mut ZoneState, input: ZoneInput) -> AppliedTick {
        state.run_tick(state.draft(vec![input])).unwrap()
    }
    #[test]
    fn transfers_consume_once_freeze_response_and_stop_at_two_even_at_85() {
        let (mut s, e, a) = fixture(85);
        let first = apply(&mut s, command(e, a, 10, 1));
        assert!(first.dispositions.is_empty());
        let frozen = s
            .class_player(e, a, SessionGeneration(1))
            .unwrap()
            .class_state
            .receipt(uuid::Uuid::from_u128(10))
            .unwrap()
            .result
            .clone();
        let duplicate = apply(&mut s, command(e, a, 10, 1));
        assert!(duplicate.events.iter().all(|ev| !matches!(
            ev,
            ZoneEvent::ClassChanged { .. } | ZoneEvent::ClassTransfer { .. }
        )));
        let conflict = apply(&mut s, command(e, a, 10, 4));
        assert_eq!(conflict.dispositions[0].reason, RejectReason::TransferConflict);
        assert!(apply(&mut s, command(e, a, 11, 2)).dispositions.is_empty());
        let p = s.class_player(e, a, SessionGeneration(1)).unwrap();
        assert_eq!(p.class_state.successful_transfer_receipts.len(), 2);
        assert_eq!(p.class_state.token_tier_1_count + p.class_state.token_tier_2_count, 0);
        assert_eq!(
            p.class_state
                .receipt(uuid::Uuid::from_u128(10))
                .unwrap()
                .result,
            frozen
        );
        assert_eq!(
            apply(&mut s, command(e, a, 12, 88)).dispositions[0].reason,
            RejectReason::TransferIneligible
        );
    }
    #[test]
    fn level_parent_race_tokens_death_and_source_are_checked_without_consumption() {
        let (mut s, e, a) = fixture(19);
        assert_eq!(
            apply(&mut s, command(e, a, 10, 1)).dispositions[0].reason,
            RejectReason::TransferIneligible
        );
        let (mut s, e, a) = fixture(40);
        for target in [2, 19, 999, 88] {
            assert_eq!(
                apply(&mut s, command(e, a, 10, target)).dispositions[0].reason,
                RejectReason::TransferIneligible
            );
        }
        let mut forged = command(e, a, 10, 1);
        forged.source = CommandSource::System;
        assert_eq!(apply(&mut s, forged).dispositions[0].reason, RejectReason::NotPermitted);
        assert_eq!(
            apply(&mut s, command(e, AccountId::from_uuid(uuid::Uuid::nil()), 10, 1)).dispositions
                [0]
            .reason,
            RejectReason::NotPermitted
        );
        s.entities.get_mut(&e).unwrap().targeting.dead = true;
        assert_eq!(
            apply(&mut s, command(e, a, 10, 1)).dispositions[0].reason,
            RejectReason::DeadActor
        );
        let p = s.class_player(e, a, SessionGeneration(1)).unwrap();
        assert_eq!(p.class_state.token_tier_1_count, 1);
        assert!(p.class_state.successful_transfer_receipts.is_empty());
    }
    #[test]
    fn generation_fences_use_all_64_bits_and_range_is_inclusive() {
        let (mut s, e, a) = fixture(20);
        s.entities.get_mut(&e).unwrap().generation = SessionGeneration(u64::from(u32::MAX) + 1);
        assert_eq!(
            apply(&mut s, command(e, a, 1, 1)).dispositions[0].reason,
            RejectReason::StaleSession
        );
        s.entities.get_mut(&e).unwrap().generation = SessionGeneration(1);
        s.entities.get_mut(&e).unwrap().pos =
            Vec2Fixed::new(Fixed::from_raw(129_001), Fixed::from_raw(128_000));
        assert!(s
            .transfer_unmet(e, ClassId(1))
            .contains(&RejectReason::ClassMasterTooFar));
        s.entities.get_mut(&e).unwrap().pos = Vec2Fixed::from_tiles(129, 128);
        assert!(!s
            .transfer_unmet(e, ClassId(1))
            .contains(&RejectReason::ClassMasterTooFar));
    }
    #[test]
    fn combat_refuses_transfer_without_cancelling_pending_attack() {
        let (mut s, e, a) = fixture(20);
        s.entities
            .get_mut(&e)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .auto_attack = true;
        assert!(s
            .transfer_unmet(e, ClassId(1))
            .contains(&RejectReason::InCombat));
        assert_eq!(
            apply(&mut s, command(e, a, 1, 1)).dispositions[0].reason,
            RejectReason::InCombat
        );
        assert!(s
            .class_player(e, a, SessionGeneration(1))
            .unwrap()
            .class_state
            .successful_transfer_receipts
            .is_empty());
    }
    #[test]
    fn new_record_bytes_and_mid_transfer_restore_are_exact() {
        let (mut s, e, a) = fixture(40);
        let initial = s.snapshot();
        let inputs = [
            command(e, a, 1, 1),
            command(e, a, 1, 1),
            command(e, a, 2, 2),
            command(e, a, 3, 88),
        ];
        let mut replay =
            ZoneState::from_snapshot(decode_snapshot(&encode_snapshot(&initial).unwrap()).unwrap())
                .unwrap();
        for input in inputs {
            let tick = apply(&mut s, input);
            let record = AppliedTickRecord::from_applied(ZoneId(1), &tick);
            let decoded = AppliedTickRecord::decode(&record.encode()).unwrap();
            assert_eq!(decoded.encode(), record.encode());
            let again = replay
                .run_tick(AppliedTickDraft {
                    epoch: tick.epoch,
                    tick: tick.tick,
                    commands: tick.commands.clone(),
                })
                .unwrap();
            assert!(record.reproduced_by(&AppliedTickRecord::from_applied(ZoneId(1), &again)));
            assert!(record
                .with_output_digests()
                .reproduced_by(&AppliedTickRecord::from_applied(ZoneId(1), &again)));
            replay = ZoneState::from_snapshot(replay.snapshot()).unwrap();
            assert_eq!(s.state_digest(), replay.state_digest());
        }
        assert_eq!(s.snapshot().meta.digest_version, StateDigestVersion::BinaryV3);
    }
    #[test]
    fn restored_identity_and_receipts_cannot_cross_characters() {
        let (mut s, e, a) = fixture(20);
        apply(&mut s, command(e, a, 1, 1));
        let mut snapshot = s.snapshot();
        if let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut snapshot.entities[0].combat.as_mut().unwrap().role
        {
            p.identity.account_id = AccountId::from_uuid(uuid::Uuid::nil());
        }
        assert!(ZoneState::from_snapshot(snapshot).is_err());
    }
    #[test]
    fn transfer_resources_floor_and_preserve_alive_minimum() {
        assert_eq!(transfer_resource(1, 1000, 20, 1), 1);
        assert_eq!(transfer_resource(37, 100, 151, 1), 55);
        assert_eq!(transfer_resource(0, 0, 100, 0), 0);
        assert_eq!(transfer_resource(100, 100, 151, 0), 151);
    }
    #[test]
    fn exact_growth_movement_collision_and_inherited_auto_get_are_used() {
        let (mut s, e, a) = fixture(40);
        let before = s.entities[&e].combat.as_ref().unwrap().sheet;
        let t = apply(&mut s, command(e, a, 1, 1));
        assert!(t.dispositions.is_empty());
        let player = &s.entities[&e];
        let c = player.combat.as_ref().unwrap();
        let p = s.class_player(e, a, SessionGeneration(1)).unwrap();
        let raw = s.classes.as_ref().unwrap().growth(ClassId(1), 40).unwrap();
        let con = s
            .rules
            .as_ref()
            .unwrap()
            .bonus()
            .bonus(StatKind::Con, 43)
            .unwrap();
        assert_eq!(c.sheet.max_hp(), resource_max(raw.hp.raw().into(), con).unwrap());
        assert_eq!(p.max_cp, resource_max(raw.cp.raw().into(), con).unwrap());
        assert_eq!(player.speed.milli_tiles_per_tick(), 359);
        assert_eq!(c.collision_radius.raw(), 281);
        assert_eq!(c.hp, transfer_resource(before.max_hp(), before.max_hp(), c.sheet.max_hp(), 1));
        let expected = crate::domain::character_progression::auto_get_metadata(
            s.classes.as_ref().unwrap(),
            ClassId(1),
            40,
        )
        .unwrap();
        assert_eq!(p.class_state.learned_skills, expected);
    }
    #[test]
    #[allow(clippy::print_stderr)] // Reports the measured artifact size for coordinator acceptance.
    fn full_catalogue_snapshot_fits_compressed_budget_and_expansion_is_bounded() {
        let (s, _, _) = fixture(40);
        let snapshot = s.snapshot();
        let raw = serde_json::to_vec(&snapshot).unwrap();
        let encoded = encode_snapshot(&snapshot).unwrap();
        eprintln!(
            "PHASE2_SNAPSHOT raw={} encoded={} classes={} skill_rows={}",
            raw.len(),
            encoded.len(),
            snapshot.classes.as_ref().unwrap().classes().len(),
            snapshot
                .classes
                .as_ref()
                .unwrap()
                .classes()
                .iter()
                .map(|c| c.skill_tree.len())
                .sum::<usize>()
        );
        assert!(encoded.len() < 1024 * 1024 - 1024);
        assert_eq!(decode_snapshot(&encoded).unwrap(), snapshot);
        let mut bomb = encoded.clone();
        bomb[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_snapshot(&bomb).is_err());
        let mut truncated = encoded.clone();
        truncated.pop();
        assert!(decode_snapshot(&truncated).is_err());
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert!(decode_snapshot(&trailing).is_err());
        let mut dishonest = encoded.clone();
        dishonest[8..12].copy_from_slice(&1_u32.to_le_bytes());
        assert!(decode_snapshot(&dishonest).is_err());
    }
    #[test]
    fn every_playable_branch_enforces_threshold_parent_and_token_and_thirds_stay_metadata() {
        let (catalog, _, _) = fixture(1);
        let registry = catalog.classes.as_ref().unwrap();
        let mut counts = [0; 4];
        for class in registry.classes().iter().filter(|c| c.tier > 0) {
            counts[usize::from(class.tier)] += 1;
            let level = if class.tier == 3 { 85 } else { class.min_level };
            let (mut s, e, a) = fixture_class(level, class.base_class_id);
            let mut parents = registry.ancestors(class.id);
            parents.reverse();
            for parent in parents.into_iter().skip(1) {
                assert!(apply(&mut s, command(e, a, u128::from(parent.0) + 1, parent.0))
                    .dispositions
                    .is_empty());
            }
            if class.tier == 3 {
                assert_eq!(
                    apply(&mut s, command(e, a, 900, class.id.0)).dispositions[0].reason,
                    RejectReason::TransferIneligible
                );
                continue;
            }
            let mut below = s.clone();
            let c = below.entities.get_mut(&e).unwrap().combat.as_mut().unwrap();
            let CombatRole::Player {
                progression: Some(p),
                xp,
                ..
            } = &mut c.role
            else {
                panic!("player")
            };
            *xp = below.rules.as_ref().unwrap().xp_to_level(level).unwrap() - 1;
            c.sheet = profession_stats(
                below.rules.as_ref().unwrap(),
                registry,
                &p.identity,
                &p.class_state,
                level - 1,
            )
            .unwrap()
            .0;
            assert!(below
                .transfer_unmet(e, class.id)
                .contains(&RejectReason::TransferIneligible));
            let mut empty = s.clone();
            let CombatRole::Player {
                progression: Some(p),
                ..
            } = &mut empty
                .entities
                .get_mut(&e)
                .unwrap()
                .combat
                .as_mut()
                .unwrap()
                .role
            else {
                panic!("player")
            };
            p.class_state.token_tier_1_count = 0;
            p.class_state.token_tier_2_count = 0;
            assert!(empty
                .transfer_unmet(e, class.id)
                .contains(&RejectReason::TransferRequirement));
            assert!(
                apply(&mut s, command(e, a, 900, class.id.0))
                    .dispositions
                    .is_empty(),
                "class {}",
                class.id.0
            );
            assert_eq!(
                s.class_player(e, a, SessionGeneration(1))
                    .unwrap()
                    .class_state
                    .current_class_id,
                class.id
            );
        }
        assert_eq!(counts, [0, 18, 31, 31]);
    }

    #[test]
    fn dormant_rule_changes_bind_digest_and_forged_restore_hash_is_rejected() {
        let (s, _, _) = fixture(40);
        let mut json = serde_json::to_value(s.classes.as_deref().unwrap()).unwrap();
        let class = json["classes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|c| c["id"] == 55)
            .unwrap();
        let learn = class["skill_tree"]
            .as_array_mut()
            .unwrap()
            .first_mut()
            .unwrap();
        learn["auto_get"] = serde_json::Value::Bool(!learn["auto_get"].as_bool().unwrap());
        let changed: ClassRegistry = serde_json::from_value(json).unwrap();
        let mut forged = s.snapshot();
        forged.classes = Some(Arc::new(changed.clone()));
        assert!(ZoneState::from_snapshot(forged).is_err());
        let changed = s.clone().with_classes(Arc::new(changed)).unwrap();
        assert_ne!(s.meta.classes_rules_hash, changed.meta.classes_rules_hash);
        assert_ne!(s.state_digest(), changed.state_digest());
        assert_eq!(s.meta.classes_hash, changed.meta.classes_hash);
        let snap = s.snapshot();
        assert!(Arc::ptr_eq(s.classes.as_ref().unwrap(), snap.classes.as_ref().unwrap()));
    }

    #[test]
    fn elf_source_run_and_evasion_and_dark_elf_critical_floor_have_independent_oracles() {
        let (s, e, _) = fixture_class(40, ClassId(18));
        assert_eq!(s.entities[&e].speed.milli_tiles_per_tick(), 400);
        let rules = s.rules.as_ref().unwrap();
        let legacy = StatSheet::for_player(
            rules,
            rules.class("elven_fighter").unwrap(),
            40,
            Some(rules.starter_weapon()),
        )
        .unwrap();
        let expected =
            ((legacy.evasion().raw() * 103 + 50_000_000) / 100_000_000).min(200) * 1_000_000;
        assert_eq!(
            s.entities[&e]
                .combat
                .as_ref()
                .unwrap()
                .sheet
                .evasion()
                .raw(),
            expected
        );
        let (dark, de, _) = fixture_class(40, ClassId(31));
        let sheet = dark.entities[&de].combat.as_ref().unwrap().sheet;
        let target = legacy;
        for spread in [-10, 0, 10] {
            let expected =
                (76_i128 * i128::from(sheet.p_atk().raw()) * 2 * (100 + i128::from(spread)) * 105
                    / (i128::from(target.p_def().raw()) * 100 * 100))
                    .max(1);
            assert_eq!(
                super::super::super::combat_math::physical_damage_with_critical_bonus(
                    rules.constants(),
                    &sheet,
                    &target,
                    true,
                    spread,
                    105
                )
                .unwrap(),
                u32::try_from(expected).unwrap()
            );
            assert_eq!(
                super::super::super::combat_math::physical_damage_with_critical_bonus(
                    rules.constants(),
                    &sheet,
                    &target,
                    false,
                    spread,
                    105
                )
                .unwrap(),
                physical_damage(rules.constants(), &sheet, &target, false, spread).unwrap()
            );
        }
    }

    #[test]
    fn human_keltir_reward_is_29_and_real_crossings_learn_metadata_without_token_grants() {
        for level in [19, 39] {
            let (mut s, e, a) = fixture(level);
            let next_xp = s.rules.as_ref().unwrap().xp_to_level(level + 1).unwrap();
            let CombatRole::Player {
                xp,
                progression: Some(p),
                ..
            } = &mut s
                .entities
                .get_mut(&e)
                .unwrap()
                .combat
                .as_mut()
                .unwrap()
                .role
            else {
                panic!("player")
            };
            *xp = next_xp - 29;
            p.class_state.token_tier_1_count = 0;
            p.class_state.token_tier_2_count = 0;
            let zone = crate::infrastructure::zone_data::parse_zone(
                crate::infrastructure::zone_data::TEST_ZONE_TOML,
            )
            .unwrap();
            let mut npc =
                NpcCombat::from_template(s.rules.as_ref().unwrap(), &zone.npc_templates[0])
                    .unwrap();
            assert_eq!(npc.xp_reward, 28);
            npc.stats.max_hp = 1;
            apply(
                &mut s,
                ZoneInput::system(ZoneCommand::SpawnNpc {
                    name: "Keltir".into(),
                    pos: Vec2Fixed::from_tiles(126, 127),
                    speed: Speed::DEFAULT,
                    combat: Some(Box::new(npc)),
                }),
            );
            let npc = s
                .entities
                .values()
                .find(|other| other.kind == EntityKind::Npc)
                .unwrap()
                .id;
            apply(
                &mut s,
                ZoneInput::session(
                    e,
                    SessionGeneration(1),
                    1,
                    ZoneCommand::SetTarget {
                        entity: e,
                        target: Some(npc),
                    },
                ),
            );
            let mut tick = apply(
                &mut s,
                ZoneInput::session(e, SessionGeneration(1), 2, ZoneCommand::Attack { entity: e }),
            );
            let mut reward = None;
            for _ in 0..100 {
                if let Some(amount) = tick.events.iter().find_map(|event| {
                    if let ZoneEvent::XpGained { amount, .. } = event {
                        Some(*amount)
                    } else {
                        None
                    }
                }) {
                    reward = Some(amount);
                    break;
                }
                tick = s.run_tick(s.draft(vec![])).unwrap();
            }
            assert_eq!(reward, Some(29));
            let c = s.entities[&e].combat.as_ref().unwrap();
            assert_eq!(c.sheet.level(), level + 1);
            let p = s.class_player(e, a, SessionGeneration(1)).unwrap();
            assert!(!p.class_state.learned_skills.is_empty());
            assert_eq!(p.class_state.token_tier_1_count, 0);
            assert_eq!(p.class_state.token_tier_2_count, 0);
            assert_eq!(p.class_state.milestone_claimed_mask, 0);
        }
    }
}
