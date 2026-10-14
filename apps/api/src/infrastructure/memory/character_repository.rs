use crate::domain::class::ClassRegistry;
use crate::domain::zone::{Phase3Checkpoint, Phase3Rules};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::application::ports::{transfer_fingerprint, RepositoryError};
use crate::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, CreateOutcome,
    IdempotencyKey, ProgressionState,
};
use crate::domain::character_progression::base_class_profile;
use crate::domain::{AccountId, Character, CharacterId, DomainEvent};

#[derive(Default, Clone)]
struct State {
    lane_quarantines: BTreeMap<CharacterId, crate::application::ports::LaneQuarantine>,
    characters: HashMap<CharacterId, Character>,
    item_receipts: BTreeMap<
        (AccountId, crate::application::ports::ItemMutationOperation, IdempotencyKey),
        crate::application::ports::ItemMutationReceipt,
    >,
    item_ledger: Vec<crate::application::ports::ItemLedgerEntry>,
    item_inventories: BTreeMap<CharacterId, crate::domain::zone::Phase4Checkpoint>,
    item_ranges:
        BTreeMap<(crate::domain::zone::ZoneId, u64, u64), crate::application::ports::ItemIdRange>,
    pending_item_requests:
        BTreeMap<CharacterId, Vec<crate::application::ports::PendingItemRequest>>,
    names: HashMap<String, CharacterId>,
    /// `(account, key)`: keys are scoped to the account, like the Postgres primary key.
    keys: HashMap<(AccountId, IdempotencyKey), (String, CharacterId)>,
    /// Events staged with each create, standing in for the `outbox` table.
    outbox: Vec<DomainEvent>,
    /// Progression columns that live beside the aggregate, keyed by character.
    progression: HashMap<CharacterId, ProgressionState>,
    /// `(account, operation, key)` -> `(fingerprint, revision)` for checkpoints.
    checkpoint_keys: HashMap<(AccountId, String, IdempotencyKey), (String, u64)>,
    learn_receipts: BTreeMap<
        (AccountId, IdempotencyKey),
        (String, crate::application::ports::FrozenLearnResult),
    >,
}

impl State {
    fn character_view(&self, character: &Character) -> Character {
        let mut view = character.clone();
        view.item_metadata = self
            .item_inventories
            .get(&character.id)
            .map(crate::domain::CharacterItemMetadata::from);
        view
    }

    /// D77: like the Postgres adapter, every retained item needs its append-only birth row;
    /// provenance corruption is a permanent named constraint that fences the lane.
    fn item_checkpoint(
        &self,
        character: CharacterId,
    ) -> Result<Option<crate::domain::zone::Phase4Checkpoint>, CheckpointError> {
        let Some(mut checkpoint) = self.item_inventories.get(&character).cloned() else {
            return Ok(None);
        };
        let named = |name: &str| CheckpointError::Constraint(name.into());
        for item in &mut checkpoint.instances {
            let birth = self
                .item_ledger
                .iter()
                .find(|row| {
                    row.character_id == character
                        && row.instance_id == item.id
                        && row.delta.before.is_none()
                })
                .ok_or_else(|| named("item_creation_ledger_missing"))?;
            if birth
                .delta
                .after
                .as_ref()
                .is_none_or(|born| born.created_tick != item.created_tick)
            {
                return Err(named("item_creation_ledger_tick"));
            }
            let recorded = crate::domain::zone::ItemCreationOrigin {
                zone: birth.zone_id,
                epoch: birth.epoch,
            };
            if item
                .creation_origin
                .is_some_and(|origin| origin != recorded)
            {
                return Err(named("item_creation_ledger_origin"));
            }
            item.creation_origin = Some(recorded);
            item.validate().map_err(anyhow::Error::from)?;
        }
        Ok(Some(checkpoint))
    }

    fn put(&mut self, c: &Character) {
        self.progression.insert(
            c.id,
            ProgressionState {
                phase4: None,
                phase3: None,
                position: c.position,
                level: c.level,
                xp: c.xp,
                hp: None,
                mp: None,
                alive: true,
                class_profile: base_class_profile(c.class_state.base_class_id)
                    .unwrap_or("invalid")
                    .to_owned(),
                identity: c.identity(),
                name: c.name.clone(),
                class_state: c.class_state.clone(),
                revision: 0,
            },
        );
    }
}

/// Map-backed repository guarded by one mutex, so each method is atomic like a transaction.
#[derive(Default)]
pub struct InMemoryCharacterRepository {
    state: Mutex<State>,
    classes: Option<Arc<ClassRegistry>>,
    phase3_rules: Option<Arc<Phase3Rules>>,
    phase4_rules: Option<Arc<crate::domain::zone::Phase4Rules>>,
}

impl InMemoryCharacterRepository {
    /// Uses the same validated startup catalogue as creation and the zone.
    #[must_use]
    pub fn with_classes(mut self, classes: Arc<ClassRegistry>) -> Self {
        self.classes = Some(classes);
        self
    }
    /// Injects the full immutable V5 bundle used by the actor; no timer-only policy.
    #[must_use]
    pub fn with_phase3_rules(mut self, rules: Arc<Phase3Rules>) -> Self {
        self.phase3_rules = Some(rules);
        self
    }

