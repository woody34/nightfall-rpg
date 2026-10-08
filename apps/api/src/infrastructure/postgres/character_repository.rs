use std::future::Future;

use async_trait::async_trait;
use sea_orm::{
    ActiveValue, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    RuntimeErr, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::entities::{characters, outbox};
use super::idempotency::{self, operation, Claim};
use crate::application::ports::RepositoryError;
use crate::application::{CharacterRepository, CreateOutcome, IdempotencyKey};
use crate::domain::{
    AccountId, BaseStats, Character, CharacterId, CharacterName, DomainEvent, Position, Race,
};
use crate::infrastructure::telemetry::Metrics;

/// Character persistence in Postgres.
#[derive(Clone)]
pub struct PgCharacterRepository {
    db: DatabaseConnection,
    metrics: Option<Metrics>,
}

impl PgCharacterRepository {
    /// Wraps a connection (a `sqlx::PgPool` converts into one).
    #[must_use]
    pub fn new(db: impl Into<DatabaseConnection>) -> Self {
        Self {
            db: db.into(),
            metrics: None,
        }
    }

    /// Records every query in `db_query_seconds{repo="character",op}`.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Metrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Runs `fut`, timing it when metrics are attached.
    async fn timed<T>(&self, op: &'static str, fut: impl Future<Output = T>) -> T {
        match &self.metrics {
            Some(m) => m.time_db("character", op, fut).await,
            None => fut.await,
        }
    }
}

fn model_to_character(m: characters::Model) -> anyhow::Result<Character> {
    let race = Race::parse(&m.race)
        .ok_or_else(|| anyhow::anyhow!("unknown race in database: {}", m.race))?;
    let stat = |v: i16| -> anyhow::Result<u32> { Ok(u32::try_from(v)?) };
    Ok(Character {
        id: CharacterId::from_uuid(m.id),
        account_id: AccountId::from_uuid(m.account_id),
        name: CharacterName::new(m.name)?,
        race,
        level: u32::try_from(m.level)?,
        stats: BaseStats {
            str: stat(m.str)?,
            dex: stat(m.dex)?,
            con: stat(m.con)?,
            int: stat(m.int)?,
            wit: stat(m.wit)?,
            men: stat(m.men)?,
        },
        position: Position {
            x: m.pos_x,
            y: m.pos_y,
        },
    })
}

/// What a `create_character` idempotency record stores: the id of the created character.
#[derive(Serialize, Deserialize)]
struct StoredResponse {
    character_id: Uuid,
}

pub(super) fn is_unique_violation(e: &DbErr, constraint: &str) -> bool {
    let (DbErr::Exec(RuntimeErr::SqlxError(e)) | DbErr::Query(RuntimeErr::SqlxError(e))) = e else {
        return false;
    };
    e.as_database_error()
        .is_some_and(|d| d.code().as_deref() == Some("23505") && d.constraint() == Some(constraint))
}

fn character_active_model(c: &Character) -> anyhow::Result<characters::ActiveModel> {
    fn set<V: Into<sea_orm::Value>>(v: V) -> ActiveValue<V> {
        ActiveValue::Set(v)
    }
    Ok(characters::ActiveModel {
        id: set(c.id.as_uuid()),
        account_id: set(c.account_id.as_uuid()),
        name: set(c.name.as_str().to_owned()),
        name_normalized: set(c.name.normalized()),
        race: set(c.race.as_str().to_owned()),
        level: set(i32::try_from(c.level)?),
        str: set(i16::try_from(c.stats.str)?),
        dex: set(i16::try_from(c.stats.dex)?),
        con: set(i16::try_from(c.stats.con)?),
        int: set(i16::try_from(c.stats.int)?),
        wit: set(i16::try_from(c.stats.wit)?),
        men: set(i16::try_from(c.stats.men)?),
        pos_x: set(c.position.x),
        pos_y: set(c.position.y),
        created_at: ActiveValue::NotSet,
        updated_at: ActiveValue::NotSet,
    })
}

#[async_trait]
impl CharacterRepository for PgCharacterRepository {
    async fn get(&self, id: CharacterId) -> anyhow::Result<Option<Character>> {
        self.timed("get", characters::Entity::find_by_id(id.as_uuid()).one(&self.db))
            .await?
            .map(model_to_character)
            .transpose()
    }

    async fn list_by_account(&self, account: AccountId) -> anyhow::Result<Vec<Character>> {
        let rows = self
            .timed(
                "list_by_account",
                characters::Entity::find()
                    .filter(characters::Column::AccountId.eq(account.as_uuid()))
                    // uuid v7: id order is creation order.
                    .order_by_asc(characters::Column::Id)
                    .all(&self.db),
            )
            .await?;
        rows.into_iter().map(model_to_character).collect()
    }

    /// One transaction:
    /// 1. Claim `(account, create_character, key)` (`idempotency::claim`). The primary key
    ///    serializes concurrent retries; the loser sees no row and replays.
    /// 2. Insert the character. A unique violation on `characters_name_normalized_key` rolls
    ///    everything back (including the key) and surfaces as `NameTaken`.
    /// 3. Stage the `CharacterCreated` event in `outbox` so a relay can publish it even if the
    ///    process dies right after commit.
    async fn create_idempotent(
        &self,
        key: &IdempotencyKey,
        fingerprint: &str,
        character: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        self.timed("create_idempotent", self.create_idempotent_tx(key, fingerprint, character))
            .await
    }
}

impl PgCharacterRepository {
    async fn create_idempotent_tx(
        &self,
        key: &IdempotencyKey,
        fingerprint: &str,
        character: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        let tx = self.db.begin().await.map_err(anyhow::Error::from)?;

        let response = serde_json::to_value(StoredResponse {
            character_id: character.id.as_uuid(),
        })
        .map_err(anyhow::Error::from)?;
        let claim = idempotency::claim(
            &tx,
            character.account_id,
            operation::CREATE_CHARACTER,
            key,
            fingerprint,
            response,
        )
        .await?;

        if let Claim::Existing {
            fingerprint: stored_fp,
            response,
        } = claim
        {
            if stored_fp != fingerprint {
                tx.commit().await.map_err(anyhow::Error::from)?;
                return Ok(CreateOutcome::KeyReused);
            }
            let stored: StoredResponse =
                serde_json::from_value(response).map_err(anyhow::Error::from)?;
            let existing = characters::Entity::find_by_id(stored.character_id)
                .one(&tx)
                .await
                .map_err(anyhow::Error::from)?
                .ok_or_else(|| anyhow::anyhow!("idempotency key points at missing character"))?;
            tx.commit().await.map_err(anyhow::Error::from)?;
            return Ok(CreateOutcome::Replayed(model_to_character(existing)?));
        }

        let insert = characters::Entity::insert(character_active_model(character)?)
            .exec_without_returning(&tx)
            .await;
        if let Err(e) = insert {
            if is_unique_violation(&e, "characters_name_normalized_key") {
                tx.rollback().await.map_err(anyhow::Error::from)?;
                return Err(RepositoryError::NameTaken);
            }
            return Err(anyhow::Error::from(e).into());
        }

        let event = DomainEvent::CharacterCreated {
            character_id: character.id,
            account_id: character.account_id,
            race: character.race,
        };
        let staged = outbox::ActiveModel {
            subject: ActiveValue::Set(event.subject().to_owned()),
            payload: ActiveValue::Set(serde_json::to_value(&event).map_err(anyhow::Error::from)?),
            ..Default::default()
        };
        outbox::Entity::insert(staged)
            .exec_without_returning(&tx)
            .await
            .map_err(anyhow::Error::from)?;

        tx.commit().await.map_err(anyhow::Error::from)?;
        Ok(CreateOutcome::Created(character.clone()))
    }
}
