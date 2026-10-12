//! Strict retained item projections, always read under the parent character lock.
use super::{entities::characters, PgCharacterRepository};
use crate::application::ports::{ItemInventoryRepository, RetainedInventory};
use crate::domain::{
    zone::{EquipSlot, ItemInstance, ItemInstanceId, ItemLocation, Phase4Checkpoint, Tick},
    CharacterId,
};
use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};

#[async_trait::async_trait]
impl ItemInventoryRepository for PgCharacterRepository {
    async fn retained_inventory(
        &self,
        character: CharacterId,
    ) -> anyhow::Result<Option<RetainedInventory>> {
        use sea_orm::{EntityTrait, QuerySelect};
        let tx = self.db.begin().await?;
        let parent = characters::Entity::find_by_id(character.as_uuid())
            .lock_shared()
            .one(&tx)
            .await?;
        let result = if let Some(parent) = parent {
            let phase3 = super::phase3_checkpoint::load(&tx, &parent).await?;
            let checkpoint = load(&tx, &parent, phase3.as_ref()).await?;
            Some(RetainedInventory {
                character_id: character,
                character_revision: u64::try_from(parent.revision)?,
                checkpoint,
            })
        } else {
            None
        };
        tx.commit().await?;
        Ok(result)
    }
}

pub(super) async fn load<C: ConnectionTrait>(
    db: &C,
    parent: &characters::Model,
    phase3: Option<&crate::domain::zone::Phase3Checkpoint>,
) -> anyhow::Result<Option<Phase4Checkpoint>> {
    let rows=db.query_all_raw(Statement::from_sql_and_values(DbBackend::Postgres,
        "SELECT id,template_id,count,location,slot,enchant,augment_id,charge_bits,bound,created_tick,revision FROM item_instances WHERE character_id=$1 ORDER BY id",
        [parent.id.into()])).await?;
    let Some(value) = &parent.phase4_checkpoint else {
        anyhow::ensure!(rows.is_empty(), "item rows exist without a Phase4 checkpoint");
        return Ok(None);
    };
    let phase3 =
        phase3.ok_or_else(|| anyhow::anyhow!("Phase4 checkpoint requires Phase3 state"))?;
    let checkpoint: Phase4Checkpoint = serde_json::from_value(value.clone())?;
    checkpoint.validate_structure(phase3.modifier_install_order.len())?;
    let instances = rows
        .iter()
        .map(decode_item)
        .collect::<anyhow::Result<Vec<_>>>()?;
    anyhow::ensure!(
        instances == checkpoint.instances,
        "retained item rows disagree with Phase4 checkpoint"
    );
    Ok(Some(checkpoint))
}

fn decode_item(row: &sea_orm::QueryResult) -> anyhow::Result<ItemInstance> {
    anyhow::ensure!(!row.try_get::<bool>("", "bound")?, "bound item is not admitted");
    let location = match row.try_get::<String>("", "location")?.as_str() {
        "inventory" => ItemLocation::Inventory,
        "paperdoll" => ItemLocation::Paperdoll,
        _ => anyhow::bail!("unknown item location"),
    };
    let item = ItemInstance {
        creation_origin: None,
        id: ItemInstanceId::new(u64::try_from(row.try_get::<i64>("", "id")?)?)?,
        template_id: u32::try_from(row.try_get::<i32>("", "template_id")?)?,
        count: u64::try_from(row.try_get::<i64>("", "count")?)?,
        location,
        slot: row
            .try_get::<Option<i16>>("", "slot")?
            .map(slot_from_db)
            .transpose()?,
        enchant: u32::try_from(row.try_get::<i64>("", "enchant")?)?,
        augment_id: row
            .try_get::<Option<i64>>("", "augment_id")?
            .map(u32::try_from)
            .transpose()?,
        charges: u8::try_from(row.try_get::<i16>("", "charge_bits")?)?,
        created_tick: Tick(u64::try_from(row.try_get::<i64>("", "created_tick")?)?),
        revision: u64::try_from(row.try_get::<i64>("", "revision")?)?,
    };
    item.validate()?;
    Ok(item)
}

fn slot_from_db(value: i16) -> anyhow::Result<EquipSlot> {
    let slots = [
        EquipSlot::Under,
        EquipSlot::Head,
        EquipSlot::Hair,
        EquipSlot::Hair2,
        EquipSlot::Neck,
        EquipSlot::Rhand,
        EquipSlot::Chest,
        EquipSlot::Lhand,
        EquipSlot::Rear,
        EquipSlot::Lear,
        EquipSlot::Gloves,
        EquipSlot::Legs,
        EquipSlot::Feet,
        EquipSlot::Rfinger,
        EquipSlot::Lfinger,
        EquipSlot::Lbracelet,
        EquipSlot::Rbracelet,
        EquipSlot::Deco1,
        EquipSlot::Deco2,
        EquipSlot::Deco3,
        EquipSlot::Deco4,
        EquipSlot::Deco5,
        EquipSlot::Deco6,
        EquipSlot::Cloak,
        EquipSlot::Belt,
    ];
    slots
        .get(usize::try_from(value)?)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("invalid item slot"))
}

pub(super) async fn persist_retained<C: ConnectionTrait>(
    db: &C,
    character: CharacterId,
    checkpoint: &Phase4Checkpoint,
) -> anyhow::Result<()> {
    let changed = db
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE characters SET phase4_checkpoint=$2 WHERE id=$1",
            [
                character.as_uuid().into(),
                serde_json::to_value(checkpoint)?.into(),
            ],
        ))
        .await?;
    anyhow::ensure!(changed.rows_affected() == 1, "retained item parent disappeared");
    Ok(())
}
