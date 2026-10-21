//! Real zone ticks settled on both adapters: D95 Refine + `CancelRefine` at the blacksmith,
//! D100 credit after a plain debit (partial Destroy of a crystal stack, then a Crystallize
//! refund into it), the D113 enchanted dev grant (birth enchant level, no RNG) and the D47/B4
//! paid skill learn (book 8618 consumed by the learn receipt, zone b63f0ec).
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::too_many_arguments
)]
mod common;
use api::application::checkpoint::CheckpointService;
use api::application::ports::*;
use api::application::replay_log::{restore_snapshot, start_epoch, AppliedTickRecord, EventLog};
use api::domain::character_progression::{FrozenTransferResult, SuccessfulTransferReceipt};
use api::domain::subclass::LearnedSkill;
use api::domain::zone::item_grant::{grant_enchant_limit, grant_fingerprint_with_enchant};
use api::domain::zone::phase4_projection::{ItemOperation, ItemRefineOutcome};
use api::domain::zone::*;
use api::domain::{AccountId, Character, CharacterId, CharacterName, Race};
use api::infrastructure::eventlog::InMemoryEventLog;
use api::infrastructure::memory::{InMemoryCharacterRepository, InMemorySessionAudit};
use api::infrastructure::postgres::PgCharacterRepository;
use api::infrastructure::telemetry::Metrics;
use api::infrastructure::{class_data, item_data, rules_data};
use nightfall_api as api;
use std::sync::Arc;
use uuid::Uuid;

fn player_load(loaded: api::application::ProgressionState) -> PlayerLoad {
    PlayerLoad {
        phase3: loaded.phase3.or_else(|| Some(Phase3Checkpoint::new())),
        phase4: loaded.phase4,
        progression: Some(Box::new(PlayerProgression {
            identity: loaded.identity,
            class_state: loaded.class_state,
            max_cp: 0,
        })),
        checkpoint_revision: Some(loaded.revision),
        class: loaded.class_profile,
        level: loaded.level,
        xp: loaded.xp,
        hp: loaded.hp,
        mp: loaded.mp,
        alive: loaded.alive,
    }
}

/// A zone, an admitted owner at the blacksmith with `grants` materialized, and its settlement.
struct Scene<R> {
    repo: Arc<R>,
    log: Arc<dyn EventLog>,
    state: ZoneState,
    service: CheckpointService,
    zone: ZoneId,
    character: Character,
    entity: EntityId,
    npc: effects::TargetBinding,
    seq: u32,
}

