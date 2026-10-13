//! Real sockets consume actual V5 life and hydration producers behind the durable actor.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod common;
#[path = "common/hydration.rs"]
mod hydration_fixture;

use std::{future::Future, sync::Arc, time::Duration};

use api::application::checkpoint::CheckpointService;
use api::application::ports::CharacterRepository;
use api::application::replay_log::{start_epoch, DurableTickGate, GateConfig, NoReplayMetrics};
use api::application::zone_actor::{manual_ticks, ManualTickDriver, ZoneActor};
use api::application::{CharacterCheckpoint, IdempotencyKey};
use api::domain::character_progression::ClassState;
use api::domain::class::ClassId;
use api::domain::zone::*;
use api::domain::{AccountId, Character, CharacterId, CharacterName, Position, Race};
use api::infrastructure::class_data::{load_classes, ClassSource};
use api::infrastructure::eventlog::{InMemoryEventLog, Recording};
use api::infrastructure::memory::{InMemoryCharacterRepository, InMemorySessionAudit};
use api::infrastructure::rules_data::{load_rules, RulesSource};
use api::infrastructure::skill_data::{load_phase3_rules, Phase3DataSource};
use api::infrastructure::telemetry::Metrics;
use api::interface::grpc::pb::{
    self, client_message::Intent, server_message::Payload, world_event::Event,
};
use common::{ws, TestApp};
use nightfall_api as api;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

async fn drive<T>(driver: &ManualTickDriver, future: impl Future<Output = T>) -> T {
    tokio::pin!(future);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            tokio::select! {
                biased;
                result = &mut future => return result,
                () = tokio::time::sleep(Duration::from_millis(2)) => {
                    driver.step().await.unwrap();
                }
            }
        }
    })
    .await
    .unwrap()
}

fn player(c: &Character) -> ws::Player {
    ws::Player {
        account: c.account_id.as_uuid(),
        character: c.id,
        name: c.name.to_string(),
    }
}

fn event(message: &pb::ServerMessage) -> Option<&Event> {
    match &message.payload {
        Some(Payload::Event(world)) => world.event.as_ref(),
        _ => None,
    }
}

async fn join(
    app: &TestApp,
    driver: &ManualTickDriver,
    c: &Character,
) -> (ws::Ws, Vec<pb::ServerMessage>) {
    let mut socket = ws::join(app, &player(c)).await;
    let frames = drive(driver, socket.until(|m| matches!(event(m), Some(Event::TargetChanged(t)) if t.entity == c.id.to_string() && t.baseline))).await;
    (socket, frames)
}

async fn intent(
    socket: &mut ws::Ws,
    driver: &ManualTickDriver,
    seq: u32,
    intent: Intent,
) -> Vec<pb::ServerMessage> {
    socket
        .send(&pb::ClientMessage {
            seq,
            intent: Some(intent),
        })
        .await;
    drive(
        driver,
        socket.until(|m| matches!(&m.payload, Some(Payload::Ack(a)) if a.seq == seq)),
    )
    .await
}

fn private_absent(frames: &[pb::ServerMessage], owner: CharacterId) {
    let owner = owner.to_string();
    for frame in frames {
        match event(frame) {
            Some(Event::StatsChanged(e)) => assert_ne!(e.entity, owner),
            Some(Event::SkillReuse(e)) => assert_ne!(e.entity, owner),
            Some(Event::SkillLearned(e)) => assert_ne!(e.entity, owner),
            Some(Event::SkillStateSync(e)) => assert_ne!(e.entity, owner),
            Some(Event::SpGained(e)) => assert_ne!(e.entity, owner),
            Some(Event::TargetChanged(e)) => assert_ne!(e.entity, owner),
            Some(Event::ResurrectOffer(e)) => assert_ne!(e.target, owner),
            _ => {},
        }
    }
}

