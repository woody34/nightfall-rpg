//! Story 4.6: the nine Phase 5 read RPCs (design §13.4). All are reads (E14, E31): no method
//! writes a row; the read flag of a mail moves only through the `MailMarkRead` intent.
//!
//! Conventions (Phase 4): the caller must own `character_id` (16 UUID bytes) — malformed bytes
//! are `INVALID_ARGUMENT`, an unknown character `NOT_FOUND`, another account's character
//! `PERMISSION_DENIED`; a missing runtime port is `UNAVAILABLE`.

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::application::economy::EconomyError;
use crate::application::economy_reads::{
    AuctionRow, ContainerItem, EconomyReadRepository, MailRow,
};
use crate::application::{AppError, CharacterRepository};
use crate::domain::zone::{ItemInstance, Phase4Rules, Phase5Rules, RecipeBook};
use crate::domain::{AccountId, CharacterId};

/// What the zone runtime read port knows about an admitted character (design §13.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedView {
    /// The character stands inside the zone's peace square (Phase 3 `PeaceSquare`).
    pub in_peace_square: bool,
}

/// The vendor state the zone read port returns for `ListBuylist` at its current tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendorState {
    /// The pinned NPC id of the merchant entity.
    pub merchant_npc_id: u32,
    /// The zone tick of the read.
    pub state_tick: u64,
    /// `(list_id, template) → (stock, max)` for limited entries (read clamped at 0).
    pub stock: Vec<((u64, u32), (u64, u64))>,
    /// The caller's own refund tab, oldest first.
    pub refund_tab: Vec<ItemInstance>,
}

/// The zone runtime **read** port (design §13.4). `Err(Unavailable)` when no zone is running.
#[async_trait]
pub trait EconomyZoneReads: Send + Sync {
    /// `Some` when the character is admitted in a running zone.
    async fn admitted(&self, character: Uuid) -> Result<Option<AdmittedView>, AppError>;
    /// Vendor state for an admitted caller and a merchant entity; `None` when the entity is not
    /// a bound merchant of the zone.
    async fn vendor(
        &self,
        character: Uuid,
        merchant: Uuid,
    ) -> Result<Option<VendorState>, AppError>;
}

/// One buylist entry with its static castle-0 price (§6.2) and live stock.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct BuylistEntryView {
    pub template_id: u32,
    pub unit_price: u64,
    pub limited: bool,
    pub stock: u64,
    pub max_stock: u64,
}

/// `ListBuylist`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct BuylistView {
    pub list_id: u64,
    pub merchant_npc_id: u32,
    pub castle_tax_percent: u32,
    pub entries: Vec<BuylistEntryView>,
    pub refund_tab: Vec<ItemInstance>,
    pub state_tick: u64,
}

/// `GetWarehouse` / `GetFreight`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct ContainerView {
    pub items: Vec<ContainerItem>,
    pub capacity: u32,
    pub revision: u64,
}

/// `GetRecipeBook`: the checkpoint view (tick 0, incarnation 0 detached sentinel, D36 pattern).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct RecipeBookView {
    pub book: RecipeBook,
    pub dwarven_limit: u32,
    pub common_limit: u32,
}

/// `ReadMail`: attachments only when the zone confirms the reader in a peace square.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct ReadMailView {
    pub header: MailRow,
    pub body: String,
    pub attachments: Vec<ContainerItem>,
}

/// One auction as `ListItemAuctions` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct AuctionViewRow {
    pub auction_id: u64,
    pub state: u8,
    pub template_id: u32,
    pub count: u64,
    pub init_bid: u64,
    pub highest_bid: u64,
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
    pub own_bid: u64,
    pub own_cancelled: bool,
}

/// `ListItemAuctions`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[allow(missing_docs)]
pub struct AuctionsView {
    pub current: Option<AuctionViewRow>,
    pub next: Option<AuctionViewRow>,
    pub own_bids: Vec<AuctionViewRow>,
}

/// `GetManorData` (fixture lists with current amounts; Gludio only, E28).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[allow(missing_docs)]
pub struct ManorView {
    pub castle_id: u32,
    pub seeds: Vec<(u32, u64, u64, u64)>,
    pub crops: Vec<(u32, u64, u64, u64, u32)>,
}

