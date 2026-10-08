//! Accounts: who is playing. The identity provider owns credentials; the game only knows the
//! account id, which is the identity provider's `sub` claim.

use super::ids::uuid_id;

uuid_id!(
    /// Newtype over the account UUID (the Keycloak user id).
    AccountId
);

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn parses_uuid_and_rejects_garbage() {
        let id: AccountId = "fb71069a-b366-48df-9a4f-7093b3ae7da7".parse().unwrap();
        assert_eq!(id.to_string(), "fb71069a-b366-48df-9a4f-7093b3ae7da7");
        assert!("testplayer".parse::<AccountId>().is_err());
        assert!("".parse::<AccountId>().is_err());
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
