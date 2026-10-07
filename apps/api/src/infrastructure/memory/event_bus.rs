use async_trait::async_trait;
use parking_lot::Mutex;

use crate::application::EventBus;
use crate::domain::DomainEvent;

/// Records every published event so tests can assert on them.
#[derive(Default)]
pub struct InMemoryEventBus {
    events: Mutex<Vec<DomainEvent>>,
}

impl InMemoryEventBus {
    /// Snapshot of everything published so far, in order.
    #[must_use]
    pub fn published(&self) -> Vec<DomainEvent> {
        self.events.lock().clone()
    }
}

#[async_trait]
impl EventBus for InMemoryEventBus {
    async fn publish(&self, event: &DomainEvent) -> anyhow::Result<()> {
        self.events.lock().push(event.clone());
        Ok(())
    }
}
