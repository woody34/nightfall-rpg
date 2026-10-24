//! D162: the recorded order of one tick's concurrent checkpoint saves.
//!
//! The checkpoint lane saves the due owners of one tick concurrently, but every recorded
//! order must stay the order of the serial path. Two mechanisms give that:
//!
//! 1. **Pre-assigned identity values.** Before any save of the tick starts, the lane reserves
//!    the outbox and item-ledger identity values that the tick's rows will take, in owner
//!    (entity) order, through [`super::ports::CharacterRepository::reserve_commit_order`]. Each
//!    owner's save receives its slice as a [`CommitOrderReservation`] and the adapter writes
//!    those values (`OVERRIDING SYSTEM VALUE`), so the outbox sequence and the ledger row
//!    positions do not depend on which transaction inserts first.
//! 2. **Commit turns.** The outbox relay publishes rows in id order and must never see a later
//!    owner's rows before an earlier owner's rows are committed. Each owner save runs inside a
//!    [`Slot`] scope; the adapter calls [`wait_turn`] after the whole body of its transaction
//!    (load, validation, every insert) and right before the shared item range high-water mark
//!    and the commit. The turn opens only when the previous owner (in entity order) has
//!    finished its save (committed, replayed or failed), so commits become visible in id order
//!    and the shared range row is never contended. A failed predecessor leaves a gap in the
//!    reserved ids and opens the turn: every started save settles, as before D162, and the
//!    lane reports the first failure in entity order after the batch.
//!
//! Only owners that write ordered rows (outbox facts, a learn receipt, item rows) are in the
//! chain. An owner that writes none has nothing with a global order and keeps the old
//! unchained, fully concurrent save ([`unordered`]).
//!
//! Outside a slot scope (recovery, direct adapter calls, tests) [`wait_turn`] returns at once
//! and [`reserved`] is `None`, so the adapters fall back to identity defaults.

use std::future::Future;

use tokio::sync::watch;

/// The identity values one owner's save writes, in row order: `outbox_ids[i]` is the id of the
/// owner's `i`-th outbox row and `ledger_ids[i]` of its `i`-th item ledger row. Empty vectors
/// mean "let the database assign" (a tick with at most one owner that writes rows, or an
/// adapter without identity columns).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitOrderReservation {
    /// Pre-assigned `outbox.id` values, ascending.
    pub outbox_ids: Vec<i64>,
    /// Pre-assigned `item_ledger.id` values, ascending.
    pub ledger_ids: Vec<i64>,
}

impl CommitOrderReservation {
    /// Splits off the first `outbox_rows` outbox ids and `ledger_rows` ledger ids for one owner.
    /// Returns `None` when the reservation is shorter than requested.
    pub fn take(&mut self, outbox_rows: usize, ledger_rows: usize) -> Option<Self> {
        if self.outbox_ids.len() < outbox_rows || self.ledger_ids.len() < ledger_rows {
            return None;
        }
        let rest_outbox = self.outbox_ids.split_off(outbox_rows);
        let rest_ledger = self.ledger_ids.split_off(ledger_rows);
        Some(Self {
            outbox_ids: std::mem::replace(&mut self.outbox_ids, rest_outbox),
            ledger_ids: std::mem::replace(&mut self.ledger_ids, rest_ledger),
        })
    }

    /// True when nothing is pre-assigned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.outbox_ids.is_empty() && self.ledger_ids.is_empty()
    }
}

/// The predecessor's completion signal: the sender side is dropped when the predecessor's save
/// returns, whatever its outcome.
#[derive(Clone, Debug)]
pub struct Turn {
    previous: Option<watch::Receiver<()>>,
}

impl Turn {
    /// Waits until the predecessor has finished; returns at once without a predecessor.
    async fn wait(&self) {
        if let Some(previous) = &self.previous {
            let mut previous = previous.clone();
            // The sender never sends; `changed` fails only when the predecessor dropped it.
            while previous.changed().await.is_ok() {}
        }
    }
}

/// Everything one owner's save needs from the tick's commit order.
#[derive(Clone, Debug)]
pub struct Slot {
    turn: Turn,
    reservation: CommitOrderReservation,
}