    /// Install the exact item/skill pair selected for this V6 epoch, never a mixed catalogue.
    pub fn with_phase4_rules(
        mut self,
        items: Arc<crate::domain::zone::Phase4Rules>,
        skills: Arc<Phase3Rules>,
    ) -> anyhow::Result<Self> {
        items.validate_phase3(&skills).map_err(anyhow::Error::msg)?;
        self.phase3_rules = Some(skills);
        self.phase4_rules = Some(items);
        Ok(self)
    }

    fn phase3_rules(&self) -> anyhow::Result<Arc<Phase3Rules>> {
        static RULES: std::sync::OnceLock<Result<Arc<Phase3Rules>, String>> =
            std::sync::OnceLock::new();
        if let Some(rules) = &self.phase3_rules {
            return Ok(rules.clone());
        }
        RULES
            .get_or_init(|| {
                crate::infrastructure::skill_data::load_phase3_rules(
                    &crate::infrastructure::skill_data::Phase3DataSource::embedded(),
                    &[],
                )
                .map(Arc::new)
                .map_err(|e| e.to_string())
            })
            .clone()
            .map_err(anyhow::Error::msg)
    }

    /// D116 read-side upgrade of a retained V5 skill checkpoint under V6 rules.
    fn read_phase3(
        &self,
        phase3: Option<Phase3Checkpoint>,
        c: &Character,
    ) -> anyhow::Result<Option<Phase3Checkpoint>> {
        if phase3.is_none() {
            return Ok(None);
        }
        Ok(crate::infrastructure::character_validation::upgrade_v6_skill_checkpoint(
            phase3,
            &*self.phase3_rules()?,
            &c.class_state.learned_skills,
        ))
    }

    fn validate_versioned(
        &self,
        c: &Character,
        ledger: &crate::domain::character_progression::ClassState,
        phase3: Option<&Phase3Checkpoint>,
        alive: bool,
    ) -> anyhow::Result<()> {
        let registry =
            crate::infrastructure::character_validation::registry(self.classes.as_ref())?;
        if let Some(phase3) = phase3 {
            let rules = self.phase3_rules()?;
            rules
                .validate_catalogue(&registry)
                .map_err(anyhow::Error::msg)?;
            phase3.validate(&rules, &ledger.learned_skills, alive)?;
            ledger.validate_for_skill_rules(&registry, &c.identity(), c.id, &rules)?;
        } else {
            ledger.validate_for(&registry, &c.identity(), c.id)?;
        }
        Ok(())
    }

    fn validate(&self, c: &Character) -> anyhow::Result<()> {
        let registry =
            crate::infrastructure::character_validation::registry(self.classes.as_ref())?;
        c.class_state.validate_for(&registry, &c.identity(), c.id)?;
        Ok(())
    }

    /// Seeds a character directly, bypassing idempotency. Test helper.
    pub fn insert_for_test(&self, c: Character) {
        let mut s = self.state.lock();
        s.names.insert(c.name.normalized(), c.id);
        s.put(&c);
        s.characters.insert(c.id, c);
    }

    /// Reads a character directly. Test helper.
    #[must_use]
    pub fn get_for_test(&self, id: CharacterId) -> Option<Character> {
        self.state.lock().characters.get(&id).cloned()
    }

    /// Events staged for publication, in order (what the Postgres adapter writes to `outbox`).
    #[must_use]
    pub fn staged_events(&self) -> Vec<DomainEvent> {
        self.state.lock().outbox.clone()
    }

    /// Ordered append-only item rows for dependency-free adapter assertions.
    #[must_use]
    pub fn item_ledger_for_test(&self) -> Vec<crate::application::ports::ItemLedgerEntry> {
        self.state.lock().item_ledger.clone()
    }

