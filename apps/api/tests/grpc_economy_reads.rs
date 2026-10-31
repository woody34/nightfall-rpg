//! Story 4.6: the nine Phase 5 read RPCs over real gRPC sockets (design §13.4), with the
//! memory economy adapter and a stub zone read port. The per-RPC contract table covers
//! authentication, ownership and malformed input for every RPC, then each RPC's own
//! `FAILED_PRECONDITION` / `UNAVAILABLE` / empty cases. No RPC writes a row (E31).
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::arithmetic_side_effects
)]

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use common::ws::{seed_player, Player};
use common::{Game, TestApp};
use nightfall_api::application::use_cases::economy_queries::{AdmittedView, VendorState};
use nightfall_api::application::use_cases::{EconomyQueries, EconomyZoneReads};
use nightfall_api::application::AppError;
use nightfall_api::domain::zone::{Phase4Rules, Phase5Rules};
use nightfall_api::infrastructure::memory::{InMemoryEconomyStore, StoredRow, MEMORY_EPOCH_MS};
use nightfall_api::interface::grpc::pb;
use parking_lot::Mutex;
use tonic::Code;
use uuid::Uuid;

const MERCHANT: Uuid = Uuid::from_u128(0x00A1_F5ED);
const ALFRED: u32 = 35363;
const HOUR: i64 = 3_600_000;

fn rules() -> (Arc<Phase4Rules>, Arc<Phase5Rules>) {
    static RULES: OnceLock<(Arc<Phase4Rules>, Arc<Phase5Rules>)> = OnceLock::new();
    RULES
        .get_or_init(|| {
            let phase4 = Arc::new(
                nightfall_api::infrastructure::item_data::load_embedded_phase4_rules([7; 32])
                    .unwrap(),
            );
            let phase5 =
                nightfall_api::infrastructure::economy_data::phase5_rules_for(&phase4).unwrap();
            (phase4, phase5)
        })
        .clone()
}

/// The stub zone read port: who is admitted (and where), and one bound merchant.
#[derive(Default)]
struct StubZone {
    admitted: Mutex<BTreeMap<Uuid, bool>>,
    calls: Mutex<u32>,
}

#[async_trait]
impl EconomyZoneReads for StubZone {
    async fn admitted(&self, character: Uuid) -> Result<Option<AdmittedView>, AppError> {
        *self.calls.lock() += 1;
        Ok(self
            .admitted
            .lock()
            .get(&character)
            .map(|peace| AdmittedView {
                in_peace_square: *peace,
            }))
    }
    async fn vendor(
        &self,
        character: Uuid,
        merchant: Uuid,
    ) -> Result<Option<VendorState>, AppError> {
        *self.calls.lock() += 1;
        if merchant != MERCHANT || !self.admitted.lock().contains_key(&character) {
            return Ok(None);
        }
        Ok(Some(VendorState {
            merchant_npc_id: ALFRED,
            state_tick: 77,
            stock: vec![((353_630, 8751), (0, 1))],
            refund_tab: vec![],
        }))
    }
}

