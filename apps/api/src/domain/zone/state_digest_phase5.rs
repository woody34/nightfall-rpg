//! `BinaryV7` canonical Phase 5 layout (design §12.4): after the complete V6 digest, the
//! `Phase5Rules` hash, ground items, NPC sidecars, parties, requests, trades, stores, vendor
//! stock and per-player recipes/refund/flood, each in key order with explicit little-endian
//! encodings. DB-only rows are not zone state and are not digested.
use super::Writer;
use crate::domain::zone::phase5::{
    FloodKind, GroundItem, GroundSource, NpcEconomy, Party, Phase5Player, Phase5Snapshot,
    PrivateStore, PrivateStoreKind, TradeList, TxKind, TxRequest, VendorStock,
};
use crate::domain::zone::phase5_command::LootMode;
use crate::domain::zone::{ItemInstance, ItemLocation, Tick};

pub(super) fn append(w: &mut Writer, p: &Phase5Snapshot) {
    w.0.extend_from_slice(&p.rules_hash);
    w.len(p.ground.len());
    for (id, ground) in &p.ground {
        w.id(*id);
        ground_item(w, ground);
    }
    w.len(p.npcs.len());
    for (id, npc) in &p.npcs {
        w.id(*id);
        npc_economy(w, npc);
    }
    w.len(p.parties.len());
    for (id, party) in &p.parties {
        w.u64(id.0);
        party_record(w, party);
    }
    w.u64(p.next_party);
    w.len(p.requests.len());
    for (target, request) in &p.requests {
        w.id(*target);
        tx_request(w, request);
    }
    w.len(p.trades.len());
    for ((a, b), trade) in &p.trades {
        w.id(*a);
        w.id(*b);
        w.id(trade.a);
        w.id(trade.b);
        trade_list(w, &trade.list_a);
        trade_list(w, &trade.list_b);
    }
    w.len(p.stores.len());
    for (owner, store) in &p.stores {
        w.id(*owner);
        private_store(w, store);
    }
    w.len(p.stock.len());
    for ((buylist, template), stock) in &p.stock {
        w.u64(*buylist);
        w.u32(*template);
        vendor_stock(w, stock);
    }
    w.len(p.players.len());
    for (id, player) in &p.players {
        w.id(*id);
        phase5_player(w, player);
    }
}

fn tick(w: &mut Writer, tick: Tick) {
    w.u64(tick.0);
}

fn loot_mode(w: &mut Writer, mode: LootMode) {
    w.u8(LootMode::ALL
        .iter()
        .position(|m| *m == mode)
        .and_then(|p| u8::try_from(p).ok())
        .unwrap_or(u8::MAX));
}

fn instance(w: &mut Writer, item: &ItemInstance) {
    w.option(item.creation_origin.as_ref(), |w, origin| {
        w.u32(origin.zone.0);
        w.u64(origin.epoch);
    });
    w.u64(item.id.get());
    w.u32(item.template_id);
    w.u64(item.count);
    w.u8(match item.location {
        ItemLocation::Inventory => 0,
        ItemLocation::Paperdoll => 1,
        ItemLocation::Ground => 2,
        ItemLocation::Refund => 3,
        ItemLocation::Warehouse => 4,
        ItemLocation::Freight => 5,
        ItemLocation::Mail => 6,
        ItemLocation::Delivery => 7,
        ItemLocation::AuctionBid => 8,
    });
    // z-review-2 P3-5: the V6 slot code, so a variant rename cannot move V7 digests.
    w.option(item.slot.as_ref(), |w, slot| w.u8(*slot as u8));
    w.u32(item.enchant);
    w.option(item.augment_id.as_ref(), |w, id| w.u32(*id));
    w.u8(item.charges);
    w.u64(item.created_tick.0);
    w.u64(item.revision);
}

fn ground_item(w: &mut Writer, g: &GroundItem) {
    w.id(g.entity);
    instance(w, &g.item);
    w.pos(g.pos);
    tick(w, g.dropped_tick);
    match g.source {
        GroundSource::NpcLoot { npc } => {
            w.u8(0);
            w.id(npc);
        },
        GroundSource::PkDrop { victim } => {
            w.u8(1);
            w.id(victim);
        },
    }
    w.option(g.loot_owner.as_ref(), |w, owner| {
        w.id(owner.owner);
        tick(w, owner.until);
    });
    w.option(g.drop_protection.as_ref(), |w, protect| {
        w.id(protect.owner);
        tick(w, protect.until);
    });
    w.option(g.decay_due.as_ref(), |w, due| tick(w, *due));
}