    /// Number of stored characters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.lock().characters.len()
    }

    /// True when nothing is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl InMemoryCharacterRepository {
    #[allow(clippy::too_many_lines)] // Keep key/fence/validation/atomic mutation ordering visible.
    fn checkpoint_locked(
        &self,
        s: &mut State,
        checkpoint: &CharacterCheckpoint,
        events: &[DomainEvent],
        fingerprint: String,
        items: Option<&crate::application::ports::ItemCheckpointWork>,
    ) -> Result<CheckpointOutcome, CheckpointError> {
        // Item audit facts require the atomic inventory/ledger/receipt projection lane.
        if items.is_none() && events.iter().any(DomainEvent::is_item) {
            return Err(CheckpointError::Constraint("item_projection_required".into()));
        }
        let account = s
            .characters
            .get(&checkpoint.character_id)
            .ok_or(CheckpointError::NotFound)?
            .account_id;
        let scoped = (account, checkpoint.idempotency.0.clone(), checkpoint.idempotency.1);
        if let Some((stored_fp, revision)) = s.checkpoint_keys.get(&scoped) {
            return if *stored_fp == fingerprint {
                Ok(CheckpointOutcome::Replayed(*revision))
            } else {
                Err(CheckpointError::KeyReused)
            };
        }
        let current = s
            .progression
            .get(&checkpoint.character_id)
            .map(|p| p.revision)
            .ok_or(CheckpointError::NotFound)?;
        if current != checkpoint.revision_seen {
            return Ok(CheckpointOutcome::Stale);
        }
        if let Some(before) = s.item_checkpoint(checkpoint.character_id)? {
            s.item_inventories.insert(checkpoint.character_id, before);
        }
        if let Some(name) = checkpoint.violated_constraint() {
            return Err(CheckpointError::Constraint(name.to_owned()));
        }
        if checkpoint.phase3.is_none()
            && s.progression
                .get(&checkpoint.character_id)
                .is_some_and(|p| p.phase3.is_some())
        {
            return Err(CheckpointError::Constraint("characters_phase3_downgrade".into()));
        }
        if let Some(work) = items {
            work.validate_before(s.item_inventories.get(&checkpoint.character_id))?;
        } else {
            crate::application::ports::validate_item_retention(
                s.item_inventories.get(&checkpoint.character_id),
                checkpoint.phase4.as_ref(),
            )?;
        }
        if let Some(ledger) = &checkpoint.class_state {
            let character = s
                .characters
                .get(&checkpoint.character_id)
                .ok_or(CheckpointError::NotFound)?;
            self.validate_versioned(
                character,
                ledger,
                checkpoint.phase3.as_ref(),
                checkpoint.alive,
            )
            .map_err(|_| CheckpointError::Constraint("characters_class_state_valid".into()))?;
            if ledger.base_class_id != character.class_state.base_class_id {
                return Err(CheckpointError::Constraint("characters_base_class_immutable".into()));
            }
            for known in &character.class_state.learned_skills {
                if !ledger
                    .learned_skills
                    .iter()
                    .any(|skill| skill.key == known.key && skill.level >= known.level)
                {
                    return Err(CheckpointError::Constraint(
                        "character_learned_skills_monotone".into(),
                    ));
                }
            }
            for known in &character.class_state.successful_transfer_receipts {
                if ledger.receipt(known.key) != Some(known) {
                    return Err(CheckpointError::Constraint(
                        "character_transfer_receipts_immutable".into(),
                    ));
                }
            }
            for receipt in &ledger.successful_transfer_receipts {
                if receipt.result.character_id != checkpoint.character_id
                    || receipt.result.identity != character.identity()
                {
                    return Err(CheckpointError::Constraint(
                        "character_transfer_receipts_identity".into(),
                    ));
                }
                let fp = transfer_fingerprint(checkpoint.character_id, receipt.target_class_id);
                for other in s.characters.values().filter(|c| c.account_id == account) {
                    if let Some(known) = other.class_state.receipt(receipt.key) {
                        if fp
                            != transfer_fingerprint(
                                known.result.character_id,
                                known.target_class_id,
                            )
                            || known != receipt
                        {
                            return Err(CheckpointError::KeyReused);
                        }
                    }
                }
            }
        }
        let revision = current
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("revision overflow"))?;
        if let Some(p) = s.progression.get_mut(&checkpoint.character_id) {
            p.position = checkpoint.position;
            p.level = checkpoint.level;
            p.xp = checkpoint.xp;
            p.hp = Some(checkpoint.hp);
            p.mp = Some(checkpoint.mp);
            p.alive = checkpoint.alive;
            p.phase3.clone_from(&checkpoint.phase3);
            p.revision = revision;
            if let Some(ledger) = &checkpoint.class_state {
                p.class_state = ledger.clone();
            }
        }
        if let Some(c) = s.characters.get_mut(&checkpoint.character_id) {
            c.level = checkpoint.level;
            c.position = checkpoint.position;
            c.xp = checkpoint.xp;
            if let Some(ledger) = &checkpoint.class_state {
                c.class_state = ledger.clone();
            }
        }
        if let Some(phase4) = &checkpoint.phase4 {
            s.item_inventories
                .insert(checkpoint.character_id, phase4.clone());
        }
        s.outbox.extend_from_slice(events);
        s.checkpoint_keys.insert(scoped, (fingerprint, revision));
        Ok(CheckpointOutcome::Applied(revision))
    }
}

