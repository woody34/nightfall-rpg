//! Postgres read side of the economy (stories 4.5 / 4.6): `SELECT` only (E31). DB time is
//! Postgres `now()`. Mail and delivery timestamps leave this module only as epoch milliseconds
//! for the read RPCs and never inside a pre-image (E12). The auction window is the exception:
//! the zone writer itself stores it from integer epoch milliseconds (exact, no float), so its
//! pre-image carries the same integers back.

use std::collections::BTreeSet;

use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DbBackend, QueryResult, Statement};
use uuid::Uuid;

use super::economy::{classify, PgEconomyRepository};
use crate::application::economy::{
    auction_bid_owner, DeliveryKind, DeliveryRow, EconomyError, FrozenRow, LeaseKey, LeaseKind,
};
use crate::application::economy_reads::{
    AuctionRow, BidRow, ContainerItem, DeliveryState, EconomyReadRepository, MailRow, ManorRows,
};

const ITEM_COLUMNS: &str = "id,template_id,count,enchant,augment_id,charge_bits,revision,location,owner_kind,character_id,container_id,mail_id,delivery_id,created_tick";
const MAIL_SELECT: &str = "SELECT m.id,m.sender_id,m.receiver_id,s.name AS sender_name,r.name AS receiver_name,m.subject,m.body,m.cod_adena,m.has_attachments,m.returned,m.system_kind,m.read,m.deleted_by_sender,m.deleted_by_receiver,(extract(epoch FROM m.created_at)*1000)::bigint AS created_ms,(extract(epoch FROM m.expires_at)*1000)::bigint AS expires_ms,m.revision FROM mail_messages m JOIN characters s ON s.id=m.sender_id JOIN characters r ON r.id=m.receiver_id";

fn other(error: sea_orm::DbErr) -> EconomyError {
    EconomyError::Other(error.into())
}
fn get<T: sea_orm::TryGetable>(row: &QueryResult, column: &str) -> Result<T, EconomyError> {
    row.try_get("", column).map_err(other)
}
fn unsigned<T: TryFrom<i64>>(value: i64) -> Result<T, EconomyError> {
    T::try_from(value).map_err(|_| EconomyError::Constraint("economy_read_out_of_range".into()))
}
fn signed(value: u64) -> Result<i64, EconomyError> {
    i64::try_from(value).map_err(|_| EconomyError::Constraint("economy_read_out_of_range".into()))
}

fn item(row: &QueryResult) -> Result<ContainerItem, EconomyError> {
    Ok(ContainerItem {
        instance_id: unsigned(get::<i64>(row, "id")?)?,
        template_id: unsigned(i64::from(get::<i32>(row, "template_id")?))?,
        count: unsigned(get::<i64>(row, "count")?)?,
        enchant: unsigned(get::<i64>(row, "enchant")?)?,
        augment_id: get::<Option<i64>>(row, "augment_id")?
            .map(unsigned)
            .transpose()?,
        charge_bits: unsigned(i64::from(get::<i16>(row, "charge_bits")?))?,
        revision: unsigned(get::<i64>(row, "revision")?)?,
    })
}

/// The integer/string columns of an item row, as the writer's `ContainerDelta` names them.
fn item_columns(
    row: &QueryResult,
) -> Result<serde_json::Map<String, serde_json::Value>, EconomyError> {
    let mut c = serde_json::Map::new();
    c.insert("template_id".into(), get::<i32>(row, "template_id")?.into());
    c.insert("count".into(), get::<i64>(row, "count")?.into());
    c.insert("enchant".into(), get::<i64>(row, "enchant")?.into());
    if let Some(augment) = get::<Option<i64>>(row, "augment_id")? {
        c.insert("augment_id".into(), augment.into());
    }
    c.insert("charge_bits".into(), get::<i16>(row, "charge_bits")?.into());
    c.insert("location".into(), get::<String>(row, "location")?.into());
    c.insert("owner_kind".into(), get::<String>(row, "owner_kind")?.into());
    c.insert("created_tick".into(), get::<i64>(row, "created_tick")?.into());
    for name in ["character_id", "container_id", "mail_id", "delivery_id"] {
        if let Some(id) = get::<Option<Uuid>>(row, name)? {
            c.insert(name.into(), id.to_string().into());
        }
    }
    Ok(c)
}

