//! Phase 5 (V7) canonical zone state: `Phase5Snapshot`, its records and `Phase5Zone`
//! (design §1.2–§1.11). Everything here is zone-canonical and digested (§12.4); DB-only
//! rows (warehouse, freight, mail, auction, manor, deliveries) are never snapshot state.
//! Legacy `Entity`, `NpcAi` and `CombatState` gain no fields: every V7 field lives here.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::phase5_command::LootMode;
use super::phase5_event::RecipeBook;
use super::{
    Entity, EntityId, EntityKind, ItemError, ItemInstance, ItemInstanceId, ItemLocation,
    Phase4Rules, Phase4Snapshot, Phase5Rules, Tick, Vec2Fixed, ZoneId, ZoneSeed,
};

/// Zone-local party identity: a monotone allocator from 1, never a UUID (design §1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PartyId(pub u64);

/// Where a ground item came from (design §1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum GroundSource {
    NpcLoot { npc: EntityId },
    PkDrop { victim: EntityId },
}

/// The `ItemTable` loot-owner mechanism (design §3.7): `ownerId` + the `ResetOwner` deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct LootOwner {
    pub owner: EntityId,
    pub until: Tick,
}

/// The `DropProtection` mechanism (design §3.7): `_owner` + `PROTECTED_MILLIS_TIME`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct DropProtect {
    pub owner: EntityId,
    pub until: Tick,
}

/// A ground item: a zone entity that carries exactly one `ItemInstanceId` (B5, design §1.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct GroundItem {
    /// Derived by rule P1; incarnation always 1.
    pub entity: EntityId,
    /// Location is always `Ground` (design §11.2).
    pub item: ItemInstance,
    pub pos: Vec2Fixed,
    pub dropped_tick: Tick,
    pub source: GroundSource,
    pub loot_owner: Option<LootOwner>,
    pub drop_protection: Option<DropProtect>,
    /// `None` = never auto-destroyed (a player drop).
    pub decay_due: Option<Tick>,
}

impl GroundItem {
    /// Rule P1 (design §1.2): `uuid_v8(SHA-256("nightfall.ground.1\0" ‖ zone u32 LE ‖ epoch
    /// u64 LE ‖ item u64 LE ‖ dropped_tick u64 LE)[0..16])`. Draws no RNG.
    #[must_use]
    pub fn entity_id(
        zone: ZoneId,
        epoch: u64,
        item: ItemInstanceId,
        dropped_tick: Tick,
    ) -> EntityId {
        let mut hash = Sha256::new();
        hash.update(b"nightfall.ground.1\0");
        hash.update(zone.0.to_le_bytes());
        hash.update(epoch.to_le_bytes());
        hash.update(item.get().to_le_bytes());
        hash.update(dropped_tick.0.to_le_bytes());
        let digest = hash.finalize();
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(digest.get(..16).unwrap_or(&[0; 16]));
        // RFC 9562 version 8 (custom) and the RFC 4122 variant.
        if let Some(byte) = bytes.get_mut(6) {
            *byte = (*byte & 0x0f) | 0x80;
        }
        if let Some(byte) = bytes.get_mut(8) {
            *byte = (*byte & 0x3f) | 0x80;
        }
        EntityId::from_uuid(uuid::Uuid::from_bytes(bytes))
    }
}

/// The Seed handler binding on an NPC (design §1.3): seed template and the seeder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SeedBinding {
    pub seed: u32,
    pub seeder: EntityId,
}

/// The V7 NPC economy sidecar (design §1.3), keyed by the NPC's `EntityId`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct NpcEconomy {
    /// Set by Spoil on the living monster; evaluated through the sweeper's party at sweep.
    pub spoiler: Option<EntityId>,
    pub seed: Option<SeedBinding>,
    pub seeded: bool,
    /// Crop template and count; one-shot.
    pub harvest: Option<(u32, u64)>,
    /// Template/count holders rolled at death; `None` = nothing or taken. No instance ids.
    pub sweep: Option<Vec<(u32, u64)>>,
}

/// A pending loot-mode change vote (design §1.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct LootVote {
    pub mode: LootMode,
    pub answers: BTreeSet<EntityId>,
    pub expires: Tick,
}

/// A party (design §1.4): pinned member list order, index 0 is the leader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Party {
    pub id: PartyId,
    pub members: Vec<EntityId>,
    pub distribution: LootMode,
    /// `BY_TURN` cursor; never adjusted on membership change.
    pub item_last_loot: u32,
    pub loot_vote: Option<LootVote>,
    pub pending_invite: Option<Tick>,
}