fn cols(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

struct World {
    app: TestApp,
    owner: Player,
    peer: Player,
    store: Arc<InMemoryEconomyStore>,
    zone: Arc<StubZone>,
    mail: Uuid,
}

/// The owner's warehouse (2 rows) and freight (1 row); a mail from the peer to the owner with
/// one attachment; an expired mail; a started auction both bid on; Gludio manor rows.
fn seed(store: &InMemoryEconomyStore, owner: Uuid, peer: Uuid, mail: Uuid) {
    for (id, kind, template, count) in [
        (900_002, "warehouse", 57, 100),
        (900_001, "warehouse", 1864, 5),
        (900_003, "freight", 1865, 2),
    ] {
        store.put_row(
            "item_instance",
            &id.to_string(),
            StoredRow {
                revision: if id == 900_001 { 4 } else { 1 },
                columns: cols(&[
                    ("character_id", serde_json::json!(owner.to_string())),
                    ("owner_kind", serde_json::json!(kind)),
                    ("location", serde_json::json!(kind)),
                    ("template_id", serde_json::json!(template)),
                    ("count", serde_json::json!(count)),
                ]),
            },
        );
    }
    store.put_row(
        "item_instance",
        "900010",
        StoredRow {
            revision: 0,
            columns: cols(&[
                ("mail_id", serde_json::json!(mail.to_string())),
                ("owner_kind", serde_json::json!("mail")),
                ("location", serde_json::json!("mail")),
                ("template_id", serde_json::json!(57)),
                ("count", serde_json::json!(500)),
            ]),
        },
    );
    let message = |subject: &str, attachments: bool| StoredRow {
        revision: 0,
        columns: cols(&[
            ("sender_id", serde_json::json!(peer.to_string())),
            ("receiver_id", serde_json::json!(owner.to_string())),
            ("subject", serde_json::json!(subject)),
            ("body", serde_json::json!("body text")),
            ("cod_adena", serde_json::json!(0)),
            ("has_attachments", serde_json::json!(attachments)),
        ]),
    };
    store.put_row("mail_message", &mail.to_string(), message("adena", true));
    store.set_mail_times(mail, MEMORY_EPOCH_MS - HOUR, MEMORY_EPOCH_MS + 359 * HOUR);
    let expired = Uuid::from_u128(0x7199);
    store.put_row("mail_message", &expired.to_string(), message("old", false));
    store.set_mail_times(expired, MEMORY_EPOCH_MS - 400 * HOUR, MEMORY_EPOCH_MS - 40 * HOUR);
    store.put_row("mail_message", &Uuid::from_u128(0x7198).to_string(), message("plain", false));
    store.set_mail_times(
        Uuid::from_u128(0x7198),
        MEMORY_EPOCH_MS - 2 * HOUR,
        MEMORY_EPOCH_MS + 358 * HOUR,
    );
    store.put_row(
        "item_auction",
        "1",
        StoredRow {
            revision: 2,
            columns: cols(&[
                ("instance_npc", serde_json::json!(32320)),
                ("item_index", serde_json::json!(0)),
                ("state", serde_json::json!(1)),
                ("highest_bidder", serde_json::json!(peer.to_string())),
                ("highest_bid", serde_json::json!(120_000)),
                ("start_unix_ms", serde_json::json!(MEMORY_EPOCH_MS - HOUR)),
                ("end_unix_ms", serde_json::json!(MEMORY_EPOCH_MS + 4 * HOUR)),
            ]),
        },
    );
    for (who, bid) in [(owner, 110_000), (peer, 120_000)] {
        store.put_row(
            "item_auction_bid",
            &format!("1:{who}"),
            StoredRow {
                revision: 0,
                columns: cols(&[
                    ("auction_id", serde_json::json!(1)),
                    ("character_id", serde_json::json!(who.to_string())),
                    ("last_bid", serde_json::json!(bid)),
                ]),
            },
        );
    }
    store.put_row(
        "manor_seed_production",
        "1:5016",
        StoredRow {
            revision: 3,
            columns: cols(&[
                ("castle_id", serde_json::json!(1)),
                ("seed_id", serde_json::json!(5016)),
                ("price", serde_json::json!(5)),
                ("start_amount", serde_json::json!(8100)),
                ("amount", serde_json::json!(8000)),
            ]),
        },
    );
    store.set_character_name(owner, "EcoOwner");
    store.set_character_name(peer, "EcoPeer");
}

async fn world(with_zone: bool) -> World {
    let store = Arc::new(InMemoryEconomyStore::default());
    let zone = Arc::new(StubZone::default());
    let (phase4, phase5) = rules();
    let (reads, port) = (store.clone(), zone.clone());
    let app = TestApp::spawn_with_economy(
        |_| {},
        Box::new(move |deps| {
            Arc::new(EconomyQueries::new(
                deps.characters.clone(),
                reads,
                phase4,
                phase5,
                with_zone.then_some(port as Arc<dyn EconomyZoneReads>),
            ))
        }),
    )
    .await;
    let owner = seed_player(&app, "EcoOwner", 10.0, 10.0);
    let peer = seed_player(&app, "EcoPeer", 12.0, 10.0);
    let mail = Uuid::from_u128(0x7100);
    seed(&store, owner.character.as_uuid(), peer.character.as_uuid(), mail);
    World {
        app,
        owner,
        peer,
        store,
        zone,
        mail,
    }
}

#[derive(Clone, Copy, Debug)]
enum Rpc {
    Buylist,
    RecipeBook,
    Warehouse,
    Freight,
    Mail,
    Read,
    Auctions,
}

const OWNED: [Rpc; 7] = [
    Rpc::Buylist,
    Rpc::RecipeBook,
    Rpc::Warehouse,
    Rpc::Freight,
    Rpc::Mail,
    Rpc::Read,
    Rpc::Auctions,
];

async fn call(client: &mut Game, rpc: Rpc, id: Vec<u8>, mail: Uuid) -> Result<(), tonic::Status> {
    match rpc {
        Rpc::Buylist => client
            .list_buylist(pb::ListBuylistRequest {
                character_id: id,
                merchant: MERCHANT.to_string(),
            })
            .await
            .map(drop),
        Rpc::RecipeBook => client
            .get_recipe_book(pb::GetRecipeBookRequest { character_id: id })
            .await
            .map(drop),
        Rpc::Warehouse => client
            .get_warehouse(pb::GetWarehouseRequest { character_id: id })
            .await
            .map(drop),
        Rpc::Freight => client
            .get_freight(pb::GetFreightRequest { character_id: id })
            .await
            .map(drop),
        Rpc::Mail => client
            .list_mail(pb::ListMailRequest {
                character_id: id,
                mail_box: pb::MailBox::Inbox.into(),
            })
            .await
            .map(drop),
        Rpc::Read => client
            .read_mail(pb::ReadMailRequest {
                character_id: id,
                message_id: mail.to_string(),
            })
            .await
            .map(drop),
        Rpc::Auctions => client
            .list_item_auctions(pb::ListItemAuctionsRequest {
                instance_npc_id: 32320,
                character_id: id,
            })
            .await
            .map(drop),
    }
}

#[tokio::test]
async fn every_read_rpc_follows_the_auth_ownership_and_malformed_contract() {
    let w = world(true).await;
    w.zone
        .admitted
        .lock()
        .insert(w.owner.character.as_uuid(), true);
    let own = w.owner.character.as_uuid().as_bytes().to_vec();
    let unknown = Uuid::from_u128(0xDEAD).as_bytes().to_vec();
    let before = (w.app.characters.staged_events(), w.store.ledger(), w.store.outbox());
    for rpc in OWNED {
        let table: [(Game, Vec<u8>, Code); 4] = [
            (w.app.anon_grpc(), own.clone(), Code::Unauthenticated),
            (w.app.game_as(w.owner.account), vec![1, 2, 3], Code::InvalidArgument),
            (w.app.game_as(w.owner.account), unknown.clone(), Code::NotFound),
            (w.app.game_as(w.peer.account), own.clone(), Code::PermissionDenied),
        ];
        for (mut client, id, expected) in table {
            let code = call(&mut client, rpc, id, w.mail).await.unwrap_err().code();
            assert_eq!(code, expected, "{rpc:?}");
        }
    }
    // The static reads take no character: auth, then a malformed (unknown) id.
    let anon = w
        .app
        .anon_grpc()
        .get_multisell_list(pb::GetMultisellListRequest { list_id: 1 })
        .await;
    assert_eq!(anon.unwrap_err().code(), Code::Unauthenticated);
    let anon = w
        .app
        .anon_grpc()
        .get_manor_data(pb::GetManorDataRequest { castle_id: 1 })
        .await;
    assert_eq!(anon.unwrap_err().code(), Code::Unauthenticated);
    let mut game = w.app.game_as(w.owner.account);
    let missing = game
        .get_multisell_list(pb::GetMultisellListRequest { list_id: 0 })
        .await;
    assert_eq!(missing.unwrap_err().code(), Code::NotFound);
    // Malformed per-RPC fields.
    let bad_merchant = game
        .list_buylist(pb::ListBuylistRequest {
            character_id: own.clone(),
            merchant: "npc".into(),
        })
        .await;
    assert_eq!(bad_merchant.unwrap_err().code(), Code::InvalidArgument);
    let no_box = game
        .list_mail(pb::ListMailRequest {
            character_id: own.clone(),
            mail_box: pb::MailBox::Unspecified.into(),
        })
        .await;
    assert_eq!(no_box.unwrap_err().code(), Code::InvalidArgument);
    let bad_box = game
        .list_mail(pb::ListMailRequest {
            character_id: own.clone(),
            mail_box: 9,
        })
        .await;
    assert_eq!(bad_box.unwrap_err().code(), Code::InvalidArgument);
    let bad_message = game
        .read_mail(pb::ReadMailRequest {
            character_id: own.clone(),
            message_id: "1".into(),
        })
        .await;
    assert_eq!(bad_message.unwrap_err().code(), Code::InvalidArgument);
    let unknown_message = game
        .read_mail(pb::ReadMailRequest {
            character_id: own.clone(),
            message_id: Uuid::from_u128(5).to_string(),
        })
        .await;
    assert_eq!(unknown_message.unwrap_err().code(), Code::NotFound);
    let unknown_instance = game
        .list_item_auctions(pb::ListItemAuctionsRequest {
            instance_npc_id: 1,
            character_id: own.clone(),
        })
        .await;
    assert_eq!(unknown_instance.unwrap_err().code(), Code::NotFound);
    let unknown_merchant = game
        .list_buylist(pb::ListBuylistRequest {
            character_id: own,
            merchant: Uuid::from_u128(9).to_string(),
        })
        .await;
    assert_eq!(unknown_merchant.unwrap_err().code(), Code::NotFound);
    assert_eq!(
        (w.app.characters.staged_events(), w.store.ledger(), w.store.outbox()),
        before,
        "reads write nothing"
    );
}

#[tokio::test]
async fn list_buylist_needs_an_admitted_caller_and_a_running_zone() {
    let w = world(true).await;
    let own = w.owner.character.as_uuid().as_bytes().to_vec();
    let mut game = w.app.game_as(w.owner.account);
    let request = || pb::ListBuylistRequest {
        character_id: own.clone(),
        merchant: MERCHANT.to_string(),
    };
    assert_eq!(game.list_buylist(request()).await.unwrap_err().code(), Code::FailedPrecondition);
    w.zone
        .admitted
        .lock()
        .insert(w.owner.character.as_uuid(), false);
    let view = game.list_buylist(request()).await.unwrap().into_inner();
    assert_eq!(
        (view.list_id, view.merchant_npc_id, view.castle_tax_percent, view.state_tick),
        (353_630, ALFRED, 0, 77)
    );
    assert_eq!(view.entries.len(), 12, "merchants.toml: buylist 353630 holds 12 entries");
    let limited = view.entries.iter().find(|e| e.template_id == 8751).unwrap();
    // Static castle-0 price (merchants.toml unit_prices: item 8751, tax 10) and live stock.
    assert_eq!(
        (limited.unit_price, limited.limited, limited.stock, limited.max_stock),
        (950_400, true, 0, 1)
    );
    assert!(view
        .entries
        .iter()
        .filter(|e| !e.limited)
        .all(|e| e.stock == 0 && e.max_stock == 0));

    let absent = world(false).await;
    let own = absent.owner.character.as_uuid().as_bytes().to_vec();
    let refused = absent
        .app
        .game_as(absent.owner.account)
        .list_buylist(pb::ListBuylistRequest {
            character_id: own,
            merchant: MERCHANT.to_string(),
        })
        .await;
    assert_eq!(refused.unwrap_err().code(), Code::Unavailable, "no zone: UNAVAILABLE");
}

#[tokio::test]
async fn get_multisell_list_is_static_and_keeps_the_pinned_entry_order() {
    let w = world(true).await;
    let (_, phase5) = rules();
    let mut game = w.app.game_as(w.owner.account);
    let list = &phase5.parts().multisell.list[0];
    let view = game
        .get_multisell_list(pb::GetMultisellListRequest { list_id: list.id })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (view.list_id, view.apply_taxes, view.maintain_enchantment),
        (list.id, list.apply_taxes, list.maintain_enchantment)
    );
    assert_eq!(view.allowed_npc_ids, list.npcs);
    assert_eq!(view.entries.len(), list.entries.len());
    assert_eq!(view.entries[0].entry_id, 1, "entry ids are the 1-based pinned order");
    assert_eq!(view.entries[0].ingredients[0].count, list.entries[0].ingredients[0].count);
    // Lists with a FAME / clan-reputation ingredient omit only those entries (P5-Q15).
    let fame = phase5.parts().catalogue.special_multisell_ingredients[0].lists[0];
    let source = phase5
        .parts()
        .multisell
        .list
        .iter()
        .find(|l| l.id == fame)
        .unwrap();
    let view = game
        .get_multisell_list(pb::GetMultisellListRequest { list_id: fame })
        .await
        .unwrap()
        .into_inner();
    let plain = source
        .entries
        .iter()
        .filter(|e| e.ingredients.iter().all(|i| i.id > 0) && e.products.iter().all(|p| p.id > 0))
        .count();
    assert_eq!(view.entries.len(), plain);
    assert!(view.entries.len() < source.entries.len());
    assert!(view
        .entries
        .windows(2)
        .all(|p| p[0].entry_id < p[1].entry_id));
}

