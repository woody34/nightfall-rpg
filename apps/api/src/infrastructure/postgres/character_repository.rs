use crate::domain::class::ClassRegistry;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, DbErr, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, RuntimeErr, Statement, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::character_ledger;
use super::entities::{characters, outbox};
use super::idempotency::{self, operation, Claim};
use crate::application::ports::{MutationReceiptLookup, RepositoryError};
use crate::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, CreateOutcome,
    IdempotencyKey, ProgressionState,
};
use crate::domain::character_progression::base_class_profile;
use crate::domain::subclass::Sex;
use crate::domain::{
    AccountId, BaseStats, Character, CharacterId, CharacterName, DomainEvent, Position, Race,
};
use crate::infrastructure::telemetry::Metrics;

/// Character persistence in Postgres.
#[derive(Clone)]
pub struct PgCharacterRepository {
    db: DatabaseConnection,
    metrics: Option<Metrics>,
    classes: Option<Arc<ClassRegistry>>,
}

impl PgCharacterRepository {
    /// Wraps a connection (a `sqlx::PgPool` converts into one).
    #[must_use]
    pub fn new(db: impl Into<DatabaseConnection>) -> Self {
        Self {
            db: db.into(),
            metrics: None,
            classes: None,
        }
    }

    /// Shares the zone's validated startup registry for contextual admission validation.
    #[must_use]
    pub fn with_classes(mut self, classes: Arc<ClassRegistry>) -> Self {
        self.classes = Some(classes);
        self
    }

    fn validate_character(&self, c: &Character) -> anyhow::Result<()> {
        let registry =
            crate::infrastructure::character_validation::registry(self.classes.as_ref())?;
        c.class_state.validate_for(&registry, &c.identity(), c.id)?;
        Ok(())
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
        appearance: character_ledger::appearance(&m)?,
        class_state: character_ledger::state(&m)?,
        xp: u64::try_from(m.xp)?,
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
        xp: set(i64::try_from(c.xp)?),
        hp: ActiveValue::NotSet,
        mp: ActiveValue::NotSet,
        class_profile: set(base_class_profile(c.class_state.base_class_id)
            .ok_or_else(|| anyhow::anyhow!("unknown base class profile"))?
            .to_owned()),
        base_class_id: set(i32::try_from(c.class_state.base_class_id.0)?),
        current_class_id: set(i32::try_from(c.class_state.current_class_id.0)?),
        active_class_slot: set(0),
        sex: set(match c.appearance.sex {
            Sex::Male => "male",
            Sex::Female => "female",
        }
        .to_owned()),
        hair_style: set(i32::try_from(c.appearance.hair_style)?),
        hair_color: set(i32::try_from(c.appearance.hair_color)?),
        face: set(i32::try_from(c.appearance.face)?),
        sp: set(i64::try_from(c.class_state.sp)?),
        cp: set(i32::try_from(c.class_state.cp)?),
        token_tier_1_count: set(i32::try_from(c.class_state.token_tier_1_count)?),
        token_tier_2_count: set(i32::try_from(c.class_state.token_tier_2_count)?),
        milestone_claimed_mask: set(i16::from(c.class_state.milestone_claimed_mask)),
        alive: ActiveValue::NotSet,
        revision: ActiveValue::NotSet,
    })
}

#[async_trait]
impl CharacterRepository for PgCharacterRepository {
    async fn get(&self, id: CharacterId) -> anyhow::Result<Option<Character>> {
        self.timed("get", async {
            let tx = self.db.begin().await?;
            let model = characters::Entity::find_by_id(id.as_uuid())
                .lock_shared()
                .one(&tx)
                .await?;
            let mut character = model.map(model_to_character).transpose()?;
            if let Some(c) = &mut character {
                character_ledger::hydrate(&tx, c).await?;
                self.validate_character(c)?;
            }
            tx.commit().await?;
            Ok(character)
        })
        .await
    }

    async fn list_by_account(&self, account: AccountId) -> anyhow::Result<Vec<Character>> {
        self.timed("list_by_account", async {
            let tx = self.db.begin().await?;
            let rows = characters::Entity::find()
                .filter(characters::Column::AccountId.eq(account.as_uuid()))
                .order_by_asc(characters::Column::Id)
                .lock_shared()
                .all(&tx)
                .await?;
            let mut out = Vec::with_capacity(rows.len());
            for model in rows {
                let mut character = model_to_character(model)?;
                character_ledger::hydrate(&tx, &mut character).await?;
                self.validate_character(&character)?;
                out.push(character);
            }
            tx.commit().await?;
            Ok(out)
        })
        .await
    }