/// What a transaction request asks (design §1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum TxKind {
    Trade,
    PartyInvite { mode: LootMode },
}

/// The single per-target request slot shared by trade and party invites (design §1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct TxRequest {
    pub requester: EntityId,
    pub target: EntityId,
    pub kind: TxKind,
    pub expires: Tick,
}

/// One side of a trade (design §1.5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct TradeList {
    pub items: Vec<(ItemInstanceId, u64)>,
    pub confirmed: bool,
    pub locked: bool,
}

/// An open trade; `a` is the requester, the map key is the canonical `(min, max)` pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct TradeSession {
    pub a: EntityId,
    pub b: EntityId,
    pub list_a: TradeList,
    pub list_b: TradeList,
}

impl TradeSession {
    /// The canonical pair key.
    #[must_use]
    pub fn key(a: EntityId, b: EntityId) -> (EntityId, EntityId) {
        if a <= b {
            (a, b)
        } else {
            (b, a)
        }
    }
}

/// `PrivateStoreType` `SELL` / `PACKAGE_SELL` / `BUY` / `MANUFACTURE` (design §1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum PrivateStoreKind {
    Sell,
    PackageSell,
    Buy,
    Manufacture,
}

/// A private store (design §1.6); zone-only, closed on departure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PrivateStore {
    pub kind: PrivateStoreKind,
    /// ≤ 29 characters.
    pub title: String,
    pub sell: Vec<super::phase5_command::StoreSellEntry>,
    pub buy: Vec<super::phase5_command::StoreBuyEntry>,
    pub manufacture: Vec<(u32, u64)>,
}

/// Limited vendor stock (design §1.7), keyed by `(buylist_id, template)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct VendorStock {
    /// May go negative as the pinned `AtomicLong` does; read clamped at 0.
    pub count: i64,
    pub max: u64,
    pub restock_ticks: u64,
    pub due: Option<Tick>,
}

/// The pinned flood protectors a Phase 5 intent gates on (design §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum FloodKind {
    Transaction,
    MultiSell,
    Manufacture,
    Manor,
    SendMail,
    ServerBypass,
}

impl FloodKind {
    /// Every kind in digest order.
    pub const ALL: [Self; 6] = [
        Self::Transaction,
        Self::MultiSell,
        Self::Manufacture,
        Self::Manor,
        Self::SendMail,
        Self::ServerBypass,
    ];

    /// Digest ordinal.
    #[must_use]
    pub fn ordinal(self) -> u8 {
        Self::ALL
            .iter()
            .position(|kind| *kind == self)
            .and_then(|position| u8::try_from(position).ok())
            .unwrap_or(u8::MAX)
    }
}

/// Per-player V7 state (design §1.11): recipes, refund tab, party, trade, flood gates.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase5Player {
    pub recipes: RecipeBook,
    /// Location `Refund`, oldest first; ≤ 12 entries.
    pub refund: Vec<ItemInstance>,
    pub party: Option<PartyId>,
    pub trade: Option<(EntityId, EntityId)>,
    /// Next-allowed tick per pinned protector.
    pub flood: BTreeMap<FloodKind, Tick>,
}

/// The maximum refund-tab length (`PcRefund`, design §1.9).
pub const REFUND_TAB_LIMIT: usize = 12;
/// The pinned party size cap (design §1.4).
pub const PARTY_SIZE_CAP: usize = 9;

/// The Snapshot11 Phase 5 block (design §1.11). The rules are hash-referenced (D27).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase5Snapshot {
    pub rules_hash: [u8; 32],
    pub ground: BTreeMap<EntityId, GroundItem>,
    pub npcs: BTreeMap<EntityId, NpcEconomy>,
    pub parties: BTreeMap<PartyId, Party>,
    pub next_party: u64,
    /// Keyed by the target.
    pub requests: BTreeMap<EntityId, TxRequest>,
    #[serde(with = "pairs")]
    pub trades: BTreeMap<(EntityId, EntityId), TradeSession>,
    pub stores: BTreeMap<EntityId, PrivateStore>,
    #[serde(with = "pairs")]
    pub stock: BTreeMap<(u64, u32), VendorStock>,
    pub players: BTreeMap<EntityId, Phase5Player>,
}