/// One multisell ingredient or product: `(template, count, tax, maintain)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct MultisellItemView {
    pub template_id: u32,
    pub count: u64,
    pub tax: bool,
    pub maintain: bool,
}

/// One multisell entry; `entry_id` is the pinned 1-based file order (`MultisellChoose`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct MultisellEntryView {
    pub entry_id: u32,
    pub ingredients: Vec<MultisellItemView>,
    pub products: Vec<MultisellItemView>,
    pub stackable: bool,
}

/// `GetMultisellList`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct MultisellView {
    pub list_id: u32,
    pub maintain_enchantment: bool,
    pub apply_taxes: bool,
    pub allowed_npc_ids: Vec<u32>,
    pub entries: Vec<MultisellEntryView>,
}

/// The mail box limit (`inbox_size` / `outbox_size`, 240).
pub const MAIL_BOX_LIMIT: u32 = 240;

fn economy(error: EconomyError) -> AppError {
    match error {
        EconomyError::Busy(_) | EconomyError::Transient(_) => {
            AppError::Unavailable("economy store is busy".into())
        },
        other @ (EconomyError::LeaseViolation(_)
        | EconomyError::Constraint(_)
        | EconomyError::Other(_)) => AppError::Infrastructure(anyhow::Error::new(other)),
    }
}

/// The nine read RPCs over the economy read port, the character repository, the epoch's
/// `Phase5Rules` and the optional zone read port.
pub struct EconomyQueries {
    characters: Arc<dyn CharacterRepository>,
    items: Arc<Phase4Rules>,
    reads: Arc<dyn EconomyReadRepository>,
    rules: Arc<Phase5Rules>,
    zone: Option<Arc<dyn EconomyZoneReads>>,
}

impl EconomyQueries {
    /// The use cases over their ports; without a zone read port the zone-backed reads are
    /// `UNAVAILABLE`.
    #[must_use]
    pub fn new(
        characters: Arc<dyn CharacterRepository>,
        reads: Arc<dyn EconomyReadRepository>,
        items: Arc<Phase4Rules>,
        rules: Arc<Phase5Rules>,
        zone: Option<Arc<dyn EconomyZoneReads>>,
    ) -> Self {
        Self {
            characters,
            items,
            reads,
            rules,
            zone,
        }
    }

    /// Whether the character is admitted. Without a zone read port nobody is admitted.
    async fn admitted(&self, character: Uuid) -> Result<Option<AdmittedView>, AppError> {
        match &self.zone {
            Some(zone) => zone.admitted(character).await,
            None => Ok(None),
        }
    }

