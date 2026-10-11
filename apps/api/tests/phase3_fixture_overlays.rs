//! Harness-owned bridge into the production bootstrap and admission validators.
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{ensure, Context as _};
use nightfall_api::domain::character_progression::{
    CharacterAppearance, CharacterIdentity, ClassState,
};
use nightfall_api::domain::class::ClassId;
use nightfall_api::domain::subclass::LearnedSkill;
use nightfall_api::domain::zone::{
    EntityId, Phase3Checkpoint, PlayerLoad, PlayerProgression, SessionGeneration, Speed, Vec2Fixed,
    ZoneCommand, ZoneInput, ZoneSeed,
};
use nightfall_api::domain::{AccountId, Race};
use nightfall_api::infrastructure::{
    class_data::{load_classes, ClassSource},
    rules_data::load_rules_dir,
    skill_data::Phase3DataSource,
    zone_data::load_zone,
};
use nightfall_api::zone_runtime::prepare_phase3_bootstrap;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
struct Role {
    character_id: EntityId,
    class_id: u32,
    base_class_id: u32,
    race: Race,
    class_profile: String,
    level: u32,
    xp: u64,
    sp: u64,
    position: [i32; 2],
    learned_ranks: BTreeMap<String, u32>,
    hp: Option<u32>,
    mp: Option<u32>,
    cp: u32,
    alive: bool,
    death_penalty_level: u8,
    modifier_install_order: Vec<nightfall_api::domain::zone::effects::RetainedModifierSource>,
}

fn validate(root: &Path, manifest: &Path) -> anyhow::Result<()> {
    let prepared = prepare_phase3_bootstrap(
        &Phase3DataSource::from_dir(root)?,
        load_rules_dir(root)?,
        load_classes(&ClassSource::from_dir(root)?)?,
        load_zone(&root.join("zones/test_zone.toml"))?,
    )?;
    let packs: BTreeMap<String, BTreeMap<String, Role>> =
        serde_json::from_slice(&std::fs::read(manifest)?)?;
    ensure!(!packs.is_empty(), "no fixture packs");
    for (pack, roles) in packs {
        ensure!(!roles.is_empty(), "empty roles for {pack}");
        let mut state = prepared.state(
            ZoneSeed {
                zone: prepared.zone.zone,
                epoch: 1,
            },
            0,
        )?;
        for (index, (name, role)) in roles.into_iter().enumerate() {
            let mut ledger = ClassState::new(ClassId(role.base_class_id));
            ledger.current_class_id = ClassId(role.class_id);
            ledger.sp = role.sp;
            ledger.cp = role.cp;
            ledger.learned_skills = role
                .learned_ranks
                .into_iter()
                .map(|(id, level)| LearnedSkill {
                    key: format!("l2.skill.{id}"),
                    level,
                })
                .collect();
            let mut checkpoint = Phase3Checkpoint::new();
            checkpoint.death_penalty_level = role.death_penalty_level;
            checkpoint.modifier_install_order = role.modifier_install_order;
            checkpoint
                .stage_restore(
                    &prepared.phase3,
                    &ledger.learned_skills,
                    role.alive,
                    1,
                    nightfall_api::domain::zone::Tick(1),
                    1,
                    1,
                )
                .with_context(|| format!("{pack}/{name}: stage_restore"))?;
            let account = u128::try_from(index)?
                .checked_add(1)
                .context("account overflow")?;
            let [x, y] = role.position;
            let load = PlayerLoad {
                phase3: Some(checkpoint),
                progression: Some(Box::new(PlayerProgression {
                    identity: CharacterIdentity {
                        account_id: AccountId::from_uuid(uuid::Uuid::from_u128(account)),
                        race: role.race,
                        base_class_id: ClassId(role.base_class_id),
                        appearance: CharacterAppearance::default(),
                    },
                    class_state: ledger,
                    max_cp: 0,
                })),
                checkpoint_revision: Some(0),
                class: role.class_profile,
                level: role.level,
                xp: role.xp,
                hp: role.hp,
                mp: role.mp,
                alive: role.alive,
            };
            let draft = state.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity: role.character_id,
                name: name.clone(),
                pos: Vec2Fixed::from_tiles(x, y),
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(load)),
            })]);
            let admitted = state.run_tick(draft)?;
            ensure!(admitted.dispositions.is_empty(), "{pack}/{name}: {:?}", admitted.dispositions);
            ensure!(state.entity(role.character_id).is_some(), "{pack}/{name}: no admitted entity");
            let _ = nightfall_api::domain::zone::ZoneState::from_snapshot(state.snapshot())?;
        }
    }
    Ok(())
}

