//! The generic idempotency record (api-guidelines.md section 2): one row per
//! `(account_id, operation, key)` holding the request fingerprint and the stored response.
//! Always used inside the caller's transaction, so the record commits or rolls back with the
//! write it protects.

use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue, DatabaseTransaction, EntityTrait, TryInsertResult};

use super::entities::idempotency_keys;
use crate::application::IdempotencyKey;
use crate::domain::AccountId;

/// Operation names stored in `idempotency_keys.operation`.
pub(super) mod operation {
    /// `GameService.CreateCharacter`.
    pub(crate) const CREATE_CHARACTER: &str = "create_character";
    /// `SessionService.IssuePlayTicket`.
    pub(crate) const ISSUE_PLAY_TICKET: &str = "issue_play_ticket";
}

/// Outcome of [`claim`].
pub(super) enum Claim {
    /// The key is new and now belongs to this request; carry on with the write.
    Claimed,
    /// The key was used before: replay or conflict, depending on the fingerprint.
    Existing {
        /// Fingerprint of the original request.
        fingerprint: String,
        /// Response stored by the original request.
        response: serde_json::Value,
    },
}

/// `INSERT ... ON CONFLICT DO NOTHING` on the key. The primary key serializes concurrent
/// retries: the loser waits for the winner's transaction, sees no inserted row, and reads the
/// winner's record (Read Committed sees it once the winner commits).
pub(super) async fn claim(
    tx: &DatabaseTransaction,
    account: AccountId,
    operation: &str,
    key: &IdempotencyKey,
    fingerprint: &str,
    response: serde_json::Value,
) -> anyhow::Result<Claim> {
    let row = idempotency_keys::ActiveModel {
        account_id: ActiveValue::Set(account.as_uuid()),
        operation: ActiveValue::Set(operation.to_owned()),
        key: ActiveValue::Set(key.as_uuid()),
        fingerprint: ActiveValue::Set(fingerprint.to_owned()),
        response: ActiveValue::Set(response),
        created_at: ActiveValue::NotSet,
    };
    let inserted = idempotency_keys::Entity::insert(row)
        .on_conflict(
            OnConflict::columns([
                idempotency_keys::Column::AccountId,
                idempotency_keys::Column::Operation,
                idempotency_keys::Column::Key,
            ])
            .do_nothing()
            .to_owned(),
        )
        .try_insert()
        .exec_without_returning(tx)
        .await?;
    if matches!(inserted, TryInsertResult::Inserted(1)) {
        return Ok(Claim::Claimed);
    }
    // The generated entity orders its primary key by column position: (key, account_id,
    // operation).
    let stored = idempotency_keys::Entity::find_by_id((
        key.as_uuid(),
        account.as_uuid(),
        operation.to_owned(),
    ))
    .one(tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("idempotency key vanished after conflict"))?;
    Ok(Claim::Existing {
        fingerprint: stored.fingerprint,
        response: stored.response,
    })
}