async fn app(repo: Arc<InMemoryCharacterRepository>) -> (TestApp, ManualTickDriver) {
    let state = ZoneState::new_phase3(
        ZoneSeed {
            zone: ZoneId(93),
            epoch: 1,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        load_rules(&RulesSource::embedded()).unwrap().rules,
        load_classes(&ClassSource::embedded()).unwrap().registry,
        load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap(),
    )
    .unwrap();
    let log = Arc::new(InMemoryEventLog::default());
    let started = start_epoch(log.as_ref(), &state.snapshot()).await.unwrap();
    let gate = DurableTickGate::new(
        &started,
        log,
        Arc::new(NoReplayMetrics),
        GateConfig::default(),
        CancellationToken::new(),
    );
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn_gated(state, ticks, gate);
    let metrics = Metrics::detached();
    zone.checkpoints.install(
        CheckpointService::new(
            repo.clone(),
            Arc::new(InMemorySessionAudit::default()),
            Arc::new(metrics.clone()),
        )
        .with_learning(repo.clone()),
    );
    (TestApp::spawn_experimental_phase3(repo, zone, metrics).await, driver)
}

async fn ordinary(repo: &InMemoryCharacterRepository, name: &str, x: f32) -> Character {
    let mut c = Character::create(
        AccountId::from_uuid(Uuid::now_v7()),
        CharacterName::new(name).unwrap(),
        Race::Human,
    );
    c.class_state = ClassState::new(ClassId(10));
    c.stats = load_classes(&ClassSource::embedded())
        .unwrap()
        .registry
        .get(ClassId(10))
        .unwrap()
        .base_stats;
    c.level = 10;
    c.xp = load_rules(&RulesSource::embedded())
        .unwrap()
        .rules
        .xp_to_level(10)
        .unwrap();
    c.position = Position { x, y: 10.0 };
    c.class_state.cp = 50;
    repo.insert_for_test(c.clone());
    let mut phase3 = Phase3Checkpoint::new();
    phase3.karma = 7;
    phase3.pk_count = 3;
    phase3.pvp_count = 4;
    phase3.flag_remaining_ticks = 600;
    repo.checkpoint(
        &CharacterCheckpoint {
            phase4: None,
            character_id: c.id,
            revision_seen: 0,
            level: c.level,
            xp: c.xp,
            hp: 100,
            mp: 100,
            alive: true,
            position: c.position,
            idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
            class_state: Some(c.class_state.clone()),
            phase3: Some(phase3),
        },
        &[],
    )
    .await
    .unwrap();
    c
}

fn retained_root(rules: &Phase3Rules) -> effects::RetainedEffect {
    use skill::{RankValue, SkillId, SkillRef};
    let skill = rules.skills().get(SkillId(110)).unwrap();
    let abnormal = skill.abnormal.as_ref().unwrap();
    let flag = |value: &RankValue<bool>| *value.at(1, skill.max_level).unwrap();
    effects::RetainedEffect {
        skill: SkillRef {
            id: skill.id,
            rank: 1,
        },
        source: effects::EffectSource {
            binding: None,
            kind: effects::EffectSourceKind::System,
        },
        target_incarnation: 1,
        pool: effects::EffectPool::Buff,
        abnormal_type: abnormal.r#type.clone(),
        abnormal_level: abnormal.level[0],
        remaining_ticks: u64::try_from(abnormal.time_s[0])
            .unwrap()
            .checked_mul(1000)
            .unwrap()
            .checked_div(TICK_MS)
            .unwrap(),
        flags: effects::EffectFlags {
            stay_after_death: flag(&skill.stay_after_death),
            stay_on_subclass_change: flag(&skill.stay_on_subclass_change),
            removed_on_damage: flag(&skill.removed_on_damage),
            removed_on_action_except_move: flag(&skill.removed_on_action_except_move),
            irreplaceable: flag(&skill.irreplaceable_buff),
        },
        handlers: skill
            .effects
            .iter()
            .enumerate()
            .filter(|(_, handler)| effects::continuous_handler(&handler.name))
            .map(|(ordinal, _)| effects::RetainedHandler {
                xml_ordinal: u32::try_from(ordinal).unwrap(),
                periodic: None,
            })
            .collect(),
    }
}

#[tokio::test]
async fn socket_identity_has_final_status_before_effect_hydration() {
    let rules = Arc::new(load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap());
    let repo = Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules.clone()));
    let rooted = ordinary(&repo, "RootedMage", 10.0).await;
    let viewer = ordinary(&repo, "RootViewer", 11.0).await;
    let stored = repo.load_for_admission(rooted.id).await.unwrap().unwrap();
    let mut phase3 = stored.phase3.unwrap();
    phase3.effect_remainders.push(retained_root(&rules));
    phase3
        .modifier_install_order
        .push(effects::RetainedModifierSource::Effect { effect_ordinal: 0 });
    repo.checkpoint(
        &CharacterCheckpoint {
            phase4: None,
            character_id: rooted.id,
            revision_seen: stored.revision,
            level: stored.level,
            xp: stored.xp,
            hp: stored.hp.unwrap(),
            mp: stored.mp.unwrap(),
            alive: stored.alive,
            position: stored.position,
            idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
            class_state: Some(stored.class_state),
            phase3: Some(phase3),
        },
        &[],
    )
    .await
    .unwrap();
    let (app, driver) = app(repo).await;
    let (_owner, owner_frames) = join(&app, &driver, &rooted).await;
    let (_observer, observer_frames) = join(&app, &driver, &viewer).await;
    for frames in [&owner_frames, &observer_frames] {
        let identity = frames.iter().position(|m| matches!(event(m), Some(Event::Spawn(s)) if s.entity_id == rooted.id.to_string())).unwrap();
        let Some(Event::Spawn(spawn)) = event(&frames[identity]) else {
            unreachable!()
        };
        assert_eq!(spawn.status_flags, effects::StatusFlags::ROOT.bits());
        let last_identity = frames
            .iter()
            .rposition(|m| matches!(event(m), Some(Event::Spawn(_))))
            .unwrap();
        let hydration = frames.iter().position(|m| matches!(event(m), Some(Event::EffectApplied(e)) if e.target == rooted.id.to_string() && e.skill_id == 110 && e.baseline)).unwrap();
        assert!(hydration > last_identity);
    }
    private_absent(&observer_frames, rooted.id);
}

