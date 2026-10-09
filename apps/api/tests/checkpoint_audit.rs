//! Checkpoint save acknowledgements travel on the session audit stream, never as wire frames.
#![allow(missing_docs, clippy::unwrap_used)]
use futures_util::StreamExt as _;
use nightfall_api::application::checkpoint::CheckpointAck;
use nightfall_api::application::{IdempotencyKey, SessionAudit};
use nightfall_api::domain::{CharacterId, SessionId};
use nightfall_api::infrastructure::eventlog::{CheckpointAudit, JetStreamEventLog};
use nightfall_api::infrastructure::memory::{AuditedFrame, InMemorySessionAudit};
use nightfall_api::infrastructure::telemetry::Metrics;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

#[tokio::test]
async fn committed_ack_is_recorded_on_the_session_checkpoint_subject() {
    let Ok(url) = std::env::var("NATS_URL") else {
        return;
    };
    let client = async_nats::connect(url).await.unwrap();
    let metrics = Metrics::detached();
    let _log = JetStreamEventLog::connect(client.clone(), metrics.clone())
        .await
        .unwrap();
    let memory = Arc::new(InMemorySessionAudit::default());
    let session = SessionId::new();
    let subject = format!("nightfall.session.{session}.checkpoint");
    let mut sub = client.subscribe(subject).await.unwrap();
    let audit = CheckpointAudit::spawn(client, memory.clone(), Arc::new(metrics));
    let ack = CheckpointAck {
        character_id: CharacterId::new(),
        key: IdempotencyKey::from_uuid(Uuid::now_v7()),
        revision: 7,
    };
    audit.record_checkpoint(session, &ack);
    let message = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::from_slice::<CheckpointAck>(&message.payload).unwrap(), ack);
    assert_eq!(memory.frames(session), vec![AuditedFrame::Checkpoint(ack)]);
}
