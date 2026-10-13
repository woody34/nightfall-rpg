//! Actor control scheduling; durable reservation constraints have separate memory/Pg tests.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::too_many_lines)]
use super::*;
use crate::application::ports::{ItemAdmissionRepository, ItemIdRange, PendingItemRequest};
use crate::application::CheckpointError;
use crate::domain::zone::{AppliedTickDraft, ItemInstanceRange, ZoneCommand, ZoneId};
use crate::domain::CharacterId;
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
    let start = 1_u64 << 32;
    let end = start.checked_add(10).unwrap();
    for mode in 0..4 {
        let (repo, service, mut initial, owner, known) =
            super::super::checkpoint::item_tests::allocator_fixture().await;
        let ledger_count = repo.item_ledger_for_test().len();
        initial.phase4.as_mut().unwrap().allocator =
            ItemInstanceRange::new(start, end, end).unwrap();
        let reservations = Arc::new(Reservations {
            calls: AtomicUsize::new(0),
            mode: if mode == 3 { 1 } else { mode },
        });
        let attempts = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let state = crate::application::replay_log::restore_snapshot(initial.clone()).unwrap();
        let (ticks, driver) = manual_ticks();
        let handle = ZoneActor::spawn_gated(state, ticks, HeldGate(attempts.clone()));
        handle
            .checkpoints
            .install(service.with_item_admission(reservations.clone()));
        let mut outputs = handle.subscribe();
        let command = if mode == 3 {
            known
        } else {
            crate::domain::zone::ItemCommand::new(
                owner,
                uuid::Uuid::now_v7(),
                crate::domain::zone::ItemCommandKind::Crystallize {
                    instance: crate::domain::zone::ItemInstanceId::new(99_999).unwrap(),
                },
            )
            .unwrap()
        };
        handle
            .send(ZoneInput::session(
                owner,
                crate::domain::zone::SessionGeneration(1),
                99,
                ZoneCommand::Item(Box::new(command)),
            ))
            .unwrap();
        if mode == 0 {
            for index in 1..COMMAND_QUEUE {
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
        assert_eq!(reservations.calls.load(Ordering::SeqCst), usize::from(mode != 3));
        assert!(outputs.try_recv().is_err());
        if mode == 1 || mode == 2 {
            handle.stopped().await;
            assert!(handle.persistence_failed());
            assert!(attempts.lock().is_empty());
            continue;
        }
        assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(initial.tick));
        let first = outputs.recv().await.unwrap();
        if mode == 3 {
            assert_eq!(repo.item_ledger_for_test().len(), ledger_count);
            assert_eq!(first.commands.len(), 1);
            assert!(
                matches!(&first.commands[0].command, ZoneCommand::Item(item) if item.known_result().unwrap().is_some())
            );
            let mut cold =
                crate::application::replay_log::restore_snapshot(initial.clone()).unwrap();
            assert_eq!(
                *first,
                cold.run_tick(AppliedTickDraft {
                    epoch: first.epoch,
                    tick: first.tick,
                    commands: first.commands.clone()
                })
                .unwrap()
            );
            assert_eq!(cold.item_instance_range().unwrap().bounds(), (start, end, end));
            assert_eq!(reservations.calls.load(Ordering::SeqCst), 0);
            drop(driver);
            handle.stopped().await;
            assert!(!handle.persistence_failed());
            continue;
        }
        assert_eq!(first.commands.len(), PENDING_LIMIT);
        assert!(matches!(first.commands[0].command, ZoneCommand::ExtendItemRange { .. }));
        for (index, command) in first.commands.iter().skip(2).enumerate() {
            assert!(matches!(command.command, ZoneCommand::StopMove { entity }
                if entity.as_uuid().as_u128() == u128::try_from(index).unwrap().checked_add(2).unwrap()));
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
        assert_eq!(reservations.calls.load(Ordering::SeqCst), usize::from(mode != 3));
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
