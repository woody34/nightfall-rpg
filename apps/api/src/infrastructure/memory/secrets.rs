use std::sync::atomic::{AtomicU64, Ordering};

use crate::application::SecretGenerator;
use crate::domain::PlayTicket;

/// Deterministic, distinct tickets: the counter in the first 8 bytes. Tests only.
#[derive(Default)]
pub struct SequentialSecretGenerator {
    next: AtomicU64,
}

impl SecretGenerator for SequentialSecretGenerator {
    fn play_ticket(&self) -> anyhow::Result<PlayTicket> {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        let mut bytes = [0x5a; PlayTicket::LEN];
        for (b, c) in bytes.iter_mut().zip(n.to_be_bytes()) {
            *b = c;
        }
        Ok(PlayTicket::from_bytes(bytes))
    }
}

/// Always the same ticket, so a test can look for it in logs. Issue it once per store.
pub struct FixedSecretGenerator(pub [u8; PlayTicket::LEN]);

impl SecretGenerator for FixedSecretGenerator {
    fn play_ticket(&self) -> anyhow::Result<PlayTicket> {
        Ok(PlayTicket::from_bytes(self.0))
    }
}