fn mail(row: &QueryResult) -> Result<MailRow, EconomyError> {
    Ok(MailRow {
        id: get(row, "id")?,
        sender_id: get(row, "sender_id")?,
        receiver_id: get(row, "receiver_id")?,
        sender_name: get(row, "sender_name")?,
        receiver_name: get(row, "receiver_name")?,
        subject: get(row, "subject")?,
        body: get(row, "body")?,
        cod_adena: unsigned(get::<i64>(row, "cod_adena")?)?,
        has_attachments: get(row, "has_attachments")?,
        returned: get(row, "returned")?,
        system_kind: unsigned(i64::from(get::<i16>(row, "system_kind")?))?,
        read: get(row, "read")?,
        deleted_by_sender: get(row, "deleted_by_sender")?,
        deleted_by_receiver: get(row, "deleted_by_receiver")?,
        created_unix_ms: get(row, "created_ms")?,
        expires_unix_ms: get(row, "expires_ms")?,
        revision: unsigned(get::<i64>(row, "revision")?)?,
    })
}

fn frozen(
    lease: LeaseKey,
    kind: &str,
    key: String,
    revision: i64,
    columns: serde_json::Map<String, serde_json::Value>,
) -> Result<FrozenRow, EconomyError> {
    Ok(FrozenRow {
        lease,
        row_kind: kind.into(),
        row_key: key,
        revision: unsigned(revision)?,
        columns,
    })
}

impl PgEconomyRepository {
    async fn rows(
        &self,
        sql: &str,
        values: Vec<sea_orm::Value>,
    ) -> Result<Vec<QueryResult>, EconomyError> {
        self.db
            .query_all_raw(Statement::from_sql_and_values(DbBackend::Postgres, sql, values))
            .await
            .map_err(classify)
    }

    async fn items(
        &self,
        filter: &str,
        value: sea_orm::Value,
    ) -> Result<Vec<QueryResult>, EconomyError> {
        self.rows(
            &format!("SELECT {ITEM_COLUMNS} FROM item_instances WHERE {filter} ORDER BY id"),
            vec![value],
        )
        .await
    }

    /// The manor pre-image: seeds then crops of the castle, each by id.
    async fn manor_pre_image(
        &self,
        key: LeaseKey,
        out: &mut Vec<FrozenRow>,
    ) -> Result<(), EconomyError> {
        let castle =
            i64::try_from(key.owner).map_err(|_| EconomyError::Constraint("castle id".into()))?;
        for row in self.rows("SELECT castle_id,seed_id,price,start_amount,amount,revision FROM manor_seed_production WHERE castle_id=$1 ORDER BY seed_id", vec![castle.into()]).await? {
            let mut c = serde_json::Map::new();
            c.insert("castle_id".into(), i64::from(get::<i16>(&row, "castle_id")?).into());
            c.insert("seed_id".into(), get::<i32>(&row, "seed_id")?.into());
            for name in ["price", "start_amount", "amount"] {
                c.insert(name.into(), get::<i64>(&row, name)?.into());
            }
            out.push(frozen(key, "manor_seed_production", format!("{castle}:{}", get::<i32>(&row, "seed_id")?), get(&row, "revision")?, c)?);
        }
        for row in self.rows("SELECT castle_id,crop_id,price,start_amount,amount,reward_type,revision FROM manor_crop_procure WHERE castle_id=$1 ORDER BY crop_id", vec![castle.into()]).await? {
            let mut c = serde_json::Map::new();
            c.insert("castle_id".into(), i64::from(get::<i16>(&row, "castle_id")?).into());
            c.insert("crop_id".into(), get::<i32>(&row, "crop_id")?.into());
            for name in ["price", "start_amount", "amount"] {
                c.insert(name.into(), get::<i64>(&row, name)?.into());
            }
            c.insert("reward_type".into(), get::<i16>(&row, "reward_type")?.into());
            out.push(frozen(key, "manor_crop_procure", format!("{castle}:{}", get::<i32>(&row, "crop_id")?), get(&row, "revision")?, c)?);
        }
        Ok(())
    }
}