    async fn mutation_receipt_lookup(
        &self,
        caller: AccountId,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<MutationReceiptLookup> {
        self.timed(
            "mutation_receipt_lookup",
            character_ledger::receipt_lookup(&self.db, caller, key, fingerprint),
        )
        .await
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

    async fn load_for_admission(
        &self,
        character_id: CharacterId,
    ) -> anyhow::Result<Option<ProgressionState>> {
        self.timed("load_for_admission", async {
            let tx = self.db.begin().await?;
            let model = characters::Entity::find_by_id(character_id.as_uuid())
                .lock_shared()
                .one(&tx)
                .await?;
            let progression = if let Some(m) = model {
                let hp = m.hp.map(u32::try_from).transpose()?;
                let mp = m.mp.map(u32::try_from).transpose()?;
                let alive = m.alive;
                let class_profile = m.class_profile.clone();
                let revision = u64::try_from(m.revision)?;
                let mut c = model_to_character(m)?;
                character_ledger::hydrate(&tx, &mut c).await?;
                self.validate_character(&c)?;
                Some(ProgressionState {
                    position: c.position,
                    level: c.level,
                    xp: c.xp,
                    hp,
                    mp,
                    alive,
                    class_profile,
                    revision,
                    identity: c.identity(),
                    name: c.name,
                    class_state: c.class_state,
                })
            } else {
                None
            };
            tx.commit().await?;
            Ok(progression)
        })
        .await
    }

    /// One transaction:
    /// 1. Read the owner (immutable) to scope the idempotency key.
    /// 2. Claim `(account, operation, key)` with the body fingerprint and the revision this
    ///    checkpoint will produce. Existing key: replay or `KeyReused`; this precedes the
    ///    revision check because a retry arrives after the revision moved on.
    /// 3. `UPDATE ... WHERE id AND revision = revision_seen`; zero rows is `Stale` and the
    ///    transaction (claim included) rolls back. Check violations roll back as `Constraint`.
    /// 4. Stage the events in `outbox`; commit.
    async fn checkpoint(
        &self,
        checkpoint: &CharacterCheckpoint,
        events: &[DomainEvent],
    ) -> Result<CheckpointOutcome, CheckpointError> {
        self.timed("checkpoint", self.checkpoint_tx(checkpoint, events))
            .await
    }
}

/// What a checkpoint idempotency record stores: the revision the checkpoint produced.
#[derive(Serialize, Deserialize)]
struct StoredCheckpoint {
    revision: u64,
}

/// Maps a failed statement to `Constraint` when Postgres reports a check violation (23514).
pub(super) fn checkpoint_db_error(e: DbErr) -> CheckpointError {
    let (DbErr::Exec(RuntimeErr::SqlxError(s)) | DbErr::Query(RuntimeErr::SqlxError(s))) = &e
    else {
        return CheckpointError::Other(e.into());
    };
    let violated = s
        .as_database_error()
        .filter(|d| d.code().as_deref() == Some("23514"))
        .and_then(|d| d.constraint().map(str::to_owned));
    violated.map_or_else(|| CheckpointError::Other(e.into()), CheckpointError::Constraint)
}

impl PgCharacterRepository {
    async fn checkpoint_tx(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
    ) -> Result<CheckpointOutcome, CheckpointError> {
        let other = |e: DbErr| CheckpointError::Other(e.into());
        if let Some(name) = cp.violated_constraint() {
            // Values the column types cannot even hold; no statement needed to know.
            return Err(CheckpointError::Constraint(name.to_owned()));
        }
        let fingerprint = cp.fingerprint(events);
        let tx = self.db.begin().await.map_err(other)?;

        let original = characters::Entity::find_by_id(cp.character_id.as_uuid())
            .one(&tx)
            .await
            .map_err(other)?
            .ok_or(CheckpointError::NotFound)?;
        let original = model_to_character(original)?;
        let account = original.account_id.as_uuid();

        let produced = cp
            .revision_seen
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("revision overflow"))?;
        let claim = idempotency::claim(
            &tx,
            AccountId::from_uuid(account),
            &cp.idempotency.0,
            &cp.idempotency.1,
            &fingerprint,
            serde_json::to_value(StoredCheckpoint { revision: produced })
                .map_err(anyhow::Error::from)?,
        )
        .await?;
        if let Claim::Existing {
            fingerprint: stored_fp,
            response,
        } = claim
        {
            tx.commit().await.map_err(other)?;
            if stored_fp != fingerprint {
                return Err(CheckpointError::KeyReused);
            }
            let stored: StoredCheckpoint =
                serde_json::from_value(response).map_err(anyhow::Error::from)?;
            return Ok(CheckpointOutcome::Replayed(stored.revision));
        }

        let to_i32 = |v: u32| i32::try_from(v).map_err(anyhow::Error::from);
        let mut update = characters::Entity::update_many()
            .col_expr(characters::Column::Level, Expr::value(to_i32(cp.level)?))
            .col_expr(
                characters::Column::Xp,
                Expr::value(i64::try_from(cp.xp).map_err(anyhow::Error::from)?),
            )
            .col_expr(characters::Column::Hp, Expr::value(to_i32(cp.hp)?))
            .col_expr(characters::Column::Mp, Expr::value(to_i32(cp.mp)?))
            .col_expr(characters::Column::Alive, Expr::value(cp.alive))
            .col_expr(characters::Column::PosX, Expr::value(cp.position.x))
            .col_expr(characters::Column::PosY, Expr::value(cp.position.y))
            .col_expr(
                characters::Column::Revision,
                Expr::value(i64::try_from(produced).map_err(anyhow::Error::from)?),
            )
            .col_expr(characters::Column::UpdatedAt, Expr::current_timestamp())
            .filter(characters::Column::Id.eq(cp.character_id.as_uuid()))
            .filter(
                characters::Column::Revision
                    .eq(i64::try_from(cp.revision_seen).map_err(anyhow::Error::from)?),
            );
        if let Some(ledger) = &cp.class_state {
            update = update
                .col_expr(
                    characters::Column::CurrentClassId,
                    Expr::value(to_i32(ledger.current_class_id.0)?),
                )
                .col_expr(
                    characters::Column::Sp,
                    Expr::value(i64::try_from(ledger.sp).map_err(anyhow::Error::from)?),
                )
                .col_expr(characters::Column::Cp, Expr::value(to_i32(ledger.cp)?))
                .col_expr(
                    characters::Column::TokenTier1Count,
                    Expr::value(to_i32(ledger.token_tier_1_count)?),
                )
                .col_expr(
                    characters::Column::TokenTier2Count,
                    Expr::value(to_i32(ledger.token_tier_2_count)?),
                )
                .col_expr(
                    characters::Column::MilestoneClaimedMask,
                    Expr::value(i16::from(ledger.milestone_claimed_mask)),
                );
        }
        let updated = update.exec(&tx).await.map_err(checkpoint_db_error)?;
        if updated.rows_affected == 0 {
            tx.rollback().await.map_err(other)?;
            return Ok(CheckpointOutcome::Stale);
        }

        if let Some(ledger) = &cp.class_state {
            let registry =
                crate::infrastructure::character_validation::registry(self.classes.as_ref())?;
            ledger
                .validate_for(&registry, &original.identity(), cp.character_id)
                .map_err(|_| CheckpointError::Constraint("characters_class_state_valid".into()))?;
        }
        character_ledger::persist(&tx, cp, &original).await?;

        for event in events {
            let staged = outbox::ActiveModel {
                subject: ActiveValue::Set(event.subject().to_owned()),
                payload: ActiveValue::Set(
                    serde_json::to_value(event).map_err(anyhow::Error::from)?,
                ),
                ..Default::default()
            };
            outbox::Entity::insert(staged)
                .exec_without_returning(&tx)
                .await
                .map_err(other)?;
        }

        tx.commit().await.map_err(other)?;
        Ok(CheckpointOutcome::Applied(produced))
    }

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
                .lock_shared()
                .one(&tx)
                .await
                .map_err(anyhow::Error::from)?
                .ok_or_else(|| anyhow::anyhow!("idempotency key points at missing character"))?;
            let mut character = model_to_character(existing)?;
            character_ledger::hydrate(&tx, &mut character).await?;
            self.validate_character(&character)?;
            tx.commit().await.map_err(anyhow::Error::from)?;
            return Ok(CreateOutcome::Replayed(character));
        }

        // The idempotency claim precedes capacity checks: a retry replays even at seven slots.
        // A transaction advisory lock serializes distinct-key creates for the same account,
        // including older accounts which predate the accounts table's auth upsert.
        tx.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 723041))",
            [character.account_id.to_string().into()],
        ))
        .await
        .map_err(anyhow::Error::from)?;
        let count = characters::Entity::find()
            .filter(characters::Column::AccountId.eq(character.account_id.as_uuid()))
            .count(&tx)
            .await
            .map_err(anyhow::Error::from)?;
        if count >= 7 {
            return Err(RepositoryError::SlotsFull);
        }
        self.validate_character(character)?;
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

        character_ledger::create_main_slot(&tx, character).await?;
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