#[async_trait]
impl crate::application::ports::LearnReceiptRepository for InMemoryCharacterRepository {
    async fn learn_receipt_lookup(
        &self,
        caller: AccountId,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<crate::application::ports::LearnReceiptLookup> {
        use crate::application::ports::LearnReceiptLookup;
        Ok(match self.state.lock().learn_receipts.get(&(caller, *key)) {
            None => LearnReceiptLookup::Unknown,
            Some((stored, result)) if stored == fingerprint => {
                LearnReceiptLookup::Known(result.clone())
            },
            Some(_) => LearnReceiptLookup::Conflict,
        })
    }
}

#[async_trait]
impl crate::application::ports::LearningCheckpointRepository for InMemoryCharacterRepository {
    async fn checkpoint_learning(
        &self,
        checkpoint: &CharacterCheckpoint,
        events: &[DomainEvent],
        receipt: &crate::application::ports::SuccessfulLearnReceipt,
    ) -> Result<crate::application::ports::LearningCheckpointOutcome, CheckpointError> {
        use crate::application::ports::LearningCheckpointOutcome;
        let mut state = self.state.lock();
        let owner = state
            .characters
            .get(&checkpoint.character_id)
            .ok_or(CheckpointError::NotFound)?
            .account_id;
        if receipt.account_id != owner {
            return Err(CheckpointError::Constraint("character_learn_receipt_valid".into()));
        }
        // D47: a rank that requires items settles only with its item payments.
        crate::application::ports::validate_learn_payments(
            &[],
            checkpoint,
            Some(receipt),
            &*crate::infrastructure::character_validation::registry(self.classes.as_ref())?,
        )?;
        let fingerprint = receipt.checkpoint_fingerprint(checkpoint, events)?;
        let scoped = (owner, checkpoint.idempotency.0.clone(), checkpoint.idempotency.1);
        if let Some((stored, revision)) = state.checkpoint_keys.get(&scoped) {
            return if *stored == fingerprint {
                Ok(LearningCheckpointOutcome::Checkpoint(CheckpointOutcome::Replayed(*revision)))
            } else {
                Err(CheckpointError::KeyReused)
            };
        }
        if let Some((stored, result)) = state.learn_receipts.get(&(receipt.account_id, receipt.key))
        {
            return if *stored == receipt.request_fingerprint {
                Ok(LearningCheckpointOutcome::ReceiptReplayed(result.clone()))
            } else {
                Err(CheckpointError::KeyReused)
            };
        }
        receipt.validate(checkpoint, owner)?;
        let outcome = self.checkpoint_locked(&mut state, checkpoint, events, fingerprint, None)?;
        if matches!(outcome, CheckpointOutcome::Applied(_) | CheckpointOutcome::Replayed(_)) {
            state.learn_receipts.insert(
                (receipt.account_id, receipt.key),
                (receipt.request_fingerprint.clone(), receipt.result.clone()),
            );
        }
        Ok(LearningCheckpointOutcome::Checkpoint(outcome))
    }
}

#[async_trait]
impl crate::application::ports::ItemReceiptRepository for InMemoryCharacterRepository {
    async fn item_receipt_lookup(
        &self,
        account: AccountId,
        operation: crate::application::ports::ItemMutationOperation,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<crate::application::ports::ItemReceiptLookup> {
        use crate::application::ports::ItemReceiptLookup;
        let state = self.state.lock();
        Ok(match state.item_receipts.get(&(account, operation, *key)) {
            None => ItemReceiptLookup::Unknown,
            Some(receipt) if receipt.fingerprint != fingerprint => ItemReceiptLookup::Conflict {
                stored_fingerprint: receipt.fingerprint.clone(),
            },
            Some(receipt) => {
                receipt.validate()?;
                ItemReceiptLookup::Known(Box::new(receipt.clone()))
            },
        })
    }
}

#[async_trait]
impl crate::application::ports::ItemCheckpointRepository for InMemoryCharacterRepository {
    #[allow(clippy::too_many_lines)] // Same key/fence/child/outbox transaction ordering as Pg.
    async fn checkpoint_items(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
        work: &crate::application::ports::ItemCheckpointWork,
        learning: Option<&crate::application::ports::SuccessfulLearnReceipt>,
    ) -> Result<CheckpointOutcome, CheckpointError> {
        use crate::application::ports::PendingItemStatus;
        let mut current = self.state.lock();
        let owner = current
            .characters
            .get(&cp.character_id)
            .ok_or(CheckpointError::NotFound)?
            .account_id;
        let fingerprint = work.fingerprint(cp, events, learning)?;
        let checkpoint_key = (owner, cp.idempotency.0.clone(), cp.idempotency.1);
        if let Some((stored, revision)) = current.checkpoint_keys.get(&checkpoint_key) {
            return if *stored == fingerprint {
                Ok(CheckpointOutcome::Replayed(*revision))
            } else {
                Err(CheckpointError::KeyReused)
            };
        }
        let rules = self
            .phase4_rules
            .as_ref()
            .ok_or_else(|| CheckpointError::Constraint("item_checkpoint_rules".into()))?;
        if rules.hash() != work.projection.checkpoint.rules_hash {
            return Err(CheckpointError::Constraint("item_checkpoint_rules".into()));
        }
        work.validate(cp, owner)?;
        work.validate_catalogue(rules)?;
        work.validate_events(cp, events)?;
        crate::application::ports::validate_learn_payments(
            &work.projection.ledger,
            cp,
            learning,
            &*crate::infrastructure::character_validation::registry(self.classes.as_ref())?,
        )?;
        let receipts = work.receipts(cp)?;
        for receipt in &receipts {
            if let Some(known) = current.item_receipts.get(&(
                owner,
                receipt.operation,
                IdempotencyKey::from_uuid(receipt.key),
            )) {
                return if known.fingerprint == receipt.fingerprint {
                    Ok(CheckpointOutcome::Stale)
                } else {
                    Err(CheckpointError::KeyReused)
                };
            }
        }
        if let Some(receipt) = learning {
            if let Some((stored, _)) = current.learn_receipts.get(&(owner, receipt.key)) {
                return if *stored == receipt.request_fingerprint {
                    Ok(CheckpointOutcome::Stale)
                } else {
                    Err(CheckpointError::KeyReused)
                };
            }
            receipt.validate(cp, owner)?;
        }
        let mut staged = current.clone();
        let before = staged.item_checkpoint(cp.character_id)?;
        if let Some(before) = &before {
            staged
                .item_inventories
                .insert(cp.character_id, before.clone());
        }
        let outcome = self.checkpoint_locked(&mut staged, cp, events, fingerprint, Some(work))?;
        if !matches!(outcome, CheckpointOutcome::Applied(_)) {
            return Ok(outcome);
        }
        work.validate_pending(
            staged
                .pending_item_requests
                .get(&cp.character_id)
                .map_or(&[], Vec::as_slice),
        )?;
        let (start, next, end) = work.allocation.bounds();
        if end > 1_u64 << 32 {
            let range = staged
                .item_ranges
                .get_mut(&(work.zone, work.epoch, start))
                .ok_or_else(|| CheckpointError::Constraint("item_checkpoint_reservation".into()))?;
            if range.range_start != start || range.range_end_exclusive != end {
                return Err(CheckpointError::Constraint("item_checkpoint_reservation".into()));
            }
            range.next_reserved = range.next_reserved.max(next);
        }
        for row in work.ledger_entries()? {
            if row.delta.before.is_none()
                && !before
                    .as_ref()
                    .is_some_and(|cp| cp.instances.iter().any(|item| item.id == row.instance_id))
                && staged
                    .item_ledger
                    .iter()
                    .any(|known| known.instance_id == row.instance_id)
            {
                return Err(CheckpointError::Constraint("item_instance_identity_reused".into()));
            }
            if staged.item_ledger.iter().any(|known| {
                (
                    known.zone_id,
                    known.epoch,
                    known.tick,
                    known.character_id,
                    known.instance_id,
                    known.operation,
                    known.operation_ordinal,
                ) == (
                    row.zone_id,
                    row.epoch,
                    row.tick,
                    row.character_id,
                    row.instance_id,
                    row.operation,
                    row.operation_ordinal,
                )
            }) {
                return Err(CheckpointError::Constraint("item_ledger_logical_key".into()));
            }
            staged.item_ledger.push(row);
        }
        // D76 row positions are enforced by `validate_before` before any row is staged; the
        // Postgres item_ledger_row_ordinal trigger is storage defence in depth with no memory copy.
        for item in &work.projection.checkpoint.instances {
            if staged.item_inventories.iter().any(|(id, inventory)| {
                *id != cp.character_id
                    && inventory.instances.iter().any(|known| known.id == item.id)
            }) {
                return Err(CheckpointError::Constraint("item_instance_owner_immutable".into()));
            }
        }
        for receipt in receipts {
            staged.item_receipts.insert(
                (owner, receipt.operation, IdempotencyKey::from_uuid(receipt.key)),
                receipt,
            );
        }
        for completion in &work.projection.materializations {
            let pending = staged
                .pending_item_requests
                .get_mut(&cp.character_id)
                .and_then(|requests| {
                    requests.iter_mut().find(|request| {
                        request.request_id == completion.request_id
                            && request.account_id == work.projection.account
                            && request.operation == request.kind.operation()
                    })
                })
                .ok_or_else(|| {
                    CheckpointError::Constraint("pending_item_completion_missing".into())
                })?;
            pending.status = PendingItemStatus::Materialized {
                revision: completion.inventory_revision,
                receipt: completion.receipt.key,
            };
        }
        if let Some(receipt) = learning {
            staged.learn_receipts.insert(
                (owner, receipt.key),
                (receipt.request_fingerprint.clone(), receipt.result.clone()),
            );
        }
        *current = staged;
        Ok(outcome)
    }
}

#[async_trait]
impl CharacterRepository for InMemoryCharacterRepository {
    async fn get(&self, id: CharacterId) -> anyhow::Result<Option<Character>> {
        let state = self.state.lock();
        if let Some(c) = state.characters.get(&id) {
            let progression = state
                .progression
                .get(&id)
                .ok_or_else(|| anyhow::anyhow!("progression missing"))?;
            self.validate_versioned(
                c,
                &c.class_state,
                self.read_phase3(progression.phase3.clone(), c)?.as_ref(),
                progression.alive,
            )?;
        }
        Ok(state
            .characters
            .get(&id)
            .map(|character| state.character_view(character)))
    }