/// Tuple-keyed maps as sorted `[key, value]` pair lists (JSON keys must be strings); a
/// duplicate or unsorted key fails closed.
mod pairs {
    use std::collections::BTreeMap;

    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<K: Serialize + Ord, V: Serialize, S: Serializer>(
        map: &BTreeMap<K, V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(map.iter())
    }

    pub(super) fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord + Clone,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let entries: Vec<(K, V)> = Vec::deserialize(deserializer)?;
        let mut map = BTreeMap::new();
        for (key, value) in entries {
            if map.last_key_value().is_some_and(|(last, _)| *last >= key) {
                return Err(D::Error::custom("unsorted or duplicate pair key"));
            }
            map.insert(key, value);
        }
        Ok(map)
    }
}

/// Runtime pairing of the canonical block with its resolved rules (never serialized).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase5Zone {
    /// The digested, snapshotted state.
    pub canonical: Phase5Snapshot,
    /// The rules the epoch is bound to; the snapshot carries only their hash.
    pub rules: Arc<Phase5Rules>,
    /// A V7 invariant failure latched by an infallible hook; turned into a tick error before
    /// anything is flushed. Never snapshot state.
    pub fault: Option<String>,
}

/// Inputs the invariants read from the rest of the zone (design §1.11).
pub struct Phase5Context<'a> {
    /// The zone's items rules (stackability, catalogue flags).
    pub items: &'a Phase4Rules,
    /// The V6 block: admitted players and their inventories.
    pub phase4: &'a Phase4Snapshot,
    /// Every live entity (NPC existence, corpse timing).
    pub entities: &'a BTreeMap<EntityId, Entity>,
    /// Zone and epoch, for the P1 derivation.
    pub seed: ZoneSeed,
    /// The next tick the zone will run.
    pub tick: Tick,
}

impl Phase5Snapshot {
    /// A fresh V7 epoch: nothing but the rules binding and the party allocator at 1.
    #[must_use]
    pub fn new(rules: &Phase5Rules) -> Self {
        Self {
            rules_hash: rules.hash(),
            ground: BTreeMap::new(),
            npcs: BTreeMap::new(),
            parties: BTreeMap::new(),
            next_party: 1,
            requests: BTreeMap::new(),
            trades: BTreeMap::new(),
            stores: BTreeMap::new(),
            stock: BTreeMap::new(),
            players: BTreeMap::new(),
        }
    }