#[async_trait]
impl EconomyReadRepository for PgEconomyRepository {
    async fn db_now_ms(&self) -> Result<i64, EconomyError> {
        let rows = self
            .rows("SELECT (extract(epoch FROM now())*1000)::bigint AS ms", vec![])
            .await?;
        rows.first().map_or_else(
            || Err(EconomyError::Constraint("no clock row".into())),
            |row| get(row, "ms"),
        )
    }

    async fn container_items(
        &self,
        owner_kind: &str,
        character: Uuid,
    ) -> Result<Vec<ContainerItem>, EconomyError> {
        self.rows(
            &format!("SELECT {ITEM_COLUMNS} FROM item_instances WHERE owner_kind=$1 AND character_id=$2 ORDER BY id"),
            vec![owner_kind.into(), character.into()],
        )
        .await?
        .iter()
        .map(item)
        .collect()
    }

    async fn mail_list(&self, character: Uuid, outbox: bool) -> Result<Vec<MailRow>, EconomyError> {
        let side = if outbox {
            "m.sender_id=$1 AND NOT m.deleted_by_sender"
        } else {
            "m.receiver_id=$1 AND NOT m.deleted_by_receiver"
        };
        self.rows(&format!("{MAIL_SELECT} WHERE {side} AND m.expires_at > now() ORDER BY m.created_at DESC, m.id"), vec![character.into()])
            .await?
            .iter()
            .map(mail)
            .collect()
    }

    async fn mail(&self, character: Uuid, message: Uuid) -> Result<Option<MailRow>, EconomyError> {
        let rows = self
            .rows(
                &format!("{MAIL_SELECT} WHERE m.id=$2 AND m.expires_at > now() AND ((m.receiver_id=$1 AND NOT m.deleted_by_receiver) OR (m.sender_id=$1 AND NOT m.deleted_by_sender))"),
                vec![character.into(), message.into()],
            )
            .await?;
        rows.first().map(mail).transpose()
    }

    async fn mail_attachments(&self, message: Uuid) -> Result<Vec<ContainerItem>, EconomyError> {
        self.items("mail_id=$1", message.into())
            .await?
            .iter()
            .map(item)
            .collect()
    }

    async fn expired_mail(&self, limit: u32) -> Result<Vec<MailRow>, EconomyError> {
        self.rows(
            &format!(
                "{MAIL_SELECT} WHERE m.expires_at <= now() ORDER BY m.expires_at, m.id LIMIT $1"
            ),
            vec![i64::from(limit).into()],
        )
        .await?
        .iter()
        .map(mail)
        .collect()
    }

    async fn mail_exists(&self, message: Uuid) -> Result<bool, EconomyError> {
        Ok(!self
            .rows("SELECT 1 FROM mail_messages WHERE id=$1", vec![message.into()])
            .await?
            .is_empty())
    }

    async fn auctions(&self, instance_npc: u32) -> Result<Vec<AuctionRow>, EconomyError> {
        self.rows(
            "SELECT id,instance_npc,item_index,state,highest_bidder,highest_bid,(extract(epoch FROM start_at)*1000)::bigint AS start_ms,(extract(epoch FROM end_at)*1000)::bigint AS end_ms,revision FROM item_auctions WHERE instance_npc=$1 ORDER BY start_at,id",
            vec![i64::from(instance_npc).into()],
        )
        .await?
        .iter()
        .map(|row| {
            Ok(AuctionRow {
                id: unsigned(get::<i64>(row, "id")?)?,
                instance_npc,
                item_index: unsigned(i64::from(get::<i32>(row, "item_index")?))?,
                state: unsigned(i64::from(get::<i16>(row, "state")?))?,
                highest_bidder: get(row, "highest_bidder")?,
                highest_bid: get::<Option<i64>>(row, "highest_bid")?.map(unsigned).transpose()?,
                start_unix_ms: get(row, "start_ms")?,
                end_unix_ms: get(row, "end_ms")?,
                revision: unsigned(get::<i64>(row, "revision")?)?,
            })
        })
        .collect()
    }