impl<R> Scene<R>
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + LearningCheckpointRepository
        + 'static,
{
    async fn new(
        repo: Arc<R>,
        name: &str,
        race: Race,
        level: u32,
        class: Option<u32>,
        skills: Vec<LearnedSkill>,
        grants: &[(u32, u64, u32)],
    ) -> Self {
        Self::new_at(repo, name, race, level, class, skills, grants, None).await
    }

    /// As `new`, with the owner placed at `at` (tiles) instead of the blacksmith.
    async fn new_at(
        repo: Arc<R>,
        name: &str,
        race: Race,
        level: u32,
        class: Option<u32>,
        skills: Vec<LearnedSkill>,
        grants: &[(u32, u64, u32)],
        at: Option<(i32, i32)>,
    ) -> Self {
        let bundle = item_data::embedded_phase4_bundle().unwrap();
        let rules = rules_data::load_rules(&rules_data::RulesSource::embedded())
            .unwrap()
            .rules;
        let mut c = Character::create(
            AccountId::from_uuid(Uuid::from_u128(1)),
            CharacterName::new(name).unwrap(),
            race,
        );
        c.level = level;
        c.xp = rules.xp_to_level(level).unwrap();
        repo.create_idempotent(&IdempotencyKey::new(), name, &c)
            .await
            .unwrap();
        if class.is_some() || !skills.is_empty() {
            // A recorded class transfer and manual (NPC-taught) ranks learned after creation,
            // persisted as a checkpoint would.
            let loaded = repo.load_for_admission(c.id).await.unwrap().unwrap();
            let mut class_state = loaded.class_state.clone();
            if let Some(class) = class {
                let target = api::domain::class::ClassId(class);
                class_state.current_class_id = target;
                class_state
                    .record_success(SuccessfulTransferReceipt {
                        key: Uuid::now_v7(),
                        target_class_id: target,
                        result: FrozenTransferResult {
                            character_id: c.id,
                            identity: c.identity(),
                            name: c.name.clone(),
                            current_class_id: target,
                            level,
                            xp: c.xp,
                            sp: class_state.sp,
                            stats: c.stats,
                            position_millitiles: [0, 0],
                            hp: 100,
                            mp: 100,
                            cp: 0,
                            max_hp: 100,
                            max_mp: 100,
                            max_cp: 100,
                            token_tier_1_count: 0,
                            token_tier_2_count: 0,
                            granted_skill_keys: vec![],
                        },
                    })
                    .unwrap();
            }
            // A learned executable passive (Crystallize is operate type P) is installed as a
            // retained modifier source at its exact rank.
            let mut phase3 = loaded.phase3.clone().unwrap_or_else(Phase3Checkpoint::new);
            for skill in &skills {
                let id = skill
                    .key
                    .strip_prefix("l2.skill.")
                    .and_then(|id| id.parse().ok())
                    .unwrap();
                phase3
                    .modifier_install_order
                    .push(effects::RetainedModifierSource::Passive {
                        skill: skill::SkillRef {
                            id: skill::SkillId(id),
                            rank: skill.level,
                        },
                    });
            }
            class_state.merge_learned_skills(skills);
            let checkpoint = api::application::CharacterCheckpoint {
                character_id: c.id,
                revision_seen: loaded.revision,
                level: loaded.level,
                xp: loaded.xp,
                hp: loaded.hp.unwrap_or(100),
                mp: loaded.mp.unwrap_or(100),
                alive: loaded.alive,
                position: loaded.position,
                idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
                class_state: Some(class_state),
                phase3: Some(phase3),
                phase4: None,
            };
            assert!(matches!(
                repo.checkpoint(&checkpoint, &[]).await.unwrap(),
                api::application::CheckpointOutcome::Applied(_)
            ));
        }
        let zone = ZoneId(u32::try_from(Uuid::now_v7().as_u128() & 0x7fff_ffff).unwrap());
        let range = repo.reserve_item_range(zone, 1).await.unwrap();
        let classes = class_data::load_classes(&class_data::ClassSource::embedded()).unwrap();
        let mut state = ZoneState::new_phase4(
            ZoneSeed { zone, epoch: 1 },
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
            0,
            rules,
            classes.registry,
            (*bundle.skills).clone(),
            bundle.items.clone(),
            ItemInstanceRange::new(
                range.range_start,
                range.next_reserved,
                range.range_end_exclusive,
            )
            .unwrap(),
        )
        .unwrap();
        let log: Arc<dyn EventLog> = Arc::new(InMemoryEventLog::default());
        start_epoch(log.as_ref(), &state.snapshot()).await.unwrap();
        let mut service = CheckpointService::new(
            repo.clone(),
            Arc::new(InMemorySessionAudit::default()),
            Arc::new(Metrics::detached()),
        )
        .with_items(repo.clone())
        .with_learning(repo.clone());
        let loaded = repo.load_for_admission(c.id).await.unwrap().unwrap();
        let blacksmith = &bundle.items.parts().capacities.blacksmith;
        let (x, y) = at.unwrap_or((blacksmith.x, blacksmith.y));
        let core = ItemPausedCore {
            name: c.name.as_str().to_owned(),
            position: Vec2Fixed::from_tiles(x, y),
            load: player_load(loaded),
        };
        let entity = EntityId::from_uuid(c.id.as_uuid());
        let grants = grants
            .iter()
            .map(|(template, count, enchant)| {
                ZoneInput::system(ZoneCommand::MaterializePendingItems(Box::new(
                    ItemGrantCommand {
                        entity,
                        account: c.account_id,
                        request_id: Uuid::now_v7(),
                        template_id: *template,
                        count: *count,
                        fingerprint: grant_fingerprint_with_enchant(
                            c.id, *template, *count, *enchant,
                        ),
                        rules_hash: bundle.items.hash(),
                        paused: Some(Box::new(core.clone())),
                        known: None,
                        conflict: None,
                        enchant: *enchant,
                    },
                )))
            })
            .collect();
        let grant = state.run_tick(state.draft(grants)).unwrap();
        assert!(grant.dispositions.is_empty(), "{:?}", grant.dispositions);
        log.append_applied(&AppliedTickRecord::from_applied(zone, &grant))
            .await
            .unwrap();
        service.admitted(&grant, &state.snapshot()).await.unwrap();
        let loaded = repo.load_for_admission(c.id).await.unwrap().unwrap();
        let admitted = state
            .run_tick(state.draft(vec![
                ZoneInput::system(ZoneCommand::SpawnPlayer {
                    entity,
                    name: c.name.as_str().to_owned(),
                    pos: core.position,
                    speed: Speed::DEFAULT,
                    generation: SessionGeneration(1),
                    load: Some(Box::new(player_load(loaded))),
                }),
                ZoneInput::system(ZoneCommand::SpawnNpc {
                    name: blacksmith.template.clone(),
                    pos: core.position,
                    speed: Speed::from_milli_tiles_per_tick(0),
                    combat: None,
                }),
            ]))
            .unwrap();
        assert!(admitted.dispositions.is_empty(), "{:?}", admitted.dispositions);
        log.append_applied(&AppliedTickRecord::from_applied(zone, &admitted))
            .await
            .unwrap();
        service
            .admitted(&admitted, &state.snapshot())
            .await
            .unwrap();
        let npc = effects::TargetBinding {
            entity: state
                .snapshot()
                .entities
                .iter()
                .find(|npc| npc.name == blacksmith.template)
                .unwrap()
                .id,
            incarnation: 0,
        };
        Self {
            repo,
            log,
            state,
            service,
            zone,
            character: c,
            entity,
            npc,
            seq: 0,
        }
    }

    fn instances(&self) -> Vec<ItemInstance> {
        let snapshot = self.state.snapshot();
        snapshot
            .phase4
            .as_ref()
            .unwrap()
            .checkpoint(self.entity, snapshot.tick, snapshot.phase3.as_ref().unwrap())
            .unwrap()
            .instances
    }

    /// A live server grant (the `GrantItem` command) for the admitted owner, cold-replayed,
    /// logged and settled; returns the tick's private projection.
    async fn grant_live(&mut self, template_id: u32, count: u64, enchant: u32) -> Phase4Projection {
        let character = CharacterId::from_uuid(self.entity.as_uuid());
        let command = ItemGrantCommand {
            entity: self.entity,
            account: self.character.account_id,
            request_id: Uuid::now_v7(),
            template_id,
            count,
            fingerprint: grant_fingerprint_with_enchant(character, template_id, count, enchant),
            rules_hash: item_data::embedded_phase4_bundle().unwrap().items.hash(),
            paused: None,
            known: None,
            conflict: None,
            enchant,
        };
        let draft =
            self.state
                .draft(vec![ZoneInput::system(ZoneCommand::MaterializePendingItems(
                    Box::new(command),
                ))]);
        let mut cold = restore_snapshot(self.state.snapshot()).unwrap();
        let applied = self.state.run_tick(draft.clone()).unwrap();
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        let record = AppliedTickRecord::from_applied(self.zone, &applied);
        assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
        self.log.append_applied(&record).await.unwrap();
        self.service
            .admitted(&applied, &self.state.snapshot())
            .await
            .unwrap();
        applied
            .events
            .iter()
            .find_map(|event| {
                if let ZoneEvent::ItemCheckpoint(p) = event {
                    Some(p.as_ref().clone())
                } else {
                    None
                }
            })
            .unwrap()
    }

    /// One session tick of `commands`, cold-replayed, logged and settled through the service.
    async fn tick(&mut self, commands: Vec<ItemCommandKind>) -> Phase4Projection {
        let inputs = commands
            .into_iter()
            .map(|kind| {
                self.seq = self.seq.checked_add(1).unwrap();
                ZoneInput::session(
                    self.entity,
                    SessionGeneration(1),
                    self.seq,
                    ZoneCommand::Item(Box::new(
                        ItemCommand::new(self.entity, Uuid::now_v7(), kind)
                            .unwrap()
                            .with_account(self.character.account_id)
                            .unwrap(),
                    )),
                )
            })
            .collect();
        let draft = self.state.draft(inputs);
        let mut cold = restore_snapshot(self.state.snapshot()).unwrap();
        let applied = self.state.run_tick(draft.clone()).unwrap();
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        self.log
            .append_applied(&AppliedTickRecord::from_applied(self.zone, &applied))
            .await
            .unwrap();
        self.service
            .admitted(&applied, &self.state.snapshot())
            .await
            .unwrap();
        let projection = applied
            .events
            .iter()
            .find_map(|event| {
                if let ZoneEvent::ItemCheckpoint(p) = event {
                    Some(p.as_ref().clone())
                } else {
                    None
                }
            })
            .unwrap();
        let retained = self
            .repo
            .retained_inventory(self.character.id)
            .await
            .unwrap()
            .unwrap()
            .checkpoint
            .unwrap();
        assert_eq!(retained, projection.checkpoint, "settled tick is the retained state");
        projection
    }
}