#[tokio::test]
async fn socket_swing_identity_batch_pvp_selection_reset_and_cp_are_authoritative(
) -> anyhow::Result<()> {
    let repo = Arc::new(InMemoryCharacterRepository::default());
    let a = ordinary(&repo, "SwingAlpha", 10.0).await;
    let b = ordinary(&repo, "SwingBravo", 11.0).await;
    let viewer = ordinary(&repo, "SwingViewer", 12.0).await;
    let (app, driver) = app(repo).await;
    let (mut first, initial) = join(&app, &driver, &a).await;
    assert!(initial.iter().any(|m| matches!(event(m), Some(Event::TargetChanged(t)) if t.target.is_empty() && t.target_incarnation.is_none() && t.tick == t.baseline_tick)));
    let (mut second, _) = join(&app, &driver, &b).await;
    for (socket, target) in [(&mut first, &b), (&mut second, &a)] {
        intent(
            socket,
            &driver,
            1,
            Intent::SetTarget(pb::SetTargetRequest {
                entity_id: target.id.to_string(),
            }),
        )
        .await;
        let selection = drive(&driver, socket.until(|m| matches!(event(m), Some(Event::TargetChanged(t)) if t.target == target.id.to_string()))).await;
        assert!(selection.iter().any(|m| matches!(event(m), Some(Event::TargetChanged(t)) if !t.baseline && t.target_incarnation == Some(1))));
    }
    first
        .send(&pb::ClientMessage {
            seq: 2,
            intent: Some(Intent::Attack(pb::AttackRequest {})),
        })
        .await;
    second
        .send(&pb::ClientMessage {
            seq: 2,
            intent: Some(Intent::Attack(pb::AttackRequest {})),
        })
        .await;
    drive(&driver, first.until(|m| matches!(event(m), Some(Event::AttackStarted(_))))).await;
    let before = app.zone.snapshot().await.unwrap();
    let pending: Vec<_> = before
        .entities
        .iter()
        .filter_map(|e| e.combat.as_ref()?.swing.map(|s| (e.id.to_string(), s)))
        .collect();
    assert_eq!(pending.len(), 2);
    let (mut observer, baseline) = join(&app, &driver, &viewer).await;
    let last_identity = baseline
        .iter()
        .rposition(|m| matches!(event(m), Some(Event::Spawn(_))))
        .unwrap();
    for (id, swing) in pending {
        let index = baseline.iter().position(|m| matches!(event(m), Some(Event::AttackStarted(s)) if s.attacker == id && s.baseline)).unwrap();
        assert!(index > last_identity);
        let Some(Event::AttackStarted(s)) = event(&baseline[index]) else {
            unreachable!()
        };
        assert_eq!(
            (s.tick, s.impact_tick, s.ready_tick),
            (swing.start.0, swing.impact.0, swing.ready.0)
        );
        assert!(s.tick < s.baseline_tick);
        assert_eq!(s.attacker_incarnation, Some(1));
        assert!(!s.target.is_empty());
        assert!(baseline.iter().any(|m| matches!(event(m), Some(Event::Spawn(p)) if p.entity_id == s.target && p.life_incarnation == s.target_incarnation)));
    }
    for frame in &baseline {
        if let Some(Event::Spawn(s)) = event(frame) {
            assert!(
                s.pending_swing.is_none()
                    && s.pending_cast.is_none()
                    && s.active_effects.is_empty()
            );
        }
    }
    assert!(baseline.iter().any(|m| matches!(event(m), Some(Event::PvpStateChanged(p)) if p.entity == a.id.to_string() && p.baseline && p.karma == 7 && p.pk_count == 3 && p.pvp_count == 4 && p.flag_until_tick > p.tick)));
    hydration_fixture::write("pending-swings", &baseline)?;
    private_absent(&baseline, a.id);
    private_absent(&baseline, b.id);
    let hits = drive(
        &driver,
        observer.until(|m| matches!(event(m), Some(Event::AttackResult(r)) if r.cp_damage > 0)),
    )
    .await;
    let Some(Event::AttackResult(hit)) = hits.last().and_then(event) else {
        unreachable!()
    };
    assert_eq!(hit.damage, 0);
    assert!(hit.cp_damage > 0);
    private_absent(&hits, a.id);
    private_absent(&hits, b.id);
    Ok(())
}