    /// Ownership before any economy state is read (Phase 4 order).
    async fn owned(&self, caller: AccountId, raw_id: &[u8]) -> Result<Uuid, AppError> {
        let uuid = Uuid::from_slice(raw_id).map_err(|_| {
            AppError::InvalidArgument("character_id must be exactly 16 UUID bytes".into())
        })?;
        let character = self
            .characters
            .get(CharacterId::from_uuid(uuid))
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "character",
                id: uuid.to_string(),
            })?;
        if character.account_id != caller {
            return Err(AppError::PermissionDenied("character belongs to another account".into()));
        }
        Ok(uuid)
    }

    fn zone(&self) -> Result<&Arc<dyn EconomyZoneReads>, AppError> {
        self.zone
            .as_ref()
            .ok_or_else(|| AppError::Unavailable("zone runtime is unavailable".into()))
    }

    /// `ListBuylist`: static §6.2 castle-0 prices; stock and refund tab from the zone read port.
    pub async fn list_buylist(
        &self,
        caller: AccountId,
        raw_id: &[u8],
        merchant: &str,
    ) -> Result<BuylistView, AppError> {
        let character = self.owned(caller, raw_id).await?;
        let merchant: Uuid = merchant
            .parse()
            .map_err(|_| AppError::InvalidArgument("merchant must be an entity UUID".into()))?;
        let zone = self.zone()?;
        if zone.admitted(character).await?.is_none() {
            return Err(AppError::FailedPrecondition(
                "ListBuylist requires an admitted character".into(),
            ));
        }
        let state = zone
            .vendor(character, merchant)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "merchant",
                id: merchant.to_string(),
            })?;
        let parts = self.rules.parts();
        let buylist = parts
            .merchants
            .buylists
            .iter()
            .find(|b| b.npc == state.merchant_npc_id)
            .ok_or_else(|| AppError::NotFound {
                entity: "buylist",
                id: state.merchant_npc_id.to_string(),
            })?;
        let mut entries = Vec::new();
        for entry in parts
            .merchants
            .entries
            .iter()
            .filter(|e| e.list_ids.contains(&buylist.list_id))
        {
            let unit_price = parts
                .merchants
                .unit_prices
                .iter()
                .find(|p| p.item == entry.item && p.tax == entry.tax)
                .map(|p| p.castle_0)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "buylist {} item {} has no castle-0 price",
                        buylist.list_id,
                        entry.item
                    )
                })?;
            let (stock, max_stock) = if entry.limited {
                state
                    .stock
                    .iter()
                    .find(|((list, template), _)| {
                        *list == buylist.list_id && *template == entry.item
                    })
                    .map_or((0, u64::try_from(entry.count).unwrap_or(0)), |(_, v)| *v)
            } else {
                (0, 0)
            };
            entries.push(BuylistEntryView {
                template_id: entry.item,
                unit_price,
                limited: entry.limited,
                stock,
                max_stock,
            });
        }
        Ok(BuylistView {
            list_id: buylist.list_id,
            merchant_npc_id: state.merchant_npc_id,
            castle_tax_percent: 0,
            entries,
            refund_tab: state.refund_tab,
            state_tick: state.state_tick,
        })
    }

    /// `GetMultisellList`: static, from `Phase5Rules`. `entry_id` is the pinned 1-based entry
    /// order. Entries with a special (negative id) ingredient or product (FAME, clan
    /// reputation; capability deferred in `catalogue.toml`) are omitted: the wire
    /// `template_id` is unsigned; the capability is deferred (E52, P5-D20). An entry is stackable when every product is
    /// (pinned `multisell/Entry.java:52-60`).
    pub fn get_multisell_list(&self, list_id: u32) -> Result<MultisellView, AppError> {
        let list = self
            .rules
            .parts()
            .multisell
            .list
            .iter()
            .find(|l| l.id == list_id)
            .ok_or_else(|| AppError::NotFound {
                entity: "multisell list",
                id: list_id.to_string(),
            })?;
        let items = &self.items.parts().items;
        let mut entries = Vec::new();
        for (index, entry) in list.entries.iter().enumerate() {
            let ingredients: Option<Vec<MultisellItemView>> = entry
                .ingredients
                .iter()
                .map(|i| {
                    u32::try_from(i.id)
                        .ok()
                        .map(|template_id| MultisellItemView {
                            template_id,
                            count: i.count,
                            tax: i.is_tax,
                            maintain: i.maintain,
                        })
                })
                .collect();
            let products: Option<Vec<MultisellItemView>> = entry
                .products
                .iter()
                .map(|p| {
                    u32::try_from(p.id)
                        .ok()
                        .map(|template_id| MultisellItemView {
                            template_id,
                            count: p.count,
                            tax: false,
                            maintain: false,
                        })
                })
                .collect();
            let (Some(ingredients), Some(products)) = (ingredients, products) else {
                continue;
            };
            let stackable = products
                .iter()
                .all(|p| items.get(&p.template_id).is_some_and(|d| d.stackable));
            let entry_id = u32::try_from(index)
                .ok()
                .and_then(|i| i.checked_add(1))
                .ok_or_else(|| anyhow::anyhow!("multisell entry index"))?;
            entries.push(MultisellEntryView {
                entry_id,
                ingredients,
                products,
                stackable,
            });
        }
        Ok(MultisellView {
            list_id: list.id,
            maintain_enchantment: list.maintain_enchantment,
            apply_taxes: list.apply_taxes,
            allowed_npc_ids: list.npcs.clone(),
            entries,
        })
    }

    /// `GetRecipeBook`: the checkpoint view out of world; `FAILED_PRECONDITION` while admitted
    /// (`RecipeBookSync` is the live baseline, D15 pattern).
    pub async fn get_recipe_book(
        &self,
        caller: AccountId,
        raw_id: &[u8],
    ) -> Result<RecipeBookView, AppError> {
        let character = self.owned(caller, raw_id).await?;
        if self.admitted(character).await?.is_some() {
            return Err(AppError::FailedPrecondition(
                "admitted characters receive RecipeBookSync over the world channel".into(),
            ));
        }
        let loaded = self
            .characters
            .load_for_admission(CharacterId::from_uuid(character))
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "character",
                id: character.to_string(),
            })?;
        let config = &self.rules.parts().recipes.config;
        Ok(RecipeBookView {
            book: loaded.phase5.map(|cp| cp.recipes).unwrap_or_default(),
            dwarven_limit: config.dwarf_recipe_limit,
            common_limit: config.common_recipe_limit,
        })
    }

    /// `GetWarehouse` (`freight = false`) / `GetFreight`: DB rows, owner only; capacity is the
    /// base (R40); `revision` is the highest row revision (0 when empty).
    pub async fn get_container(
        &self,
        caller: AccountId,
        raw_id: &[u8],
        freight: bool,
    ) -> Result<ContainerView, AppError> {
        let character = self.owned(caller, raw_id).await?;
        let kind = if freight { "freight" } else { "warehouse" };
        let items = self
            .reads
            .container_items(kind, character)
            .await
            .map_err(economy)?;
        let constants = &self.rules.parts().storage_mail;
        let capacity = if freight {
            constants.freight.slots
        } else {
            let dwarf = self
                .characters
                .get(CharacterId::from_uuid(character))
                .await?
                .is_some_and(|c| c.identity().race == crate::domain::Race::Dwarf);
            if dwarf {
                constants.warehouse.slots_dwarf
            } else {
                constants.warehouse.slots_no_dwarf
            }
        };
        let revision = items.iter().map(|i| i.revision).max().unwrap_or(0);
        Ok(ContainerView {
            items,
            capacity,
            revision,
        })
    }

    /// `ListMail`: DB rows of one box; expired and side-deleted messages hidden.
    pub async fn list_mail(
        &self,
        caller: AccountId,
        raw_id: &[u8],
        outbox: Option<bool>,
    ) -> Result<(Vec<MailRow>, u32, u32), AppError> {
        let character = self.owned(caller, raw_id).await?;
        let outbox = outbox
            .ok_or_else(|| AppError::InvalidArgument("mail_box must be INBOX or OUTBOX".into()))?;
        let rows = self
            .reads
            .mail_list(character, outbox)
            .await
            .map_err(economy)?;
        let count =
            u32::try_from(rows.len()).map_err(|_| anyhow::anyhow!("mail count overflow"))?;
        Ok((rows, count, MAIL_BOX_LIMIT))
    }

    /// `ReadMail`: content only, **writes nothing** (E31). A message with attachments needs the
    /// zone read port to confirm the reader admitted inside a peace square; admitted elsewhere
    /// it is `FAILED_PRECONDITION`; a reader out of world gets the header and body only.
    pub async fn read_mail(
        &self,
        caller: AccountId,
        raw_id: &[u8],
        message: &str,
    ) -> Result<ReadMailView, AppError> {
        let character = self.owned(caller, raw_id).await?;
        let message: Uuid = message
            .parse()
            .map_err(|_| AppError::InvalidArgument("message_id must be a UUID".into()))?;
        let row = self
            .reads
            .mail(character, message)
            .await
            .map_err(economy)?
            .ok_or_else(|| AppError::NotFound {
                entity: "mail",
                id: message.to_string(),
            })?;
        let mut attachments = Vec::new();
        if row.has_attachments {
            match self.admitted(character).await? {
                Some(view) if view.in_peace_square => {
                    attachments = self
                        .reads
                        .mail_attachments(message)
                        .await
                        .map_err(economy)?;
                },
                Some(_) => {
                    return Err(AppError::FailedPrecondition(
                        "a mail with attachments opens only in a peace zone".into(),
                    ));
                },
                None => {},
            }
        }
        Ok(ReadMailView {
            body: row.body.clone(),
            header: row,
            attachments,
        })
    }

    /// `ListItemAuctions`: DB rows; `own_bids` are the caller's bids that
    /// `ItemAuctionCancelBid` can reclaim: not on a CREATED auction, not cancelled, and not the
    /// highest bid (pinned `ItemAuction.java:359-394`).
    pub async fn list_item_auctions(
        &self,
        caller: AccountId,
        raw_id: &[u8],
        instance_npc: u32,
    ) -> Result<AuctionsView, AppError> {
        let character = self.owned(caller, raw_id).await?;
        let parts = self.rules.parts();
        let instance = parts
            .auction
            .instance
            .iter()
            .find(|i| i.id == instance_npc)
            .ok_or_else(|| AppError::NotFound {
                entity: "auction instance",
                id: instance_npc.to_string(),
            })?;
        let rows = self.reads.auctions(instance_npc).await.map_err(economy)?;
        let mut views = Vec::new();
        for row in &rows {
            let bids = self.reads.bids(row.id).await.map_err(economy)?;
            let own = bids.iter().find(|b| b.character_id == character);
            views.push((row.clone(), Self::view(instance, row, own.map(|b| b.last_bid))?));
        }
        let current = views
            .iter()
            .rev()
            .find(|(r, _)| r.state >= 1)
            .map(|(_, v)| v.clone());
        let next = views
            .iter()
            .find(|(r, _)| r.state == 0)
            .map(|(_, v)| v.clone());
        let own_bids = views
            .iter()
            .filter(|(r, v)| {
                r.state != 0
                    && v.own_bid > 0
                    && !v.own_cancelled
                    && r.highest_bidder != Some(character)
            })
            .map(|(_, v)| v.clone())
            .collect();
        Ok(AuctionsView {
            current,
            next,
            own_bids,
        })
    }

    fn view(
        instance: &crate::domain::zone::phase5_rules::AuctionInstance,
        row: &AuctionRow,
        own: Option<i64>,
    ) -> Result<AuctionViewRow, AppError> {
        let item = instance
            .items
            .get(usize::try_from(row.item_index).map_err(|_| anyhow::anyhow!("item index"))?)
            .ok_or_else(|| {
                anyhow::anyhow!("auction {} item index {} out of range", row.id, row.item_index)
            })?;
        Ok(AuctionViewRow {
            auction_id: row.id,
            state: row.state,
            template_id: item.item_id,
            count: item.count,
            init_bid: item.init_bid,
            highest_bid: row.highest_bid.unwrap_or(0),
            start_unix_ms: row.start_unix_ms,
            end_unix_ms: row.end_unix_ms,
            own_bid: own.and_then(|b| u64::try_from(b).ok()).unwrap_or(0),
            own_cancelled: own == Some(-1),
        })
    }

    /// `GetManorData`: the fixture lists with their current DB amounts; Gludio only (E28).
    pub async fn get_manor_data(&self, castle_id: u32) -> Result<ManorView, AppError> {
        let fixture = &self.rules.parts().manor.fixture_gludio;
        if castle_id != fixture.castle_id {
            return Ok(ManorView {
                castle_id,
                ..ManorView::default()
            });
        }
        let rows = self.reads.manor(castle_id).await.map_err(economy)?;
        let seeds = fixture
            .seed_production
            .iter()
            .map(|s| {
                let current = rows.seeds.iter().find(|r| r.0 == s.seed_id);
                (s.seed_id, s.price, s.amount, current.map_or(s.amount, |r| r.3))
            })
            .collect();
        let crops = fixture
            .crop_procurement
            .iter()
            .map(|c| {
                let current = rows.crops.iter().find(|r| r.0 == c.crop_id);
                (c.crop_id, c.price, c.amount, current.map_or(c.amount, |r| r.3), c.reward_type)
            })
            .collect();
        Ok(ManorView {
            castle_id,
            seeds,
            crops,
        })
    }
}
