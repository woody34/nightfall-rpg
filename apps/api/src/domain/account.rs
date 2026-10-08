//! Accounts: who is playing. The identity provider owns credentials; the game only knows the
//! account id, which is the identity provider's `sub` claim.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Newtype over the account UUID (the Keycloak user id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(Uuid);

impl AccountId {
    /// Wraps an existing UUID (from storage or a verified token).
    #[must_use]
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Parses the textual form, e.g. a token's `sub`.
    pub fn parse(raw: &str) -> Result<Self, AccountIdError> {
        Uuid::parse_str(raw).map(Self).map_err(|_| AccountIdError)
    }

    /// The underlying UUID.
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The input was not a UUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("account id must be a UUID")]
pub struct AccountIdError;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_uuid_and_rejects_garbage() {
        let id = AccountId::parse("fb71069a-b366-48df-9a4f-7093b3ae7da7").unwrap();
        assert_eq!(id.to_string(), "fb71069a-b366-48df-9a4f-7093b3ae7da7");
        assert_eq!(AccountId::parse("testplayer"), Err(AccountIdError));
        assert_eq!(AccountId::parse(""), Err(AccountIdError));
    }

    #[test]
    fn serializes_as_a_bare_uuid() {
        let id = AccountId::from_uuid(Uuid::nil());
        assert_eq!(
            serde_json::to_value(id).unwrap(),
            serde_json::json!("00000000-0000-0000-0000-000000000000")
        );
    }
}
