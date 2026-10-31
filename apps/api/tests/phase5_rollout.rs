//! Real sockets prove the additive Phase 5 economy contract fails closed before the economy
//! runtime (E3/E4) is installed: reads refuse `UNAVAILABLE`, intents reject
//! `NOT_YET_IMPLEMENTED`, and nothing reaches the zone actor or a repository.
#![allow(missing_docs, clippy::unwrap_used, clippy::too_many_lines)]

mod common;

use common::ws::{join, rejected, seed_player};
use common::TestApp;
use nightfall_api::interface::grpc::pb::{
    self, client_message::Intent, server_message::Payload, world_event::Event,
};
use std::time::Duration;
use tonic::Code;

#[tokio::test]
async fn economy_reads_require_authentication_and_refuse_unavailable_without_writes() {
    let app = TestApp::spawn().await;
    let owner = seed_player(&app, "EconomyReader", 10.0, 10.0);
    let character_id = owner.character.as_uuid().as_bytes().to_vec();
    let before = app.characters.staged_events();
    for (mut client, expected) in [
        (app.anon_grpc(), Code::Unauthenticated),
        (app.game_as(owner.account), Code::Unavailable),
    ] {
        let buylist = client
            .list_buylist(pb::ListBuylistRequest {
                character_id: character_id.clone(),
                merchant: String::new(),
            })
            .await;
        assert_eq!(buylist.unwrap_err().code(), expected);
        let multisell = client
            .get_multisell_list(pb::GetMultisellListRequest { list_id: 1 })
            .await;
        assert_eq!(multisell.unwrap_err().code(), expected);
        let recipe_book = client
            .get_recipe_book(pb::GetRecipeBookRequest {
                character_id: character_id.clone(),
            })
            .await;
        assert_eq!(recipe_book.unwrap_err().code(), expected);
        let warehouse = client
            .get_warehouse(pb::GetWarehouseRequest {
                character_id: character_id.clone(),
            })
            .await;
        assert_eq!(warehouse.unwrap_err().code(), expected);
        let freight = client
            .get_freight(pb::GetFreightRequest {
                character_id: character_id.clone(),
            })
            .await;
        assert_eq!(freight.unwrap_err().code(), expected);
        let mail = client
            .list_mail(pb::ListMailRequest {
                character_id: character_id.clone(),
                mail_box: pb::MailBox::Inbox.into(),
            })
            .await;
        assert_eq!(mail.unwrap_err().code(), expected);
        let read = client
            .read_mail(pb::ReadMailRequest {
                character_id: character_id.clone(),
                message_id: "1".into(),
            })
            .await;
        assert_eq!(read.unwrap_err().code(), expected);
        let auctions = client
            .list_item_auctions(pb::ListItemAuctionsRequest {
                instance_npc_id: 1,
                character_id: character_id.clone(),
            })
            .await;
        assert_eq!(auctions.unwrap_err().code(), expected);
        let manor = client
            .get_manor_data(pb::GetManorDataRequest { castle_id: 1 })
            .await;
        assert_eq!(manor.unwrap_err().code(), expected);
    }
    assert_eq!(app.characters.staged_events(), before);
    assert!(app.bus.published().is_empty());
}

