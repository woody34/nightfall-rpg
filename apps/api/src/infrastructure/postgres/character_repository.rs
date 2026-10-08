use std::future::Future;

use async_trait::async_trait;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::application::ports::RepositoryError;
use crate::application::{CharacterRepository, CreateOutcome, IdempotencyKey};
use crate::domain::{
    BaseStats, Character, CharacterId, CharacterName, DomainEvent, Position, Race,
};
use crate::infrastructure::telemetry::Metrics;

/// Character persistence in Postgres.
#[derive(Clone)]
pub struct PgCharacterRepository {
    pool: PgPool,
    metrics: Option<Metrics>,
}

impl PgCharacterRepository {
    /// Wraps a pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
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

const SELECT_CHARACTER_BY_ID: &str = "SELECT id, account_id, name, race, level, str, dex, con, \
                                      \"int\", wit, men, pos_x, pos_y FROM characters WHERE id = $1";

fn row_to_character(row: &sqlx::postgres::PgRow) -> anyhow::Result<Character> {
    let race_str: String = row.try_get("race")?;
    let race = Race::parse(&race_str)
        .ok_or_else(|| anyhow::anyhow!("unknown race in database: {race_str}"))?;
    let name: String = row.try_get("name")?;
    let level: i32 = row.try_get("level")?;
    let stat = |col: &str| -> anyhow::Result<u32> {
        let v: i16 = row.try_get(col)?;
        Ok(u32::try_from(v)?)
    };
    Ok(Character {
        id: CharacterId::from_uuid(row.try_get("id")?),
        account_id: row.try_get("account_id")?,
        name: CharacterName::new(name)?,
        race,
        level: u32::try_from(level)?,
        stats: BaseStats {
            str: stat("str")?,
            dex: stat("dex")?,
            con: stat("con")?,
            int: stat("int")?,
            wit: stat("wit")?,
            men: stat("men")?,
        },
        position: Position {
            x: row.try_get("pos_x")?,
            y: row.try_get("pos_y")?,
        },
    })
}

fn is_unique_violation(e: &sqlx::Error, constraint: &str) -> bool {
    e.as_database_error()
        .is_some_and(|d| d.code().as_deref() == Some("23505") && d.constraint() == Some(constraint))
}

async fn fetch_by_id(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> anyhow::Result<Option<Character>> {
    let row = sqlx::query(SELECT_CHARACTER_BY_ID)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
    row.as_ref().map(row_to_character).transpose()
}

#[async_trait]
impl CharacterRepository for PgCharacterRepository {
    async fn get(&self, id: CharacterId) -> anyhow::Result<Option<Character>> {
        let row = self
            .timed(
                "get",
                sqlx::query(SELECT_CHARACTER_BY_ID)
                    .bind(id.as_uuid())
                    .fetch_optional(&self.pool),
            )
            .await?;
        row.as_ref().map(row_to_character).transpose()
    }

    /// One transaction:
    /// 1. `INSERT ... ON CONFLICT DO NOTHING` on the idempotency key. The unique index
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
        let mut tx = self.pool.begin().await.map_err(anyhow::Error::from)?;

        let claimed = sqlx::query(
            "INSERT INTO idempotency_keys (key, fingerprint, character_id) VALUES ($1, $2, $3) \
             ON CONFLICT (key) DO NOTHING",
        )
        .bind(key.as_uuid())
        .bind(fingerprint)
        .bind(character.id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?
        .rows_affected()
            == 1;

        if !claimed {
            let row = sqlx::query(
                "SELECT fingerprint, character_id FROM idempotency_keys WHERE key = $1",
            )
            .bind(key.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(anyhow::Error::from)?;
            let stored_fp: String = row.try_get("fingerprint").map_err(anyhow::Error::from)?;
            let stored_id: Uuid = row.try_get("character_id").map_err(anyhow::Error::from)?;
            tx.commit().await.map_err(anyhow::Error::from)?;
            if stored_fp != fingerprint {
                return Ok(CreateOutcome::KeyReused);
            }
            let existing =
                fetch_by_id(&mut self.pool.begin().await.map_err(anyhow::Error::from)?, stored_id)
                    .await?
                    .ok_or_else(|| {
                        anyhow::anyhow!("idempotency key points at missing character")
                    })?;
            return Ok(CreateOutcome::Replayed(existing));
        }

        let insert = sqlx::query(
            "INSERT INTO characters \
             (id, account_id, name, name_normalized, race, level, str, dex, con, \"int\", wit, men, pos_x, pos_y) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(character.id.as_uuid())
        .bind(character.account_id)
        .bind(character.name.as_str())
        .bind(character.name.normalized())
        .bind(character.race.as_str())
        .bind(i32::try_from(character.level).map_err(anyhow::Error::from)?)
        .bind(i16::try_from(character.stats.str).map_err(anyhow::Error::from)?)
        .bind(i16::try_from(character.stats.dex).map_err(anyhow::Error::from)?)
        .bind(i16::try_from(character.stats.con).map_err(anyhow::Error::from)?)
        .bind(i16::try_from(character.stats.int).map_err(anyhow::Error::from)?)
        .bind(i16::try_from(character.stats.wit).map_err(anyhow::Error::from)?)
        .bind(i16::try_from(character.stats.men).map_err(anyhow::Error::from)?)
        .bind(character.position.x)
        .bind(character.position.y)
        .execute(&mut *tx)
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
        sqlx::query("INSERT INTO outbox (subject, payload) VALUES ($1, $2)")
            .bind(event.subject())
            .bind(serde_json::to_value(&event).map_err(anyhow::Error::from)?)
            .execute(&mut *tx)
            .await
            .map_err(anyhow::Error::from)?;

        tx.commit().await.map_err(anyhow::Error::from)?;
        Ok(CreateOutcome::Created(character.clone()))
    }
}
