//! Story 4.6: protobuf mapping of the nine Phase 5 read RPCs (design §13.4). The use cases are
//! in `application::use_cases::economy_queries`; this module only maps views to the wire.

use super::pb;
use crate::application::economy_reads::{ContainerItem, MailRow};
use crate::application::use_cases::economy_queries::{
    AuctionViewRow, AuctionsView, BuylistView, ContainerView, ManorView, MultisellItemView,
    MultisellView, ReadMailView, RecipeBookView,
};
use crate::domain::zone::ItemInstance;

fn container_item(item: &ContainerItem, location: pb::ItemLocation) -> pb::OwnedItem {
    pb::OwnedItem {
        instance_id: item.instance_id,
        template_id: item.template_id,
        count: item.count,
        enchant: item.enchant,
        augment_id: item.augment_id,
        location: location.into(),
        slot: 0,
        charge_bits: item.charge_bits,
        revision: item.revision,
    }
}

fn refund_item(item: &ItemInstance) -> pb::OwnedItem {
    pb::OwnedItem {
        instance_id: item.id.get(),
        template_id: item.template_id,
        count: item.count,
        enchant: item.enchant,
        augment_id: item.augment_id,
        location: pb::ItemLocation::Refund.into(),
        slot: 0,
        charge_bits: u32::from(item.charges),
        revision: item.revision,
    }
}

pub(super) fn buylist(view: &BuylistView) -> pb::ListBuylistResponse {
    pb::ListBuylistResponse {
        list_id: view.list_id,
        merchant_npc_id: view.merchant_npc_id,
        castle_tax_percent: view.castle_tax_percent,
        entries: view
            .entries
            .iter()
            .map(|e| pb::BuylistEntry {
                template_id: e.template_id,
                unit_price: e.unit_price,
                limited: e.limited,
                stock: e.stock,
                max_stock: e.max_stock,
            })
            .collect(),
        refund_tab: view.refund_tab.iter().map(refund_item).collect(),
        state_tick: view.state_tick,
    }
}

fn multisell_item(item: &MultisellItemView) -> pb::MultisellIngredient {
    pb::MultisellIngredient {
        template_id: item.template_id,
        count: item.count,
        tax_ingredient: item.tax,
        maintain_ingredient: item.maintain,
        enchant: 0,
    }
}

pub(super) fn multisell(view: MultisellView) -> pb::GetMultisellListResponse {
    pb::GetMultisellListResponse {
        list_id: view.list_id,
        maintain_enchantment: view.maintain_enchantment,
        apply_taxes: view.apply_taxes,
        allowed_npc_ids: view.allowed_npc_ids,
        entries: view
            .entries
            .iter()
            .map(|e| pb::MultisellEntry {
                entry_id: e.entry_id,
                ingredients: e.ingredients.iter().map(multisell_item).collect(),
                products: e.products.iter().map(multisell_item).collect(),
                stackable: e.stackable,
            })
            .collect(),
    }
}

/// The detached checkpoint view: `tick = 0`, `incarnation = 0` (D36 pattern).
pub(super) fn recipe_book(view: RecipeBookView) -> pb::GetRecipeBookResponse {
    pb::GetRecipeBookResponse {
        book: Some(pb::RecipeBookSync {
            dwarven: view.book.dwarven.into_iter().collect(),
            common: view.book.common.into_iter().collect(),
            dwarven_limit: view.dwarven_limit,
            common_limit: view.common_limit,
            baseline: true,
            tick: 0,
            incarnation: 0,
        }),
    }
}

pub(super) fn container(view: &ContainerView, freight: bool) -> (Vec<pb::OwnedItem>, u32, u64) {
    let location = if freight {
        pb::ItemLocation::Freight
    } else {
        pb::ItemLocation::Warehouse
    };
    (
        view.items
            .iter()
            .map(|i| container_item(i, location))
            .collect(),
        view.capacity,
        view.revision,
    )
}

fn mail_header(row: &MailRow) -> pb::MailHeader {
    pb::MailHeader {
        message_id: row.id.to_string(),
        sender_name: row.sender_name.clone(),
        receiver_name: row.receiver_name.clone(),
        subject: row.subject.clone(),
        has_attachments: row.has_attachments,
        cod_adena: row.cod_adena,
        read: row.read,
        returned: row.returned,
        system_kind: row.system_kind,
        expires_unix_ms: row.expires_unix_ms,
    }
}

pub(super) fn mail_list(rows: &[MailRow], count: u32, limit: u32) -> pb::ListMailResponse {
    pb::ListMailResponse {
        messages: rows.iter().map(mail_header).collect(),
        count,
        limit,
    }
}

pub(super) fn read_mail(view: &ReadMailView) -> pb::ReadMailResponse {
    pb::ReadMailResponse {
        header: Some(mail_header(&view.header)),
        body: view.body.clone(),
        attachments: view
            .attachments
            .iter()
            .map(|i| container_item(i, pb::ItemLocation::Mail))
            .collect(),
    }
}

fn auction(view: &AuctionViewRow) -> pb::AuctionView {
    // `item_auctions.state` is 0 CREATED, 1 STARTED, 2 FINISHED; the wire reserves 0.
    let state = match view.state {
        0 => pb::AuctionState::Created,
        1 => pb::AuctionState::Started,
        2 => pb::AuctionState::Finished,
        _ => pb::AuctionState::Unspecified,
    };
    pb::AuctionView {
        auction_id: view.auction_id,
        state: state.into(),
        template_id: view.template_id,
        count: view.count,
        init_bid: view.init_bid,
        highest_bid: view.highest_bid,
        start_unix_ms: view.start_unix_ms,
        end_unix_ms: view.end_unix_ms,
        own_bid: view.own_bid,
        own_cancelled: view.own_cancelled,
    }
}

pub(super) fn auctions(view: &AuctionsView) -> pb::ListItemAuctionsResponse {
    pb::ListItemAuctionsResponse {
        current: view.current.as_ref().map(auction),
        next: view.next.as_ref().map(auction),
        own_bids: view.own_bids.iter().map(auction).collect(),
    }
}

pub(super) fn manor(view: &ManorView) -> pb::GetManorDataResponse {
    pb::GetManorDataResponse {
        castle_id: view.castle_id,
        seeds: view
            .seeds
            .iter()
            .map(|&(seed_id, price, start_amount, amount)| pb::SeedProductionView {
                seed_id,
                price,
                start_amount,
                amount,
            })
            .collect(),
        crops: view
            .crops
            .iter()
            .map(|&(crop_id, price, start_amount, amount, reward_type)| pb::CropProcureView {
                crop_id,
                price,
                start_amount,
                amount,
                reward_type,
            })
            .collect(),
    }
}