    async fn list_by_account(&self, account: AccountId) -> anyhow::Result<Vec<Character>> {
        let state = self.state.lock();
        let mut out = Vec::new();
        for c in state
            .characters
            .values()
            .filter(|c| c.account_id == account)
        {
            let progression = state
                .progression
                .get(&c.id)
                .ok_or_else(|| anyhow::anyhow!("progression missing"))?;
            self.validate_versioned(
                c,
                &c.class_state,
                self.read_phase3(progression.phase3.clone(), c)?.as_ref(),
                progression.alive,
            )?;
            out.push(state.character_view(c));
        }
        out.sort_by_key(|c| c.id.as_uuid());
        Ok(out)
    }

    async fn create_idempotent(
        &self,
        key: &IdempotencyKey,
        fingerprint: &str,
        character: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        let mut s = self.state.lock();
        let scoped = (character.account_id, *key);
        if let Some((stored_fp, id)) = s.keys.get(&scoped) {
            if stored_fp != fingerprint {
                return Ok(CreateOutcome::KeyReused);
            }
            let existing = s
                .characters
                .get(id)
                .map(|character| s.character_view(character))
                .ok_or_else(|| {
                    RepositoryError::Other(anyhow::anyhow!(
                        "idempotency key points at missing character"
                    ))
                })?;
            return Ok(CreateOutcome::Replayed(existing));
        }
        self.validate(character)?;
        let norm = character.name.normalized();
        if s.names.contains_key(&norm) {
            return Err(RepositoryError::NameTaken);
        }
        if s.characters
            .values()
            .filter(|c| c.account_id == character.account_id)
            .count()
            >= 7
        {
            return Err(RepositoryError::SlotsFull);
        }
        let starter = crate::infrastructure::item_manifests::starter_request(character)?;
        s.pending_item_requests.insert(character.id, vec![starter]);
        s.names.insert(norm, character.id);
        s.keys
            .insert(scoped, (fingerprint.to_owned(), character.id));
        s.put(character);
        s.characters.insert(character.id, character.clone());
        s.outbox.push(DomainEvent::CharacterCreated {
            character_id: character.id,
            account_id: character.account_id,
            race: character.race,
        });
        Ok(CreateOutcome::Created(character.clone()))
    }

