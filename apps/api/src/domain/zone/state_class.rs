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
        self.meta.schema_version = 8;
        self.meta.digest_version = StateDigestVersion::BinaryV4;
        Ok(self)
    }

    /// Enables approved once-ever token supply only on an empty new epoch.
    /// Recovery must replay its old policy before bootstrap creates this boundary.
    pub fn with_token_policy(mut self) -> Result<Self, SnapshotError> {
        if self.classes.is_none() || self.next_tick != Tick(0) || !self.entities.is_empty() {
            return Err(SnapshotError::CombatMismatch);
        }
        self.meta.schema_version = 8;
        self.meta.digest_version = StateDigestVersion::BinaryV4;
        Ok(self)
    }

    pub(super) fn reconcile_player_tokens(
        &mut self,
        tick: Tick,
        entity: EntityId,
        crossing_from: Option<u32>,
        events: &mut Vec<ZoneEvent>,
    ) {
        if self.meta.digest_version != StateDigestVersion::BinaryV4 {
            return;
        }
        let Some(registry) = self.classes.as_deref() else {
            return;
        };
        let Some(combat) = self
            .entities
            .get_mut(&entity)
            .and_then(|e| e.combat.as_mut())
        else {
            return;
        };
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut combat.role
        else {
            return;
        };
        // The entire ledger was validated at admission/restore and every mutation preserves
        // it. Validation here fails closed by retaining an unchanged ledger.
        let Ok(adjustment) = p.class_state.reconcile_tokens(
            registry,
            &p.identity,
            CharacterId::from_uuid(entity.as_uuid()),
            combat.sheet.level(),
            crossing_from,
        ) else {
            return;
        };
        if adjustment.claimed_mask != 0 {
            events.push(ZoneEvent::TokensReconciled {
                tick,
                entity,
                adjustment,
                source: if crossing_from.is_some() {
                    crate::domain::character_progression::TokenSource::LevelUp
                } else {
                    crate::domain::character_progression::TokenSource::Admission
                },
            });
        }
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

    /// Concrete failed predicates for the catalogue UI; command refusal codes stay stable.
    pub fn transfer_unmet_details(&self, entity: EntityId, target: ClassId) -> Vec<String> {
        let Some(actor) = self.entities.get(&entity) else {
            return vec!["Character is not online".into()];
        };
        let Some(combat) = actor.combat.as_ref() else {
            return vec!["Character progression is unavailable".into()];
        };
        let Some(registry) = self.classes.as_deref() else {
            return vec!["Class catalogue is unavailable".into()];
        };
        let Some(class) = registry.get(target) else {
            return vec!["Unknown class".into()];
        };
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &combat.role
        else {
            return vec!["Character progression is unavailable".into()];
        };
        let mut details = Vec::new();
        for reason in self.transfer_unmet(entity, target) {
            if reason == RejectReason::TransferIneligible {
                if class.tier > 2 {
                    details.push("Third transfers are not available".into());
                }
                if combat.sheet.level() < class.min_level {
                    details.push(format!(
                        "Requires level {} (current {})",
                        class.min_level,
                        combat.sheet.level()
                    ));
                }
                if class.parent != Some(player.class_state.current_class_id) {
                    let parent = class
                        .parent
                        .and_then(|id| registry.get(id))
                        .map_or("a different base class", |c| c.display_name.as_str());
                    details.push(format!("Requires current class {parent}"));
                }
                if class.race != player.identity.race {
                    details.push("This class is unavailable for your race".into());
                } else if class.base_class_id != player.identity.base_class_id {
                    let base = registry
                        .get(class.base_class_id)
                        .map_or("another starting class", |c| c.display_name.as_str());
                    details.push(format!("Requires starting class {base}"));
                }
            } else if reason == RejectReason::ClassMasterTooFar {
                let distance = actor
                    .pos
                    .distance_sq(Vec2Fixed::from_tiles(126, 128))
                    .isqrt();
                details.push(format!(
                    "Move within 3 tiles of Class Master (126, 128); currently {}.{:03} tiles away",
                    distance / 1000,
                    distance % 1000
                ));
            } else if reason == RejectReason::TransferRequirement && class.tier <= 2 {
                let count = if class.tier == 1 {
                    player.class_state.token_tier_1_count
                } else {
                    player.class_state.token_tier_2_count
                };
                if count == 0 {
                    details.push(format!(
                        "Requires 1 tier {} transfer token (current {count})",
                        class.tier
                    ));
                }
                if !class.transfer.quest_hooks.is_empty() {
                    details.push("Required transfer quest is not available yet".into());
                }
                let expected = if class.tier == 1 {
                    "class_transfer_token_1"
                } else {
                    "class_transfer_token_2"
                };
                if class.transfer.requires.len() != 1
                    || class
                        .transfer
                        .requires
                        .iter()
                        .any(|r| r.item != expected || r.count != 1)
                {
                    details.push("Required transfer items are not available yet".into());
                }
            } else if reason != RejectReason::TransferRequirement {
                details.push(reason.detail().to_owned());
            }
        }
        details
    }

    /// Ordered direct children, including locked third-tier metadata.
    pub fn class_options(&self, entity: EntityId) -> Vec<(ClassId, Vec<String>)> {
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
                    .map(|id| (id, self.transfer_unmet_details(entity, id)))
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
        class_state.milestone_claimed_mask = 3;
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
        assert_eq!(s.snapshot().meta.digest_version, StateDigestVersion::BinaryV4);
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
            p.class_state.milestone_claimed_mask = 0;
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
    fn human_keltir_real_crossings_supply_only_in_new_policy_and_learn_metadata() {
        for (level, enabled) in [(19, false), (39, false), (19, true), (39, true)] {
            let (mut s, e, a) = fixture(level);
            if !enabled {
                s.meta.digest_version = StateDigestVersion::BinaryV3;
                s.meta.schema_version = 7;
            }
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
            p.class_state.milestone_claimed_mask = 0;
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
            assert_eq!(p.class_state.token_tier_1_count, u32::from(enabled && level == 19));
            assert_eq!(p.class_state.token_tier_2_count, u32::from(enabled && level == 39));
            assert_eq!(
                p.class_state.milestone_claimed_mask,
                if enabled {
                    if level == 19 {
                        1
                    } else {
                        2
                    }
                } else {
                    0
                }
            );
            let before = p.class_state.clone();
            for _ in 0..20 {
                s.run_tick(s.draft(vec![])).unwrap();
            }
            assert_eq!(
                s.class_player(e, a, SessionGeneration(1))
                    .unwrap()
                    .class_state,
                before
            );
        }
    }
    #[test]
    fn options_explain_only_failed_requirements_with_levels_tokens_and_master_distance() {
        let (mut s, e, a) = fixture(1);
        let actor = s.entities.get_mut(&e).unwrap();
        actor.pos = Vec2Fixed::from_tiles(0, 0);
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut actor.combat.as_mut().unwrap().role
        else {
            panic!("player")
        };
        p.class_state.token_tier_1_count = 0;
        assert_eq!(
            s.transfer_unmet_details(e, ClassId(1)),
            vec![
                "Requires level 20 (current 1)",
                "Move within 3 tiles of Class Master (126, 128); currently 179.610 tiles away",
                "Requires 1 tier 1 transfer token (current 0)",
            ]
        );
        assert!(s
            .transfer_unmet_details(e, ClassId(2))
            .iter()
            .any(|detail| detail == "Requires current class Steel Initiate"));
        let female = crate::domain::character_progression::CharacterIdentity {
            appearance: CharacterAppearance {
                sex: crate::domain::subclass::Sex::Female,
                ..Default::default()
            },
            ..s.class_player(e, a, SessionGeneration(1))
                .unwrap()
                .identity
                .clone()
        };
        assert_eq!(
            profession_stats(
                s.rules.as_ref().unwrap(),
                s.classes.as_ref().unwrap(),
                &female,
                &ClassState::new(ClassId(0)),
                1
            )
            .unwrap()
            .3
            .raw(),
            250
        );
        let (ready, e, _) = fixture(20);
        assert!(ready.transfer_unmet_details(e, ClassId(1)).is_empty());
    }
    #[test]
    fn transfer_identity_is_public_owner_resources_are_private_and_late_aoi_gets_current_class() {
        let (mut state, owner, account) = fixture(40);
        let mut identity = state
            .class_player(owner, account, SessionGeneration(1))
            .unwrap()
            .identity
            .clone();
        identity.account_id = AccountId::from_uuid(uuid::Uuid::from_u128(4));
        let observer = EntityId::from_uuid(uuid::Uuid::from_u128(3));
        let spawn = |entity, position| {
            ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity,
                name: "Observer".into(),
                pos: position,
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(PlayerLoad {
                    progression: Some(Box::new(PlayerProgression {
                        identity: identity.clone(),
                        class_state: ClassState::new(ClassId(0)),
                        max_cp: 0,
                    })),
                    ..PlayerLoad::fresh("human_fighter")
                })),
            })
        };
        apply(&mut state, spawn(observer, Vec2Fixed::from_tiles(126, 126)));
        let changed = apply(&mut state, command(owner, account, 1, 1));
        assert!(changed.outputs[&observer].iter().any(|out| matches!(out,
            ObserverOutput::Event(ZoneEvent::ClassChanged { entity, class_id: ClassId(1), .. }) if *entity == owner)));
        assert!(!changed.outputs[&observer].iter().any(|out| matches!(out,
            ObserverOutput::Event(ZoneEvent::StatsChanged { entity, .. }) if *entity == owner)));
        assert!(changed.outputs[&owner].iter().any(|out| matches!(out,
            ObserverOutput::Event(ZoneEvent::StatsChanged { class: Some(class), .. }) if class.class_id == ClassId(1) && class.token_tier_1_count == 0)));
        assert!(changed
            .outputs
            .values()
            .flatten()
            .all(|out| !matches!(out, ObserverOutput::Event(ZoneEvent::ClassTransfer { .. }))));
        let late = EntityId::from_uuid(uuid::Uuid::from_u128(5));
        let entered = apply(&mut state, spawn(late, Vec2Fixed::from_tiles(126, 126)));
        assert!(entered.outputs[&late].iter().any(|out| matches!(out,
            ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, identity: Some(public), .. }) if *entity == owner && public.class_id == ClassId(1))));
        let reconnected = apply(
            &mut state,
            ZoneInput::system(ZoneCommand::ReplaceSession {
                entity: owner,
                generation: SessionGeneration(2),
            }),
        );
        assert!(reconnected.outputs[&owner].iter().any(|out| matches!(out,
            ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, identity: Some(public), .. }) if *entity == owner && public.class_id == ClassId(1))));
    }
    #[test]
    fn token_admission_and_replay_preserve_vitals_and_legacy_bytes() {
        use crate::application::replay_log::{decode_snapshot, encode_snapshot, AppliedTickRecord};
        let (legacy, e, a) = fixture(40);
        let p = legacy.class_player(e, a, SessionGeneration(1)).unwrap();
        let mut ledger = p.class_state.clone();
        ledger.token_tier_1_count = 0;
        ledger.token_tier_2_count = 0;
        ledger.milestone_claimed_mask = 0;
        for (enabled, balance) in [(false, 0), (true, 0), (true, 7)] {
            ledger.token_tier_1_count = balance;
            ledger.token_tier_2_count = balance;
            let mut state = ZoneState::new(legacy.seed(), legacy.bounds(), 0)
                .with_rules(legacy.rules.clone().unwrap())
                .with_classes(legacy.classes.clone().unwrap())
                .unwrap();
            if !enabled {
                state.meta.digest_version = StateDigestVersion::BinaryV3;
                state.meta.schema_version = 7;
            }
            let initial = state.snapshot();
            let encoded = encode_snapshot(&initial).unwrap();
            assert_eq!(encode_snapshot(&decode_snapshot(&encoded).unwrap()).unwrap(), encoded);
            let mut replay = ZoneState::from_snapshot(decode_snapshot(&encoded).unwrap()).unwrap();
            let load = PlayerLoad {
                progression: Some(Box::new(PlayerProgression {
                    identity: p.identity.clone(),
                    class_state: ledger.clone(),
                    max_cp: 0,
                })),
                checkpoint_revision: Some(0),
                level: 40,
                xp: state.rules.as_ref().unwrap().xp_to_level(40).unwrap(),
                hp: Some(17),
                mp: Some(9),
                ..PlayerLoad::fresh("human_fighter")
            };
            let draft = state.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity: e,
                name: "Tester".into(),
                pos: Vec2Fixed::from_tiles(126, 126),
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(load)),
            })]);
            let applied = state.run_tick(draft.clone()).unwrap();
            let repeated = replay.run_tick(draft).unwrap();
            let record = AppliedTickRecord::from_applied(state.seed().zone, &applied);
            assert!(record
                .reproduced_by(&AppliedTickRecord::from_applied(state.seed().zone, &repeated)));
            assert_eq!(
                record.encode(),
                AppliedTickRecord::decode(&record.encode())
                    .unwrap()
                    .encode()
            );
            let combat = state.entities[&e].combat.as_ref().unwrap();
            assert_eq!((combat.hp, combat.mp, combat.sheet.level()), (17, 9, 40));
            let p = state.class_player(e, a, SessionGeneration(1)).unwrap();
            assert_eq!(p.class_state.milestone_claimed_mask, if enabled { 3 } else { 0 });
            assert_eq!(p.class_state.token_tier_1_count, balance.max(u32::from(enabled)));
            assert_eq!(p.class_state.token_tier_2_count, balance.max(u32::from(enabled)));
            let adjustments: Vec<_> = applied
                .events
                .iter()
                .filter_map(|event| {
                    if let ZoneEvent::TokensReconciled { adjustment, .. } = event {
                        Some((adjustment.claimed_mask, adjustment.granted_mask))
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                adjustments,
                if enabled {
                    vec![(3, if balance == 0 { 3 } else { 0 })]
                } else {
                    vec![]
                }
            );
            assert!(applied
                .outputs
                .values()
                .flatten()
                .all(|o| !matches!(o, ObserverOutput::Event(ZoneEvent::TokensReconciled { .. }))));
            let reconnected = apply(
                &mut state,
                ZoneInput::system(ZoneCommand::ReplaceSession {
                    entity: e,
                    generation: SessionGeneration(2),
                }),
            );
            assert!(!reconnected
                .events
                .iter()
                .any(|e| matches!(e, ZoneEvent::TokensReconciled { .. })));
        }
    }
    fn kill_token_npc(s: &mut ZoneState, e: EntityId, reward: u64) -> AppliedTick {
        let zone = crate::infrastructure::zone_data::parse_zone(
            crate::infrastructure::zone_data::TEST_ZONE_TOML,
        )
        .unwrap();
        let mut npc =
            NpcCombat::from_template(s.rules.as_ref().unwrap(), &zone.npc_templates[0]).unwrap();
        npc.stats.max_hp = 1;
        npc.xp_reward = reward;
        let spawn = apply(
            s,
            ZoneInput::system(ZoneCommand::SpawnNpc {
                name: "Tokenvictim".into(),
                pos: s.entities[&e].pos,
                speed: Speed::DEFAULT,
                combat: Some(Box::new(npc)),
            }),
        );
        let npc = spawn
            .events
            .iter()
            .find_map(|event| {
                if let ZoneEvent::EntitySpawn {
                    entity,
                    kind: EntityKind::Npc,
                    ..
                } = event
                {
                    Some(*entity)
                } else {
                    None
                }
            })
            .unwrap();
        apply(
            s,
            ZoneInput::session(
                e,
                SessionGeneration(1),
                100,
                ZoneCommand::SetTarget {
                    entity: e,
                    target: Some(npc),
                },
            ),
        );
        let mut tick = apply(
            s,
            ZoneInput::session(e, SessionGeneration(1), 101, ZoneCommand::Attack { entity: e }),
        );
        for _ in 0..100 {
            if tick
                .events
                .iter()
                .any(|event| matches!(event, ZoneEvent::XpGained {entity,..} if *entity==e))
            {
                return tick;
            }
            tick = s.run_tick(s.draft(vec![])).unwrap();
        }
        panic!("seeded lethal impact never landed");
    }

    #[test]
    fn combat_milestones_survive_actual_death_respawn_and_relevel_without_refill() {
        // Independent source oracle: token-native-xp-learning-oracle.json, pinned L2J source.
        for (level, initial, reward, after_kill, after_death, after_second, bit) in [
            (19, 835_861, 10_235, 846_607, 832_278, 843_024, 1),
            (39, 15_422_928, 62_751, 15_488_816, 15_400_965, 15_466_853, 2),
        ] {
            let (mut s, e, a) = fixture(level);
            if let CombatRole::Player {
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
            {
                *xp = initial;
                p.class_state.token_tier_1_count = 0;
                p.class_state.token_tier_2_count = 0;
                p.class_state.milestone_claimed_mask = u8::from(level == 39);
            }
            let first = kill_token_npc(&mut s, e, reward);
            assert!(first.events.iter().any(|event| matches!(event,
                ZoneEvent::TokensReconciled {adjustment,..} if adjustment.granted_mask==bit)));
            let xp_of = |s: &ZoneState| match s.entities[&e].combat.as_ref().unwrap().role {
                CombatRole::Player { xp, .. } => xp,
                CombatRole::Npc { .. } => panic!("player"),
            };
            assert_eq!(xp_of(&s), after_kill);
            // Consume through the actual transfer command (at the Master, out of combat).
            apply(
                &mut s,
                ZoneInput::session(
                    e,
                    SessionGeneration(1),
                    102,
                    ZoneCommand::StopAttack { entity: e },
                ),
            );
            for _ in 0..20 {
                s.run_tick(s.draft(vec![])).unwrap();
            }
            if level == 19 {
                assert!(apply(&mut s, command(e, a, 44, 1)).dispositions.is_empty());
            } else {
                // Tier two pending on base profession: spending in a later transfer cannot refill it.
                if let CombatRole::Player {
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
                {
                    p.class_state.token_tier_2_count = 0;
                }
            }
            let receipts = serde_json::to_vec(
                &s.class_player(e, a, SessionGeneration(1))
                    .unwrap()
                    .class_state
                    .successful_transfer_receipts,
            )
            .unwrap();
            let mut deaths = Vec::new();
            s.kill(s.next_tick(), e, None, &mut deaths);
            assert_eq!(xp_of(&s), after_death);
            assert_eq!(s.entities[&e].combat.as_ref().unwrap().sheet.level(), level);
            let respawn = apply(
                &mut s,
                ZoneInput::session(
                    e,
                    SessionGeneration(1),
                    103,
                    ZoneCommand::Respawn { entity: e },
                ),
            );
            assert!(respawn.dispositions.is_empty());
            assert_eq!(xp_of(&s), after_death);
            let second = kill_token_npc(&mut s, e, reward);
            assert_eq!(xp_of(&s), after_second);
            assert!(!second
                .events
                .iter()
                .any(|e| matches!(e, ZoneEvent::TokensReconciled { .. })));
            let p = s.class_player(e, a, SessionGeneration(1)).unwrap();
            assert_eq!(
                (p.class_state.token_tier_1_count, p.class_state.token_tier_2_count),
                (0, 0)
            );
            assert_eq!(p.class_state.milestone_claimed_mask, if level == 19 { 1 } else { 3 });
            assert_eq!(
                serde_json::to_vec(&p.class_state.successful_transfer_receipts).unwrap(),
                receipts
            );
        }
    }

    #[test]
    fn one_combat_kill_crosses_both_milestones_and_repeated_kills_at_cap_do_not_refill() {
        let (mut s, e, a) = fixture(19);
        if let CombatRole::Player {
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
        {
            *xp = 835_861;
            p.class_state.token_tier_1_count = 0;
            p.class_state.token_tier_2_count = 0;
            p.class_state.milestone_claimed_mask = 0;
        }
        let jump = kill_token_npc(&mut s, e, 13_892_446);
        assert_eq!(jump.progression().next().unwrap().xp, 15_422_929);
        assert_eq!(jump.progression().next().unwrap().levels_gained, (20..=40).collect::<Vec<_>>());
        assert!(jump.events.iter().any(|event| matches!(event,ZoneEvent::TokensReconciled{adjustment,..} if adjustment.granted_mask==3)));
        let cap = kill_token_npc(&mut s, e, u64::MAX);
        assert_eq!(cap.progression().next().unwrap().level, 85);
        assert!(!cap
            .events
            .iter()
            .any(|event| matches!(event, ZoneEvent::TokensReconciled { .. })));
        for _ in 0..2 {
            let repeated = kill_token_npc(&mut s, e, u64::MAX);
            assert!(!repeated
                .events
                .iter()
                .any(|event| matches!(event, ZoneEvent::TokensReconciled { .. })));
        }
        let p = s.class_player(e, a, SessionGeneration(1)).unwrap();
        assert_eq!(
            (
                p.class_state.milestone_claimed_mask,
                p.class_state.token_tier_1_count,
                p.class_state.token_tier_2_count
            ),
            (3, 1, 1)
        );
    }
    #[test]
    fn snapshot_schema_digest_and_catalogue_pairs_are_validated_at_every_boundary() {
        let (state, _, _) = fixture(40);
        let valid = state.snapshot();
        for schema in 4..=8 {
            for digest in [
                StateDigestVersion::JsonV1,
                StateDigestVersion::BinaryV2,
                StateDigestVersion::BinaryV3,
                StateDigestVersion::BinaryV4,
            ] {
                let mut candidate = valid.clone();
                candidate.meta.schema_version = schema;
                candidate.meta.digest_version = digest;
                let accepted = matches!(
                    (schema, digest),
                    (7, StateDigestVersion::BinaryV3) | (8, StateDigestVersion::BinaryV4)
                );
                assert_eq!(ZoneState::from_snapshot(candidate.clone()).is_ok(), accepted);
                assert_eq!(encode_snapshot(&candidate).is_ok(), accepted);
                let raw = serde_json::to_vec(&candidate).unwrap();
                assert_eq!(decode_snapshot(&raw).is_ok(), accepted);
                if accepted {
                    let bytes = encode_snapshot(&candidate).unwrap();
                    assert_eq!(encode_snapshot(&decode_snapshot(&bytes).unwrap()).unwrap(), bytes);
                    let mut wrong_envelope = bytes;
                    wrong_envelope[6] = if schema == 7 { b'8' } else { b'7' };
                    assert!(decode_snapshot(&wrong_envelope).is_err());
                }
            }
        }
        let legacy = ZoneState::new(state.seed(), state.bounds(), 0).snapshot();
        for schema in 4..=8 {
            for digest in [
                StateDigestVersion::JsonV1,
                StateDigestVersion::BinaryV2,
                StateDigestVersion::BinaryV3,
                StateDigestVersion::BinaryV4,
            ] {
                let mut candidate = legacy.clone();
                candidate.meta.schema_version = schema;
                candidate.meta.digest_version = digest;
                let accepted = matches!(digest, StateDigestVersion::JsonV1)
                    || schema >= 6 && digest == StateDigestVersion::BinaryV2;
                assert_eq!(ZoneState::from_snapshot(candidate.clone()).is_ok(), accepted);
                assert_eq!(encode_snapshot(&candidate).is_ok(), accepted);
                assert_eq!(
                    decode_snapshot(&serde_json::to_vec(&candidate).unwrap()).is_ok(),
                    accepted
                );
            }
        }
    }
}
