//! Admission to the real-time channel: play tickets and session generations
//! (plan Revision 1, items 7 and 8).

use std::fmt;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

/// A single-use secret that admits one WebSocket connection. 32 random bytes.
///
/// `Debug` is redacted and there is no `Display`: the only way to see the secret is
/// [`PlayTicket::encode`], which the transport calls once to build the response.
#[derive(Clone, PartialEq, Eq)]
pub struct PlayTicket([u8; PlayTicket::LEN]);

impl PlayTicket {
    /// Length of the secret in bytes.
    pub const LEN: usize = 32;
    /// How long a ticket stays valid after issue.
    pub const TTL_SECONDS: i64 = 60;

    /// Wraps random bytes. Callers obtain them from a cryptographically secure source.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// The wire form: base64url without padding.
    #[must_use]
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    /// Parses the wire form. Anything that is not exactly 32 bytes of base64url is rejected.
    pub fn parse(raw: &str) -> Result<Self, TicketFormatError> {
        let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| TicketFormatError)?;
        <[u8; Self::LEN]>::try_from(bytes.as_slice())
            .map(Self)
            .map_err(|_| TicketFormatError)
    }

    /// SHA-256 of the secret: what is stored and looked up, so the database never holds a
    /// usable ticket in its ticket table.
    #[must_use]
    pub fn hash(&self) -> TicketHash {
        let digest = ring::digest::digest(&ring::digest::SHA256, &self.0);
        let mut out = [0u8; TicketHash::LEN];
        out.copy_from_slice(digest.as_ref());
        TicketHash(out)
    }
}

impl fmt::Debug for PlayTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PlayTicket(<redacted>)")
    }
}

/// The ticket was not 32 bytes of base64url.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("malformed play ticket")]
pub struct TicketFormatError;

/// SHA-256 of a [`PlayTicket`]. Not secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TicketHash([u8; TicketHash::LEN]);

impl TicketHash {
    /// Digest length in bytes.
    pub const LEN: usize = 32;

    /// Wraps a stored digest; `None` unless it is exactly 32 bytes.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        <[u8; Self::LEN]>::try_from(bytes).ok().map(Self)
    }

    /// The digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

/// Per-account admission counter. Every issued ticket bumps it; a ticket (and, in Epic 4, a
/// socket) carrying an older generation than the account's current one has been superseded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionGeneration(i32);

impl SessionGeneration {
    /// The generation of an account's first ticket.
    pub const FIRST: Self = Self(1);

    /// Wraps a stored value; `None` unless positive.
    #[must_use]
    pub const fn new(value: i32) -> Option<Self> {
        if value > 0 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// The next generation, `None` on overflow (two billion tickets for one account).
    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// The stored value.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn ticket_round_trips_through_its_wire_form() {
        let t = PlayTicket::from_bytes([7; 32]);
        let wire = t.encode();
        assert_eq!(wire.len(), 43, "32 bytes in unpadded base64url");
        assert!(!wire.contains('='));
        assert_eq!(PlayTicket::parse(&wire).unwrap(), t);
    }

    #[test]
    fn ticket_parse_rejects_wrong_length_and_alphabet() {
        assert!(PlayTicket::parse("").is_err());
        assert!(PlayTicket::parse(&URL_SAFE_NO_PAD.encode([1u8; 31])).is_err());
        assert!(PlayTicket::parse(&URL_SAFE_NO_PAD.encode([1u8; 33])).is_err());
        let padded = base64::engine::general_purpose::URL_SAFE.encode([1u8; 32]);
        assert!(PlayTicket::parse(&padded).is_err(), "padding is not the wire form");
        assert!(PlayTicket::parse(&"+".repeat(43)).is_err(), "standard alphabet");
    }

    #[test]
    fn debug_never_shows_the_secret() {
        let t = PlayTicket::from_bytes([0xAB; 32]);
        let shown = format!("{t:?}");
        assert!(!shown.contains(&t.encode()));
        assert!(!shown.to_lowercase().contains("abab"));
    }

    #[test]
    fn hash_is_sha256() {
        // SHA-256 of 32 zero bytes.
        let h = PlayTicket::from_bytes([0; 32]).hash();
        assert_eq!(h.as_bytes()[..4], [0x66, 0x68, 0x7a, 0xad]);
        assert_ne!(PlayTicket::from_bytes([1; 32]).hash(), h);
    }

    #[test]
    fn generation_boundaries() {
        assert_eq!(SessionGeneration::new(0), None);
        assert_eq!(SessionGeneration::new(1), Some(SessionGeneration::FIRST));
        assert_eq!(SessionGeneration::FIRST.next().unwrap().get(), 2);
        assert_eq!(SessionGeneration::new(i32::MAX).unwrap().next(), None);
    }
}