fn npc_economy(w: &mut Writer, n: &NpcEconomy) {
    w.option(n.spoiler.as_ref(), |w, id| w.id(*id));
    w.option(n.seed.as_ref(), |w, seed| {
        w.u32(seed.seed);
        w.id(seed.seeder);
    });
    w.bool(n.seeded);
    w.option(n.harvest.as_ref(), |w, (template, count)| {
        w.u32(*template);
        w.u64(*count);
    });
    w.option(n.sweep.as_ref(), |w, holders| {
        w.len(holders.len());
        for (template, count) in holders {
            w.u32(*template);
            w.u64(*count);
        }
    });
}

fn party_record(w: &mut Writer, p: &Party) {
    w.u64(p.id.0);
    w.len(p.members.len());
    for member in &p.members {
        w.id(*member);
    }
    loot_mode(w, p.distribution);
    w.u32(p.item_last_loot);
    w.option(p.loot_vote.as_ref(), |w, vote| {
        loot_mode(w, vote.mode);
        w.len(vote.answers.len());
        for answer in &vote.answers {
            w.id(*answer);
        }
        tick(w, vote.expires);
    });
    w.option(p.pending_invite.as_ref(), |w, until| tick(w, *until));
}

fn tx_request(w: &mut Writer, r: &TxRequest) {
    w.id(r.requester);
    w.id(r.target);
    match r.kind {
        TxKind::Trade => w.u8(0),
        TxKind::PartyInvite { mode } => {
            w.u8(1);
            loot_mode(w, mode);
        },
    }
    tick(w, r.expires);
}

fn trade_list(w: &mut Writer, list: &TradeList) {
    w.len(list.items.len());
    for (instance, count) in &list.items {
        w.u64(instance.get());
        w.u64(*count);
    }
    w.bool(list.confirmed);
    w.bool(list.locked);
}

fn private_store(w: &mut Writer, s: &PrivateStore) {
    w.u8(match s.kind {
        PrivateStoreKind::Sell => 0,
        PrivateStoreKind::PackageSell => 1,
        PrivateStoreKind::Buy => 2,
        PrivateStoreKind::Manufacture => 3,
    });
    w.string(&s.title);
    w.len(s.sell.len());
    for entry in &s.sell {
        w.u64(entry.instance.get());
        w.u64(entry.count);
        w.u64(entry.price);
    }
    w.len(s.buy.len());
    for entry in &s.buy {
        w.u32(entry.template);
        w.u32(entry.enchant);
        w.u64(entry.count);
        w.u64(entry.price);
    }
    w.len(s.manufacture.len());
    for (recipe, cost) in &s.manufacture {
        w.u32(*recipe);
        w.u64(*cost);
    }
}

fn vendor_stock(w: &mut Writer, s: &VendorStock) {
    w.i64(s.count);
    w.u64(s.max);
    w.u64(s.restock_ticks);
    w.option(s.due.as_ref(), |w, due| tick(w, *due));
}

fn phase5_player(w: &mut Writer, p: &Phase5Player) {
    w.len(p.recipes.dwarven.len());
    for recipe in &p.recipes.dwarven {
        w.u32(*recipe);
    }
    w.len(p.recipes.common.len());
    for recipe in &p.recipes.common {
        w.u32(*recipe);
    }
    w.len(p.refund.len());
    for item in &p.refund {
        instance(w, item);
    }
    w.option(p.party.as_ref(), |w, party| w.u64(party.0));
    w.option(p.trade.as_ref(), |w, (a, b)| {
        w.id(*a);
        w.id(*b);
    });
    w.len(p.flood.len());
    for (kind, next) in &p.flood {
        w.u8(FloodKind::ordinal(*kind));
        tick(w, *next);
    }
    // P5-D17 herb reuse, after the §12.4 item-9 fields.
    w.len(p.herb_reuse.len());
    for reuse in &p.herb_reuse {
        w.u32(reuse.skill);
        w.u32(reuse.level);
        tick(w, reuse.until);
    }
}