    async fn bids(&self, auction: u64) -> Result<Vec<BidRow>, EconomyError> {
        self.rows("SELECT auction_id,character_id,last_bid FROM item_auction_bids WHERE auction_id=$1 ORDER BY character_id", vec![signed(auction)?.into()])
            .await?
            .iter()
            .map(|row| Ok(BidRow { auction_id: auction, character_id: get(row, "character_id")?, last_bid: get(row, "last_bid")? }))
            .collect()
    }

    async fn manor(&self, castle: u32) -> Result<ManorRows, EconomyError> {
        let castle = i64::from(castle);
        let seeds = self.rows("SELECT seed_id,price,start_amount,amount FROM manor_seed_production WHERE castle_id=$1 ORDER BY seed_id", vec![castle.into()]).await?;
        let crops = self.rows("SELECT crop_id,price,start_amount,amount,reward_type FROM manor_crop_procure WHERE castle_id=$1 ORDER BY crop_id", vec![castle.into()]).await?;
        Ok(ManorRows {
            seeds: seeds
                .iter()
                .map(|r| {
                    Ok((
                        unsigned(i64::from(get::<i32>(r, "seed_id")?))?,
                        unsigned(get::<i64>(r, "price")?)?,
                        unsigned(get::<i64>(r, "start_amount")?)?,
                        unsigned(get::<i64>(r, "amount")?)?,
                    ))
                })
                .collect::<Result<_, EconomyError>>()?,
            crops: crops
                .iter()
                .map(|r| {
                    Ok((
                        unsigned(i64::from(get::<i32>(r, "crop_id")?))?,
                        unsigned(get::<i64>(r, "price")?)?,
                        unsigned(get::<i64>(r, "start_amount")?)?,
                        unsigned(get::<i64>(r, "amount")?)?,
                        unsigned(i64::from(get::<i16>(r, "reward_type")?))?,
                    ))
                })
                .collect::<Result<_, EconomyError>>()?,
        })
    }

    async fn delivery(&self, id: Uuid) -> Result<Option<DeliveryState>, EconomyError> {
        let rows = self.rows("SELECT id,target_character,kind,payload,status FROM economy_deliveries WHERE id=$1", vec![id.into()]).await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let kind: String = get(row, "kind")?;
        let status: String = get(row, "status")?;
        Ok(Some(DeliveryState {
            row: DeliveryRow {
                id: get(row, "id")?,
                target_character: get(row, "target_character")?,
                kind: match kind.as_str() {
                    "cod_payment" => DeliveryKind::CodPayment,
                    "mail_expiry_return" => DeliveryKind::MailExpiryReturn,
                    "auction_award" => DeliveryKind::AuctionAward,
                    _ => return Err(EconomyError::Constraint("economy_deliveries_kind".into())),
                },
                payload: get(row, "payload")?,
            },
            pending: status == "pending",
        }))
    }

    async fn delivery_targets(&self) -> Result<Vec<Uuid>, EconomyError> {
        self.rows("SELECT DISTINCT target_character FROM economy_deliveries WHERE status='pending' ORDER BY target_character", vec![])
            .await?
            .iter()
            .map(|row| get(row, "target_character"))
            .collect()
    }

    async fn mailbox_counts(&self, character: Uuid) -> Result<(u32, u32), EconomyError> {
        let rows = self
            .rows(
                "SELECT count(*) FILTER (WHERE receiver_id=$1 AND NOT deleted_by_receiver) AS inbox, count(*) FILTER (WHERE sender_id=$1 AND NOT deleted_by_sender) AS outbox FROM mail_messages",
                vec![character.into()],
            )
            .await?;
        let row = rows
            .first()
            .ok_or_else(|| EconomyError::Constraint("no count row".into()))?;
        Ok((unsigned(get::<i64>(row, "inbox")?)?, unsigned(get::<i64>(row, "outbox")?)?))
    }