#[test]
fn generated_overlay_and_all_seed_roles_pass_real_admission() -> anyhow::Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .context("repository root")?;
    let temporary =
        std::env::temp_dir().join(format!("nightfall-phase3-harness-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir(&temporary)?;
    let result = (|| -> anyhow::Result<()> {
        let generated = std::process::Command::new("python3")
            .arg(repo.join("apps/client-unreal/Scripts/phase3-validation-inputs.py"))
            .arg(&temporary)
            .status()?;
        ensure!(generated.success(), "fixture input generation failed");
        let root = temporary.join("data");
        let manifest = temporary.join("packs.json");
        validate(&root, &manifest)?;
        let target = root.join("npcs/skill_target.toml");
        let source = std::fs::read_to_string(&target)?;
        let (prefix, _) = source
            .split_once("# BEGIN GENERATED PHASE3 TEMPLATE")
            .context("generated NPC profile marker")?;
        std::fs::write(&target, prefix)?;
        ensure!(validate(&root, &manifest).is_err(), "profile-less NPC accepted");
        std::fs::write(&target, source.replace("sp_reward = 2\n", ""))?;
        ensure!(validate(&root, &manifest).is_err(), "SP-less NPC accepted");
        std::fs::write(&target, &source)?;
        let mut packs: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest)?)?;
        let install = packs
            .pointer_mut("/phase3-penalty-cap/owner/modifier_install_order")
            .context("penalty modifier")?;
        *install = serde_json::json!([]);
        std::fs::write(&manifest, serde_json::to_vec(&packs)?)?;
        let error = validate(&root, &manifest)
            .err()
            .context("missing modifier accepted")?;
        ensure!(
            format!("{error:#}").contains("missing modifier installation source"),
            "unexpected admission failure: {error:#}"
        );
        Ok(())
    })();
    std::fs::remove_dir_all(&temporary)?;
    result
}

// These recordings are generated from production admission/ticks, never synthetic JSONL.
// Fixture level/ranks: Scripts/phase3-fixtures.py:79-106. Formula values are read from
// the shipped registry (pinned datapack 01100-01199.xml:1630,1826; 01000-01099.xml:1540).
fn evidence_load(role: &Role, account: u128) -> PlayerLoad {
    let mut ledger = ClassState::new(ClassId(role.base_class_id));
    ledger.current_class_id = ClassId(role.class_id);
    ledger.sp = role.sp;
    ledger.cp = role.cp;
    ledger.learned_skills = role
        .learned_ranks
        .iter()
        .map(|(id, level)| LearnedSkill {
            key: format!("l2.skill.{id}"),
            level: *level,
        })
        .collect();
    let mut checkpoint = Phase3Checkpoint::new();
    checkpoint.death_penalty_level = role.death_penalty_level;
    checkpoint
        .modifier_install_order
        .clone_from(&role.modifier_install_order);
    PlayerLoad {
        phase3: Some(checkpoint),
        progression: Some(Box::new(PlayerProgression {
            identity: CharacterIdentity {
                account_id: AccountId::from_uuid(uuid::Uuid::from_u128(account)),
                race: role.race,
                base_class_id: ClassId(role.base_class_id),
                appearance: CharacterAppearance::default(),
            },
            class_state: ledger,
            max_cp: 0,
        })),
        checkpoint_revision: Some(0),
        class: role.class_profile.clone(),
        level: role.level,
        xp: role.xp,
        hp: role.hp,
        mp: role.mp,
        alive: role.alive,
    }
}