/// D95/P2-1: Refine then `CancelRefine` of a necklace (accessory stone 12754, admitted) at the
/// real blacksmith settles; both audits are re-derived at the trust boundary and published.
async fn refine_contract<R>(repo: Arc<R>) -> CharacterId
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + LearningCheckpointRepository
        + 'static,
{
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let parts = bundle.items.parts();
    let (necklace, stone) = (916, 12754);
    let cost = parts
        .augment_costs
        .iter()
        .find(|cost| cost.grade == parts.items[&necklace].grade)
        .unwrap();
    let gems = u64::from(cost.accessory_count);
    let mut scene = Scene::new(
        repo,
        "RefineSettle",
        Race::Human,
        46,
        None,
        Vec::new(),
        &[
            (necklace, 1, 0),
            (stone, 2, 0),
            (cost.gemstone_id, gems * 2, 0),
            (57, 5_000_000, 0),
        ],
    )
    .await;
    let target = scene
        .instances()
        .iter()
        .find(|item| item.template_id == necklace)
        .unwrap()
        .id;
    let stone_id = scene
        .instances()
        .iter()
        .find(|item| item.template_id == stone)
        .unwrap()
        .id;
    let npc = scene.npc;
    let projection = scene
        .tick(vec![
            ItemCommandKind::Refine {
                npc,
                target,
                stone: stone_id,
            },
            ItemCommandKind::CancelRefine { npc, target },
        ])
        .await;
    assert_eq!(
        projection
            .refines
            .iter()
            .map(|refine| refine.outcome)
            .collect::<Vec<_>>(),
        vec![ItemRefineOutcome::Applied, ItemRefineOutcome::Removed]
    );
    assert!(projection
        .ledger
        .iter()
        .any(|row| row.operation == ItemOperation::Refine && row.instance == target));
    scene.character.id
}

