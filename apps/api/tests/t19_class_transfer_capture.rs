//! T19 captured history (i3-15c `2-class-transfer`, the owned Phase 2 fixture booted V6): the
//! class/Phase 3 RPCs rebuilt the live zone with the V5-only `ZoneState::from_snapshot` and
//! answered UNAVAILABLE "invalid zone snapshot". The recording replays byte-exact, and at every
//! tick the live snapshot restores through the version-resolving `restore_snapshot` that the
//! RPCs now use, with the recorded player's class state and transfer options readable.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]
use api::application::replay_log::{restore_snapshot, AppliedTickRecord};
use api::domain::zone::{AppliedTickDraft, ZoneCommand, ZoneState};
use api::infrastructure::eventlog::Recording;
use nightfall_api as api;

const CAPTURE: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/sessions/t19-v6-class-transfer.nfr");

#[test]
fn captured_v6_class_transfer_snapshots_restore_for_the_class_rpcs() {
    let recording = Recording::read(std::path::Path::new(CAPTURE)).expect("V6 bundle by hash");
    assert!(recording.snapshot.phase4.is_some(), "V6 epoch");
    let zone = recording.snapshot.seed.zone;
    let epoch = recording.snapshot.seed.epoch;
    let mut state = restore_snapshot(recording.snapshot.clone()).unwrap();
    let mut players = Vec::new();
    let mut checked = 0_usize;
    for record in &recording.records {
        let tick = state
            .run_tick(AppliedTickDraft {
                epoch,
                tick: record.tick,
                commands: record.commands.clone(),
            })
            .unwrap();
        assert!(record.reproduced_by(&AppliedTickRecord::from_applied(zone, &tick)));
        for command in &record.commands {
            if let ZoneCommand::SpawnPlayer {
                entity,
                generation,
                load: Some(load),
                ..
            } = &command.command
            {
                let account = load.progression.as_ref().unwrap().identity.account_id;
                players.push((*entity, account, *generation));
            }
        }
        let snapshot = state.snapshot();
        let online: Vec<_> = players
            .iter()
            .filter(|(entity, ..)| snapshot.entities.iter().any(|e| e.id == *entity))
            .copied()
            .collect();
        if online.is_empty() {
            continue;
        }
        // The old read path refuses every V6 snapshot; the RPC read path restores it.
        assert!(ZoneState::from_snapshot(snapshot.clone()).is_err());
        let live = restore_snapshot(snapshot).unwrap();
        for (entity, account, generation) in online {
            live.class_player(entity, account, generation).unwrap();
            let _options = live.class_options(entity);
            checked += 1;
        }
    }
    assert!(checked > 0, "the recorded player was online");
}