    async fn open_ground_refund(
        &self,
        zone: u32,
        epoch: u64,
    ) -> Result<Vec<crate::application::economy_reads::OpenChain>, EconomyError> {
        let zone = i32::try_from(zone)
            .map_err(|_| EconomyError::Constraint("economy_read_out_of_range".into()))?;
        let rows = self
            .rows(
                "SELECT epoch,owner_kind,owner_id,instance_id,delta->'after' AS after FROM (SELECT DISTINCT ON (owner_kind,owner_id,instance_id) epoch,owner_kind,owner_id,instance_id,delta FROM item_ledger WHERE zone_id=$1 AND owner_kind IN ('ground','refund') ORDER BY owner_kind,owner_id,instance_id,id DESC) last WHERE epoch<$2 AND jsonb_typeof(delta->'after')='object' ORDER BY epoch,owner_kind,owner_id,instance_id",
                vec![zone.into(), signed(epoch)?.into()],
            )
            .await?;
        rows.iter()
            .map(|row| {
                Ok(crate::application::economy_reads::OpenChain {
                    epoch: unsigned(get::<i64>(row, "epoch")?)?,
                    owner_kind: get(row, "owner_kind")?,
                    owner: get(row, "owner_id")?,
                    instance: unsigned(get::<i64>(row, "instance_id")?)?,
                    after: get(row, "after")?,
                })
            })
            .collect()
    }

    async fn lease_epochs(&self, zone: u32, epoch: u64) -> Result<Vec<u64>, EconomyError> {
        let zone = i32::try_from(zone)
            .map_err(|_| EconomyError::Constraint("economy_read_out_of_range".into()))?;
        self.rows(
            "SELECT DISTINCT epoch FROM economy_leases WHERE zone_id=$1 AND epoch<$2 ORDER BY epoch",
            vec![zone.into(), signed(epoch)?.into()],
        )
        .await?
        .iter()
        .map(|row| unsigned(get::<i64>(row, "epoch")?))
        .collect()
    }