/// D100: [partial Destroy of the crystal stack, then the Crystallize refund into it] is legal
/// D69 cause order on one instance; the zone freezes it and both adapters settle it.
async fn crystal_contract<R>(repo: Arc<R>) -> (CharacterId, ItemInstanceId)
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + LearningCheckpointRepository
        + 'static,
{
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let parts = bundle.items.parts();
    let weapon = parts
        .items
        .iter()
        .filter(|(template, definition)| {
            definition.kind == "Weapon"
                && !definition.stackable
                && definition.flags.get("is_crystallizable") == Some(&true)
                && definition.flags.get("is_hero") != Some(&true)
                && definition.crystal_count > 0
                && parts
                    .grades
                    .iter()
                    .any(|g| g.grade == definition.grade && g.id == 1)
                && parts
                    .capabilities
                    .get(template)
                    .is_some_and(|capability| capability.admitted && capability.storable)
        })
        .map(|(template, _)| *template)
        .next()
        .unwrap();
    let mut scene = Scene::new(
        repo,
        "CrystalRefund",
        Race::Dwarf,
        40,
        // Artisan (56): Crystallize (248) is in its manual tree (artisan.toml, level 1 at 20).
        Some(56),
        vec![LearnedSkill {
            key: "l2.skill.248".into(),
            level: 1,
        }],
        &[(weapon, 1, 0), (weapon, 1, 0)],
    )
    .await;
    let weapons: Vec<_> = scene
        .instances()
        .iter()
        .filter(|item| item.template_id == weapon)
        .map(|item| item.id)
        .collect();
    assert_eq!(weapons.len(), 2);
    // First Crystallize creates the crystal stack.
    scene
        .tick(vec![ItemCommandKind::Crystallize {
            instance: weapons[0],
        }])
        .await;
    let crystal = scene
        .instances()
        .into_iter()
        .find(|item| {
            parts
                .grades
                .iter()
                .any(|g| g.crystal_id == item.template_id)
        })
        .unwrap();
    assert!(crystal.count > 1, "a partial Destroy needs more than one crystal");
    let projection = scene
        .tick(vec![
            ItemCommandKind::Destroy {
                instance: crystal.id,
                count: 1,
            },
            ItemCommandKind::Crystallize {
                instance: weapons[1],
            },
        ])
        .await;
    let rows: Vec<_> = projection
        .ledger
        .iter()
        .filter(|row| row.instance == crystal.id)
        .map(|row| (row.operation_ordinal, row.count_delta))
        .collect();
    assert_eq!(rows.len(), 2, "{:?}", projection.ledger);
    assert_eq!((rows[0].0, rows[1].0), (0, 1));
    assert!(rows[0].1 == -1 && rows[1].1 > 0, "debit then credit: {rows:?}");
    (scene.character.id, crystal.id)
}

fn memory_repo() -> Arc<InMemoryCharacterRepository> {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    Arc::new(
        InMemoryCharacterRepository::default()
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    )
}

async fn pg_repo() -> Option<(sqlx::PgPool, Arc<PgCharacterRepository>)> {
    let pool = common::pg::migrated_pool().await?;
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        PgCharacterRepository::new(pool.clone())
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    Some((pool, repo))
}

#[tokio::test]
async fn memory_real_refine_and_cancel_refine_tick_settles_and_publishes_both_audits() {
    let repo = memory_repo();
    refine_contract(repo.clone()).await;
    let kinds: Vec<_> = repo
        .staged_events()
        .iter()
        .map(api::domain::DomainEvent::subject)
        .filter(|subject| subject.contains("refine"))
        .collect();
    assert_eq!(kinds, vec!["nightfall.item.refined", "nightfall.item.refine_cancelled"]);
}

#[tokio::test]
async fn postgres_real_refine_and_cancel_refine_tick_settles_and_publishes_both_audits() {
    let Some((pool, repo)) = pg_repo().await else {
        return;
    };
    let id = refine_contract(repo).await;
    let subjects: Vec<String> = sqlx::query_scalar(
        "SELECT subject FROM outbox WHERE subject LIKE 'nightfall.item.refine%' AND payload->'fact'->>'character_id'=$1 ORDER BY id",
    )
    .bind(id.to_string())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(subjects, vec!["nightfall.item.refined", "nightfall.item.refine_cancelled"]);
}

#[tokio::test]
async fn memory_real_partial_destroy_then_crystallize_refund_settles() {
    crystal_contract(memory_repo()).await;
}

