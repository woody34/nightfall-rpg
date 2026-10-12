//! Actor control scheduling; durable reservation constraints have separate memory/Pg tests.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::too_many_lines)]
use super::*;
use crate::application::ports::{ItemAdmissionRepository, ItemIdRange, PendingItemRequest};
use crate::application::{CharacterRepository, CheckpointError};
use crate::domain::zone::{
    AppliedTickDraft, ItemInstanceRange, Vec2Fixed, ZoneBounds, ZoneCommand, ZoneId, ZoneSeed,
};
use crate::domain::CharacterId;
use crate::infrastructure::memory::{InMemoryCharacterRepository, InMemorySessionAudit};
use crate::infrastructure::{class_data, item_data, rules_data, telemetry::Metrics};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Reservations {
    calls: AtomicUsize,
    mode: u8,
}
#[async_trait::async_trait]
impl ItemAdmissionRepository for Reservations {
    async fn reserve_item_range(&self, _: ZoneId, _: u64) -> Result<ItemIdRange, CheckpointError> {
        Err(CheckpointError::Constraint("unexpected_initial_reservation".into()))
    }
    async fn reserve_item_range_extension(
        &self,
        zone: ZoneId,
        epoch: u64,
        start: u64,
    ) -> Result<ItemIdRange, CheckpointError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.mode == 1 {
            return Err(CheckpointError::Constraint("not_durably_exhausted".into()));
        }
        let range_start = start.checked_add(10).unwrap();
        Ok(ItemIdRange {
            zone_id: zone,
            epoch,
            range_start,
            range_end_exclusive: range_start.checked_add(10).unwrap(),
            next_reserved: range_start.checked_add(u64::from(self.mode == 2)).unwrap(),
        })
    }
    async fn pending_item_requests(
        &self,
        _: CharacterId,
    ) -> anyhow::Result<Vec<PendingItemRequest>> {
        Ok(Vec::new())
    }
}

#[derive(Clone)]
struct HeldGate(Arc<parking_lot::Mutex<Vec<AppliedTick>>>);
impl TickGate for HeldGate {
    fn max_record_bytes(&self) -> Option<usize> {
        Some(1_048_576)
    }
    fn durable(&self) -> bool {
        true
    }
    fn admit(&self, tick: &AppliedTick) -> impl Future<Output = Result<(), GateError>> + Send {
        let mut attempts = self.0.lock();
        attempts.push(tick.clone());
        std::future::ready(if attempts.len() == 1 {
            Err(GateError("withheld acknowledgement".into()))
        } else {
            Ok(())
        })
    }
}

#[tokio::test]
async fn automatic_allocator_extension_shares_queue_budget_and_retries_exact_record() {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        InMemoryCharacterRepository::default()
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    let start = 1_u64 << 32;
    let end = start.checked_add(10).unwrap();
    let initial = ZoneState::new_phase4(
        ZoneSeed {
            zone: ZoneId(92),
            epoch: 7,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        rules_data::load_rules(&rules_data::RulesSource::embedded())
            .unwrap()
            .rules,
        class_data::load_classes(&class_data::ClassSource::embedded())
            .unwrap()
            .registry,
        (*bundle.skills).clone(),
        bundle.items,
        ItemInstanceRange::new(start, end, end).unwrap(),
    )
    .unwrap()
    .snapshot();
    for mode in 0..3 {
        let reservations = Arc::new(Reservations {
            calls: AtomicUsize::new(0),
            mode,
        });
        let attempts = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let state = crate::application::replay_log::restore_snapshot(initial.clone()).unwrap();
        let (ticks, driver) = manual_ticks();
        let handle = ZoneActor::spawn_gated(state, ticks, HeldGate(attempts.clone()));
        handle.checkpoints.install(
            super::super::checkpoint::CheckpointService::new(
                repo.clone() as Arc<dyn CharacterRepository>,
                Arc::new(InMemorySessionAudit::default()),
                Arc::new(Metrics::detached()),
            )
            .with_items(repo.clone())
            .with_item_admission(reservations.clone()),
        );
        let mut outputs = handle.subscribe();
        if mode == 0 {
            for index in 0..COMMAND_QUEUE {
                handle
                    .send(ZoneInput::system(ZoneCommand::StopMove {
                        entity: EntityId::from_uuid(uuid::Uuid::from_u128(
                            u128::try_from(index).unwrap().checked_add(1).unwrap(),
                        )),
                    }))
                    .unwrap();
            }
        }
        assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(initial.tick));
        assert_eq!(reservations.calls.load(Ordering::SeqCst), 1);
        assert!(outputs.try_recv().is_err());
        if mode != 0 {
            handle.stopped().await;
            assert!(handle.persistence_failed());
            assert!(attempts.lock().is_empty());
            continue;
        }
        assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(initial.tick));
        let first = outputs.recv().await.unwrap();
        assert_eq!(first.commands.len(), PENDING_LIMIT);
        assert!(matches!(first.commands[0].command, ZoneCommand::ExtendItemRange { .. }));
        for (index, command) in first.commands.iter().skip(1).enumerate() {
            assert!(matches!(command.command, ZoneCommand::StopMove { entity }
                if entity.as_uuid().as_u128() == u128::try_from(index).unwrap().checked_add(1).unwrap()));
        }
        {
            let calls = attempts.lock();
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[0], calls[1]);
        }
        driver.step().await.unwrap();
        let tail = outputs.recv().await.unwrap();
        assert_eq!(tail.commands.len(), 1);
        assert!(
            matches!(tail.commands[0].command, ZoneCommand::StopMove { entity } if entity.as_uuid().as_u128() == u128::try_from(COMMAND_QUEUE).unwrap())
        );
        assert_eq!(reservations.calls.load(Ordering::SeqCst), 1);
        let mut cold = crate::application::replay_log::restore_snapshot(initial.clone()).unwrap();
        for applied in [first, tail] {
            assert_eq!(
                *applied,
                cold.run_tick(AppliedTickDraft {
                    epoch: applied.epoch,
                    tick: applied.tick,
                    commands: applied.commands.clone(),
                })
                .unwrap()
            );
        }
        assert_eq!(
            cold.item_instance_range().unwrap().bounds(),
            (end, end, end.checked_add(10).unwrap())
        );
        drop(driver);
        handle.stopped().await;
        assert!(!handle.persistence_failed());
    }
}
