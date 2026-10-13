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
    /// D113: dev-fixture birth enchant level of the single granted equipment instance.
    /// Zero is omitted, so every earlier recorded grant keeps its exact canonical bytes.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub enchant: u32,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if signature.
const fn is_zero(value: &u32) -> bool {
    *value == 0
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
            || (self.enchant != 0 && self.count != 1)
            || self.fingerprint
                != grant_fingerprint_with_enchant(
                    CharacterId::from_uuid(self.entity.as_uuid()),
                    self.template_id,
                    self.count,
                    self.enchant,
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

/// D113 body hash: identical to [`grant_fingerprint`] without an enchant level, so existing
/// keys and recordings are unchanged; a birth enchant level is part of the body.
#[must_use]
pub fn grant_fingerprint_with_enchant(
    character: CharacterId,
    template: u32,
    count: u64,
    enchant: u32,
) -> String {
    if enchant == 0 {
        return grant_fingerprint(character, template, count);
    }
    let mut hash = Sha256::new();
    hash.update(b"nightfall.item.grant.2\0");
    hash.update(character.as_uuid().as_bytes());
    hash.update(template.to_be_bytes());
    hash.update(count.to_be_bytes());
    hash.update(enchant.to_be_bytes());
    format!("{:x}", hash.finalize())
}

/// Ceiling for a template whose matching scroll has no maximum (`max_enchant = 0`).
pub const MAX_GRANT_ENCHANT: u32 = 65_535;

/// D113: the highest birth enchant level a dev grant may carry for `template`, i.e. the
/// level the template's enchant rules can reach with some matching scroll: an admitted,
/// enchantable, non-stackable weapon or armor outside the blacklist, at most one past the
/// highest matching scroll's `max_enchant` (or [`MAX_GRANT_ENCHANT`] when one is unbounded).
/// Zero means the template cannot carry an enchant level.
#[must_use]
pub fn grant_enchant_limit(rules: &super::Phase4Rules, template: u32) -> u32 {
    let parts = rules.parts();
    let Some(definition) = parts.items.get(&template) else {
        return 0;
    };
    if definition.stackable
        || !matches!(definition.kind.as_str(), "Weapon" | "Armor")
        || definition.enchant_enabled == 0
        || parts.enchant_blacklist.contains(&template)
        || !parts
            .capabilities
            .get(&template)
            .is_some_and(|c| c.admitted && c.storable)
    {
        return 0;
    }
    let grade = match definition.grade.as_str() {
        "S80" | "S84" => "S",
        grade => grade,
    };
    parts
        .enchant_scrolls
        .values()
        .filter(|rule| {
            rule.target_grade == grade
                && rule.weapon == (definition.kind == "Weapon")
                && (rule.allowed_items.is_empty() || rule.allowed_items.contains(&template))
        })
        .map(|rule| {
            if rule.max_enchant == 0 {
                MAX_GRANT_ENCHANT
            } else {
                rule.max_enchant.saturating_add(1).min(MAX_GRANT_ENCHANT)
            }
        })
        .max()
        .unwrap_or(0)
}
