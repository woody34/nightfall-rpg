//! Story 3.1: the Phase 5 block (design §1.11) round-trips through the Snapshot 11 codec,
//! every field is digested in the §12.4 order, the invariants fail closed, and V5/V6
//! snapshots and digests are untouched.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::semicolon_if_nothing_returned
)]
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nightfall_api::{
    application::replay_log::{decode_snapshot, encode_snapshot, restore_snapshot},
    domain::zone::{
        phase5::{
            DropProtect, LootOwner, LootVote, NpcEconomy, Party, PartyId, PrivateStore,
            PrivateStoreKind, SeedBinding, TradeList, TradeSession, TxKind, TxRequest, VendorStock,
        },
        phase5_command::{LootMode, StoreBuyEntry, StoreSellEntry},
        phase5_event::{Phase5Checkpoint, RecipeBook},
        *,
    },
    infrastructure::{class_data, economy_data, item_data, rules_data},
};

fn bundle() -> item_data::Phase4RuleBundle {
    item_data::embedded_phase4_bundle().unwrap()
}

fn v7_zone() -> ZoneState {
    let bundle = bundle();
    ZoneState::new_phase5(
        ZoneSeed {
            zone: ZoneId(75),
            epoch: 5,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        rules_data::load_rules(&rules_data::RulesSource::embedded())
            .unwrap()
            .rules,
        class_data::load_classes(&class_data::ClassSource::embedded())
            .unwrap()
            .registry,
        (*bundle.skills).clone(),
        bundle.items.clone(),
        // Ids below 50 count as allocated in this epoch, so the fixture's NPC loot (id 7)
        // satisfies the allocator rule (z-review-2 P3-7).
        ItemInstanceRange::new(1, 50, 100).unwrap(),
        economy_data::embedded_phase5_rules().unwrap(),
    )
    .unwrap()
}

fn v6_zone() -> ZoneState {
    let bundle = bundle();
    ZoneState::new_phase4(
        ZoneSeed {
            zone: ZoneId(75),
            epoch: 5,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        rules_data::load_rules(&rules_data::RulesSource::embedded())
            .unwrap()
            .rules,
        class_data::load_classes(&class_data::ClassSource::embedded())
            .unwrap()
            .registry,
        (*bundle.skills).clone(),
        bundle.items,
        ItemInstanceRange::new(1, 1, 100).unwrap(),
    )
    .unwrap()
}

fn id(n: u128) -> EntityId {
    EntityId::from_uuid(uuid::Uuid::from_u128(n))
}

fn instance(id: u64, template: u32, count: u64, location: ItemLocation) -> ItemInstance {
    ItemInstance {
        creation_origin: Some(ItemCreationOrigin {
            zone: ZoneId(75),
            epoch: 5,
        }),
        id: ItemInstanceId::new(id).unwrap(),
        template_id: template,
        count,
        location,
        slot: None,
        enchant: 0,
        augment_id: None,
        charges: 0,
        created_tick: Tick(3),
        revision: 1,
    }
}

fn ground(seed: ZoneSeed, item: u64, template: u32, count: u64, tick: Tick) -> GroundItem {
    let instance = instance(item, template, count, ItemLocation::Ground);
    GroundItem {
        entity: GroundItem::entity_id(seed.zone, seed.epoch, instance.id, tick),
        item: instance,
        pos: Vec2Fixed::from_tiles(10, 10),
        dropped_tick: tick,
        source: GroundSource::NpcLoot { npc: id(900) },
        loot_owner: Some(LootOwner {
            owner: id(1),
            until: Tick(tick.0 + 150),
        }),
        drop_protection: Some(DropProtect {
            owner: id(1),
            until: Tick(tick.0 + 150),
        }),
        decay_due: Some(Tick(tick.0 + 6000)),
    }
}

/// Give every ground record of the block its `GroundItem` entity and AOI cell (design §1.2:
/// a ground item is a zone entity), as the engine does when it drops one.
fn materialize_ground(snapshot: &mut ZoneSnapshot) {
    let records: Vec<GroundItem> = snapshot
        .phase5
        .as_ref()
        .map(|b| b.ground.values().cloned().collect())
        .unwrap_or_default();
    for g in records {
        if snapshot.entities.iter().any(|e| e.id == g.entity) {
            continue;
        }
        snapshot.entities.push(Entity {
            id: g.entity,
            kind: EntityKind::GroundItem,
            name: String::new(),
            pos: g.pos,
            dest: None,
            speed: Speed::from_milli_tiles_per_tick(0),
            generation: SessionGeneration::default(),
            targeting: TargetingState::default(),
            combat: None,
            ai: None,
        });
    }
    snapshot.entities.sort_by_key(|e| e.id);
    let mut aoi = AoiIndex::default();
    for e in &snapshot.entities {
        aoi.insert(e.id, e.pos);
    }
    snapshot.aoi = aoi.to_cells();
}

/// One of everything, for the digest and codec tests (not validated: no admitted players).
fn populated(block: &mut Phase5Snapshot) {
    let seed = ZoneSeed {
        zone: ZoneId(75),
        epoch: 5,
    };
    let g = ground(seed, 7, 57, 120, Tick(0));
    block.ground.insert(g.entity, g);
    block.npcs.insert(
        id(900),
        NpcEconomy {
            spoiler: Some(id(1)),
            seed: Some(SeedBinding {
                seed: 5016,
                seeder: id(2),
            }),
            seeded: true,
            harvest: Some((5073, 3)),
            sweep: Some(vec![(1864, 2), (57, 10)]),
        },
    );
    block.parties.insert(
        PartyId(1),
        Party {
            id: PartyId(1),
            members: vec![id(1), id(2)],
            distribution: LootMode::ByTurn,
            item_last_loot: 1,
            loot_vote: Some(LootVote {
                mode: LootMode::Random,
                answers: [id(1)].into_iter().collect(),
                expires: Tick(150),
            }),
            pending_invite: Some(Tick(120)),
        },
    );
    block.next_party = 2;
    block.requests.insert(
        id(3),
        TxRequest {
            requester: id(1),
            target: id(3),
            kind: TxKind::PartyInvite {
                mode: LootMode::FindersKeepers,
            },
            expires: Tick(150),
        },
    );
    block.trades.insert(
        (id(1), id(2)),
        TradeSession {
            a: id(2),
            b: id(1),
            list_a: TradeList {
                items: vec![(ItemInstanceId::new(8).unwrap(), 1)],
                confirmed: true,
                locked: false,
            },
            list_b: TradeList::default(),
        },
    );
    block.stores.insert(
        id(4),
        PrivateStore {
            kind: PrivateStoreKind::Buy,
            title: "wares".into(),
            sell: vec![StoreSellEntry {
                instance: ItemInstanceId::new(9).unwrap(),
                count: 1,
                price: 10,
            }],
            buy: vec![StoreBuyEntry {
                template: 1864,
                enchant: 0,
                count: 5,
                price: 7,
            }],
            manufacture: vec![(1666, 500)],
        },
    );
    block.stock.insert(
        (353_630, 8751),
        VendorStock {
            count: 0,
            max: 1,
            restock_ticks: 72_000,
            due: Some(Tick(72_000)),
        },
    );
    block.players.insert(
        id(1),
        Phase5Player {
            recipes: RecipeBook {
                dwarven: [1].into_iter().collect(),
                common: BTreeSet::new(),
            },
            refund: vec![instance(11, 1864, 2, ItemLocation::Refund)],
            party: Some(PartyId(1)),
            trade: Some((id(1), id(2))),
            flood: [
                (FloodKind::Transaction, Tick(40)),
                (FloodKind::ServerBypass, Tick(5)),
            ]
            .into_iter()
            .collect(),
        },
    );
}

#[test]
fn a_fresh_v7_epoch_is_snapshot_11_with_an_empty_bound_block() {
    let zone = v7_zone();
    let snapshot = zone.snapshot();
    assert_eq!(snapshot.meta.schema_version, 11);
    assert_eq!(snapshot.meta.digest_version, StateDigestVersion::BinaryV7);
    assert_eq!(
        snapshot.meta.phase3_admission_policy,
        Some(Phase3AdmissionPolicy::version_three())
    );
    assert!(zone.phase5_enabled() && zone.phase4_enabled() && zone.phase3_enabled());
    let block = snapshot.phase5.as_ref().unwrap();
    assert_eq!(block.rules_hash, economy_data::embedded_phase5_rules().unwrap().hash());
    assert_eq!(block.next_party, 1);
    assert!(block.ground.is_empty() && block.players.is_empty() && block.stock.is_empty());
    snapshot.validate_version().unwrap();
    // The V7 digest differs from the V6 digest of the same inputs: the block is digested.
    assert_ne!(zone.state_digest(), v6_zone().state_digest());
}

#[test]
fn snapshot_round_trips_through_the_codec_and_restore() {
    let mut zone = v7_zone();
    let snapshot = zone.snapshot();
    let bytes = encode_snapshot(&snapshot).unwrap();
    assert!(bytes.starts_with(b"NFSNAP11"));
    let decoded = decode_snapshot(&bytes).unwrap();
    assert_eq!(decoded, snapshot);
    let restored = restore_snapshot(decoded).unwrap();
    assert_eq!(restored.snapshot(), snapshot);
    assert_eq!(restored.state_digest(), zone.state_digest());
    // A populated block (one ground item, one stock row) round-trips and validates too.
    let seed = snapshot.seed;
    let rules = economy_data::embedded_phase5_rules().unwrap();
    let mut populated_snapshot = snapshot.clone();
    let block = populated_snapshot.phase5.as_mut().unwrap();
    let g = ground(seed, 7, 57, 120, Tick(0));
    block.ground.insert(g.entity, g);
    block.stock.insert(
        (353_630, 8751),
        VendorStock {
            count: 1,
            max: 1,
            restock_ticks: 72_000,
            due: None,
        },
    );
    materialize_ground(&mut populated_snapshot);
    let bytes = encode_snapshot(&populated_snapshot).unwrap();
    let decoded = decode_snapshot(&bytes).unwrap();
    assert_eq!(decoded, populated_snapshot);
    let restored =
        ZoneState::from_snapshot_with_bundles(decoded, Some(bundle().items), Some(rules.clone()))
            .unwrap();
    assert_eq!(restored.snapshot(), populated_snapshot);
    // Restoration needs the bound rules: missing or mismatched economy rules are refused.
    assert!(
        ZoneState::from_snapshot_with_items(populated_snapshot.clone(), Some(bundle().items))
            .is_err()
    );
    let mut wrong_hash = populated_snapshot.clone();
    wrong_hash.phase5.as_mut().unwrap().rules_hash[0] ^= 1;
    assert!(
        ZoneState::from_snapshot_with_bundles(wrong_hash, Some(bundle().items), Some(rules))
            .is_err()
    );
    // Running a tick keeps the block and the V7 version.
    let applied = zone.run_tick(zone.draft(Vec::new())).unwrap();
    assert_eq!(applied.digest_version, StateDigestVersion::BinaryV7);
    assert!(zone.snapshot().phase5.is_some());
}

#[test]
fn every_phase5_field_is_digested_in_design_order() {
    let mut zone = v7_zone();
    let mut block = zone.snapshot().phase5.unwrap();
    populated(&mut block);
    let json = serde_json::to_value(&block).unwrap();
    let back: Phase5Snapshot = serde_json::from_value(json).unwrap();
    assert_eq!(back, block);
    zone.set_phase5_block_for_test(block.clone());
    let base = zone.state_digest();
    assert_ne!(base, v7_zone().state_digest());
    let mutations: Vec<(&str, Box<dyn Fn(&mut Phase5Snapshot)>)> = vec![
        ("rules_hash", Box::new(|b| b.rules_hash[31] ^= 1)),
        (
            "ground.pos",
            Box::new(|b| b.ground.values_mut().next().unwrap().pos = Vec2Fixed::from_tiles(11, 10)),
        ),
        (
            "ground.count",
            Box::new(|b| b.ground.values_mut().next().unwrap().item.count += 1),
        ),
        (
            "ground.dropped_tick",
            Box::new(|b| b.ground.values_mut().next().unwrap().dropped_tick = Tick(1)),
        ),
        (
            "ground.source",
            Box::new(|b| {
                b.ground.values_mut().next().unwrap().source =
                    GroundSource::PkDrop { victim: id(5) }
            }),
        ),
        (
            "ground.loot_owner",
            Box::new(|b| b.ground.values_mut().next().unwrap().loot_owner = None),
        ),
        (
            "ground.drop_protection.until",
            Box::new(|b| {
                b.ground
                    .values_mut()
                    .next()
                    .unwrap()
                    .drop_protection
                    .as_mut()
                    .unwrap()
                    .until = Tick(1)
            }),
        ),
        (
            "ground.decay_due",
            Box::new(|b| b.ground.values_mut().next().unwrap().decay_due = None),
        ),
        ("npc.spoiler", Box::new(|b| b.npcs.values_mut().next().unwrap().spoiler = None)),
        (
            "npc.seed",
            Box::new(|b| {
                b.npcs
                    .values_mut()
                    .next()
                    .unwrap()
                    .seed
                    .as_mut()
                    .unwrap()
                    .seeder = id(9)
            }),
        ),
        ("npc.seeded", Box::new(|b| b.npcs.values_mut().next().unwrap().seeded = false)),
        (
            "npc.harvest",
            Box::new(|b| b.npcs.values_mut().next().unwrap().harvest = Some((5073, 4))),
        ),
        (
            "npc.sweep",
            Box::new(|b| {
                b.npcs
                    .values_mut()
                    .next()
                    .unwrap()
                    .sweep
                    .as_mut()
                    .unwrap()
                    .pop();
            }),
        ),
        (
            "party.members",
            Box::new(|b| b.parties.values_mut().next().unwrap().members.swap(0, 1)),
        ),
        (
            "party.mode",
            Box::new(|b| b.parties.values_mut().next().unwrap().distribution = LootMode::Random),
        ),
        (
            "party.cursor",
            Box::new(|b| b.parties.values_mut().next().unwrap().item_last_loot = 2),
        ),
        (
            "party.vote",
            Box::new(|b| {
                b.parties
                    .values_mut()
                    .next()
                    .unwrap()
                    .loot_vote
                    .as_mut()
                    .unwrap()
                    .expires = Tick(151)
            }),
        ),
        (
            "party.vote.answers",
            Box::new(|b| {
                b.parties
                    .values_mut()
                    .next()
                    .unwrap()
                    .loot_vote
                    .as_mut()
                    .unwrap()
                    .answers
                    .insert(id(2));
            }),
        ),
        (
            "party.pending_invite",
            Box::new(|b| b.parties.values_mut().next().unwrap().pending_invite = None),
        ),
        ("next_party", Box::new(|b| b.next_party = 3)),
        (
            "request.kind",
            Box::new(|b| b.requests.values_mut().next().unwrap().kind = TxKind::Trade),
        ),
        (
            "request.expires",
            Box::new(|b| b.requests.values_mut().next().unwrap().expires = Tick(151)),
        ),
        (
            "trade.items",
            Box::new(|b| b.trades.values_mut().next().unwrap().list_a.items[0].1 = 2),
        ),
        (
            "trade.confirmed",
            Box::new(|b| b.trades.values_mut().next().unwrap().list_a.confirmed = false),
        ),
        (
            "trade.locked",
            Box::new(|b| b.trades.values_mut().next().unwrap().list_b.locked = true),
        ),
        (
            "trade.requester",
            Box::new(|b| {
                let t = b.trades.values_mut().next().unwrap();
                std::mem::swap(&mut t.a, &mut t.b);
            }),
        ),
        (
            "store.kind",
            Box::new(|b| b.stores.values_mut().next().unwrap().kind = PrivateStoreKind::Sell),
        ),
        (
            "store.title",
            Box::new(|b| b.stores.values_mut().next().unwrap().title = "ware".into()),
        ),
        (
            "store.sell.price",
            Box::new(|b| b.stores.values_mut().next().unwrap().sell[0].price = 11),
        ),
        (
            "store.buy.enchant",
            Box::new(|b| b.stores.values_mut().next().unwrap().buy[0].enchant = 1),
        ),
        (
            "store.manufacture",
            Box::new(|b| b.stores.values_mut().next().unwrap().manufacture[0].1 = 501),
        ),
        ("stock.count", Box::new(|b| b.stock.values_mut().next().unwrap().count = -1)),
        ("stock.max", Box::new(|b| b.stock.values_mut().next().unwrap().max = 2)),
        (
            "stock.restock",
            Box::new(|b| b.stock.values_mut().next().unwrap().restock_ticks = 1),
        ),
        ("stock.due", Box::new(|b| b.stock.values_mut().next().unwrap().due = None)),
        (
            "player.recipes.dwarven",
            Box::new(|b| {
                b.players
                    .values_mut()
                    .next()
                    .unwrap()
                    .recipes
                    .dwarven
                    .insert(2);
            }),
        ),
        (
            "player.recipes.common",
            Box::new(|b| {
                b.players
                    .values_mut()
                    .next()
                    .unwrap()
                    .recipes
                    .common
                    .insert(1000);
            }),
        ),
        (
            "player.refund",
            Box::new(|b| b.players.values_mut().next().unwrap().refund[0].count = 3),
        ),
        (
            "player.party",
            Box::new(|b| b.players.values_mut().next().unwrap().party = None),
        ),
        (
            "player.trade",
            Box::new(|b| b.players.values_mut().next().unwrap().trade = None),
        ),
        (
            "player.flood",
            Box::new(|b| {
                b.players
                    .values_mut()
                    .next()
                    .unwrap()
                    .flood
                    .insert(FloodKind::Transaction, Tick(41));
            }),
        ),
        (
            "player.flood.kind",
            Box::new(|b| {
                let f = &mut b.players.values_mut().next().unwrap().flood;
                f.remove(&FloodKind::ServerBypass);
                f.insert(FloodKind::Manor, Tick(5));
            }),
        ),
    ];
    let mut digests: BTreeMap<[u8; 32], &str> = BTreeMap::new();
    digests.insert(base, "base");
    for (label, mutate) in &mutations {
        let mut mutated = block.clone();
        mutate(&mut mutated);
        assert_ne!(mutated, block, "{label}: mutation is a no-op");
        let mut zone = v7_zone();
        zone.set_phase5_block_for_test(mutated);
        let digest = zone.state_digest();
        assert!(
            digests.insert(digest, label).is_none(),
            "{label}: digest collides with {}",
            digests[&digest]
        );
    }
    assert_eq!(digests.len(), mutations.len() + 1);
}

#[test]
fn invariants_fail_closed_at_restore() {
    let zone = v7_zone();
    let snapshot = zone.snapshot();
    let seed = snapshot.seed;
    let rules = economy_data::embedded_phase5_rules().unwrap();
    let restore = |edit: &dyn Fn(&mut Phase5Snapshot)| {
        let mut s = snapshot.clone();
        edit(s.phase5.as_mut().unwrap());
        materialize_ground(&mut s);
        ZoneState::from_snapshot_with_bundles(s, Some(bundle().items), Some(rules.clone()))
    };
    // A ground entity without its record, or a record whose entity is missing, is refused.
    let mut orphan = snapshot.clone();
    let g = ground(seed, 7, 57, 120, Tick(0));
    orphan
        .phase5
        .as_mut()
        .unwrap()
        .ground
        .insert(g.entity, g.clone());
    materialize_ground(&mut orphan);
    orphan.phase5.as_mut().unwrap().ground.clear();
    assert!(ZoneState::from_snapshot_with_bundles(
        orphan,
        Some(bundle().items),
        Some(rules.clone())
    )
    .is_err());
    let mut missing = snapshot.clone();
    missing.phase5.as_mut().unwrap().ground.insert(g.entity, g);
    assert!(ZoneState::from_snapshot_with_bundles(
        missing,
        Some(bundle().items),
        Some(rules.clone())
    )
    .is_err());
    // A valid ground item passes.
    restore(&|b| {
        let g = ground(seed, 7, 57, 120, Tick(0));
        b.ground.insert(g.entity, g);
    })
    .unwrap();
    // Wrong P1 entity id; wrong location; non-stackable count; template outside the catalogue.
    assert!(restore(&|b| {
        let g = ground(seed, 7, 57, 120, Tick(0));
        b.ground.insert(
            id(77),
            GroundItem {
                entity: id(77),
                ..g
            },
        );
    })
    .is_err());
    assert!(restore(&|b| {
        let mut g = ground(seed, 7, 57, 120, Tick(0));
        g.item.location = ItemLocation::Inventory;
        b.ground.insert(g.entity, g);
    })
    .is_err());
    assert!(restore(&|b| {
        let g = ground(seed, 7, 1, 2, Tick(0)); // Short Sword ×2
        b.ground.insert(g.entity, g);
    })
    .is_err());
    assert!(restore(&|b| {
        let g = ground(seed, 7, 999_999, 1, Tick(0));
        b.ground.insert(g.entity, g);
    })
    .is_err());
    // z-review-2 P3-7 (Phase 4 I1): NPC loot is minted from this epoch's allocated range;
    // an in-range id at or past `next` is never valid; a PK drop may carry an older id.
    for (item, pk, ok) in [
        (60, false, false),
        (200, false, false),
        (60, true, false),
        (200, true, true),
    ] {
        let accepted = restore(&|b| {
            let mut g = ground(seed, item, 57, 120, Tick(0));
            if pk {
                g.source = GroundSource::PkDrop { victim: id(5) };
                g.loot_owner = None;
                g.drop_protection = None;
                g.decay_due = None;
            }
            b.ground.insert(g.entity, g);
        })
        .is_ok();
        assert_eq!(accepted, ok, "id {item} pk {pk}");
    }
    // Duplicate instance across two ground items.
    assert!(restore(&|b| {
        let g = ground(seed, 7, 57, 120, Tick(0));
        let h = ground(seed, 7, 57, 5, Tick(1));
        b.ground.insert(g.entity, g);
        b.ground.insert(h.entity, h);
    })
    .is_err());
    // Sidecar for a missing NPC; player without an admitted V6 player; party of one;
    // request to oneself; trade without both back-references; refund tab of 13; stock
    // outside the rules; unknown recipe.
    assert!(restore(&|b| {
        b.npcs.insert(id(900), NpcEconomy::default());
    })
    .is_err());
    assert!(restore(&|b| {
        b.players.insert(id(1), Phase5Player::default());
    })
    .is_err());
    assert!(restore(&|b| {
        b.parties.insert(
            PartyId(1),
            Party {
                id: PartyId(1),
                members: vec![id(1)],
                distribution: LootMode::FindersKeepers,
                item_last_loot: 0,
                loot_vote: None,
                pending_invite: None,
            },
        );
        b.next_party = 2;
    })
    .is_err());
    assert!(restore(&|b| {
        b.requests.insert(
            id(1),
            TxRequest {
                requester: id(1),
                target: id(1),
                kind: TxKind::Trade,
                expires: Tick(1),
            },
        );
    })
    .is_err());
    assert!(restore(&|b| {
        b.stock.insert(
            (1, 57),
            VendorStock {
                count: 0,
                max: 1,
                restock_ticks: 1,
                due: None,
            },
        );
    })
    .is_err());
    assert!(restore(&|b| {
        b.stock.insert(
            (353_630, 8751),
            VendorStock {
                count: 5,
                max: 1,
                restock_ticks: 1,
                due: None,
            },
        );
    })
    .is_err());
    // The rules hash is bound: a block from other rules is refused even when well-formed.
    assert!(restore(&|b| b.rules_hash = [0; 32]).is_err());
}

#[test]
fn player_loads_need_a_matching_phase5_checkpoint() {
    let rules = economy_data::embedded_phase5_rules().unwrap();
    let good = Phase5Checkpoint {
        schema_version: 1,
        rules_hash: rules.hash_hex(),
        recipes: RecipeBook {
            dwarven: [1].into_iter().collect(),
            common: BTreeSet::new(),
        },
    };
    good.validate_rules(&rules).unwrap();
    let mut stale = good.clone();
    stale.rules_hash = "00".repeat(32);
    assert!(stale.validate_rules(&rules).is_err());
    // A V7 load without the Phase 5 block, or without the V5/V6 blocks, is malformed.
    let mut load = PlayerLoad::fresh("fighter");
    load.phase5 = Some(good);
    assert!(load.validate_phase4().is_err());
}

#[test]
fn legacy_epochs_carry_no_phase5_block_and_keep_their_bytes() {
    let zone = v6_zone();
    let snapshot = zone.snapshot();
    assert!(snapshot.phase5.is_none());
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(!json.contains("\"phase5\""));
    let bytes = encode_snapshot(&snapshot).unwrap();
    assert!(bytes.starts_with(b"NFSNAP10"));
    assert_eq!(decode_snapshot(&bytes).unwrap(), snapshot);
    // A V6 snapshot that smuggles a Phase 5 block is refused everywhere.
    let mut smuggled = snapshot.clone();
    smuggled.phase5 = Some(Phase5Snapshot::new(&economy_data::embedded_phase5_rules().unwrap()));
    assert!(smuggled.validate_version().is_err());
    assert!(encode_snapshot(&smuggled).is_err());
    assert!(ZoneState::from_snapshot_with_bundles(
        smuggled,
        Some(bundle().items),
        Some(economy_data::embedded_phase5_rules().unwrap())
    )
    .is_err());
    // And V6 restoration never accepts economy rules.
    assert!(ZoneState::from_snapshot_with_bundles(
        snapshot,
        Some(bundle().items),
        Some(economy_data::embedded_phase5_rules().unwrap())
    )
    .is_err());
    let _ = Arc::new(());
}
