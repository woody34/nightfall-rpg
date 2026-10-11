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

#[derive(Deserialize)]
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