#[tokio::test]
async fn postgres_real_partial_destroy_then_crystallize_refund_settles() {
    let Some((pool, repo)) = pg_repo().await else {
        return;
    };
    let (id, crystal) = crystal_contract(repo).await;
    let rows: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT operation_ordinal,(delta->>'count_delta')::bigint FROM item_ledger WHERE character_id=$1 AND instance_id=$2 AND tick=(SELECT max(tick) FROM item_ledger WHERE character_id=$1) ORDER BY id",
    )
    .bind(id.as_uuid())
    .bind(i64::try_from(crystal.get()).unwrap())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0], rows[1].0), ((0, -1), 1));
    assert!(rows[1].1 > 0);
}

/// One refused live grant tick: returns its single disposition reason; nothing changes.
fn refused_grant(
    state: &mut ZoneState,
    entity: EntityId,
    account: api::domain::AccountId,
    template_id: u32,
    count: u64,
    enchant: u32,
) -> RejectReason {
    let character = CharacterId::from_uuid(entity.as_uuid());
    let before = state.snapshot();
    let draft = state.draft(vec![ZoneInput::system(ZoneCommand::MaterializePendingItems(
        Box::new(ItemGrantCommand {
            entity,
            account,
            request_id: Uuid::now_v7(),
            template_id,
            count,
            fingerprint: grant_fingerprint_with_enchant(character, template_id, count, enchant),
            rules_hash: item_data::embedded_phase4_bundle().unwrap().items.hash(),
            paused: None,
            known: None,
            conflict: None,
            enchant,
        }),
    ))]);
    let applied = state.run_tick(draft).unwrap();
    assert_eq!(applied.dispositions.len(), 1, "{:?}", applied.dispositions);
    assert_eq!(state.snapshot().phase4, before.phase4);
    applied.dispositions[0].reason
}

/// D113: birth enchant levels are bounded by the template's enchant rules in play: the
/// highest matching scroll's maximum + 1 (NONE-grade weapon 13539: 24) and the first 0%-chance
/// level of its chance group (D-grade armor 40: `ARMOR_GROUP` is 0% from +20, so 20). An
/// offline (paused) owner and an admitted owner both receive the item at its level; the
/// retained row and the append-only birth row carry it. An over-limit level is `InvalidEnchant`
/// even into a full inventory, where a valid level is `InventoryFull`.
#[allow(clippy::too_many_lines)] // One shared memory/Pg D113 contract.
async fn enchanted_grant_contract<R>(repo: Arc<R>) -> (CharacterId, ItemInstanceId)
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + LearningCheckpointRepository
        + 'static,
{
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    // Reachable-in-play (D113 amended): the ceiling counts the scroll and support bonuses,
    // so an armor is rate-bound only where base plus every bonus is 0% (15397 at +20).
    let (armor, weapon) = (15397, 13539);
    assert_eq!(grant_enchant_limit(&bundle.items, armor), 20, "rate-bound");
    assert_eq!(grant_enchant_limit(&bundle.items, 40), 65_535, "bonus keeps +20 reachable");
    assert_eq!(grant_enchant_limit(&bundle.items, weapon), 24, "scroll-bound");
    assert_eq!(grant_enchant_limit(&bundle.items, 69), 65_535, "unbounded D weapon");
    assert_eq!(grant_enchant_limit(&bundle.items, 57), 0, "stackable");
    // Offline: the paused-owner grant materializes the kit and the +20 armor before login.
    let mut scene = Scene::new(
        repo,
        "EnchantedGrant",
        Race::Human,
        20,
        None,
        Vec::new(),
        &[(57, 1, 0), (armor, 1, 20)],
    )
    .await;
    let offline = scene
        .instances()
        .into_iter()
        .find(|item| item.template_id == armor)
        .unwrap();
    assert_eq!(offline.enchant, 20);
    // Live: +24 on the scroll-bound weapon.
    let projection = scene.grant_live(weapon, 1, 24).await;
    let born = projection
        .ledger
        .iter()
        .find(|row| {
            row.after
                .as_ref()
                .is_some_and(|item| item.template_id == weapon)
        })
        .unwrap();
    assert!(born.before.is_none());
    assert_eq!(born.after.as_ref().unwrap().enchant, 24);
    let id = born.instance;
    let retained = scene
        .repo
        .retained_inventory(scene.character.id)
        .await
        .unwrap()
        .unwrap()
        .checkpoint
        .unwrap();
    let level = |id| {
        retained
            .instances
            .iter()
            .find(|item| item.id == id)
            .unwrap()
            .enchant
    };
    assert_eq!((level(id), level(offline.id)), (24, 20));
    let (entity, account) = (scene.entity, scene.character.account_id);
    for (template, enchant) in [(weapon, 25), (armor, 21)] {
        assert_eq!(
            refused_grant(&mut scene.state, entity, account, template, 1, enchant),
            RejectReason::InvalidEnchant
        );
    }
    // Fill the ordinary inventory with the lightest storable non-stackable item.
    let parts = bundle.items.parts();
    let filler = parts
        .items
        .iter()
        .filter(|(template, definition)| {
            !definition.stackable
                && definition.flags.get("is_questitem") == Some(&false)
                && parts.capabilities.get(template).is_some_and(|c| c.storable)
        })
        .min_by_key(|(template, definition)| (definition.weight, **template))
        .map(|(template, _)| *template)
        .unwrap();
    let capacity = u64::from(retained.ordinary_capacity);
    let used = u64::try_from(retained.instances.len()).unwrap();
    scene
        .grant_live(filler, capacity.checked_sub(used).unwrap(), 0)
        .await;
    assert_eq!(
        refused_grant(&mut scene.state, entity, account, armor, 1, 21),
        RejectReason::InvalidEnchant,
        "the level is checked before placement"
    );
    assert_eq!(
        refused_grant(&mut scene.state, entity, account, armor, 1, 3),
        RejectReason::InventoryFull
    );
    (scene.character.id, id)
}