    async fn pre_image(&self, keys: &[LeaseKey]) -> Result<Vec<FrozenRow>, EconomyError> {
        let mut out = Vec::new();
        for key in keys {
            let uuid = Uuid::from_u128(key.owner);
            let items =
                |rows: Vec<QueryResult>, out: &mut Vec<FrozenRow>| -> Result<(), EconomyError> {
                    for row in &rows {
                        out.push(frozen(
                            *key,
                            "item_instance",
                            get::<i64>(row, "id")?.to_string(),
                            get(row, "revision")?,
                            item_columns(row)?,
                        )?);
                    }
                    Ok(())
                };
            match key.kind {
                LeaseKind::Warehouse | LeaseKind::Freight => {
                    let kind = if key.kind == LeaseKind::Warehouse {
                        "warehouse"
                    } else {
                        "freight"
                    };
                    let rows = self
                        .rows(&format!("SELECT {ITEM_COLUMNS} FROM item_instances WHERE owner_kind=$1 AND character_id=$2 ORDER BY id"), vec![kind.into(), uuid.into()])
                        .await?;
                    items(rows, &mut out)?;
                },
                LeaseKind::Mail => {
                    for row in self.rows("SELECT id,sender_id,receiver_id,subject,body,cod_adena,has_attachments,returned,system_kind,read,deleted_by_sender,deleted_by_receiver,revision FROM mail_messages WHERE id=$1", vec![uuid.into()]).await? {
                        let mut c = serde_json::Map::new();
                        for name in ["sender_id", "receiver_id"] {
                            c.insert(name.into(), get::<Uuid>(&row, name)?.to_string().into());
                        }
                        for name in ["subject", "body"] {
                            c.insert(name.into(), get::<String>(&row, name)?.into());
                        }
                        c.insert("cod_adena".into(), get::<i64>(&row, "cod_adena")?.into());
                        c.insert("system_kind".into(), get::<i16>(&row, "system_kind")?.into());
                        for name in ["has_attachments", "returned", "read", "deleted_by_sender", "deleted_by_receiver"] {
                            c.insert(name.into(), get::<bool>(&row, name)?.into());
                        }
                        out.push(frozen(*key, "mail_message", uuid.to_string(), get(&row, "revision")?, c)?);
                    }
                    items(self.items("mail_id=$1", uuid.into()).await?, &mut out)?;
                },
                LeaseKind::Delivery => {
                    for row in self.rows("SELECT target_character,kind,payload,status,revision FROM economy_deliveries WHERE id=$1", vec![uuid.into()]).await? {
                        let mut c = serde_json::Map::new();
                        c.insert("target_character".into(), get::<Uuid>(&row, "target_character")?.to_string().into());
                        c.insert("kind".into(), get::<String>(&row, "kind")?.into());
                        c.insert("payload".into(), get::<serde_json::Value>(&row, "payload")?);
                        c.insert("status".into(), get::<String>(&row, "status")?.into());
                        out.push(frozen(*key, "economy_delivery", uuid.to_string(), get(&row, "revision")?, c)?);
                    }
                    items(self.items("delivery_id=$1", uuid.into()).await?, &mut out)?;
                },
                LeaseKind::Auction => {
                    let auction = i64::try_from(key.owner)
                        .map_err(|_| EconomyError::Constraint("auction id".into()))?;
                    for row in self.rows("SELECT id,instance_npc,item_index,state,extend_state,last_bidder,highest_bidder,highest_bid,(extract(epoch FROM start_at)*1000)::bigint AS start_ms,(extract(epoch FROM end_at)*1000)::bigint AS end_ms,revision FROM item_auctions WHERE id=$1", vec![auction.into()]).await? {
                        let mut c = serde_json::Map::new();
                        c.insert("instance_npc".into(), get::<i32>(&row, "instance_npc")?.into());
                        c.insert("item_index".into(), get::<i32>(&row, "item_index")?.into());
                        c.insert("state".into(), get::<i16>(&row, "state")?.into());
                        c.insert("extend_state".into(), get::<i16>(&row, "extend_state")?.into());
                        for name in ["last_bidder", "highest_bidder"] {
                            if let Some(id) = get::<Option<Uuid>>(&row, name)? {
                                c.insert(name.into(), id.to_string().into());
                            }
                        }
                        if let Some(bid) = get::<Option<i64>>(&row, "highest_bid")? {
                            c.insert("highest_bid".into(), bid.into());
                        }
                        c.insert("start_unix_ms".into(), get::<i64>(&row, "start_ms")?.into());
                        c.insert("end_unix_ms".into(), get::<i64>(&row, "end_ms")?.into());
                        out.push(frozen(*key, "item_auction", auction.to_string(), get(&row, "revision")?, c)?);
                    }
                    let mut owners = BTreeSet::new();
                    for row in self.rows("SELECT character_id,last_bid,revision FROM item_auction_bids WHERE auction_id=$1 ORDER BY character_id", vec![auction.into()]).await? {
                        let character: Uuid = get(&row, "character_id")?;
                        let mut c = serde_json::Map::new();
                        c.insert("auction_id".into(), auction.into());
                        c.insert("character_id".into(), character.to_string().into());
                        c.insert("last_bid".into(), get::<i64>(&row, "last_bid")?.into());
                        out.push(frozen(*key, "item_auction_bid", format!("{auction}:{character}"), get(&row, "revision")?, c)?);
                        owners.insert(auction_bid_owner(unsigned(auction)?, character));
                    }
                    for owner in owners {
                        items(
                            self.items(
                                "owner_kind='auction_bid' AND container_id=$1",
                                owner.into(),
                            )
                            .await?,
                            &mut out,
                        )?;
                    }
                },
                LeaseKind::Manor => self.manor_pre_image(*key, &mut out).await?,
                LeaseKind::Mailbox => {},
            }
        }
        Ok(out)
    }
}