/// Every one of the 41 economy intents (`ClientMessage` 29..69), each with a syntactically
/// valid body, so the rejection comes from the missing runtime and not from validation.
fn economy_intents() -> Vec<Intent> {
    let key = || "0c1b2a3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d".to_owned();
    let entity = || "11111111-2222-4333-8444-555555555555".to_owned();
    let items = || {
        vec![pb::ItemCount {
            instance_id: 1,
            count: 1,
        }]
    };
    vec![
        Intent::PickUpItem(pb::PickUpItemRequest {
            idempotency_key: key(),
            ground_entity: entity(),
        }),
        Intent::VendorBuy(pb::VendorBuyRequest {
            idempotency_key: key(),
            merchant: entity(),
            list_id: 1,
            items: vec![pb::TemplateCount {
                template_id: 57,
                count: 1,
            }],
        }),
        Intent::VendorSell(pb::VendorSellRequest {
            idempotency_key: key(),
            merchant: entity(),
            list_id: 1,
            items: items(),
        }),
        Intent::VendorRefund(pb::VendorRefundRequest {
            idempotency_key: key(),
            merchant: entity(),
            list_id: 1,
            refund_indices: vec![0],
        }),
        Intent::MultisellChoose(pb::MultisellChooseRequest {
            idempotency_key: key(),
            npc: entity(),
            list_id: 1,
            entry_id: 1,
            amount: 1,
        }),
        Intent::RegisterRecipe(pb::RegisterRecipeRequest {
            idempotency_key: key(),
            recipe_instance_id: 1,
        }),
        Intent::DeleteRecipe(pb::DeleteRecipeRequest {
            idempotency_key: key(),
            recipe_id: 1,
        }),
        Intent::CraftItem(pb::CraftItemRequest {
            idempotency_key: key(),
            recipe_id: 1,
        }),
        Intent::TradeRequest(pb::TradeRequestRequest {
            idempotency_key: key(),
            target: entity(),
        }),
        Intent::TradeAnswer(pb::TradeAnswerRequest {
            idempotency_key: key(),
            accept: true,
        }),
        Intent::TradeAddItem(pb::TradeAddItemRequest {
            idempotency_key: key(),
            instance_id: 1,
            count: 1,
        }),
        Intent::TradeConfirm(pb::TradeConfirmRequest {
            idempotency_key: key(),
        }),
        Intent::TradeCancel(pb::TradeCancelRequest {
            idempotency_key: key(),
        }),
        Intent::SetPrivateStoreSell(pb::SetPrivateStoreSellRequest {
            idempotency_key: key(),
            title: "shop".into(),
            package_sale: false,
            entries: vec![pb::StoreSellEntry {
                instance_id: 1,
                count: 1,
                price: 1,
            }],
        }),
        Intent::SetPrivateStoreBuy(pb::SetPrivateStoreBuyRequest {
            idempotency_key: key(),
            title: "shop".into(),
            entries: vec![pb::StoreBuyEntry {
                template_id: 57,
                enchant: 0,
                count: 1,
                price: 1,
            }],
        }),
        Intent::SetPrivateManufacture(pb::SetPrivateManufactureRequest {
            idempotency_key: key(),
            title: "shop".into(),
            entries: vec![pb::ManufactureEntry {
                recipe_id: 1,
                cost: 1,
            }],
        }),
        Intent::ClosePrivateStore(pb::ClosePrivateStoreRequest {
            idempotency_key: key(),
        }),
        Intent::PrivateStoreBuy(pb::PrivateStoreBuyRequest {
            idempotency_key: key(),
            store_owner: entity(),
            entries: vec![pb::StoreSellEntry {
                instance_id: 1,
                count: 1,
                price: 1,
            }],
        }),
        Intent::PrivateStoreSell(pb::PrivateStoreSellRequest {
            idempotency_key: key(),
            store_owner: entity(),
            entries: vec![pb::PrivateStoreSellEntry {
                instance_id: 1,
                template_id: 57,
                count: 1,
                price: 1,
            }],
        }),
        Intent::PrivateManufactureOrder(pb::PrivateManufactureOrderRequest {
            idempotency_key: key(),
            store_owner: entity(),
            recipe_id: 1,
        }),
        Intent::PartyInvite(pb::PartyInviteRequest {
            idempotency_key: key(),
            target: entity(),
            mode: pb::LootMode::FindersKeepers.into(),
        }),
        Intent::PartyAnswer(pb::PartyAnswerRequest {
            idempotency_key: key(),
            answer: 1,
        }),
        Intent::PartyLeave(pb::PartyLeaveRequest {
            idempotency_key: key(),
        }),
        Intent::PartyKick(pb::PartyKickRequest {
            idempotency_key: key(),
            member: entity(),
        }),
        Intent::PartyChangeLeader(pb::PartyChangeLeaderRequest {
            idempotency_key: key(),
            member: entity(),
        }),
        Intent::PartyLootChangeRequest(pb::PartyLootChangeRequest {
            idempotency_key: key(),
            mode: pb::LootMode::Random.into(),
        }),
        Intent::PartyLootChangeAnswer(pb::PartyLootChangeAnswer {
            idempotency_key: key(),
            accept: true,
        }),
        Intent::WarehouseDeposit(pb::WarehouseDepositRequest {
            idempotency_key: key(),
            keeper: entity(),
            entries: items(),
        }),
        Intent::WarehouseWithdraw(pb::WarehouseWithdrawRequest {
            idempotency_key: key(),
            keeper: entity(),
            entries: items(),
        }),
        Intent::FreightSend(pb::FreightSendRequest {
            idempotency_key: key(),
            keeper: entity(),
            target_character: entity(),
            entries: items(),
        }),
        Intent::FreightWithdraw(pb::FreightWithdrawRequest {
            idempotency_key: key(),
            keeper: entity(),
            entries: items(),
        }),
        Intent::MailSend(pb::MailSendRequest {
            idempotency_key: key(),
            receiver_name: "Receiver".into(),
            subject: "s".into(),
            body: "b".into(),
            cod_adena: 0,
            attachments: vec![],
        }),
        Intent::MailTakeAttachments(pb::MailTakeAttachmentsRequest {
            idempotency_key: key(),
            message_id: "1".into(),
        }),
        Intent::MailReject(pb::MailRejectRequest {
            idempotency_key: key(),
            message_id: "1".into(),
        }),
        Intent::MailCancel(pb::MailCancelRequest {
            idempotency_key: key(),
            message_id: "1".into(),
        }),
        Intent::MailDelete(pb::MailDeleteRequest {
            idempotency_key: key(),
            message_ids: vec!["1".into()],
            sent_box: false,
        }),
        Intent::ManorBuySeed(pb::ManorBuySeedRequest {
            idempotency_key: key(),
            manager: entity(),
            manor_id: 1,
            seeds: vec![pb::TemplateCount {
                template_id: 5016,
                count: 1,
            }],
        }),
        Intent::ManorSellCrop(pb::ManorSellCropRequest {
            idempotency_key: key(),
            manager: entity(),
            entries: vec![pb::CropSale {
                instance_id: 1,
                crop_id: 5073,
                manor_id: 1,
                count: 1,
            }],
        }),
        Intent::ItemAuctionBid(pb::ItemAuctionBidRequest {
            idempotency_key: key(),
            instance_npc_id: 1,
            bid: 1,
        }),
        Intent::ItemAuctionCancelBid(pb::ItemAuctionCancelBidRequest {
            idempotency_key: key(),
            auctioneer: entity(),
            instance_npc_id: 1,
        }),
        Intent::MailMarkRead(pb::MailMarkReadRequest {
            idempotency_key: key(),
            message_id: "1".into(),
        }),
    ]
}

