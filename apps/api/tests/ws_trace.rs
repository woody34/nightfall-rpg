//! A `MoveTo` is one trace from the socket through the zone actor to the broadcast (plan §8
//! #18): `ws.frame` is the root, `zone.apply` (in the actor) and `ws.deliver` (back in the
//! session, when the response is queued) are its children.

#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use common::capture::Capture;
use common::ws::{ack, join, move_to, seed_player};
use common::TestApp;
use serde_json::Value;

/// Names of the span stack (outermost first) of every captured event with `message`.
fn stacks(logged: &str, message: &str) -> Vec<Vec<String>> {
    logged
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["fields"]["message"] == message)
        .map(|v| {
            v["spans"]
                .as_array()
                .map(|spans| {
                    spans
                        .iter()
                        .filter_map(|s| s["name"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect()
}

#[tokio::test]
async fn move_to_spans_socket_actor_and_broadcast_in_one_trace() {
    let capture = Capture::default();
    let _guard = capture.install();

    let app = TestApp::spawn().await;
    let mut ws = join(&app, &seed_player(&app, "Aria", 10.0, 10.0)).await;
    ws.send(&move_to(7, 12.0, 10.0)).await;
    ws.until(|m| ack(m).is_some_and(|a| a.seq == 7)).await;
    ws.close().await;

    let logged = capture.text();
    let applied = stacks(&logged, "zone command applied");
    assert_eq!(applied, vec![vec!["ws.frame".to_owned(), "zone.apply".to_owned()]], "{logged}");
    let delivered = stacks(&logged, "intent response queued for the socket");
    assert_eq!(
        delivered,
        vec![vec!["ws.frame".to_owned(), "ws.deliver".to_owned()]],
        "{logged}"
    );
    // The frame span is a root, not a child of the long-lived session span.
    assert!(logged.contains("\"seq\":7"), "{logged}");
}