#[tokio::test]
async fn get_recipe_book_is_the_checkpoint_view_and_refused_while_admitted() {
    let w = world(true).await;
    let (_, phase5) = rules();
    w.app.characters.set_phase5_column_for_testing(
        w.owner.character,
        serde_json::json!({"schema_version": 1, "rules_hash": phase5.hash_hex(), "recipes": {"dwarven": [], "common": [2, 1]}}),
    );
    let own = w.owner.character.as_uuid().as_bytes().to_vec();
    let mut game = w.app.game_as(w.owner.account);
    let book = game
        .get_recipe_book(pb::GetRecipeBookRequest {
            character_id: own.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .book
        .unwrap();
    assert_eq!((book.common, book.dwarven), (vec![1, 2], vec![]));
    assert_eq!(
        (
            book.dwarven_limit,
            book.common_limit,
            book.baseline,
            book.tick,
            book.incarnation
        ),
        (50, 50, true, 0, 0)
    );
    w.zone
        .admitted
        .lock()
        .insert(w.owner.character.as_uuid(), true);
    let refused = game
        .get_recipe_book(pb::GetRecipeBookRequest { character_id: own })
        .await;
    assert_eq!(refused.unwrap_err().code(), Code::FailedPrecondition);
}

#[tokio::test]
async fn warehouse_and_freight_are_db_rows_with_their_location_and_base_capacity() {
    let w = world(true).await;
    let own = w.owner.character.as_uuid().as_bytes().to_vec();
    let mut game = w.app.game_as(w.owner.account);
    let warehouse = game
        .get_warehouse(pb::GetWarehouseRequest {
            character_id: own.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    let ids: Vec<u64> = warehouse.items.iter().map(|i| i.instance_id).collect();
    assert_eq!(ids, vec![900_001, 900_002], "ascending id");
    assert!(warehouse
        .items
        .iter()
        .all(|i| i.location == pb::ItemLocation::Warehouse as i32 && i.slot == 0));
    assert_eq!(
        (warehouse.capacity, warehouse.revision),
        (100, 4),
        "Human: base 100 (R40); revision = max row"
    );
    let freight = game
        .get_freight(pb::GetFreightRequest { character_id: own })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(freight.items.len(), 1);
    assert_eq!(freight.items[0].location, pb::ItemLocation::Freight as i32);
    assert_eq!((freight.capacity, freight.revision), (200, 1));
    let peer = w.peer.character.as_uuid().as_bytes().to_vec();
    let empty = w
        .app
        .game_as(w.peer.account)
        .get_warehouse(pb::GetWarehouseRequest { character_id: peer })
        .await
        .unwrap()
        .into_inner();
    assert_eq!((empty.items.len(), empty.revision), (0, 0));
}

#[tokio::test]
async fn list_mail_hides_expired_messages_and_read_mail_writes_nothing() {
    let w = world(true).await;
    let own = w.owner.character.as_uuid().as_bytes().to_vec();
    let mut game = w.app.game_as(w.owner.account);
    let inbox = game
        .list_mail(pb::ListMailRequest {
            character_id: own.clone(),
            mail_box: pb::MailBox::Inbox.into(),
        })
        .await
        .unwrap()
        .into_inner();
    let subjects: Vec<&str> = inbox.messages.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, vec!["adena", "plain"], "newest first; the expired mail is hidden");
    assert_eq!((inbox.count, inbox.limit), (2, 240));
    assert_eq!(
        (inbox.messages[0].sender_name.as_str(), inbox.messages[0].receiver_name.as_str()),
        ("EcoPeer", "EcoOwner")
    );
    assert_eq!(inbox.messages[0].expires_unix_ms, MEMORY_EPOCH_MS + 359 * HOUR);
    let outbox = game
        .list_mail(pb::ListMailRequest {
            character_id: own.clone(),
            mail_box: pb::MailBox::Outbox.into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(outbox.count, 0);

    let before = (
        w.store.row("mail_message", &w.mail.to_string()),
        w.store.ledger(),
        w.store.outbox(),
    );
    let read = || pb::ReadMailRequest {
        character_id: own.clone(),
        message_id: w.mail.to_string(),
    };
    // Out of world: header and body, no attachment rows.
    let out = game.read_mail(read()).await.unwrap().into_inner();
    assert_eq!((out.body.as_str(), out.attachments.len()), ("body text", 0));
    assert!(out.header.unwrap().has_attachments);
    // Admitted outside a peace square: FAILED_PRECONDITION (RequestReceivedPost.java:58).
    w.zone
        .admitted
        .lock()
        .insert(w.owner.character.as_uuid(), false);
    assert_eq!(game.read_mail(read()).await.unwrap_err().code(), Code::FailedPrecondition);
    // Admitted inside a peace square: the attachment rows, location MAIL.
    w.zone
        .admitted
        .lock()
        .insert(w.owner.character.as_uuid(), true);
    let inside = game.read_mail(read()).await.unwrap().into_inner();
    assert_eq!(inside.attachments.len(), 1);
    assert_eq!(
        (inside.attachments[0].instance_id, inside.attachments[0].location),
        (900_010, pb::ItemLocation::Mail as i32)
    );
    assert!(!inside.header.unwrap().read, "the read flag moves only through MailMarkRead");
    assert_eq!(
        (
            w.store.row("mail_message", &w.mail.to_string()),
            w.store.ledger(),
            w.store.outbox()
        ),
        before
    );
    // A mail without attachments never asks the zone.
    let calls = *w.zone.calls.lock();
    game.read_mail(pb::ReadMailRequest {
        character_id: own,
        message_id: Uuid::from_u128(0x7198).to_string(),
    })
    .await
    .unwrap();
    assert_eq!(*w.zone.calls.lock(), calls);
}

#[tokio::test]
async fn list_item_auctions_reports_current_next_and_refundable_bids() {
    let w = world(true).await;
    let mut owner = w.app.game_as(w.owner.account);
    let view = owner
        .list_item_auctions(pb::ListItemAuctionsRequest {
            instance_npc_id: 32320,
            character_id: w.owner.character.as_uuid().as_bytes().to_vec(),
        })
        .await
        .unwrap()
        .into_inner();
    let current = view.current.unwrap();
    assert_eq!(current.state, pb::AuctionState::Started as i32);
    assert_eq!(
        (
            current.auction_id,
            current.template_id,
            current.init_bid,
            current.highest_bid,
            current.own_bid
        ),
        (1, 9901, 100_000, 120_000, 110_000)
    );
    assert!(view.next.is_none());
    assert_eq!(view.own_bids.len(), 1, "an outbid bid on a started auction is refundable");
    // The highest bidder cannot reclaim the leading bid (pinned ItemAuction.java:383-391).
    let peer = w
        .app
        .game_as(w.peer.account)
        .list_item_auctions(pb::ListItemAuctionsRequest {
            instance_npc_id: 32320,
            character_id: w.peer.character.as_uuid().as_bytes().to_vec(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(peer.current.unwrap().own_bid, 120_000);
    assert!(peer.own_bids.is_empty());
}

#[tokio::test]
async fn get_manor_data_is_gludio_only() {
    let w = world(true).await;
    let mut game = w.app.game_as(w.owner.account);
    let gludio = game
        .get_manor_data(pb::GetManorDataRequest { castle_id: 1 })
        .await
        .unwrap()
        .into_inner();
    let (_, phase5) = rules();
    let fixture = &phase5.parts().manor.fixture_gludio;
    assert_eq!(gludio.seeds.len(), fixture.seed_production.len());
    assert_eq!(gludio.crops.len(), fixture.crop_procurement.len());
    let seed = gludio.seeds.iter().find(|s| s.seed_id == 5016).unwrap();
    assert_eq!(
        (seed.price, seed.start_amount, seed.amount),
        (5, 8100, 8000),
        "current DB amount"
    );
    let untouched = gludio.seeds.iter().find(|s| s.seed_id == 5650).unwrap();
    assert_eq!(untouched.amount, untouched.start_amount);
    for castle in [0, 2, 9] {
        let other = game
            .get_manor_data(pb::GetManorDataRequest { castle_id: castle })
            .await
            .unwrap()
            .into_inner();
        assert_eq!((other.castle_id, other.seeds.len(), other.crops.len()), (castle, 0, 0));
    }
}