    async fn quarantine_lane(
        &self,
        quarantine: &crate::application::ports::LaneQuarantine,
    ) -> anyhow::Result<()> {
        let mut state = self.state.lock();
        anyhow::ensure!(
            state.characters.contains_key(&quarantine.character_id),
            "quarantined character missing"
        );
        state
            .lane_quarantines
            .entry(quarantine.character_id)
            .or_insert_with(|| quarantine.clone());
        Ok(())
    }

    async fn lane_quarantines(
        &self,
        zone: crate::domain::zone::ZoneId,
    ) -> anyhow::Result<Vec<crate::application::ports::LaneQuarantine>> {
        Ok(self
            .state
            .lock()
            .lane_quarantines
            .values()
            .filter(|quarantine| quarantine.zone == zone)
            .cloned()
            .collect())
    }

    async fn load_for_admission(
        &self,
        character_id: CharacterId,
    ) -> anyhow::Result<Option<ProgressionState>> {
        let state = self.state.lock();
        if let Some(quarantine) = state.lane_quarantines.get(&character_id) {
            return Err(crate::application::ports::LaneQuarantined {
                reason: quarantine.reason.clone(),
            }
            .into());
        }
        // Legacy readers must never silently drop retained V6 inventory.
        anyhow::ensure!(
            self.phase4_rules.is_some() || !state.item_inventories.contains_key(&character_id),
            "Phase4 admission requires its matching runtime bundle"
        );
        if let Some(c) = state.characters.get(&character_id) {
            let progression = state
                .progression
                .get(&character_id)
                .ok_or_else(|| anyhow::anyhow!("progression missing"))?;
            self.validate_versioned(
                c,
                &c.class_state,
                self.read_phase3(progression.phase3.clone(), c)?.as_ref(),
                progression.alive,
            )?;
        }
        let mut loaded = state.progression.get(&character_id).cloned();
        if let (Some(loaded), Some(c)) = (&mut loaded, state.characters.get(&character_id)) {
            loaded.phase3 = self.read_phase3(loaded.phase3.take(), c)?;
        }
        if let (Some(loaded), Some(rules)) = (&mut loaded, &self.phase4_rules) {
            let character = state
                .characters
                .get(&character_id)
                .ok_or_else(|| anyhow::anyhow!("progression character missing"))?;
            loaded.phase4 = Some(crate::infrastructure::item_manifests::admission_load(
                character,
                state.item_checkpoint(character_id)?,
                state
                    .pending_item_requests
                    .get(&character_id)
                    .map_or(&[], Vec::as_slice),
                loaded
                    .phase3
                    .as_ref()
                    .map_or(0, |p| p.modifier_install_order.len()),
                rules,
            )?);
        }
        Ok(loaded)
    }