struct EvidenceRecording {
    state: nightfall_api::domain::zone::ZoneState,
    recording: nightfall_api::infrastructure::eventlog::Recording,
}
impl EvidenceRecording {
    fn new(state: nightfall_api::domain::zone::ZoneState) -> Self {
        use nightfall_api::application::replay_log::{Watermark, WatermarkReason};
        let snapshot = state.snapshot();
        let watermark = Watermark {
            zone: snapshot.seed.zone,
            epoch: snapshot.seed.epoch,
            last_tick: None,
            records: 0,
            reason: WatermarkReason::Capture,
        };
        Self {
            state,
            recording: nightfall_api::infrastructure::eventlog::Recording {
                snapshot,
                watermark,
                records: Vec::new(),
            },
        }
    }
    fn tick(
        &mut self,
        inputs: Vec<ZoneInput>,
    ) -> anyhow::Result<nightfall_api::domain::zone::AppliedTick> {
        use nightfall_api::application::replay_log::AppliedTickRecord;
        let applied = self.state.run_tick(self.state.draft(inputs))?;
        ensure!(
            applied.dispositions.is_empty(),
            "fixture command rejected: {:?}",
            applied.dispositions
        );
        self.recording
            .records
            .push(AppliedTickRecord::from_applied(self.recording.snapshot.seed.zone, &applied));
        self.recording.watermark.records = u64::try_from(self.recording.records.len())?;
        self.recording.watermark.last_tick = Some(applied.tick);
        Ok(applied)
    }
    fn wait(&mut self, count: u64) -> anyhow::Result<()> {
        for _ in 0..count {
            self.tick(Vec::new())?;
        }
        Ok(())
    }
    fn spawn(&mut self, role: &Role, load: PlayerLoad) -> anyhow::Result<()> {
        let [x, y] = role.position;
        self.tick(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: role.character_id,
            name: "Harness".into(),
            pos: Vec2Fixed::from_tiles(x, y),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: Some(Box::new(load)),
        })])?;
        Ok(())
    }
    fn cast(&mut self, player: EntityId, target: EntityId, skill: u32) -> anyhow::Result<()> {
        use nightfall_api::domain::zone::{skill::SkillId, ZoneEvent};
        self.tick(vec![ZoneInput::session(
            player,
            SessionGeneration(1),
            1,
            ZoneCommand::CastSkill {
                entity: player,
                target: Some(target),
                skill_id: SkillId(skill),
            },
        )])?;
        for _ in 0..100 {
            let applied = self.tick(Vec::new())?;
            if applied
                .events
                .iter()
                .any(|e| matches!(e, ZoneEvent::SkillCastResult { .. }))
            {
                return Ok(());
            }
        }
        anyhow::bail!("cast never completed")
    }
    fn reconnect(&mut self, role: &Role) -> anyhow::Result<()> {
        use nightfall_api::domain::zone::CombatRole;
        let snapshot = self.state.snapshot();
        let entity = snapshot
            .entities
            .iter()
            .find(|e| e.id == role.character_id)
            .context("before disconnect")?;
        let combat = entity.combat.as_ref().context("combat baseline")?;
        let CombatRole::Player {
            progression, xp, ..
        } = &combat.role
        else {
            anyhow::bail!("player required");
        };
        let mut load = PlayerLoad::fresh(&role.class_profile);
        load.phase3 = Some(
            snapshot
                .phase3
                .as_ref()
                .context("phase3")?
                .checkpoint(role.character_id, self.state.next_tick())?,
        );
        load.progression.clone_from(progression);
        load.level = combat.sheet.level();
        load.xp = *xp;
        load.hp = Some(combat.hp);
        load.mp = Some(combat.mp);
        load.alive = combat.hp > 0;
        self.tick(vec![ZoneInput::system(ZoneCommand::Despawn {
            entity: role.character_id,
        })])?;
        self.wait(10)?;
        self.spawn(role, load)
    }
}