#[tokio::test]
async fn economy_intents_reject_once_with_original_sequence_and_never_reach_actor() {
    let app = TestApp::spawn().await;
    let player = seed_player(&app, "Trader", 10.0, 10.0);
    let character = app.characters.get_for_test(player.character).unwrap();
    let mut socket = join(&app, &player).await;
    socket
        .until(|message| {
            matches!(
                &message.payload,
                Some(Payload::Event(pb::WorldEvent {
                    event: Some(Event::StatsChanged(_))
                }))
            )
        })
        .await;
    let before = app.zone.snapshot().await.unwrap();
    let before_events = app.characters.staged_events();
    let mut applied = app.zone.subscribe();
    let intents = economy_intents();
    assert_eq!(intents.len(), 41);
    for (offset, intent) in (100_u32..).zip(intents) {
        let seq = offset;
        socket
            .send(&pb::ClientMessage {
                seq,
                intent: Some(intent),
            })
            .await;
        let mut messages = socket
            .until(|message| rejected(message).is_some_and(|r| r.seq == seq))
            .await;
        messages.extend(socket.drain(Duration::from_millis(120)).await);
        assert_eq!(messages.len(), 1, "seq {seq}");
        let rejection = messages.first().and_then(rejected).unwrap();
        assert_eq!(rejection.seq, seq);
        assert_eq!(rejection.reason, i32::from(pb::RejectReason::NotYetImplemented));
        assert!(!rejection.detail.is_empty());
    }
    let after = app.zone.snapshot().await.unwrap();
    assert_eq!(after.entities, before.entities);
    assert_eq!(after.rng, before.rng);
    assert_eq!(app.characters.get_for_test(player.character).unwrap(), character);
    assert_eq!(app.characters.staged_events(), before_events);
    while let Ok(tick) = applied.try_recv() {
        assert!(tick.commands.is_empty());
        assert!(tick.events.is_empty());
    }
    assert!(app.bus.published().is_empty());
}