async fn corpse_fixture() -> (TestApp, ManualTickDriver, Character, Character, Character) {
    // This reviewed pre-admission fixture has a paid/transfer-valid priest and an XP-loss
    // corpse. All offers and life transitions below come from real socket commands.
    let rec = Recording::from_bytes(include_bytes!("../fixtures/sessions/phase3-gameplay-v5.nfr"))
        .unwrap();
    let snapshot = rec.snapshot;
    let repo = Arc::new(InMemoryCharacterRepository::default());
    let mut chars = Vec::new();
    for e in &snapshot.entities {
        let Some(combat) = &e.combat else { continue };
        let CombatRole::Player {
            xp,
            progression: Some(p),
            ..
        } = &combat.role
        else {
            continue;
        };
        let mut c = Character {
            item_metadata: None,
            id: CharacterId::from_uuid(e.id.as_uuid()),
            account_id: p.identity.account_id,
            name: CharacterName::new(&e.name).unwrap(),
            race: p.identity.race,
            level: combat.sheet.level(),
            stats: load_classes(&ClassSource::embedded())
                .unwrap()
                .registry
                .get(p.class_state.current_class_id)
                .unwrap()
                .base_stats,
            position: Position { x: 10.0, y: 10.0 },
            appearance: p.identity.appearance,
            class_state: p.class_state.clone(),
            xp: *xp,
        };
        if e.targeting.dead {
            c.xp = load_rules(&RulesSource::embedded())
                .unwrap()
                .rules
                .xp_to_level(20)
                .unwrap()
                .checked_sub(1)
                .unwrap();
            c.level = 19;
        }
        repo.insert_for_test(c.clone());
        let mut phase3 = snapshot
            .phase3
            .as_ref()
            .unwrap()
            .checkpoint(e.id, Tick(snapshot.tick.0.checked_sub(1).unwrap()))
            .unwrap();
        if e.targeting.dead {
            let death = phase3.last_death.as_mut().unwrap();
            death.recoverable_death_xp_loss = 1000;
            death.pre_death_xp = c.xp.checked_add(1000).unwrap();
        }
        repo.checkpoint(
            &CharacterCheckpoint {
                phase4: None,
                character_id: c.id,
                revision_seen: 0,
                level: c.level,
                xp: c.xp,
                hp: combat.hp,
                mp: combat.mp,
                alive: !e.targeting.dead,
                position: c.position,
                idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
                class_state: Some(c.class_state.clone()),
                phase3: Some(phase3),
            },
            &[],
        )
        .await
        .unwrap();
        chars.push((e.targeting.dead, c));
    }
    let caster = chars.iter().find(|(dead, _)| !dead).unwrap().1.clone();
    let corpse = chars.iter().find(|(dead, _)| *dead).unwrap().1.clone();
    let viewer = ordinary(&repo, "LifeViewer", 12.0).await;
    let (app, driver) = app(repo).await;
    (app, driver, caster, corpse, viewer)
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Complete generated offer states and revival across owner/observer sockets.
async fn socket_offers_all_states_private_and_same_id_revival_has_final_max_hp_baseline() {
    for mode in 0..4 {
        let (app, driver, caster, corpse, viewer) = corpse_fixture().await;
        let (mut priest, _) = join(&app, &driver, &caster).await;
        let (mut dead, initial) = join(&app, &driver, &corpse).await;
        let original = initial
            .iter()
            .find_map(|m| match event(m) {
                Some(Event::Spawn(s)) if s.entity_id == corpse.id.to_string() => Some(s.clone()),
                _ => None,
            })
            .unwrap();
        let (mut observer, observer_initial) = join(&app, &driver, &viewer).await;
        private_absent(&observer_initial, corpse.id);
        intent(
            &mut priest,
            &driver,
            1,
            Intent::CastSkill(pb::CastSkillRequest {
                skill_id: 1016,
                target: Some(corpse.id.to_string()),
            }),
        )
        .await;
        let offered = drive(&driver, dead.until(|m| matches!(event(m), Some(Event::ResurrectOffer(o)) if o.state == i32::from(pb::ResurrectOfferState::Offered)))).await;
        let Some(Event::ResurrectOffer(offer)) = offered.last().and_then(event) else {
            unreachable!()
        };
        let offer = offer.clone();
        assert_eq!(offer.target, corpse.id.to_string());
        assert_eq!(offer.target_incarnation, original.life_incarnation);
        assert_eq!(offer.caster.as_deref(), Some(caster.id.to_string().as_str()));
        assert_eq!(offer.caster_incarnation, Some(1));
        let state = match mode {
            0 => pb::ResurrectOfferState::Declined,
            1 => pb::ResurrectOfferState::Accepted,
            _ => pb::ResurrectOfferState::Expired,
        };
        let mut owner = Vec::new();
        if mode == 3 {
            while app.zone.snapshot().await.unwrap().tick.0 <= offer.expiry_tick {
                driver.step().await.unwrap();
            }
        } else {
            let command = if mode == 2 {
                Intent::Respawn(pb::RespawnRequest {})
            } else {
                Intent::AcceptResurrect(pb::AcceptResurrectRequest {
                    offer_id: offer.offer_id,
                    accept: mode == 1,
                })
            };
            owner.extend(intent(&mut dead, &driver, 1, command).await);
        }
        let terminal = drive(&driver, dead.until(|m| matches!(event(m), Some(Event::ResurrectOffer(o)) if o.state == i32::from(state)))).await;
        assert!(terminal.iter().any(|m| matches!(event(m), Some(Event::ResurrectOffer(o)) if o.offer_id == offer.offer_id && o.target_incarnation == original.life_incarnation)));
        owner.extend(terminal);
        if mode == 1 || mode == 2 {
            if !owner.iter().any(|m| matches!(event(m), Some(Event::StatsChanged(s)) if s.entity == corpse.id.to_string() && s.baseline && s.incarnation > original.life_incarnation)) {
                owner.extend(drive(&driver, dead.until(|m| matches!(event(m), Some(Event::StatsChanged(s)) if s.entity == corpse.id.to_string() && s.baseline && s.incarnation > original.life_incarnation))).await);
            }
            let public = drive(&driver, observer.until(|m| matches!(event(m), Some(Event::VitalsChanged(v)) if v.entity == corpse.id.to_string() && v.baseline && v.incarnation > original.life_incarnation))).await;
            let Some(Event::VitalsChanged(v)) = public.last().and_then(event) else {
                unreachable!()
            };
            assert_eq!(v.baseline_tick, v.tick);
            assert!(v.hp > 0);
            assert!(public.iter().any(|m| matches!(event(m), Some(Event::EntityRespawned(r)) if r.entity == v.entity && r.incarnation == v.incarnation)));
            assert!(
                !public
                    .iter()
                    .any(|m| matches!(event(m), Some(Event::Spawn(s)) if s.entity_id == v.entity)),
                "unchanged AOI uses final vitals exception"
            );
            let s = owner
                .iter()
                .find_map(|m| match event(m) {
                    Some(Event::StatsChanged(s))
                        if s.entity == corpse.id.to_string()
                            && s.baseline
                            && s.incarnation > original.life_incarnation =>
                    {
                        Some(s)
                    },
                    _ => None,
                })
                .unwrap();
            assert_eq!((v.hp, v.max_hp, v.incarnation), (s.hp, s.max_hp, s.incarnation));
            if mode == 1 {
                assert_ne!(
                    v.max_hp, original.max_hp,
                    "restored XP crosses the original level boundary"
                );
            }
            private_absent(&public, caster.id);
            private_absent(&public, corpse.id);
        } else {
            intent(
                &mut observer,
                &driver,
                1,
                Intent::CancelCast(pb::CancelCastRequest { cast_id: None }),
            )
            .await;
        }
        let frames = observer.drain(Duration::from_millis(20)).await;
        private_absent(&frames, corpse.id);
        private_absent(&frames, caster.id);
    }
}

#[tokio::test]
async fn socket_replacement_hydrates_pending_offer_and_can_accept_it() -> anyhow::Result<()> {
    use anyhow::Context as _;
    let (app, driver, caster, corpse, viewer) = corpse_fixture().await;
    let (mut priest, _) = join(&app, &driver, &caster).await;
    let (mut old, _) = join(&app, &driver, &corpse).await;
    let (mut observer, _) = join(&app, &driver, &viewer).await;
    intent(
        &mut priest,
        &driver,
        1,
        Intent::CastSkill(pb::CastSkillRequest {
            skill_id: 1016,
            target: Some(corpse.id.to_string()),
        }),
    )
    .await;
    let offered = drive(&driver, old.until(|m| matches!(event(m), Some(Event::ResurrectOffer(o)) if o.state == i32::from(pb::ResurrectOfferState::Offered)))).await;
    let original = offered
        .iter()
        .find_map(|m| match event(m) {
            Some(Event::ResurrectOffer(o)) => Some(o.clone()),
            _ => None,
        })
        .context("ordinary pending offer")?;
    assert!(!original.baseline);
    assert_eq!(original.baseline_tick, 0);
    let (mut replacement, frames) = join(&app, &driver, &corpse).await;
    assert_eq!(old.closed(ws::WAIT).await, Some(4409));
    let (identity_index, identity) = frames
        .iter()
        .enumerate()
        .find_map(|(i, m)| match event(m) {
            Some(Event::Spawn(s)) if s.entity_id == corpse.id.to_string() => Some((i, s)),
            _ => None,
        })
        .context("replacement corpse identity")?;
    let (offer_index, hydrated) = frames
        .iter()
        .enumerate()
        .find_map(|(i, m)| match event(m) {
            Some(Event::ResurrectOffer(o)) => Some((i, o)),
            _ => None,
        })
        .context("replacement pending offer hydration")?;
    assert!(offer_index > identity_index);
    assert!(hydrated.baseline);
    assert_eq!(
        (hydrated.tick, hydrated.baseline_tick),
        (identity.state_tick, identity.state_tick)
    );
    assert_eq!(hydrated.target_incarnation, identity.life_incarnation);
    assert_eq!(identity.session_generation, 2);
    assert!(hydrated.tick > original.tick);
    let mut retained = original.clone();
    retained.tick = hydrated.tick;
    retained.baseline = true;
    retained.baseline_tick = hydrated.baseline_tick;
    assert_eq!(*hydrated, retained);
    assert_eq!(
        frames
            .iter()
            .filter(|m| matches!(event(m), Some(Event::ResurrectOffer(_))))
            .count(),
        1
    );
    // Accept using the offer ID received by the replacement, proving that hydration neither
    // invalidated the offer nor created a second offer. Terminal events remain ordinary.
    let mut accepted = intent(
        &mut replacement,
        &driver,
        1,
        Intent::AcceptResurrect(pb::AcceptResurrectRequest {
            offer_id: hydrated.offer_id,
            accept: true,
        }),
    )
    .await;
    accepted.extend(drive(&driver, replacement.until(|m| matches!(event(m), Some(Event::ResurrectOffer(o)) if o.state == i32::from(pb::ResurrectOfferState::Accepted)))).await);
    let terminal = accepted
        .iter()
        .find_map(|m| match event(m) {
            Some(Event::ResurrectOffer(o))
                if o.state == i32::from(pb::ResurrectOfferState::Accepted) =>
            {
                Some(o)
            },
            _ => None,
        })
        .context("accepted retained offer")?;
    assert_eq!(terminal.offer_id, original.offer_id);
    assert_eq!(terminal.target_incarnation, original.target_incarnation);
    assert!(!terminal.baseline);
    assert_eq!(terminal.baseline_tick, 0);
    let mut fixture = frames;
    fixture.extend(accepted);
    hydration_fixture::write("replacement-offer", &fixture)?;
    let public = drive(&driver, observer.until(|m| matches!(event(m), Some(Event::VitalsChanged(v)) if v.entity == corpse.id.to_string() && v.baseline))).await;
    private_absent(&public, corpse.id);
    Ok(())
}