impl Slot {
    /// A slot with `turn` and this owner's pre-assigned identity values.
    #[must_use]
    pub fn new(turn: Turn, reservation: CommitOrderReservation) -> Self {
        Self { turn, reservation }
    }
}

tokio::task_local! {
    static SLOT: Slot;
}

/// The release half that an owner holds until its save has returned; dropping it opens the
/// successor's turn.
#[derive(Debug)]
pub struct Release(#[allow(dead_code)] watch::Sender<()>);

/// Builds the turn chain for `count` owners in commit order: owner `i` waits for owner
/// `i - 1`, and the first owner does not wait.
#[must_use]
pub fn chain(count: usize) -> Vec<(Turn, Release)> {
    let mut previous = None;
    let mut turns = Vec::with_capacity(count);
    for _ in 0..count {
        let (sender, receiver) = watch::channel(());
        turns.push((
            Turn {
                previous: previous.take(),
            },
            Release(sender),
        ));
        previous = Some(receiver);
    }
    turns
}

/// A turn that does not wait and that nobody waits for: for an owner whose save writes no row
/// with a global order, so it is not in the chain.
#[must_use]
pub fn unordered() -> (Turn, Release) {
    let (sender, _) = watch::channel(());
    (Turn { previous: None }, Release(sender))
}

/// Runs `save` with `slot` visible to [`wait_turn`] and [`reserved`].
pub async fn scoped<F: Future>(slot: Slot, save: F) -> F::Output {
    SLOT.scope(slot, save).await
}

/// Waits until every earlier owner of this tick's chain has finished its save. Returns at once
/// outside a slot scope or for the first owner of the chain.
pub async fn wait_turn() {
    let turn = SLOT.try_with(|slot| slot.turn.clone()).ok();
    if let Some(turn) = turn {
        turn.wait().await;
    }
}

/// This save's pre-assigned identity values, or `None` outside a slot scope.
#[must_use]
pub fn reserved() -> Option<CommitOrderReservation> {
    SLOT.try_with(|slot| slot.reservation.clone()).ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn turns_open_in_chain_order_and_without_a_scope() {
        wait_turn().await;
        assert!(reserved().is_none());
        let order = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let mut saves = Vec::new();
        for (index, (turn, release)) in chain(4).into_iter().enumerate().rev() {
            let order = order.clone();
            saves.push(async move {
                scoped(Slot::new(turn, CommitOrderReservation::default()), async {
                    tokio::task::yield_now().await;
                    wait_turn().await;
                    order.lock().push(index);
                })
                .await;
                drop(release);
            });
        }
        futures_util::future::join_all(saves).await;
        assert_eq!(*order.lock(), vec![0, 1, 2, 3]);
    }

    #[tokio::test]
    async fn a_failed_predecessor_still_opens_the_turn_and_unordered_owners_never_wait() {
        let mut turns = chain(2).into_iter();
        let (_, first_release) = turns.next().unwrap();
        let (second_turn, _) = turns.next().unwrap();
        let (free_turn, _free_release) = unordered();
        scoped(Slot::new(free_turn, CommitOrderReservation::default()), wait_turn()).await;
        let second = scoped(Slot::new(second_turn, CommitOrderReservation::default()), async {
            wait_turn().await;
            reserved()
        });
        // The first owner "fails": its release is dropped without any commit.
        drop(first_release);
        let seen = tokio::time::timeout(std::time::Duration::from_secs(5), second)
            .await
            .unwrap();
        assert_eq!(seen, Some(CommitOrderReservation::default()));
    }

    #[test]
    fn reservation_slices_in_order() {
        let mut all = CommitOrderReservation {
            outbox_ids: vec![10, 11, 12],
            ledger_ids: vec![7, 8],
        };
        let first = all.take(2, 0).unwrap();
        assert_eq!(first.outbox_ids, vec![10, 11]);
        assert!(first.ledger_ids.is_empty());
        let second = all.take(1, 2).unwrap();
        assert_eq!(second.outbox_ids, vec![12]);
        assert_eq!(second.ledger_ids, vec![7, 8]);
        assert!(all.is_empty());
        assert!(all.take(1, 0).is_none());
    }
}