    /// The design §1.11 invariants, checked at admission, restore and every commit.
    #[allow(clippy::too_many_lines)] // One pass over every invariant, in design order.
    pub fn validate(
        &self,
        rules: &Phase5Rules,
        context: &Phase5Context<'_>,
    ) -> Result<(), ItemError> {
        if self.rules_hash != rules.hash() {
            return Err(ItemError("Phase5 rules hash mismatch"));
        }
        let fail = |message: &'static str| Err(ItemError(message));
        // Every instance id across all owners in the zone is unique.
        let mut instances: BTreeSet<ItemInstanceId> = BTreeSet::new();
        for player in context.phase4.players.values() {
            for id in player.inventory.keys() {
                if !instances.insert(*id) {
                    return fail("duplicate item instance across owners");
                }
            }
        }
        let catalogue: BTreeSet<u32> = rules
            .parts()
            .catalogue
            .templates
            .iter()
            .map(|template| template.id)
            .collect();
        for (entity, ground) in &self.ground {
            if *entity != ground.entity
                || *entity
                    != GroundItem::entity_id(
                        context.seed.zone,
                        context.seed.epoch,
                        ground.item.id,
                        ground.dropped_tick,
                    )
            {
                return fail("ground item without its P1 entity id");
            }
            ground.item.validate()?;
            if ground.item.location != ItemLocation::Ground || ground.item.slot.is_some() {
                return fail("ground item not located on the ground");
            }
            if !catalogue.contains(&ground.item.template_id) {
                return fail("ground item template outside the catalogue");
            }
            // D10: the closure's definitions join the V6 ones when items_closure.toml binds.
            let Some(definition) = context.items.parts().items.get(&ground.item.template_id) else {
                return fail("ground item template without an item definition");
            };
            if !definition.stackable && ground.item.count != 1 {
                return fail("non-stackable ground item with count != 1");
            }
            if !instances.insert(ground.item.id) {
                return fail("duplicate item instance across owners");
            }
            if ground.dropped_tick > context.tick
                || ground
                    .decay_due
                    .is_some_and(|due| due < ground.dropped_tick)
                || context
                    .entities
                    .get(entity)
                    .is_none_or(|e| e.kind != EntityKind::GroundItem || e.pos != ground.pos)
            {
                return fail("ground item timing or entity mismatch");
            }
            for owner in [
                ground.loot_owner.map(|p| p.owner),
                ground.drop_protection.map(|p| p.owner),
            ]
            .into_iter()
            .flatten()
            {
                if owner.as_uuid().is_nil() {
                    return fail("ground protection without an owner");
                }
            }
        }
        if context
            .entities
            .values()
            .any(|e| e.kind == EntityKind::GroundItem && !self.ground.contains_key(&e.id))
        {
            return fail("ground entity without a ground item record");
        }
        // Each sidecar NPC exists; a sweep holder only while dead with corpse_until > tick.
        for (npc, economy) in &self.npcs {
            let Some(entity) = context.entities.get(npc) else {
                return fail("economy sidecar for a missing npc");
            };
            if entity.kind != EntityKind::Npc {
                return fail("economy sidecar for a non-npc");
            }
            if economy.sweep.is_some() {
                let corpse = entity
                    .ai
                    .as_ref()
                    .and_then(|ai| ai.corpse_until)
                    .is_some_and(|until| until > context.tick);
                if !entity.targeting.dead || !corpse {
                    return fail("sweep holders on a live or decayed npc");
                }
            }
            if economy.harvest.is_some_and(|(_, count)| count == 0)
                || economy
                    .sweep
                    .as_ref()
                    .is_some_and(|holders| holders.iter().any(|(_, count)| *count == 0))
            {
                return fail("economy holder with a zero count");
            }
        }
        // Players: every admitted V6 player has exactly one V7 record and vice versa.
        if !self.players.keys().eq(context.phase4.players.keys()) {
            return fail("Phase5 player set differs from the admitted players");
        }
        let limits = &rules.parts().recipes.config;
        let recipe_ids: BTreeSet<u32> = rules
            .parts()
            .recipes
            .recipe
            .iter()
            .map(|recipe| recipe.id)
            .collect();
        for (id, player) in &self.players {
            if player.recipes.dwarven.len()
                > usize::try_from(limits.dwarf_recipe_limit).unwrap_or(usize::MAX)
                || player.recipes.common.len()
                    > usize::try_from(limits.common_recipe_limit).unwrap_or(usize::MAX)
                || !player.recipes.dwarven.is_disjoint(&player.recipes.common)
                || player
                    .recipes
                    .dwarven
                    .iter()
                    .chain(&player.recipes.common)
                    .any(|recipe| !recipe_ids.contains(recipe))
            {
                return fail("recipe book outside the rules");
            }
            if player.refund.len() > REFUND_TAB_LIMIT {
                return fail("refund tab longer than 12");
            }
            for item in &player.refund {
                item.validate()?;
                if item.location != ItemLocation::Refund || !instances.insert(item.id) {
                    return fail("refund entry misplaced or duplicated");
                }
            }
            if let Some(party) = player.party {
                if !self
                    .parties
                    .get(&party)
                    .is_some_and(|p| p.members.contains(id))
                {
                    return fail("player names a party it is not in");
                }
            }
            if let Some(pair) = player.trade {
                if pair != TradeSession::key(pair.0, pair.1)
                    || !self.trades.contains_key(&pair)
                    || (pair.0 != *id && pair.1 != *id)
                {
                    return fail("player names a trade it is not in");
                }
            }
            if player.flood.values().any(|next| next.0 > u64::MAX / 2) {
                return fail("flood deadline outside the tick range");
            }
        }
        // Parties: 2..=9 members, each an admitted player in exactly one party, leader first.
        let mut in_party: BTreeSet<EntityId> = BTreeSet::new();
        for (id, party) in &self.parties {
            if *id != party.id || id.0 == 0 || id.0 >= self.next_party {
                return fail("party id outside the allocator");
            }
            if party.members.len() < 2 || party.members.len() > PARTY_SIZE_CAP {
                return fail("party outside 2..=9 members");
            }
            for member in &party.members {
                if !in_party.insert(*member)
                    || self
                        .players
                        .get(member)
                        .is_none_or(|p| p.party != Some(*id))
                {
                    return fail("party member not admitted or in two parties");
                }
            }
            if let Some(vote) = &party.loot_vote {
                if !vote.answers.iter().all(|a| party.members.contains(a)) {
                    return fail("loot vote answer from a non-member");
                }
            }
        }
        // Requests: one per target (the map key), one outstanding per requester.
        let mut requesters: BTreeSet<EntityId> = BTreeSet::new();
        for (target, request) in &self.requests {
            if *target != request.target
                || request.requester == request.target
                || !self.players.contains_key(target)
                || !self.players.contains_key(&request.requester)
                || !requesters.insert(request.requester)
            {
                return fail("transaction request slot violation");
            }
        }
        // Trades: pair key consistent with both players; listed instances owned and available.
        for (key, trade) in &self.trades {
            if *key != TradeSession::key(trade.a, trade.b) || trade.a == trade.b {
                return fail("trade pair key mismatch");
            }
            for (who, list) in [(trade.a, &trade.list_a), (trade.b, &trade.list_b)] {
                if self.players.get(&who).is_none_or(|p| p.trade != Some(*key)) {
                    return fail("trade participant without the back-reference");
                }
                let Some(owner) = context.phase4.players.get(&who) else {
                    return fail("trade participant not admitted");
                };
                for (instance, count) in &list.items {
                    let Some(item) = owner.inventory.get(instance) else {
                        return fail("trade lists an item the participant does not own");
                    };
                    if item.location != ItemLocation::Inventory
                        || *count == 0
                        || *count > item.count
                    {
                        return fail("trade lists an unavailable item or count");
                    }
                }
            }
        }
        // Stores: at most one per player (the map key); sell entries owned and unequipped.
        for (owner, store) in &self.stores {
            if store.title.chars().count() > 29 {
                return fail("store title longer than 29 characters");
            }
            let Some(inventory) = context.phase4.players.get(owner) else {
                return fail("store owner not admitted");
            };
            if !self.players.contains_key(owner) {
                return fail("store owner without a Phase5 record");
            }
            for entry in &store.sell {
                let Some(item) = inventory.inventory.get(&entry.instance) else {
                    return fail("store sells an item the owner does not have");
                };
                let tradeable = context
                    .items
                    .parts()
                    .items
                    .get(&item.template_id)
                    .is_some_and(|definition| definition.flags.get("is_tradable") == Some(&true));
                if item.location != ItemLocation::Inventory
                    || !tradeable
                    || entry.count == 0
                    || entry.count > item.count
                {
                    return fail("store sells an unequipped-tradeable rule violation");
                }
            }
        }
        // Vendor stock: every key is a limited product of the rules.
        for ((buylist, template), stock) in &self.stock {
            let known = rules.parts().merchants.entries.iter().any(|entry| {
                entry.item == *template && entry.limited && entry.list_ids.contains(buylist)
            });
            if !known
                || stock.max == 0
                || stock.count > i64::try_from(stock.max).unwrap_or(i64::MAX)
            {
                return fail("vendor stock outside the rules");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn ground_entity_ids_are_v8_uuids_derived_without_rng() {
        let a = GroundItem::entity_id(ZoneId(1), 7, ItemInstanceId::new(9).unwrap(), Tick(40));
        let b = GroundItem::entity_id(ZoneId(1), 7, ItemInstanceId::new(9).unwrap(), Tick(40));
        assert_eq!(a, b);
        assert_eq!(a.as_uuid().get_version_num(), 8);
        assert_eq!(a.as_uuid().get_variant(), uuid::Variant::RFC4122);
        for other in [
            GroundItem::entity_id(ZoneId(2), 7, ItemInstanceId::new(9).unwrap(), Tick(40)),
            GroundItem::entity_id(ZoneId(1), 8, ItemInstanceId::new(9).unwrap(), Tick(40)),
            GroundItem::entity_id(ZoneId(1), 7, ItemInstanceId::new(10).unwrap(), Tick(40)),
            GroundItem::entity_id(ZoneId(1), 7, ItemInstanceId::new(9).unwrap(), Tick(41)),
        ] {
            assert_ne!(a, other);
        }
    }

    #[test]
    fn trade_keys_and_flood_ordinals_are_canonical() {
        let x = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        let y = EntityId::from_uuid(uuid::Uuid::from_u128(2));
        assert_eq!(TradeSession::key(y, x), (x, y));
        assert_eq!(TradeSession::key(x, y), (x, y));
        let ordinals: Vec<u8> = FloodKind::ALL.iter().map(|k| k.ordinal()).collect();
        assert_eq!(ordinals, [0, 1, 2, 3, 4, 5]);
        let json = serde_json::to_string(&Phase5Player {
            flood: [(FloodKind::ServerBypass, Tick(5))].into_iter().collect(),
            ..Phase5Player::default()
        })
        .unwrap();
        assert!(json.contains("\"server_bypass\":5"));
    }
}
