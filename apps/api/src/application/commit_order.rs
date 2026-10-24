//! D162: commit turns for one tick's concurrent checkpoint saves.
//!
//! The checkpoint lane saves the due owners of one tick concurrently, but every recorded
//! order must stay the order of the serial path. Each owner save runs inside a turn scope.
//! A repository adapter calls [`wait_turn`] after the expensive body of its transaction and
//! before the ordered tail: the item range high-water mark, the item ledger rows, the outbox
//! rows and the commit. The turn opens only when the previous owner (in entity order) has
//! finished its save. Thus identity values, outbox rows and commit visibility have the
//! serial order, and the relay never sees a later owner's rows before an earlier owner's.
//! Outside a turn scope (recovery, direct adapter calls, tests) [`wait_turn`] returns at once.

use std::future::Future;

use tokio::sync::watch;

/// The predecessor's completion signal. The predecessor drops its sender when its save
/// returns (success or failure); a dropped sender opens the turn.
#[derive(Clone, Debug)]
pub struct Turn {
    previous: Option<watch::Receiver<()>>,
}

tokio::task_local! {
    static TURN: Turn;
}

/// The release half that an owner holds until its save has returned.
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

/// A turn that does not wait and opens nothing: for an owner whose save writes no row with a
/// global order (no outbox, ledger or range row), so it is not in the chain.
#[must_use]
pub fn unordered() -> (Turn, Release) {
    let (sender, _) = watch::channel(());
    (Turn { previous: None }, Release(sender))
}

/// Runs `save` with `turn` visible to [`wait_turn`].
pub async fn scoped<F: Future>(turn: Turn, save: F) -> F::Output {
    TURN.scope(turn, save).await
}

/// Waits until every earlier owner of this tick's batch has finished its save.
pub async fn wait_turn() {
    let previous = TURN.try_with(|turn| turn.previous.clone()).ok().flatten();
    if let Some(mut previous) = previous {
        // The sender never sends; `changed` returns only when the predecessor drops it.
        while previous.changed().await.is_ok() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn turns_open_in_chain_order_and_without_a_scope() {
        wait_turn().await;
        let order = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let mut saves = Vec::new();
        for (index, (turn, release)) in chain(4).into_iter().enumerate().rev() {
            let order = order.clone();
            saves.push(async move {
                scoped(turn, async {
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
}
