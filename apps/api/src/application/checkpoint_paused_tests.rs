//! Recorded offline final cores, held writes and recovery without public admission.
use super::*;
use crate::domain::zone::item_grant::grant_fingerprint;
use crate::domain::zone::{ItemGrantCommand, ItemPausedCore, Phase4PlayerLoad, PlayerLoad};

async fn offline() -> (Harness, ItemPausedCore) {
    let mut h = Harness::new().await;
    let snapshot = h.state.snapshot();
    let entity = snapshot.entities.iter().find(|e| e.id == h.entity).unwrap();
    let CombatRole::Player {
        class, progression, ..
    } = &entity.combat.as_ref().unwrap().role
    else {
        panic!();
    };
    let cp = h.service.players[&h.entity].latest.as_ref().unwrap();
    let core = ItemPausedCore {
        name: entity.name.clone(),
        position: entity.pos,
        load: PlayerLoad {
            class: class.clone(),
            progression: progression.clone(),
            checkpoint_revision: Some(cp.revision_seen),
            phase3: cp.phase3.clone(),
            phase4: Some(Phase4PlayerLoad {
                checkpoint: cp.phase4.clone(),
                pending: vec![],
            }),
            level: cp.level,
            xp: cp.xp,
            hp: Some(cp.hp),
            mp: Some(cp.mp),
            alive: cp.alive,
        },
    };
    let tick = h
        .state
        .run_tick(
            h.state
                .draft(vec![ZoneInput::system(ZoneCommand::Despawn { entity: h.entity })]),
        )
        .unwrap();
    h.service
        .admitted(&tick, &h.state.snapshot())
        .await
        .unwrap();
    assert!(!h.service.players.contains_key(&h.entity));
    (h, core)
}
fn inputs(h: &Harness, core: &ItemPausedCore) -> Vec<ZoneInput> {
    [10, 21]
        .into_iter()
        .map(|count| {
            ZoneInput::system(ZoneCommand::MaterializePendingItems(Box::new(ItemGrantCommand {
                entity: h.entity,
                account: h.account,
                request_id: Uuid::from_u128(9900 + u128::from(count)),
                template_id: 57,
                count,
                fingerprint: grant_fingerprint(
                    CharacterId::from_uuid(h.entity.as_uuid()),
                    57,
                    count,
                ),
                rules_hash: core
                    .load
                    .phase4
                    .as_ref()
                    .unwrap()
                    .checkpoint
                    .as_ref()
                    .unwrap()
                    .rules_hash,
                paused: Some(Box::new(core.clone())),
                known: None,
                conflict: None,
            })))
        })
        .collect()
}

#[tokio::test]
async fn paused_item_grant_settles_complete_core_and_two_receipts_without_public_owner() {
    let (mut h, core) = offline().await;
    let before = h.state.snapshot();
    let tick = h.state.run_tick(h.state.draft(inputs(&h, &core))).unwrap();
    assert!(tick.dispositions.is_empty());
    let mut cold = crate::application::replay_log::restore_snapshot(before).unwrap();
    let replayed = cold
        .run_tick(AppliedTickDraft {
            epoch: tick.epoch,
            tick: tick.tick,
            commands: tick.commands.clone(),
        })
        .unwrap();
    assert_eq!(replayed, tick);
    assert!(!h
        .state
        .snapshot()
        .entities
        .iter()
        .any(|entity| entity.id == h.entity));
    h.service
        .admitted(&tick, &h.state.snapshot())
        .await
        .unwrap();
    assert!(!h.service.players.contains_key(&h.entity));
    assert_eq!(h.acknowledgements(), 0);
    let saved = h
        .repo
        .load_for_admission(CharacterId::from_uuid(h.entity.as_uuid()))
        .await
        .unwrap()
        .unwrap();
    let projection = tick
        .events
        .iter()
        .find_map(|event| {
            if let crate::domain::zone::ZoneEvent::ItemCheckpoint(p) = event {
                Some(p.as_ref())
            } else {
                None
            }
        })
        .unwrap();
    let final_core = projection.paused_core.as_ref().unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.hp, final_core.load.hp);
    assert_eq!(saved.mp, final_core.load.mp);
    assert_eq!(saved.phase3, final_core.load.phase3);
    assert_eq!(saved.phase4.as_ref().unwrap().checkpoint, Some(projection.checkpoint.clone()));
    assert_eq!(saved.position, position(core.position));
    assert_eq!(saved.class_state, final_core.load.progression.as_ref().unwrap().class_state);
    for receipt in &projection.receipts {
        let ItemReceiptLookup::Known(stored) = h
            .repo
            .item_receipt_lookup(
                h.account,
                ItemMutationOperation::GrantItem,
                &IdempotencyKey::from_uuid(receipt.request_id),
                &receipt.fingerprint,
            )
            .await
            .unwrap()
        else {
            panic!();
        };
        assert_eq!(stored.result, receipt.result);
        assert_eq!(stored.result.incarnation, 0);
        assert_eq!(stored.inventory.unwrap().character_revision, 2);
    }
    assert_eq!(h.repo.item_ledger_for_test().split_off(h.seeded).len(), 2);
    h.service
        .admitted(&tick, &h.state.snapshot())
        .await
        .unwrap();
    assert_eq!(h.repo.item_ledger_for_test().split_off(h.seeded).len(), 2);
}