#[tokio::test]
async fn memory_enchanted_dev_grant_is_born_at_its_level() {
    let repo = memory_repo();
    let (_, id) = enchanted_grant_contract(repo.clone()).await;
    let birth = repo
        .item_ledger_for_test()
        .into_iter()
        .find(|row| row.instance_id == id && row.delta.before.is_none())
        .unwrap();
    assert_eq!(birth.delta.after.unwrap().enchant, 24);
}

#[tokio::test]
async fn postgres_enchanted_dev_grant_is_born_at_its_level() {
    let Some((pool, repo)) = pg_repo().await else {
        return;
    };
    let (character, id) = enchanted_grant_contract(repo).await;
    let enchant: i64 = sqlx::query_scalar(
        "SELECT (delta->'after'->>'enchant')::bigint FROM item_ledger WHERE character_id=$1 AND instance_id=$2 AND delta->'before'='null'::jsonb",
    )
    .bind(character.as_uuid())
    .bind(i64::try_from(id.get()).unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(enchant, 24);
    let stored: i64 = sqlx::query_scalar("SELECT enchant FROM item_instances WHERE id=$1")
        .bind(i64::try_from(id.get()).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 24);
}

/// D47/B4 (api-review-10 P2-1): the zone's paid learn of 1405 rank 1 (a level-52 Rogue with one
/// book 8618 at the trainer) settles as one unified checkpoint: the book's Consume aggregate
/// caused by `skill_learn_payment`, the learn receipt and the learned rank. The same learn
/// without its item work (`checkpoint_learning`) is refused as `item_learn_payment`.
async fn paid_learn_contract<R>(repo: Arc<R>) -> CharacterId
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + LearningCheckpointRepository
        + 'static,
{
    use api::domain::zone::phase4_projection::ItemEvidenceValue;
    const BOOK: u32 = 8618;
    let mut scene = Scene::new_at(
        repo,
        "PaidLearn",
        Race::Human,
        52,
        Some(7),
        Vec::new(),
        &[(BOOK, 1, 0)],
        Some((126, 128)),
    )
    .await;
    let book = scene
        .instances()
        .into_iter()
        .find(|item| item.template_id == BOOK)
        .unwrap();
    let key = Uuid::now_v7();
    scene.seq = scene.seq.checked_add(1).unwrap();
    let draft = scene.state.draft(vec![ZoneInput::session(
        scene.entity,
        SessionGeneration(1),
        scene.seq,
        ZoneCommand::LearnSkill {
            entity: scene.entity,
            account: scene.character.account_id,
            request_key: key,
            skill_id: skill::SkillId(1405),
            skill_level: 1,
        },
    )]);
    let mut cold = restore_snapshot(scene.state.snapshot()).unwrap();
    let applied = scene.state.run_tick(draft.clone()).unwrap();
    assert_eq!(applied, cold.run_tick(draft).unwrap());
    assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
    let result = applied
        .events
        .iter()
        .find_map(|event| {
            if let ZoneEvent::SkillLearned { result, .. } = event {
                Some(result.as_ref().clone())
            } else {
                None
            }
        })
        .unwrap();
    let projection = applied
        .events
        .iter()
        .find_map(|event| {
            if let ZoneEvent::ItemCheckpoint(p) = event {
                Some(p.as_ref().clone())
            } else {
                None
            }
        })
        .unwrap();
    // The recorded producer shape (b63f0ec): one whole-book Consume aggregate, no item receipt,
    // caused by the learn receipt's key and rank.
    assert_eq!(projection.ledger.len(), 1);
    let row = &projection.ledger[0];
    assert_eq!(
        (row.operation, row.instance, row.count_delta),
        (ItemOperation::Consume, book.id, -1)
    );
    assert!(row.after.is_none() && row.receipt.is_none() && row.receipt_refs.is_empty());
    assert_eq!(row.causes.len(), 1);
    assert_eq!(row.causes[0].family, "skill_learn_payment");
    assert_eq!(
        row.causes[0].fields.get("receipt"),
        Some(&ItemEvidenceValue::Text(key.to_string()))
    );
    let receipt = SuccessfulLearnReceipt {
        account_id: scene.character.account_id,
        key: IdempotencyKey::from_uuid(key),
        request_fingerprint: learn_skill_fingerprint(scene.character.id, result.skill),
        result: result.clone(),
    };
    // The item-less learning path never settles a book-gated rank.
    let loaded = scene
        .repo
        .load_for_admission(scene.character.id)
        .await
        .unwrap()
        .unwrap();
    let mut class_state = loaded.class_state.clone();
    class_state.merge_learned_skills([LearnedSkill {
        key: "l2.skill.1405".into(),
        level: 1,
    }]);
    let unpaid = api::application::CharacterCheckpoint {
        character_id: scene.character.id,
        revision_seen: loaded.revision,
        level: loaded.level,
        xp: loaded.xp,
        hp: loaded.hp.unwrap_or(100),
        mp: loaded.mp.unwrap_or(100),
        alive: loaded.alive,
        position: loaded.position,
        idempotency: ("learn_skill".into(), IdempotencyKey::new()),
        class_state: Some(class_state),
        phase3: loaded.phase3.clone(),
        phase4: None,
    };
    assert!(matches!(
        scene.repo.checkpoint_learning(&unpaid, &[], &receipt).await,
        Err(CheckpointError::Constraint(name)) if name == "item_learn_payment"
    ));
    assert_eq!(
        scene
            .repo
            .learn_receipt_lookup(receipt.account_id, &receipt.key, &receipt.request_fingerprint)
            .await
            .unwrap(),
        LearnReceiptLookup::Unknown
    );
    // The real recording settles through the checkpoint lane: Applied, once.
    scene
        .log
        .append_applied(&AppliedTickRecord::from_applied(scene.zone, &applied))
        .await
        .unwrap();
    scene
        .service
        .admitted(&applied, &scene.state.snapshot())
        .await
        .unwrap();
    assert_eq!(
        scene
            .repo
            .learn_receipt_lookup(receipt.account_id, &receipt.key, &receipt.request_fingerprint)
            .await
            .unwrap(),
        LearnReceiptLookup::Known(result)
    );
    let settled = scene
        .repo
        .load_for_admission(scene.character.id)
        .await
        .unwrap()
        .unwrap();
    assert!(settled
        .class_state
        .learned_skills
        .iter()
        .any(|skill| skill.key == "l2.skill.1405" && skill.level == 1));
    let retained = scene
        .repo
        .retained_inventory(scene.character.id)
        .await
        .unwrap()
        .unwrap()
        .checkpoint
        .unwrap();
    assert_eq!(retained, projection.checkpoint, "settled tick is the retained state");
    assert!(retained
        .instances
        .iter()
        .all(|item| item.template_id != BOOK));
    scene.character.id
}

#[tokio::test]
async fn memory_real_paid_skill_learn_settles_the_book_payment() {
    let repo = memory_repo();
    let character = paid_learn_contract(repo.clone()).await;
    let payments: Vec<_> = repo
        .item_ledger_for_test()
        .into_iter()
        .filter(|row| {
            row.character_id == character
                && row.operation == ItemLedgerOperation::Consume
                && row
                    .delta
                    .causes
                    .iter()
                    .any(|cause| cause["family"] == "skill_learn_payment")
        })
        .collect();
    assert_eq!(payments.len(), 1);
}

#[tokio::test]
async fn postgres_real_paid_skill_learn_settles_the_book_payment() {
    let Some((pool, repo)) = pg_repo().await else {
        return;
    };
    let character = paid_learn_contract(repo).await;
    let payments: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM item_ledger WHERE character_id=$1 AND op='consume' AND delta->'causes' @> '[{\"family\":\"skill_learn_payment\"}]'::jsonb",
    )
    .bind(character.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(payments, 1);
    let learned: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE subject='nightfall.character.skill_learned' AND payload::text LIKE '%' || $1 || '%'",
    )
    .bind(character.as_uuid().to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(learned, 1);
}

/// T36 probe: item activity (equip + automatic soulshot, which charges the weapon), then the
/// owner leaves and logs in again. D71 resets the transient charges and toggles on the new
/// incarnation.
async fn relogin_after_shots_contract<R>(repo: Arc<R>)
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + LearningCheckpointRepository
        + 'static,
{
    let mut scene = Scene::new(
        repo,
        "ShotRelogin",
        Race::Human,
        10,
        None,
        Vec::new(),
        &[(2369, 1, 0), (1835, 100, 0)],
    )
    .await;
    let weapon = scene
        .instances()
        .iter()
        .find(|item| item.template_id == 2369)
        .unwrap()
        .id;
    scene
        .tick(vec![ItemCommandKind::Equip {
            instance: weapon,
            preferred_slot: None,
        }])
        .await;
    scene
        .tick(vec![ItemCommandKind::ToggleAutoShot {
            template: 1835,
            enabled: true,
        }])
        .await;
    let retained = scene
        .repo
        .retained_inventory(scene.character.id)
        .await
        .unwrap()
        .unwrap()
        .checkpoint
        .unwrap();
    assert!(retained.auto_shots.contains(&1835));
    // Disconnect: lifecycle flush, then Despawn.
    scene.service.flush(scene.entity).await.unwrap();
    let left = scene
        .state
        .run_tick(
            scene
                .state
                .draft(vec![ZoneInput::system(ZoneCommand::Despawn {
                    entity: scene.entity,
                })]),
        )
        .unwrap();
    scene
        .log
        .append_applied(&AppliedTickRecord::from_applied(scene.zone, &left))
        .await
        .unwrap();
    scene
        .service
        .admitted(&left, &scene.state.snapshot())
        .await
        .unwrap();
    // Re-login from the committed state.
    let loaded = scene
        .repo
        .load_for_admission(scene.character.id)
        .await
        .unwrap()
        .unwrap();
    let back = scene
        .state
        .run_tick(
            scene
                .state
                .draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
                    entity: scene.entity,
                    name: scene.character.name.as_str().to_owned(),
                    pos: Vec2Fixed::from_tiles(126, 126),
                    speed: Speed::DEFAULT,
                    generation: SessionGeneration(2),
                    load: Some(Box::new(player_load(loaded))),
                })]),
        )
        .unwrap();
    assert!(back.dispositions.is_empty(), "{:?}", back.dispositions);
    scene
        .log
        .append_applied(&AppliedTickRecord::from_applied(scene.zone, &back))
        .await
        .unwrap();
    scene
        .service
        .admitted(&back, &scene.state.snapshot())
        .await
        .unwrap();
    for _ in 0..3 {
        let idle = scene.state.run_tick(scene.state.draft(Vec::new())).unwrap();
        scene
            .log
            .append_applied(&AppliedTickRecord::from_applied(scene.zone, &idle))
            .await
            .unwrap();
        scene
            .service
            .admitted(&idle, &scene.state.snapshot())
            .await
            .unwrap();
    }
    // A second disconnect saves the new incarnation's state.
    scene.service.flush(scene.entity).await.unwrap();
}

