//! Secrets from the operating system's CSPRNG.

use ring::rand::{SecureRandom, SystemRandom};

use crate::application::SecretGenerator;
use crate::domain::PlayTicket;

/// [`SecretGenerator`] backed by `ring`'s `SystemRandom` (getrandom).
#[derive(Debug, Clone)]
pub struct OsSecretGenerator {
    rng: SystemRandom,
}

impl Default for OsSecretGenerator {
    fn default() -> Self {
        Self {
            rng: SystemRandom::new(),
        }
    }
}

impl SecretGenerator for OsSecretGenerator {
    fn play_ticket(&self) -> anyhow::Result<PlayTicket> {
        let mut bytes = [0u8; PlayTicket::LEN];
        self.rng
            .fill(&mut bytes)
            .map_err(|_| anyhow::anyhow!("system random number generator failed"))?;
        Ok(PlayTicket::from_bytes(bytes))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn tickets_are_distinct() {
        let g = OsSecretGenerator::default();
        assert_ne!(g.play_ticket().unwrap(), g.play_ticket().unwrap());
    }
}