#[tokio::test]
async fn paused_item_pending_snapshot_recovers_exact_work_before_later_empty_tick() {
    use crate::application::replay_log::{decode_snapshot, encode_snapshot, EventLog};
    let (mut h, core) = offline().await;
    let tick = h.state.run_tick(h.state.draft(inputs(&h, &core))).unwrap();
    let writer = Arc::new(FlakyItems::new(h.repo.clone(), false, true));
    h.service.item_repo = Some(writer.clone());
    assert!(h
        .service
        .admitted(&tick, &h.state.snapshot())
        .await
        .is_err());
    let attempt = writer.attempts.lock()[0].clone();
    assert_eq!(h.repo.item_ledger_for_test().split_off(h.seeded).len(), 0);
    h.service.players.get_mut(&h.entity).unwrap().fenced = false;
    let mut snapshot = h.state.snapshot();
    h.service.snapshot(&mut snapshot);
    let restored = decode_snapshot(&encode_snapshot(&snapshot).unwrap()).unwrap();
    for field in 0..6 {
        let mut bad = restored.clone();
        let lane = bad
            .checkpoints
            .iter_mut()
            .find(|p| p.entity == h.entity)
            .unwrap();
        match field {
            0 => lane.latest.as_mut().unwrap().hp += 1,
            1 => lane.latest.as_mut().unwrap().mp += 1,
            2 => lane.latest.as_mut().unwrap().revision_seen += 1,
            3 => lane.item_work.as_mut().unwrap().projection.paused_core = None,
            4 => {
                lane.item_work.as_mut().unwrap().projection.account =
                    AccountId::from_uuid(Uuid::from_u128(1234));
            },
            _ => lane.latest.as_mut().unwrap().alive = !lane.latest.as_ref().unwrap().alive,
        }
        assert!(encode_snapshot(&bad).is_err(), "corrupt offline field {field}");
    }
    let log = crate::infrastructure::eventlog::InMemoryEventLog::default();
    log.write_recovery_snapshot(&restored).await.unwrap();
    let later = h.state.run_tick(h.state.draft(vec![])).unwrap();
    log.append_applied(&crate::application::replay_log::AppliedTickRecord::from_applied(
        snapshot.seed.zone,
        &later,
    ))
    .await
    .unwrap();
    let mut recovery =
        CheckpointService::new(h.repo.clone(), h.audit.clone(), Arc::new(Metrics::detached()))
            .with_items(h.repo.clone());
    recovery.restore(&restored);
    assert_eq!(recovery.players[&h.entity].latest.as_ref().unwrap(), &attempt.0);
    assert_eq!(recovery.players[&h.entity].items.as_ref().unwrap(), &attempt.2);
    recovery
        .recover(&log, snapshot.seed.zone, snapshot.seed.epoch)
        .await
        .unwrap();
    assert_eq!(h.repo.item_ledger_for_test().split_off(h.seeded).len(), 2);
    assert_eq!(
        h.repo
            .retained_inventory(attempt.0.character_id)
            .await
            .unwrap()
            .unwrap()
            .character_revision,
        2
    );
    recovery
        .recover(&log, snapshot.seed.zone, snapshot.seed.epoch)
        .await
        .unwrap();
    assert_eq!(h.repo.item_ledger_for_test().split_off(h.seeded).len(), 2);
}
