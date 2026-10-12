//! Recorded server grants and the complete private paused owner used for offline work.
use super::phase4_projection::ItemReceiptFact;
use super::{EntityId, ItemError, PlayerLoad, Vec2Fixed};
use crate::domain::{AccountId, CharacterId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A repository-loaded owner, never a public admission or a fabricated session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemPausedCore {
    pub name: String,
    pub position: Vec2Fixed,
    pub load: PlayerLoad,
}
impl ItemPausedCore {
    /// Validate identity and retained shape; the executor also validates against epoch rules.
    pub fn validate(&self, account: AccountId) -> Result<(), ItemError> {
        crate::domain::CharacterName::new(self.name.clone())
            .map_err(|_| ItemError("invalid paused owner name"))?;
        if self
            .load
            .checkpoint_revision
            .is_none_or(|r| i64::try_from(r).is_err())
            || self
                .load
                .progression
                .as_ref()
                .is_none_or(|p| p.identity.account_id != account)
            || self.load.phase4.is_none()
        {
            return Err(ItemError("invalid paused owner fence"));
        }
        self.load.validate_phase4()
    }
}

/// Direct `GrantItem` bypasses pending rows (D48). None means the actor selected a live owner.
/// Receipt preflight is private recorded data, impossible to supply through an item intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemGrantCommand {
    pub entity: EntityId,
    pub account: AccountId,
    pub request_id: uuid::Uuid,
    pub template_id: u32,
    pub count: u64,
    pub fingerprint: String,
    pub rules_hash: [u8; 32],
    pub paused: Option<Box<ItemPausedCore>>,
    pub known: Option<ItemReceiptFact>,
    pub conflict: Option<String>,
}
impl ItemGrantCommand {
    /// Bind every normalized body and preflight identity before any allocation or mutation.
    pub fn validate(&self) -> Result<(), ItemError> {
        use super::phase4_projection::ItemOperation;
        if self.entity.as_uuid().is_nil()
            || self.account.as_uuid().is_nil()
            || self.request_id.is_nil()
            || self.template_id == 0
            || self.count == 0
            || i64::try_from(self.count).is_err()
            || self.fingerprint
                != grant_fingerprint(
                    CharacterId::from_uuid(self.entity.as_uuid()),
                    self.template_id,
                    self.count,
                )
            || (self.known.is_some() && self.conflict.is_some())
        {
            return Err(ItemError("invalid recorded grant identity"));
        }
        if let Some(core) = &self.paused {
            core.validate(self.account)?;
        }
        if let Some(known) = &self.known {
            let result = &known.result;
            result.validate()?;
            if known.request_id != self.request_id
                || known.fingerprint != self.fingerprint
                || result.request_fingerprint != known.fingerprint
                || result.entity != self.entity
                || result.receipt != self.request_id
                || result.operation() != ItemOperation::Grant
            {
                return Err(ItemError("grant receipt identity mismatch"));
            }
        }
        if self.conflict.as_ref().is_some_and(|value| {
            value == &self.fingerprint
                || value.len() != 64
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) {
            return Err(ItemError("invalid grant receipt conflict"));
        }
        Ok(())
    }
}

/// Canonical D30 body hash shared by the API boundary and deterministic replay.
#[must_use]
pub fn grant_fingerprint(character: CharacterId, template: u32, count: u64) -> String {
    let mut hash = Sha256::new();
    hash.update(b"nightfall.item.grant.1\0");
    hash.update(character.as_uuid().as_bytes());
    hash.update(template.to_be_bytes());
    hash.update(count.to_be_bytes());
    format!("{:x}", hash.finalize())
}