    async fn checkpoint(
        &self,
        checkpoint: &CharacterCheckpoint,
        events: &[DomainEvent],
    ) -> Result<CheckpointOutcome, CheckpointError> {
        let mut s = self.state.lock();
        self.checkpoint_locked(&mut s, checkpoint, events, checkpoint.fingerprint(events), None)
    }
}

#[async_trait]
impl crate::application::ports::ItemAdmissionRepository for InMemoryCharacterRepository {
    async fn reserve_item_range(
        &self,
        zone: crate::domain::zone::ZoneId,
        epoch: u64,
    ) -> Result<crate::application::ports::ItemIdRange, CheckpointError> {
        use crate::application::ports::ItemIdRange;
        if i32::try_from(zone.0).is_err() {
            return Err(CheckpointError::Constraint("item_id_ranges_zone".into()));
        }
        if i64::try_from(epoch).is_err() {
            return Err(CheckpointError::Constraint("item_id_ranges_epoch".into()));
        }
        let mut state = self.state.lock();
        if let Some((_, range)) = state
            .item_ranges
            .range((zone, epoch, 0)..=(zone, epoch, u64::MAX))
            .next_back()
        {
            return Ok(*range);
        }
        let start = state
            .item_ranges
            .values()
            .map(|range| range.range_end_exclusive)
            .max()
            .unwrap_or(1_u64 << 32);
        let end = start
            .checked_add(1_u64 << 32)
            .filter(|end| i64::try_from(*end).is_ok())
            .ok_or_else(|| CheckpointError::Constraint("item_id_ranges_exhausted".into()))?;
        let range = ItemIdRange {
            zone_id: zone,
            epoch,
            range_start: start,
            range_end_exclusive: end,
            next_reserved: start,
        };
        range.validate()?;
        state.item_ranges.insert((zone, epoch, start), range);
        Ok(range)
    }
    async fn reserve_item_range_extension(
        &self,
        zone: crate::domain::zone::ZoneId,
        epoch: u64,
        exhausted_range_start: u64,
    ) -> Result<crate::application::ports::ItemIdRange, CheckpointError> {
        use crate::application::ports::ItemIdRange;
        if i32::try_from(zone.0).is_err() {
            return Err(CheckpointError::Constraint("item_id_ranges_zone".into()));
        }
        if i64::try_from(epoch).is_err() {
            return Err(CheckpointError::Constraint("item_id_ranges_epoch".into()));
        }
        if i64::try_from(exhausted_range_start).is_err() {
            return Err(CheckpointError::Constraint("item_id_ranges_bounds".into()));
        }
        let mut state = self.state.lock();
        let old = state
            .item_ranges
            .get(&(zone, epoch, exhausted_range_start))
            .ok_or_else(|| CheckpointError::Constraint("item_id_ranges_missing".into()))?;
        old.validate()?;
        if old.next_reserved != old.range_end_exclusive {
            return Err(CheckpointError::Constraint("item_id_ranges_not_exhausted".into()));
        }
        if let Some((_, successor)) = state
            .item_ranges
            .range((zone, epoch, old.range_end_exclusive)..=(zone, epoch, u64::MAX))
            .next()
        {
            return Ok(ItemIdRange {
                next_reserved: successor.range_start,
                ..*successor
            });
        }
        let start = state
            .item_ranges
            .values()
            .map(|range| range.range_end_exclusive)
            .max()
            .ok_or_else(|| CheckpointError::Constraint("item_id_ranges_missing".into()))?;
        let end = start
            .checked_add(1_u64 << 32)
            .filter(|end| i64::try_from(*end).is_ok())
            .ok_or_else(|| CheckpointError::Constraint("item_id_ranges_exhausted".into()))?;
        let range = ItemIdRange {
            zone_id: zone,
            epoch,
            range_start: start,
            range_end_exclusive: end,
            next_reserved: start,
        };
        range.validate()?;
        state.item_ranges.insert((zone, epoch, start), range);
        Ok(range)
    }
    async fn pending_item_requests(
        &self,
        character: CharacterId,
    ) -> anyhow::Result<Vec<crate::application::ports::PendingItemRequest>> {
        let state = self.state.lock();
        anyhow::ensure!(
            state.characters.contains_key(&character),
            "pending items character missing"
        );
        Ok(state
            .pending_item_requests
            .get(&character)
            .cloned()
            .unwrap_or_default())
    }
}

#[async_trait]
impl crate::application::ports::ItemInventoryRepository for InMemoryCharacterRepository {
    async fn retained_inventory(
        &self,
        character: CharacterId,
    ) -> anyhow::Result<Option<crate::application::ports::RetainedInventory>> {
        let state = self.state.lock();
        let Some(progression) = state.progression.get(&character) else {
            return Ok(None);
        };
        let checkpoint = state.item_checkpoint(character)?;
        if let Some(checkpoint) = &checkpoint {
            let phase3 = progression
                .phase3
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Phase4 checkpoint requires Phase3 state"))?;
            checkpoint.validate_structure(phase3.modifier_install_order.len())?;
        }
        Ok(Some(crate::application::ports::RetainedInventory {
            character_id: character,
            character_revision: progression.revision,
            checkpoint,
        }))
    }
}

impl InMemoryCharacterRepository {
    /// Explicit fixture seed only, before actor admission. Never called by a grant RPC.
    pub fn insert_item_checkpoint_for_test(
        &self,
        character: CharacterId,
        checkpoint: crate::domain::zone::Phase4Checkpoint,
    ) -> anyhow::Result<()> {
        let mut state = self.state.lock();
        let progression = state
            .progression
            .get(&character)
            .ok_or_else(|| anyhow::anyhow!("fixture character missing"))?;
        let phase3 = progression
            .phase3
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Phase4 fixture requires Phase3 state"))?;
        checkpoint.validate_structure(phase3.modifier_install_order.len())?;
        state.item_inventories.insert(character, checkpoint);
        Ok(())
    }