#[test]
#[allow(clippy::too_many_lines)] // One production recorder exercises the consumer seam without external services.
fn actual_scenario_inputs_record_for_independent_evidence() -> anyhow::Result<()> {
    use nightfall_api::domain::zone::skill::SkillId;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .context("repo")?;
    let work = std::env::temp_dir()
        .join(format!("nightfall-phase3-harness-evidence-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir(&work)?;
    let result = (|| -> anyhow::Result<()> {
        let generated = std::process::Command::new("python3")
            .arg(repo.join("apps/client-unreal/Scripts/phase3-validation-inputs.py"))
            .arg(&work)
            .status()?;
        ensure!(generated.success(), "generation failed");
        let root = work.join("data");
        let prepared = prepare_phase3_bootstrap(
            &Phase3DataSource::from_dir(&root)?,
            load_rules_dir(&root)?,
            load_classes(&ClassSource::from_dir(&root)?)?,
            load_zone(&root.join("zones/test_zone.toml"))?,
        )?;
        let packs: BTreeMap<String, BTreeMap<String, Role>> =
            serde_json::from_slice(&std::fs::read(work.join("packs.json"))?)?;
        let roles = packs.get("phase3-trainer").context("trainer pack")?;
        let mut owner = roles.get("owner").context("owner")?.clone();
        let peer = roles.get("observer").context("peer")?.clone();
        let mut rec = EvidenceRecording::new(prepared.state(
            ZoneSeed {
                zone: prepared.zone.zone,
                epoch: 93,
            },
            0,
        )?);
        rec.spawn(&owner, evidence_load(&owner, 1))
            .context("initial numeric owner")?;
        rec.spawn(&peer, evidence_load(&peer, 2))
            .context("initial numeric peer")?;
        rec.tick(vec![ZoneInput::session(
            owner.character_id,
            SessionGeneration(1),
            1,
            ZoneCommand::LearnSkill {
                entity: owner.character_id,
                account: AccountId::from_uuid(uuid::Uuid::from_u128(1)),
                request_key: uuid::Uuid::from_u128(99),
                skill_id: SkillId(1177),
                skill_level: 2,
            },
        )])?;
        rec.tick(vec![ZoneInput::session(
            owner.character_id,
            SessionGeneration(1),
            2,
            ZoneCommand::MoveTo {
                entity: owner.character_id,
                dest: Vec2Fixed::from_tiles(110, 100),
            },
        )])?;
        rec.wait(150)?;
        owner.position = [110, 100];
        rec.cast(owner.character_id, peer.character_id, 1177)?;
        rec.cast(owner.character_id, peer.character_id, 1184)?;
        rec.cast(owner.character_id, owner.character_id, 1068)?;
        rec.reconnect(&owner).context("effect reconnect")?;
        // Expire the real Might duration, not a shortened test rule.
        let end = rec
            .state
            .snapshot()
            .phase3
            .as_ref()
            .context("phase3")?
            .entities
            .get(&owner.character_id)
            .context("owner sidecar")?
            .effects
            .entries()
            .values()
            .filter_map(|e| e.expires_at)
            .max()
            .context("retained effect")?;
        while rec.state.next_tick() <= end {
            rec.tick(Vec::new())?;
        }
        let out = std::env::var_os("NIGHTFALL_HARNESS_EVIDENCE_DIR")
            .map_or_else(|| work.clone(), std::path::PathBuf::from);
        std::fs::create_dir_all(&out)?;
        rec.recording.write(&out.join("actual-inputs.nfr"))?;
        let pvp_roles = packs.get("phase3-pvp").context("pvp pack")?;
        let pvp_owner = pvp_roles.get("owner").context("pvp owner")?;
        let mut pvp = EvidenceRecording::new(prepared.state(
            ZoneSeed {
                zone: prepared.zone.zone,
                epoch: 97,
            },
            0,
        )?);
        pvp.spawn(pvp_owner, evidence_load(pvp_owner, 1))?;
        pvp.spawn(&peer, evidence_load(&peer, 2))?;
        pvp.cast(pvp_owner.character_id, peer.character_id, 1177)?;
        pvp.recording.write(&out.join("pvp-inputs.nfr"))?;
        // Independent regeneration regression uses the real Regeneration skill, whose
        // regHp modifier is in datapack 01000-01099.xml (skill1044), not invented credit.
        let mut regeneration_owner = packs
            .get("phase3-resurrect")
            .context("resurrect pack")?
            .get("owner")
            .context("caster")?
            .clone();
        // Cleric Regeneration rank1: required35 / SP21000 (professions/cleric.toml:1333-1338).
        regeneration_owner.level = 35;
        regeneration_owner.xp = load_rules_dir(&root)?.rules.xp_to_level(35)?;
        regeneration_owner.sp = 21000;
        regeneration_owner.hp = Some(1);
        let mut regen = EvidenceRecording::new(prepared.state(
            ZoneSeed {
                zone: prepared.zone.zone,
                epoch: 94,
            },
            0,
        )?);
        regen
            .spawn(&regeneration_owner, evidence_load(&regeneration_owner, 1))
            .context("regen owner")?;
        regen.tick(vec![ZoneInput::session(
            regeneration_owner.character_id,
            SessionGeneration(1),
            1,
            ZoneCommand::ChangeClass {
                entity: regeneration_owner.character_id,
                account: AccountId::from_uuid(uuid::Uuid::from_u128(1)),
                request_key: uuid::Uuid::from_u128(100),
                target: ClassId(15),
            },
        )])?;
        regen.tick(vec![ZoneInput::session(
            regeneration_owner.character_id,
            SessionGeneration(1),
            2,
            ZoneCommand::LearnSkill {
                entity: regeneration_owner.character_id,
                account: AccountId::from_uuid(uuid::Uuid::from_u128(1)),
                request_key: uuid::Uuid::from_u128(101),
                skill_id: SkillId(1044),
                skill_level: 1,
            },
        )])?;
        regen.cast(regeneration_owner.character_id, regeneration_owner.character_id, 1044)?;
        regen.wait(90)?;
        regen.recording.write(&out.join("modified-regen.nfr"))?;
        for cap in [false, true] {
            let pack = packs
                .get(if cap {
                    "phase3-penalty-cap"
                } else {
                    "phase3-trainer"
                })
                .context("death pack")?;
            let mut victim = pack.get("owner").context("victim")?.clone();
            victim.position = [110, 100];
            victim.hp = Some(1);
            victim.cp = 0;
            let mut death = EvidenceRecording::new(prepared.state(
                ZoneSeed {
                    zone: prepared.zone.zone,
                    epoch: if cap { 96 } else { 95 },
                },
                0,
            )?);
            death.spawn(&victim, evidence_load(&victim, 1))?;
            death.spawn(&peer, evidence_load(&peer, 2))?;
            death.cast(peer.character_id, victim.character_id, 1177)?;
            ensure!(
                death
                    .state
                    .entity(victim.character_id)
                    .context("victim")?
                    .combat
                    .as_ref()
                    .context("combat")?
                    .hp
                    == 0,
                "death transition required"
            );
            death.reconnect(&victim).context("death reconnect")?;
            death.recording.write(&out.join(if cap {
                "death-cap.nfr"
            } else {
                "death-reconnect.nfr"
            }))?;
        }
        for name in [
            "actual-inputs",
            "modified-regen",
            "pvp-inputs",
            "death-reconnect",
            "death-cap",
        ] {
            let exported = std::process::Command::new(env!("CARGO_BIN_EXE_nightfall-replay"))
                .arg("export")
                .arg("--file")
                .arg(out.join(format!("{name}.nfr")))
                .arg("--evidence-out")
                .arg(out.join(format!("{name}.jsonl")))
                .output()?;
            ensure!(
                exported.status.success(),
                "native export: {}",
                String::from_utf8_lossy(&exported.stderr)
            );
        }
        Ok(())
    })();
    std::fs::remove_dir_all(&work)?;
    result
}