#[tokio::test]
async fn memory_relogin_after_automatic_shots_settles() {
    relogin_after_shots_contract(memory_repo()).await;
}

#[tokio::test]
async fn postgres_relogin_after_automatic_shots_settles() {
    let Some((_pool, repo)) = pg_repo().await else {
        return;
    };
    relogin_after_shots_contract(repo).await;
}

/// T37/D143: a no-op Equip (the kit weapon is already wielded) and ToggleAutoShot(on) in the
/// same tick. The zone stamps the no-op Equip receipt with the pre-tick revision and the toggle
/// with the settled one; D143 accepts the pre-tick revision on any no-op receipt.
async fn equip_and_toggle_contract<R>(repo: Arc<R>) -> Vec<(ItemOperation, u64, u64)>
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + LearningCheckpointRepository
        + 'static,
{
    let mut scene = Scene::new(
        repo,
        "EquipToggle",
        Race::Human,
        10,
        None,
        Vec::new(),
        &[(2369, 1, 0), (1835, 100, 0)],
    )
    .await;
    let weapon = scene
        .instances()
        .iter()
        .find(|item| item.template_id == 2369)
        .unwrap()
        .id;
    let inputs = [
        ItemCommandKind::Equip {
            instance: weapon,
            preferred_slot: None,
        },
        ItemCommandKind::ToggleAutoShot {
            template: 1835,
            enabled: true,
        },
    ]
    .into_iter()
    .zip(1_u32..)
    .map(|(kind, seq)| {
        ZoneInput::session(
            scene.entity,
            SessionGeneration(1),
            seq,
            ZoneCommand::Item(Box::new(
                ItemCommand::new(scene.entity, Uuid::now_v7(), kind)
                    .unwrap()
                    .with_account(scene.character.account_id)
                    .unwrap(),
            )),
        )
    })
    .collect();
    let applied = scene.state.run_tick(scene.state.draft(inputs)).unwrap();
    assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
    let projection = applied
        .events
        .iter()
        .find_map(|event| {
            if let ZoneEvent::ItemCheckpoint(p) = event {
                Some(p.as_ref().clone())
            } else {
                None
            }
        })
        .unwrap();
    let observed = projection
        .receipts
        .iter()
        .map(|receipt| {
            let ItemResultDetail::Operation { operation, .. } = &receipt.result.detail else {
                unreachable!("operation receipts only")
            };
            (
                *operation,
                receipt.result.inventory_revision,
                projection.checkpoint.inventory_revision,
            )
        })
        .collect();
    scene
        .log
        .append_applied(&AppliedTickRecord::from_applied(scene.zone, &applied))
        .await
        .unwrap();
    scene
        .service
        .admitted(&applied, &scene.state.snapshot())
        .await
        .unwrap_or_else(|error| {
            panic!("T37: {error:#}; receipts (op, receipt rev, settled rev): {observed:?}")
        });
    observed
}

#[tokio::test]
async fn memory_equip_and_toggle_in_one_tick_settles() {
    equip_and_toggle_contract(memory_repo()).await;
}

#[tokio::test]
async fn postgres_equip_and_toggle_in_one_tick_settles() {
    let Some((_pool, repo)) = pg_repo().await else {
        return;
    };
    equip_and_toggle_contract(repo).await;
}