    /// Fixture seed only: append the append-only birth row (D65/D77) that a real settlement
    /// would have written for each retained item that lacks one, at the item's own provenance.
    pub fn seed_item_birth_ledger_for_test(&self, character: CharacterId) -> anyhow::Result<()> {
        use crate::application::ports::{ItemLedgerDelta, ItemLedgerEntry, ItemLedgerOperation};
        let mut state = self.state.lock();
        let checkpoint = state
            .item_inventories
            .get(&character)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("fixture item checkpoint missing"))?;
        for item in checkpoint.instances {
            if state.item_ledger.iter().any(|row| {
                row.character_id == character
                    && row.instance_id == item.id
                    && row.delta.before.is_none()
            }) {
                continue;
            }
            let origin = item
                .creation_origin
                .ok_or_else(|| anyhow::anyhow!("fixture birth row needs explicit provenance"))?;
            state.item_ledger.push(ItemLedgerEntry {
                character_id: character,
                zone_id: origin.zone,
                epoch: origin.epoch,
                tick: item.created_tick,
                revision: item.revision,
                operation: ItemLedgerOperation::Grant,
                instance_id: item.id,
                operation_ordinal: 0,
                delta: ItemLedgerDelta {
                    before: None,
                    count_delta: i64::try_from(item.count)?,
                    after: Some(item),
                    causes: Vec::new(),
                    receipt: None,
                    receipt_refs: Vec::new(),
                },
                receipt_ref: None,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::domain::{CharacterName, Position, Race};

    const KEY_A: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e";
    const KEY_B: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8f";

    fn setup() -> (InMemoryCharacterRepository, Character) {
        let repo = InMemoryCharacterRepository::default();
        let c = Character::create(
            AccountId::from_uuid(uuid::Uuid::nil()),
            CharacterName::new("Durin").unwrap(),
            Race::Dwarf,
        );
        repo.insert_for_test(c.clone());
        (repo, c)
    }

    fn cp(c: &Character, seen: u64, key: &str) -> CharacterCheckpoint {
        CharacterCheckpoint {
            phase4: None,
            phase3: None,
            character_id: c.id,
            revision_seen: seen,
            level: 2,
            xp: 100,
            hp: 50,
            mp: 20,
            alive: true,
            position: Position { x: 1.5, y: 2.5 },
            idempotency: ("save_checkpoint".to_owned(), IdempotencyKey::parse(key).unwrap()),
            class_state: None,
        }
    }

    #[tokio::test]
    async fn checkpoint_applies_replays_and_fences() {
        let (repo, c) = setup();
        let p = repo.load_for_admission(c.id).await.unwrap().unwrap();
        assert_eq!((p.xp, p.hp, p.revision), (0, None, 0));
        assert_eq!(p.class_profile, "dwarven_fighter");

        let ev = [DomainEvent::CharacterLeveled {
            metadata: crate::domain::EventMetadata::default(),
            character_id: c.id,
            level: 2,
        }];
        let a = cp(&c, 0, KEY_A);
        assert_eq!(repo.checkpoint(&a, &ev).await.unwrap(), CheckpointOutcome::Applied(1));
        assert_eq!(repo.checkpoint(&a, &ev).await.unwrap(), CheckpointOutcome::Replayed(1));
        assert_eq!(repo.staged_events(), ev);
        let p = repo.load_for_admission(c.id).await.unwrap().unwrap();
        assert_eq!((p.level, p.xp, p.hp, p.mp, p.revision), (2, 100, Some(50), Some(20), 1));

        let mut changed = a.clone();
        changed.xp = 101;
        assert!(matches!(repo.checkpoint(&changed, &ev).await, Err(CheckpointError::KeyReused)));
        assert_eq!(
            repo.checkpoint(&cp(&c, 0, KEY_B), &ev).await.unwrap(),
            CheckpointOutcome::Stale
        );
        assert_eq!(repo.staged_events().len(), 1, "stale wrote nothing");

        let mut bad = cp(&c, 1, KEY_B);
        bad.level = 86;
        assert!(matches!(
            repo.checkpoint(&bad, &[]).await,
            Err(CheckpointError::Constraint(n)) if n == "characters_level_range"
        ));
    }
}
