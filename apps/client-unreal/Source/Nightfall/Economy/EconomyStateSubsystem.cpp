#include "EconomyStateSubsystem.h"
#include "Items/InventoryStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Bot/EconomyBotPredicates.h"
#include "Bot/SkillBotPredicates.h"
#include "NightfallWire.h"
#include "Nightfall.h"
#include "Misc/CommandLine.h"
#include "Misc/Parse.h"

namespace EconomyDetail
{
 constexpr uint32 AdenaTemplate = 57;
 constexpr uint32 SowSkill = 2097, HarvestSkill = 2098;
 constexpr double RefreshSeconds = 1.0;
 // PartyRange2 = 1,400 L2 units = 43,750 milli-tiles (design §1 units; §4.2 candidate filter), players carry radius 0.
 constexpr double PartyRange2Tiles = 43.75;
 const TCHAR* OperationNames[] = { TEXT("unknown"), TEXT("equip"), TEXT("unequip"), TEXT("use"), TEXT("destroy"), TEXT("enchant"), TEXT("toggle_auto_shot"), TEXT("refine"), TEXT("cancel_refine"), TEXT("exchange_sa"), TEXT("crystallize"), TEXT("grant"), TEXT("starter"), TEXT("backfill"),
  TEXT("pick_up_item"), TEXT("vendor_buy"), TEXT("vendor_sell"), TEXT("vendor_refund"), TEXT("multisell_choose"), TEXT("register_recipe"), TEXT("delete_recipe"), TEXT("craft_item"), TEXT("trade_confirm"), TEXT("store_buy"), TEXT("store_sell"), TEXT("private_manufacture"), TEXT("warehouse_deposit"), TEXT("warehouse_withdraw"), TEXT("freight_send"), TEXT("freight_withdraw"), TEXT("mail_send"), TEXT("mail_take"), TEXT("mail_reject"), TEXT("mail_cancel"), TEXT("mail_delete"), TEXT("manor_buy_seed"), TEXT("manor_sell_crop"), TEXT("auction_bid"), TEXT("auction_cancel_bid"), TEXT("party"), TEXT("trade_session"), TEXT("store_setup"), TEXT("mail_mark_read") };
 struct FCommandOp { const TCHAR* Name; EGrpcNightfallV1ItemOperation Op; };
 using O = EGrpcNightfallV1ItemOperation;
 const FCommandOp CommandOps[] = {
  {TEXT("nf.pickup"), O::ITEM_OPERATION_PICK_UP_ITEM}, {TEXT("nf.buy"), O::ITEM_OPERATION_VENDOR_BUY}, {TEXT("nf.sell"), O::ITEM_OPERATION_VENDOR_SELL}, {TEXT("nf.refund"), O::ITEM_OPERATION_VENDOR_REFUND},
  {TEXT("nf.multisell"), O::ITEM_OPERATION_MULTISELL_CHOOSE}, {TEXT("nf.registerrecipe"), O::ITEM_OPERATION_REGISTER_RECIPE}, {TEXT("nf.deleterecipe"), O::ITEM_OPERATION_DELETE_RECIPE}, {TEXT("nf.craft"), O::ITEM_OPERATION_CRAFT_ITEM},
  {TEXT("nf.manufactureorder"), O::ITEM_OPERATION_PRIVATE_MANUFACTURE}, {TEXT("nf.traderequest"), O::ITEM_OPERATION_TRADE_SESSION}, {TEXT("nf.tradeanswer"), O::ITEM_OPERATION_TRADE_SESSION}, {TEXT("nf.tradeadd"), O::ITEM_OPERATION_TRADE_SESSION},
  {TEXT("nf.tradeconfirm"), O::ITEM_OPERATION_TRADE_CONFIRM}, {TEXT("nf.tradecancel"), O::ITEM_OPERATION_TRADE_SESSION}, {TEXT("nf.storesell"), O::ITEM_OPERATION_STORE_SETUP}, {TEXT("nf.storebuy"), O::ITEM_OPERATION_STORE_SETUP},
  {TEXT("nf.storemanufacture"), O::ITEM_OPERATION_STORE_SETUP}, {TEXT("nf.storeclose"), O::ITEM_OPERATION_STORE_SETUP}, {TEXT("nf.storebuyfrom"), O::ITEM_OPERATION_STORE_BUY}, {TEXT("nf.storesellto"), O::ITEM_OPERATION_STORE_SELL},
  {TEXT("nf.partyinvite"), O::ITEM_OPERATION_PARTY}, {TEXT("nf.partyanswer"), O::ITEM_OPERATION_PARTY}, {TEXT("nf.partyleave"), O::ITEM_OPERATION_PARTY}, {TEXT("nf.partykick"), O::ITEM_OPERATION_PARTY}, {TEXT("nf.partyleader"), O::ITEM_OPERATION_PARTY},
  {TEXT("nf.partylootvote"), O::ITEM_OPERATION_PARTY}, {TEXT("nf.partylootanswer"), O::ITEM_OPERATION_PARTY}, {TEXT("nf.deposit"), O::ITEM_OPERATION_WAREHOUSE_DEPOSIT}, {TEXT("nf.withdraw"), O::ITEM_OPERATION_WAREHOUSE_WITHDRAW},
  {TEXT("nf.freightsend"), O::ITEM_OPERATION_FREIGHT_SEND}, {TEXT("nf.freightwithdraw"), O::ITEM_OPERATION_FREIGHT_WITHDRAW}, {TEXT("nf.mailsend"), O::ITEM_OPERATION_MAIL_SEND}, {TEXT("nf.mailtake"), O::ITEM_OPERATION_MAIL_TAKE},
  {TEXT("nf.mailreject"), O::ITEM_OPERATION_MAIL_REJECT}, {TEXT("nf.mailcancel"), O::ITEM_OPERATION_MAIL_CANCEL}, {TEXT("nf.maildelete"), O::ITEM_OPERATION_MAIL_DELETE}, {TEXT("nf.mailread"), O::ITEM_OPERATION_MAIL_MARK_READ},
  {TEXT("nf.auctionbid"), O::ITEM_OPERATION_AUCTION_BID}, {TEXT("nf.auctioncancel"), O::ITEM_OPERATION_AUCTION_CANCEL_BID}, {TEXT("nf.manorbuyseed"), O::ITEM_OPERATION_MANOR_BUY_SEED}, {TEXT("nf.manorsellcrop"), O::ITEM_OPERATION_MANOR_SELL_CROP} };
 uint64 Num(const FString& S) { uint64 V = 0; SkillBotPredicates::ParseUnsigned(S, V); return V; }
 /** `template:<id>:<a>[:<b>...]` or `<id>:<a>...` comma lists -> [id, a, b, ...]. */
 TArray<TArray<uint64>> Entries(const FString& Text)
 {
  TArray<TArray<uint64>> Out; TArray<FString> Parts; Text.ParseIntoArray(Parts, TEXT(","), true);
  for (const FString& Entry : Parts) { TArray<FString> F; Entry.ParseIntoArray(F, TEXT(":"), true); if (F.Num() && F[0] == TEXT("template")) F.RemoveAt(0); TArray<uint64>& Row = Out.Emplace_GetRef(); for (const FString& V : F) Row.Add(Num(V)); }
  return Out;
 }
 template<typename E> bool ParseEnum(const FString& Prefix, const FString& Text, E& Out)
 {
  const int64 V = StaticEnum<E>()->GetValueByNameString(Prefix + Text.ToUpper()); if (V == INDEX_NONE) return false; Out = E(V); return true;
 }
}

void UEconomyStateSubsystem::Initialize(FSubsystemCollectionBase& C)
{
 Super::Initialize(C);
 Net = C.InitializeDependency<UNetClientSubsystem>(); Session = C.InitializeDependency<USessionClient>(); Inventory = C.InitializeDependency<UInventoryStateSubsystem>();
 Net->OnEntitySpawn.AddUObject(this, &UEconomyStateSubsystem::ApplySpawn);
 Net->OnEntityDespawn.AddUObject(this, &UEconomyStateSubsystem::ApplyDespawn);
 Net->OnEntityRespawned.AddUObject(this, &UEconomyStateSubsystem::ApplyRespawn);
 Net->OnEntityDied.AddUObject(this, &UEconomyStateSubsystem::ApplyDied);
 Net->OnStatsChanged.AddUObject(this, &UEconomyStateSubsystem::ApplyStats);
 Net->OnXpGained.AddUObject(this, &UEconomyStateSubsystem::ApplyXp);
 Net->OnSpGained.AddUObject(this, &UEconomyStateSubsystem::ApplySp);
 Net->OnEffectApplied.AddUObject(this, &UEconomyStateSubsystem::ApplyEffectApplied);
 Net->OnSkillCastResult.AddUObject(this, &UEconomyStateSubsystem::ApplyCastResult);
 Net->OnInventorySync.AddUObject(this, &UEconomyStateSubsystem::ApplyInventorySync);
 Net->OnInventoryChanged.AddUObject(this, &UEconomyStateSubsystem::ApplyInventoryChanged);
 Net->OnEquipmentChanged.AddUObject(this, &UEconomyStateSubsystem::ApplyEquipmentChanged);
 Net->OnEconomyReceiptPayload.AddUObject(this, &UEconomyStateSubsystem::ApplyEconomyReceipt);
 Net->OnCorpseStateChanged.AddUObject(this, &UEconomyStateSubsystem::ApplyCorpse);
 Net->OnPartyStateChanged.AddUObject(this, &UEconomyStateSubsystem::ApplyParty);
 Net->OnPartyLootVote.AddUObject(this, &UEconomyStateSubsystem::ApplyLootVote);
 Net->OnPartyInviteReceived.AddUObject(this, &UEconomyStateSubsystem::ApplyInvite);
 Net->OnTradeRequestReceived.AddUObject(this, &UEconomyStateSubsystem::ApplyTradeRequest);
 Net->OnTradeSessionChanged.AddUObject(this, &UEconomyStateSubsystem::ApplyTrade);
 Net->OnPrivateStoreChanged.AddUObject(this, &UEconomyStateSubsystem::ApplyStore);
 Net->OnMailNotification.AddUObject(this, &UEconomyStateSubsystem::ApplyMail);
 Net->OnRecipeBookSync.AddUObject(this, &UEconomyStateSubsystem::ApplyRecipeBook);
 Net->OnItemAuctionStatusChanged.AddUObject(this, &UEconomyStateSubsystem::ApplyAuction);
 Net->OnLootDistributed.AddUObject(this, &UEconomyStateSubsystem::ApplyLootDistributed);
 Net->OnIntentAck.AddUObject(this, &UEconomyStateSubsystem::ApplyAck);
 Net->OnIntentRejected.AddUObject(this, &UEconomyStateSubsystem::ApplyRejected);
 Net->OnConnected.AddDynamic(this, &UEconomyStateSubsystem::Reset);
 Net->OnDisconnected.AddDynamic(this, &UEconomyStateSubsystem::Disconnected);
 Inventory->OnMarked.AddUObject(this, &UEconomyStateSubsystem::Marked);
 // Adena deltas read the inventory projection after it published (OnChanged), never the raw delta: order-independent of the Net delegates.
 Inventory->OnChanged.AddUObject(this, &UEconomyStateSubsystem::TrackAdena);
 // -Phase5Roles=a:<character uuid>:<name>,b:...  (the harness passes the pack roles of the unit)
 FString RoleText;
 if (FParse::Value(FCommandLine::Get(), TEXT("Phase5Roles="), RoleText))
 {
  TArray<FString> Items; RoleText.ParseIntoArray(Items, TEXT(","), true);
  for (const FString& Item : Items) { TArray<FString> F; Item.ParseIntoArray(F, TEXT(":"), false); FGuid G; if (F.Num() == 3 && F[0].Len() == 1 && FGuid::ParseExact(F[1], EGuidFormats::DigitsWithHyphens, G)) Roles.Add(F[0].ToLower(), { G.ToString(EGuidFormats::DigitsWithHyphensLower), F[2] }); }
 }
}
void UEconomyStateSubsystem::Deinitialize() { Net = nullptr; Session = nullptr; Inventory = nullptr; Super::Deinitialize(); }

void UEconomyStateSubsystem::Reset()
{
 Fence.Reset(); ResetIdentityScoped(); ResetIncarnationScoped();
 SequenceRequests.Reset(); Outstanding.Reset(); CommittedFingerprints.Reset(); ReceiptBytes.Reset(); ReceiptMatches.Reset(); StatusText.Reset();
 InstanceTemplates.Reset(); OwnInventoryTick = 0; AdenaIncrements.Reset(); LastAdena.Reset();
 BuylistView.Reset(); LastMerchant.Reset(); Sold.Reset(); WarehouseCounts.Reset(); FreightCounts.Reset(); WarehouseInstanceTemplates.Reset(); FreightInstanceTemplates.Reset(); bWarehouseKnown = bFreightKnown = false; FreightCapacityValue = 0;
 InboxView.Reset(); OutboxView.Reset(); MailSubjects.Reset(); AuctionsView.Reset(); LastAuctionNpc = 0; ++ViewSerial;
 LastBuylistRequest = LastWarehouseRequest = LastFreightRequest = LastMailRequest = LastAuctionRequest = -1e9;
 OnChanged.Broadcast();
}
void UEconomyStateSubsystem::ResetIdentityScoped()
{
 Ground.Reset(); GoneGround.Reset(); PkDropSpawns = 0; bPkDropsAllUnowned = true; bPkDropDespawned = false; Corpses.Reset();
 PartyState.Reset(); PartyTick = 0; Vote.Reset(); PendingInvite.Reset(); Loot.Reset(); TradeState.Reset(); PendingTrade.Reset(); Stores.Reset(); Auctions.Reset(); Mail.Reset();
 bOwnKillDied = false; LastOwnKillTemplate.Reset(); OwnDeathTick.Reset(); DeathRemovedIds.Reset(); DeathEquipClearOrder.Reset(); DeathRemovedOrder.Reset(); DeathEventOrder = 0;
}
void UEconomyStateSubsystem::ResetIncarnationScoped()
{
 Results.Reset(); ReceiptOrder = 0; Recipes.Reset(); Stats.Reset(); XpGainedEvents = 0; LastXp.Reset(); LastSp.Reset(); SpAfterKill.Reset();
 HerbTemplate.Reset(); HerbTick.Reset(); EffectsAfterHerb = 0; OwnCastResults.Reset(); OwnCastResultTicks.Reset(); ManufactureAckTicks.Reset(); bContainerBusyRetry = false; BusyScopes.Reset();
}

bool UEconomyStateSubsystem::Own(const FString& Id) const { return Net && Net->IsOwnEntity(Id); }
bool UEconomyStateSubsystem::OwnerVisible(const FBytes& Id, uint32 Inc) const { const FString S = NightfallItems::UuidString(Id); return !S.IsEmpty() && Own(S) && Fence.Visible(S, Inc); }
FBytes UEconomyStateSubsystem::OwnCharacterId() const { return Net ? NightfallItems::UuidBytes(Net->GetOwnEntityId()) : FBytes(); }

// --- Reducers ------------------------------------------------------------------------------------

void UEconomyStateSubsystem::ApplySpawn(const FEntitySpawn& S)
{
 const bool bNew = Fence.Spawn(S);
 if (Own(S.EntityId)) { if (bNew) { ResetIdentityScoped(); ResetIncarnationScoped(); OnChanged.Broadcast(); } return; }
 if (!bNew) return;
 if (S.Kind == uint32(EGrpcNightfallV1EntityKind::ENTITY_KIND_GROUND_ITEM) && S.GroundItem.IsSet())
 {
  FGroundItem G; G.EntityId = S.EntityId; G.Detail = *S.GroundItem; G.SpawnTick = S.StateTick; G.Position = S.Position;
  // A PK drop: a ground entity that appears while the owner lies dead (its items rolled off the corpse).
  G.bPkDrop = OwnDeathTick.IsSet() && Net && Fence.Dead(Net->GetOwnEntityId()) && S.StateTick >= *OwnDeathTick;
  if (G.bPkDrop) { ++PkDropSpawns; bPkDropsAllUnowned = bPkDropsAllUnowned && G.Detail.LootOwner.IsEmpty() && G.Detail.DropProtectOwner.IsEmpty(); }
  Ground.Add(S.EntityId.ToLower(), G); GoneGround.Remove(S.EntityId.ToLower()); OnChanged.Broadcast();
 }
}
void UEconomyStateSubsystem::ApplyDespawn(const FEntityDespawn& S)
{
 const FString K = S.EntityId.ToLower(); const uint64 Tick = Net ? Net->GetLatestReceivedTick() : 0;
 if (const FGroundItem* G = Ground.Find(K)) { GoneGround.Add(K, { G->Detail, Tick, G->bPkDrop }); if (G->bPkDrop) bPkDropDespawned = true; Ground.Remove(K); OnChanged.Broadcast(); }
 if (FCorpse* C = Corpses.Find(K)) C->DespawnTick = Tick;
 Fence.Despawn(S.EntityId);
}
void UEconomyStateSubsystem::ApplyRespawn(const FEntityRespawned& S)
{
 if (!Fence.Respawn(S)) return;
 if (Own(S.Entity)) { ResetIncarnationScoped(); OnChanged.Broadcast(); }
}
void UEconomyStateSubsystem::ApplyDied(const FEntityDied& S)
{
 if (!Fence.Died(S)) return;
 if (Own(S.Entity)) { OwnDeathTick = S.Tick; DeathRemovedIds.Reset(); DeathEquipClearOrder.Reset(); DeathRemovedOrder.Reset(); DeathEventOrder = 0; PkDropSpawns = 0; bPkDropsAllUnowned = true; bPkDropDespawned = false; }
 if (Own(S.Killer)) { bOwnKillDied = true; LastOwnKillTemplate = TemplateOf(S.Entity); SpAfterKill.FindOrAdd(LastOwnKillTemplate) = 0; }
 OnChanged.Broadcast();
}
void UEconomyStateSubsystem::ApplyStats(const FStatsChanged& S) { if (!Own(S.Entity) || !Fence.Visible(S.Entity, S.Incarnation) || (Stats.IsSet() && S.Tick < Stats->Tick)) return; Stats = S; OnChanged.Broadcast(); }
void UEconomyStateSubsystem::ApplyXp(const FXpGained& S) { if (!Own(S.Entity)) return; ++XpGainedEvents; LastXp = S.Amount; OnChanged.Broadcast(); }
void UEconomyStateSubsystem::ApplySp(const FSpGained& S)
{
 if (!Own(S.Entity) || !Fence.Visible(S.Entity, S.Incarnation)) return;
 LastSp = S.Amount; if (!LastOwnKillTemplate.IsEmpty()) ++SpAfterKill.FindOrAdd(LastOwnKillTemplate); OnChanged.Broadcast();
}
void UEconomyStateSubsystem::ApplyEffectApplied(const FEffectApplied& S) { if (Own(S.Target) && HerbTick.IsSet() && S.Tick >= *HerbTick) ++EffectsAfterHerb; }
void UEconomyStateSubsystem::ApplyCastResult(const FSkillCastResult& S) { if (!Own(S.Caster)) return; ++OwnCastResults.FindOrAdd(S.SkillId); OwnCastResultTicks.Add(S.SkillId, S.Tick); }
void UEconomyStateSubsystem::ApplyInventorySync(const FGrpcNightfallV1InventorySync& S)
{
 if (!OwnerVisible(S.EntityId, S.Incarnation)) return;
 for (const auto& I : S.Items) InstanceTemplates.Add(I.InstanceId.Value, I.TemplateId.Value);
 OwnInventoryTick = FMath::Max(OwnInventoryTick, S.Tick.Value);
}
void UEconomyStateSubsystem::TrackAdena()
{
 if (!Inventory) return; const TOptional<uint64> Now = Inventory->Count(EconomyDetail::AdenaTemplate);
 if (!Now.IsSet()) { LastAdena.Reset(); return; }
 if (LastAdena.IsSet() && *Now > *LastAdena) AdenaIncrements.Add(*Now - *LastAdena);
 LastAdena = Now;
}
void UEconomyStateSubsystem::ApplyInventoryChanged(const FGrpcNightfallV1InventoryChanged& S)
{
 if (!OwnerVisible(S.EntityId, S.Incarnation)) return;
 for (const auto& I : S.Added) InstanceTemplates.Add(I.InstanceId.Value, I.TemplateId.Value);
 for (const auto& I : S.Updated) InstanceTemplates.Add(I.InstanceId.Value, I.TemplateId.Value);
 OwnInventoryTick = FMath::Max(OwnInventoryTick, S.Tick.Value);
 if (OwnDeathTick.IsSet() && S.Tick.Value == *OwnDeathTick && !S.RemovedIds.IsEmpty() && !DeathRemovedIds.IsSet()) { DeathRemovedIds = S.RemovedIds.Num(); DeathRemovedOrder = ++DeathEventOrder; }
 OnChanged.Broadcast();
}
void UEconomyStateSubsystem::ApplyEquipmentChanged(const FGrpcNightfallV1EquipmentChanged& S)
{
 if (!OwnerVisible(S.EntityId, S.Incarnation)) return;
 if (OwnDeathTick.IsSet() && S.Tick.Value == *OwnDeathTick && !DeathEquipClearOrder.IsSet()) DeathEquipClearOrder = ++DeathEventOrder;
}
void UEconomyStateSubsystem::ApplyEconomyReceipt(const FWorldEvent& E)
{
 if (!E.EconomyResult.IsSet()) return;
 const FGrpcNightfallV1EconomyResult& R = *E.EconomyResult;
 const FString Key = R.RequestKey.Value.IsEmpty() ? R.ReceiptId : NightfallItems::UuidString(R.RequestKey);
 const FString Scope = RequestScope(R.Operation, Key); if (Scope.IsEmpty()) return;
 const bool bVisible = OwnerVisible(R.EntityId, R.Incarnation), bRequested = Outstanding.Contains(Scope);
 if (!bVisible && !ReceiptBytes.Contains(Scope) && !bRequested) return;
 TArray<uint8> Bytes = E.EconomyReceiptPayload; if (Bytes.IsEmpty() && !E.bReceivedFromWire) NightfallWire::EncodeEconomyReceipt(R, Bytes);
 Remember(R.Operation, Key, R.RequestFingerprint, Bytes);
 if (!bVisible) return;
 // One receipt per receipt id: a replay (reconnect) compares bytes above but never counts twice.
 if (Results.ContainsByPredicate([&](const FReceipt& X) { return X.Result.ReceiptId == R.ReceiptId; })) return;
 Results.Add({ R, ++ReceiptOrder });
 using O = EGrpcNightfallV1ItemOperation;
 switch (R.Operation)
 {
 case O::ITEM_OPERATION_PICK_UP_ITEM:
  if (R.Outcome.OutcomeCase == EGrpcNightfallV1EconomyResultOutcome::Pickup && R.Outcome.Pickup.IsValid() && R.Outcome.Pickup->Herb) { HerbTemplate = R.Outcome.Pickup->TemplateId.Value; HerbTick = R.Tick.Value; EffectsAfterHerb = 0; }
  break;
 case O::ITEM_OPERATION_VENDOR_BUY: case O::ITEM_OPERATION_VENDOR_SELL: case O::ITEM_OPERATION_VENDOR_REFUND: BuylistView.Reset(); RefreshBuylist(); break;
 case O::ITEM_OPERATION_WAREHOUSE_DEPOSIT: case O::ITEM_OPERATION_WAREHOUSE_WITHDRAW:
  if (R.Outcome.OutcomeCase == EGrpcNightfallV1EconomyResultOutcome::Container && R.Outcome.Container.IsValid() && bWarehouseKnown)
  {
   // Received container deltas: the moved rows name instances whose templates the owner has seen.
   for (const auto& M : R.Outcome.Container->Moved)
   {
    const TOptional<uint32> T = TemplateOfInstance(M.InstanceId.Value); if (!T.IsSet()) { bWarehouseKnown = false; break; }
    uint64& N = WarehouseCounts.FindOrAdd(*T);
    if (R.Operation == O::ITEM_OPERATION_WAREHOUSE_DEPOSIT) N += M.Count.Value; else if (N >= M.Count.Value) N -= M.Count.Value; else { bWarehouseKnown = false; break; }
   }
  }
  RefreshWarehouse(); break;
 case O::ITEM_OPERATION_FREIGHT_WITHDRAW:
  if (R.Outcome.OutcomeCase == EGrpcNightfallV1EconomyResultOutcome::Container && R.Outcome.Container.IsValid() && bFreightKnown)
  {
   for (const auto& M : R.Outcome.Container->Moved)
   {
    const uint32* T = FreightInstanceTemplates.Find(M.InstanceId.Value); uint64* N = T ? FreightCounts.Find(*T) : nullptr;
    if (!N || *N < M.Count.Value) { bFreightKnown = false; break; } *N -= M.Count.Value;
   }
  }
  RefreshFreight(); break;
 case O::ITEM_OPERATION_FREIGHT_SEND: RefreshFreight(); break;
 case O::ITEM_OPERATION_MAIL_SEND: case O::ITEM_OPERATION_MAIL_TAKE: case O::ITEM_OPERATION_MAIL_REJECT: case O::ITEM_OPERATION_MAIL_CANCEL: case O::ITEM_OPERATION_MAIL_DELETE: case O::ITEM_OPERATION_MAIL_MARK_READ: RefreshMail(); break;
 case O::ITEM_OPERATION_AUCTION_BID: case O::ITEM_OPERATION_AUCTION_CANCEL_BID: RefreshAuctions(); break;
 default: break;
 }
 OnChanged.Broadcast();
}
void UEconomyStateSubsystem::ApplyCorpse(const FGrpcNightfallV1CorpseStateChanged& S)
{
 const FString K = S.Npc.ToLower(); FCorpse* C = Corpses.Find(K);
 if (C && S.Tick.Value < C->State.Tick.Value) return;
 if (!C) { C = &Corpses.Add(K); C->EntityId = S.Npc; C->Template = TemplateOf(S.Npc); C->FirstTick = S.Tick.Value; }
 if (C->Template.IsEmpty()) C->Template = TemplateOf(S.Npc);
 C->State = S; OnChanged.Broadcast();
}
void UEconomyStateSubsystem::ApplyParty(const FGrpcNightfallV1PartyStateChanged& S)
{
 if (S.Tick.Value < PartyTick) return;
 if (S.Disbanded) { PartyState.Reset(); Vote.Reset(); PartyTick = S.Tick.Value; OnChanged.Broadcast(); return; }
 if (!S.Baseline && (!PartyState.IsSet() || PartyState->PartyId.Value != S.PartyId.Value)) return; // a delta needs its admission baseline (D §4.4): fail closed
 const bool bMembersChanged = !PartyState.IsSet() || PartyState->Members.Num() != S.Members.Num();
 PartyState = S; PartyTick = S.Tick.Value; PendingInvite.Reset(); if (bMembersChanged) Vote.Reset(); OnChanged.Broadcast();
}
void UEconomyStateSubsystem::ApplyLootVote(const FGrpcNightfallV1PartyLootVote& S) { if (!PartyState.IsSet() || PartyState->PartyId.Value != S.PartyId.Value) return; Vote = S; OnChanged.Broadcast(); }
void UEconomyStateSubsystem::ApplyInvite(const FGrpcNightfallV1PartyInviteReceived& S) { PendingInvite = S; OnChanged.Broadcast(); }
void UEconomyStateSubsystem::ApplyTradeRequest(const FGrpcNightfallV1TradeRequestReceived& S) { PendingTrade = S; OnChanged.Broadcast(); }
void UEconomyStateSubsystem::ApplyTrade(const FGrpcNightfallV1TradeSessionChanged& S) { if (TradeState.IsSet() && S.Tick.Value < TradeState->Tick.Value) return; TradeState = S; PendingTrade.Reset(); OnChanged.Broadcast(); }
void UEconomyStateSubsystem::ApplyStore(const FGrpcNightfallV1PrivateStoreChanged& S) { const FString K = S.Owner.ToLower(); if (const auto* Known = Stores.Find(K)) { if (S.Tick.Value < Known->Tick.Value) return; } Stores.Add(K, S); OnChanged.Broadcast(); }
void UEconomyStateSubsystem::ApplyMail(const FGrpcNightfallV1MailNotification& S) { Mail.Add({ S, FDateTime::UtcNow().ToUnixTimestamp() * 1000 + FDateTime::UtcNow().GetMillisecond() }); MailSubjects.Add(S.MessageId, S.Subject); RefreshMail(); OnChanged.Broadcast(); }
void UEconomyStateSubsystem::ApplyRecipeBook(const FGrpcNightfallV1RecipeBookSync& S)
{
 if (!Net || !Fence.Visible(Net->GetOwnEntityId(), S.Incarnation.Value)) return;
 if (!S.Baseline && !Recipes.IsSet()) return; // owner baseline first (D §8.1)
 if (Recipes.IsSet() && S.Tick.Value < Recipes->Tick.Value) return;
 Recipes = S; OnChanged.Broadcast();
}
void UEconomyStateSubsystem::ApplyAuction(const FGrpcNightfallV1ItemAuctionStatusChanged& S)
{
 Auctions.Add(S.InstanceNpcId.Value, S); LastAuctionNpc = S.InstanceNpcId.Value;
 if (S.State == EGrpcNightfallV1AuctionState::AUCTION_STATE_FINISHED) RefreshWarehouse();
 RefreshAuctions(); OnChanged.Broadcast();
}
void UEconomyStateSubsystem::ApplyLootDistributed(const FGrpcNightfallV1LootDistributed& S) { Loot.Add(S); OnChanged.Broadcast(); }
void UEconomyStateSubsystem::ApplyAck(const FAck& A)
{
 const FSubmittedRequest* R = SequenceRequests.Find(A.Seq); if (!R) return;
 if (R->Op == EGrpcNightfallV1ItemOperation::ITEM_OPERATION_PRIVATE_MANUFACTURE) ManufactureAckTicks.Add(A.Tick);
 if (BusyScopes.Contains(R->Scope)) bContainerBusyRetry = true;
}
void UEconomyStateSubsystem::ApplyRejected(const FIntentRejected& R)
{
 FSubmittedRequest* Found = SequenceRequests.Find(R.Seq); if (!Found || Found->Status != ERequestStatus::Pending) return;
 Found->Status = ERequestStatus::Rejected;
 if (R.Reason == uint32(EGrpcNightfallV1RejectReason::REJECT_REASON_CONTAINER_BUSY)) BusyScopes.Add(Found->Scope);
 if (auto* Seqs = Outstanding.Find(Found->Scope)) { Seqs->Remove(R.Seq); if (Seqs->IsEmpty()) Outstanding.Remove(Found->Scope); }
 StatusText = R.Detail; OnChanged.Broadcast();
}
void UEconomyStateSubsystem::Marked() { AdenaIncrements.Reset(); if (Inventory) LastAdena = Inventory->Count(EconomyDetail::AdenaTemplate); }

// --- Queries -------------------------------------------------------------------------------------

TArray<const UEconomyStateSubsystem::FGroundItem*> UEconomyStateSubsystem::GroundOf(uint32 Template) const
{
 TArray<const FGroundItem*> Out; for (const auto& P : Ground) if (P.Value.Detail.TemplateId.Value == Template) Out.Add(&P.Value);
 Out.Sort([](const FGroundItem& A, const FGroundItem& B) { return A.SpawnTick > B.SpawnTick; }); return Out;
}
FString UEconomyStateSubsystem::TemplateOf(const FString& EntityId) const
{
 const FString K = EntityId.ToLower(); const FEntitySpawn* S = Fence.Identities.Find(K); if (!S) S = Fence.Tombstones.Find(K); return S ? S->TemplateId : FString();
}
const UEconomyStateSubsystem::FCorpse* UEconomyStateSubsystem::CorpseOf(const FString& Template) const
{
 const FCorpse* Best = nullptr; for (const auto& P : Corpses) if (P.Value.Template == Template && (!Best || P.Value.FirstTick > Best->FirstTick)) Best = &P.Value; return Best;
}
bool UEconomyStateSubsystem::EntityGone(const FString& Template) const
{
 for (const auto& P : Fence.Tombstones) if (P.Value.TemplateId == Template) return true; return false;
}
bool UEconomyStateSubsystem::InParty(const FString& EntityId) const
{
 if (!PartyState.IsSet()) return false; for (const auto& M : PartyState->Members) if (M.Entity.Equals(EntityId, ESearchCase::IgnoreCase)) return true; return false;
}
FString UEconomyStateSubsystem::PartyLeader() const { return PartyState.IsSet() && PartyState->Members.Num() ? PartyState->Members[0].Entity : FString(); }
const UEconomyStateSubsystem::FReceipt* UEconomyStateSubsystem::LastReceiptOf(EGrpcNightfallV1ItemOperation Op) const
{
 for (int32 I = Results.Num() - 1; I >= 0; --I) if (Results[I].Result.Operation == Op) return &Results[I]; return nullptr;
}
int32 UEconomyStateSubsystem::ReceiptCount(EGrpcNightfallV1ItemOperation Op) const { int32 N = 0; for (const auto& R : Results) N += R.Result.Operation == Op; return N; }
TOptional<uint64> UEconomyStateSubsystem::ManufactureGateTicks() const
{
 const int32 N = ManufactureAckTicks.Num(); if (N < 2 || ManufactureAckTicks[N - 1] < ManufactureAckTicks[N - 2]) return {}; return ManufactureAckTicks[N - 1] - ManufactureAckTicks[N - 2];
}
TOptional<int64> UEconomyStateSubsystem::AdenaDelta() const
{
 if (!Inventory) return {}; const TOptional<uint64> Now = Inventory->Count(EconomyDetail::AdenaTemplate), Mark = Inventory->MarkedCount(EconomyDetail::AdenaTemplate);
 if (!Now.IsSet() || !Mark.IsSet() || *Now > uint64(MAX_int64) || *Mark > uint64(MAX_int64)) return {}; return int64(*Now) - int64(*Mark);
}
TOptional<bool> UEconomyStateSubsystem::PkUnequippedBeforeRoll() const
{
 if (!DeathRemovedOrder.IsSet()) return {}; return DeathEquipClearOrder.IsSet() && *DeathEquipClearOrder < *DeathRemovedOrder;
}
TOptional<bool> UEconomyStateSubsystem::OwnOutOfPartyRange() const
{
 const FString Leader = PartyLeader(); if (!Net || Leader.IsEmpty() || Own(Leader)) return {};
 FNetVec2 Mine, Theirs; const int64 Far = TNumericLimits<int64>::Max() / 2;
 if (!Net->Snapshots().Sample(Net->GetOwnEntityId(), Far, Mine) || !Net->Snapshots().Sample(Leader, Far, Theirs)) return {};
 const double Dx = double(Mine.X) - double(Theirs.X), Dy = double(Mine.Y) - double(Theirs.Y);
 return Dx * Dx + Dy * Dy > EconomyDetail::PartyRange2Tiles * EconomyDetail::PartyRange2Tiles;
}
TOptional<uint64> UEconomyStateSubsystem::WarehouseCount(uint32 Template) const { if (!bWarehouseKnown) return {}; const uint64* N = WarehouseCounts.Find(Template); return N ? *N : uint64(0); }
TOptional<uint64> UEconomyStateSubsystem::FreightCount(uint32 Template) const { if (!bFreightKnown) return {}; const uint64* N = FreightCounts.Find(Template); return N ? *N : uint64(0); }
TOptional<uint32> UEconomyStateSubsystem::TemplateOfInstance(uint64 Instance) const
{
 if (const uint32* T = InstanceTemplates.Find(Instance)) return *T; if (const uint32* T = WarehouseInstanceTemplates.Find(Instance)) return *T; if (const uint32* T = FreightInstanceTemplates.Find(Instance)) return *T; return {};
}
FString UEconomyStateSubsystem::NpcNamed(const FString& Name) const
{
 if (!Net) return {}; for (const auto& P : Net->GetKnownEntities()) if (!P.Value.bCombatant && P.Value.Kind == uint32(EGrpcNightfallV1EntityKind::ENTITY_KIND_NPC) && P.Value.Name.Equals(Name, ESearchCase::IgnoreCase)) return P.Value.EntityId; return {};
}

// --- Detached views ------------------------------------------------------------------------------

bool UEconomyStateSubsystem::Throttle(double& Last) { const double T = Now(); if (T - Last < EconomyDetail::RefreshSeconds) return false; Last = T; return true; }
void UEconomyStateSubsystem::RefreshBuylist()
{
 if (!Net || Net->GetOwnEntityId().IsEmpty()) return; if (LastMerchant.IsEmpty()) LastMerchant = NpcNamed(TEXT("merchant")); if (LastMerchant.IsEmpty() || !Throttle(LastBuylistRequest)) return;
 FGrpcNightfallV1ListBuylistRequest Q; Q.CharacterId = OwnCharacterId(); Q.Merchant = LastMerchant; const uint64 Serial = ViewSerial; TWeakObjectPtr<UEconomyStateSubsystem> Weak(this);
 auto Done = [Weak, Serial](const FNetResult& R, const FGrpcNightfallV1ListBuylistResponse& V) { if (Weak.IsValid() && Weak->ViewSerial == Serial && R.IsOk()) Weak->ApplyBuylist(V); };
 if (BuylistProvider) BuylistProvider(Q, Done); else if (Session) Session->ListBuylist(Q, Done);
}
void UEconomyStateSubsystem::RefreshWarehouse()
{
 if (!Net || Net->GetOwnEntityId().IsEmpty() || !Throttle(LastWarehouseRequest)) return;
 FGrpcNightfallV1GetWarehouseRequest Q; Q.CharacterId = OwnCharacterId(); const uint64 Serial = ViewSerial; TWeakObjectPtr<UEconomyStateSubsystem> Weak(this);
 auto Done = [Weak, Serial](const FNetResult& R, const FGrpcNightfallV1GetWarehouseResponse& V) { if (Weak.IsValid() && Weak->ViewSerial == Serial && R.IsOk()) Weak->ApplyWarehouse(V); };
 if (WarehouseProvider) WarehouseProvider(Q, Done); else if (Session) Session->GetWarehouse(Q, Done);
}
void UEconomyStateSubsystem::RefreshFreight()
{
 if (!Net || Net->GetOwnEntityId().IsEmpty() || !Throttle(LastFreightRequest)) return;
 FGrpcNightfallV1GetFreightRequest Q; Q.CharacterId = OwnCharacterId(); const uint64 Serial = ViewSerial; TWeakObjectPtr<UEconomyStateSubsystem> Weak(this);
 auto Done = [Weak, Serial](const FNetResult& R, const FGrpcNightfallV1GetFreightResponse& V) { if (Weak.IsValid() && Weak->ViewSerial == Serial && R.IsOk()) Weak->ApplyFreight(V); };
 if (FreightProvider) FreightProvider(Q, Done); else if (Session) Session->GetFreight(Q, Done);
}
void UEconomyStateSubsystem::RefreshMail()
{
 if (!Net || Net->GetOwnEntityId().IsEmpty() || !Throttle(LastMailRequest)) return;
 for (EGrpcNightfallV1MailBox Box : { EGrpcNightfallV1MailBox::MAIL_BOX_INBOX, EGrpcNightfallV1MailBox::MAIL_BOX_OUTBOX })
 {
  FGrpcNightfallV1ListMailRequest Q; Q.CharacterId = OwnCharacterId(); Q.MailBox = Box; const uint64 Serial = ViewSerial; TWeakObjectPtr<UEconomyStateSubsystem> Weak(this);
  auto Done = [Weak, Serial, Box](const FNetResult& R, const FGrpcNightfallV1ListMailResponse& V) { if (Weak.IsValid() && Weak->ViewSerial == Serial && R.IsOk()) Weak->ApplyMailList(Box, V); };
  if (MailProvider) MailProvider(Q, Done); else if (Session) Session->ListMail(Q, Done);
 }
}
void UEconomyStateSubsystem::RefreshAuctions()
{
 if (!Net || Net->GetOwnEntityId().IsEmpty() || !LastAuctionNpc || !Throttle(LastAuctionRequest)) return;
 FGrpcNightfallV1ListItemAuctionsRequest Q; Q.InstanceNpcId = LastAuctionNpc; Q.CharacterId = OwnCharacterId(); const uint64 Serial = ViewSerial; TWeakObjectPtr<UEconomyStateSubsystem> Weak(this);
 auto Done = [Weak, Serial](const FNetResult& R, const FGrpcNightfallV1ListItemAuctionsResponse& V) { if (Weak.IsValid() && Weak->ViewSerial == Serial && R.IsOk()) Weak->ApplyAuctions(V); };
 if (AuctionsProvider) AuctionsProvider(Q, Done); else if (Session) Session->ListItemAuctions(Q, Done);
}
void UEconomyStateSubsystem::ApplyBuylist(const FGrpcNightfallV1ListBuylistResponse& R) { if (BuylistView.IsSet() && R.StateTick.Value < BuylistView->StateTick.Value) return; BuylistView = R; OnChanged.Broadcast(); }
void UEconomyStateSubsystem::ApplyWarehouse(const FGrpcNightfallV1GetWarehouseResponse& R)
{
 WarehouseCounts.Reset(); WarehouseInstanceTemplates.Reset();
 for (const auto& I : R.Items) { WarehouseCounts.FindOrAdd(I.TemplateId.Value) += I.Count.Value; WarehouseInstanceTemplates.Add(I.InstanceId.Value, I.TemplateId.Value); }
 bWarehouseKnown = true; OnChanged.Broadcast();
}
void UEconomyStateSubsystem::ApplyFreight(const FGrpcNightfallV1GetFreightResponse& R)
{
 FreightCounts.Reset(); FreightInstanceTemplates.Reset();
 for (const auto& I : R.Items) { FreightCounts.FindOrAdd(I.TemplateId.Value) += I.Count.Value; FreightInstanceTemplates.Add(I.InstanceId.Value, I.TemplateId.Value); }
 FreightCapacityValue = R.Capacity.Value; bFreightKnown = true; OnChanged.Broadcast();
}
void UEconomyStateSubsystem::ApplyMailList(EGrpcNightfallV1MailBox Box, const FGrpcNightfallV1ListMailResponse& R)
{
 for (const auto& H : R.Messages) MailSubjects.Add(H.MessageId, H.Subject);
 if (Box == EGrpcNightfallV1MailBox::MAIL_BOX_OUTBOX) OutboxView = R; else InboxView = R; OnChanged.Broadcast();
}
void UEconomyStateSubsystem::ApplyAuctions(const FGrpcNightfallV1ListItemAuctionsResponse& R) { AuctionsView = R; OnChanged.Broadcast(); }

// --- Commands (D43 keyed replay) -----------------------------------------------------------------

TOptional<EGrpcNightfallV1ItemOperation> UEconomyStateSubsystem::OperationOf(const FString& CommandName)
{
 const FString Lower = CommandName.ToLower(); for (const auto& C : EconomyDetail::CommandOps) if (Lower == C.Name) return C.Op; return {};
}
FString UEconomyStateSubsystem::OperationName(EGrpcNightfallV1ItemOperation Op) { const uint8 I = uint8(Op); return I < UE_ARRAY_COUNT(EconomyDetail::OperationNames) ? EconomyDetail::OperationNames[I] : TEXT("unknown"); }
TOptional<EGrpcNightfallV1ItemOperation> UEconomyStateSubsystem::ParseOperation(const FString& Name)
{
 for (uint8 I = 14; I < UE_ARRAY_COUNT(EconomyDetail::OperationNames); ++I) if (Name == EconomyDetail::OperationNames[I]) return EGrpcNightfallV1ItemOperation(I); return {};
}
FString UEconomyStateSubsystem::RequestScope(EGrpcNightfallV1ItemOperation Op, const FString& Key) const
{
 const FString Normalized = NightfallItems::UuidString(NightfallItems::UuidBytes(Key));
 if (!Session || Session->AccountId().IsEmpty() || Normalized.IsEmpty() || !uint8(Op)) return {};
 return Session->AccountId() + FString::Printf(TEXT(":%u:%s"), uint8(Op), *Normalized);
}
void UEconomyStateSubsystem::Remember(EGrpcNightfallV1ItemOperation Op, const FString& Key, const FString& Fingerprint, const TArray<uint8>& Payload)
{
 const FString Scope = RequestScope(Op, Key); if (Scope.IsEmpty()) return;
 if (const TArray<uint8>* First = ReceiptBytes.Find(Scope)) ReceiptMatches = (*First == Payload); else { ReceiptBytes.Add(Scope, Payload); ReceiptMatches.Reset(); }
 // The zone's request fingerprint (design §12) is recorded as received; the client keys retries on scope + frozen arguments.
 if (!Fingerprint.IsEmpty() && !CommittedFingerprints.Contains(Scope)) CommittedFingerprints.Add(Scope, Fingerprint);
 bool bCompleted = false;
 if (TSet<uint32>* Seqs = Outstanding.Find(Scope)) { for (uint32 Seq : *Seqs) if (FSubmittedRequest* R = SequenceRequests.Find(Seq)) { R->Status = ERequestStatus::Committed; bCompleted = true; } Outstanding.Remove(Scope); }
 if (bCompleted) StatusText = IsBusy() ? TEXT("Waiting for server economy result") : TEXT("Server committed economy operation");
 OnChanged.Broadcast();
}
void UEconomyStateSubsystem::FinishSend(uint32 Seq, const FString& Scope, EGrpcNightfallV1ItemOperation Op, const FClientMessage& Body, const FString& Arguments)
{
 if (Seq) { Outstanding.FindOrAdd(Scope).Add(Seq); FSubmittedRequest R; R.Scope = Scope; R.Arguments = Arguments; R.Op = Op; R.Body = Body; SequenceRequests.Add(Seq, MoveTemp(R)); }
 StatusText = Seq ? TEXT("Waiting for server economy result") : TEXT("World channel unavailable"); OnChanged.Broadcast();
}
bool UEconomyStateSubsystem::Execute(const TArray<FString>& Tokens)
{
 FString Error; if (Tokens.IsEmpty() || !Net || !EconomyBotPredicates::ValidateCommand(Tokens, Error)) return false;
 const FString Name = Tokens[0].ToLower(); TArray<FString> A = Tokens; A.RemoveAt(0);
 const TOptional<EGrpcNightfallV1ItemOperation> Op = OperationOf(Name); if (!Op.IsSet()) return false;
 FGuid G; FString Key = FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower);
 if (!A.IsEmpty() && FGuid::ParseExact(A.Last(), EGuidFormats::DigitsWithHyphens, G)) { Key = G.ToString(EGuidFormats::DigitsWithHyphensLower); A.Pop(); }
 const FString Scope = RequestScope(*Op, Key), Arguments = FString::Join(A, TEXT(" ")); if (Scope.IsEmpty()) return false;
 // D43: the same key with the same arguments resends the frozen body; a committed key with other arguments is a conflict.
 const FSubmittedRequest* Previous = nullptr; uint32 Latest = 0;
 for (const auto& P : SequenceRequests) if (P.Value.Scope == Scope && P.Value.Status != ERequestStatus::Rejected && P.Key > Latest) { Latest = P.Key; Previous = &P.Value; }
 if (Previous)
 {
  if (Previous->Arguments != Arguments) { StatusText = TEXT("D43 request conflict: FAILED_PRECONDITION"); OnChanged.Broadcast(); return false; }
  const FClientMessage Body = Previous->Body; const uint32 Retry = Net->SendItemMessage(Body); FinishSend(Retry, Scope, *Op, Body, Arguments); return Retry != 0;
 }
 if (CommittedFingerprints.Contains(Scope)) { StatusText = TEXT("D43 request conflict: FAILED_PRECONDITION"); OnChanged.Broadcast(); return false; }
 FClientMessage Body; if (!Build(Name, A, Key, Body)) return false;
 if (Name == TEXT("nf.buy") || Name == TEXT("nf.sell") || Name == TEXT("nf.refund")) LastMerchant = NpcNamed(TEXT("merchant"));
 if (Name == TEXT("nf.sell") && Body.VendorSell.IsSet()) for (const auto& I : Body.VendorSell->Items) Sold.Add(I.InstanceId.Value);
 if (Name == TEXT("nf.auctionbid") || Name == TEXT("nf.auctioncancel")) LastAuctionNpc = uint32(EconomyDetail::Num(A[0]));
 const uint32 Seq = Net->SendItemMessage(Body); FinishSend(Seq, Scope, *Op, Body, Arguments); return Seq != 0;
}
bool UEconomyStateSubsystem::Build(const FString& Name, const TArray<FString>& A, const FString& Key, FClientMessage& M) const
{
 using namespace EconomyDetail;
 auto Instance = [&](const FString& Selector) -> TOptional<uint64> { return Inventory ? Inventory->Resolve(Selector) : TOptional<uint64>(); };
 auto Items = [&](const FString& Text, TArray<FGrpcNightfallV1ItemCount>& Out) { for (const auto& E : Entries(Text)) { if (E.Num() != 2) return false; const auto Id = Instance(FString::Printf(TEXT("template:%llu"), E[0])); if (!Id.IsSet()) return false; FGrpcNightfallV1ItemCount C; C.InstanceId = *Id; C.Count = E[1]; Out.Add(C); } return !Out.IsEmpty(); };
 auto ContainerItems = [&](const FString& Text, const TMap<uint64, uint32>& InstanceTemplatesOfView, TArray<FGrpcNightfallV1ItemCount>& Out) { for (const auto& E : Entries(Text)) { if (E.Num() != 2) return false; TOptional<uint64> Id; for (const auto& P : InstanceTemplatesOfView) if (P.Value == E[0] && (!Id.IsSet() || P.Key < *Id)) Id = P.Key; if (!Id.IsSet()) return false; FGrpcNightfallV1ItemCount C; C.InstanceId = *Id; C.Count = E[1]; Out.Add(C); } return !Out.IsEmpty(); };
 auto Npc = [&](const TCHAR* Role) { return NpcNamed(Role); };
 auto Role = [&](const FString& R) { return ResolveRole(R.ToLower()); };
 FNetVec2 OwnPos; const bool bOwnPos = Net && Net->Snapshots().Sample(Net->GetOwnEntityId(), TNumericLimits<int64>::Max() / 2, OwnPos);
 if (Name == TEXT("nf.pickup"))
 {
  const FGroundItem* Best = nullptr; double BestD = 0;
  for (const auto& P : Ground) { const FGroundItem& G = P.Value; if (A[0] != TEXT("nearest") && G.Detail.TemplateId.Value != Num(A[0].Mid(9))) continue; const double D = bOwnPos ? FMath::Square(double(G.Position.X) - double(OwnPos.X)) + FMath::Square(double(G.Position.Y) - double(OwnPos.Y)) : 0; if (!Best || D < BestD) { Best = &G; BestD = D; } }
  if (!Best) return false; FGrpcNightfallV1PickUpItemRequest Q; Q.IdempotencyKey = Key; Q.GroundEntity = Best->EntityId; M.PickUpItem = Q; return true;
 }
 if (Name == TEXT("nf.buy")) { const FString Merchant = Npc(TEXT("merchant")); if (Merchant.IsEmpty()) return false; FGrpcNightfallV1VendorBuyRequest Q; Q.IdempotencyKey = Key; Q.Merchant = Merchant; Q.ListId = Num(A[1]); FGrpcNightfallV1TemplateCount T; T.TemplateId = uint32(Num(A[2])); T.Count = Num(A[3]); Q.Items.Add(T); M.VendorBuy = Q; return true; }
 if (Name == TEXT("nf.sell")) { const FString Merchant = Npc(TEXT("merchant")); const auto Id = Instance(A[2]); if (Merchant.IsEmpty() || !Id.IsSet()) return false; FGrpcNightfallV1VendorSellRequest Q; Q.IdempotencyKey = Key; Q.Merchant = Merchant; Q.ListId = Num(A[1]); FGrpcNightfallV1ItemCount C; C.InstanceId = *Id; C.Count = Num(A[3]); Q.Items.Add(C); M.VendorSell = Q; return true; }
 if (Name == TEXT("nf.refund")) { const FString Merchant = Npc(TEXT("merchant")); if (Merchant.IsEmpty()) return false; FGrpcNightfallV1VendorRefundRequest Q; Q.IdempotencyKey = Key; Q.Merchant = Merchant; Q.ListId = Num(A[1]); Q.RefundIndices.Add(FUInt32(uint32(Num(A[2])))); M.VendorRefund = Q; return true; }
 if (Name == TEXT("nf.multisell")) { const FString Ex = Npc(TEXT("exchanger")); if (Ex.IsEmpty()) return false; FGrpcNightfallV1MultisellChooseRequest Q; Q.IdempotencyKey = Key; Q.Npc = Ex; Q.ListId = uint32(Num(A[1])); Q.EntryId = uint32(Num(A[2])); Q.Amount = Num(A[3]); M.MultisellChoose = Q; return true; }
 if (Name == TEXT("nf.registerrecipe")) { const auto Id = Instance(A[0]); if (!Id.IsSet()) return false; FGrpcNightfallV1RegisterRecipeRequest Q; Q.IdempotencyKey = Key; Q.RecipeInstanceId = *Id; M.RegisterRecipe = Q; return true; }
 if (Name == TEXT("nf.deleterecipe")) { FGrpcNightfallV1DeleteRecipeRequest Q; Q.IdempotencyKey = Key; Q.RecipeId = uint32(Num(A[0])); M.DeleteRecipe = Q; return true; }
 if (Name == TEXT("nf.craft")) { FGrpcNightfallV1CraftItemRequest Q; Q.IdempotencyKey = Key; Q.RecipeId = uint32(Num(A[0])); M.CraftItem = Q; return true; }
 if (Name == TEXT("nf.manufactureorder")) { const FString Owner = Role(A[0]); if (Owner.IsEmpty()) return false; FGrpcNightfallV1PrivateManufactureOrderRequest Q; Q.IdempotencyKey = Key; Q.StoreOwner = Owner; Q.RecipeId = uint32(Num(A[1])); M.PrivateManufactureOrder = Q; return true; }
 if (Name == TEXT("nf.traderequest")) { const FString T = Role(A[0]); if (T.IsEmpty()) return false; FGrpcNightfallV1TradeRequestRequest Q; Q.IdempotencyKey = Key; Q.Target = T; M.TradeRequest = Q; return true; }
 if (Name == TEXT("nf.tradeanswer")) { FGrpcNightfallV1TradeAnswerRequest Q; Q.IdempotencyKey = Key; Q.Accept = A[0] == TEXT("accept"); M.TradeAnswer = Q; return true; }
 if (Name == TEXT("nf.tradeadd")) { const auto Id = Instance(A[0]); if (!Id.IsSet()) return false; FGrpcNightfallV1TradeAddItemRequest Q; Q.IdempotencyKey = Key; Q.InstanceId = *Id; Q.Count = Num(A[1]); M.TradeAddItem = Q; return true; }
 if (Name == TEXT("nf.tradeconfirm")) { FGrpcNightfallV1TradeConfirmRequest Q; Q.IdempotencyKey = Key; M.TradeConfirm = Q; return true; }
 if (Name == TEXT("nf.tradecancel")) { FGrpcNightfallV1TradeCancelRequest Q; Q.IdempotencyKey = Key; M.TradeCancel = Q; return true; }
 if (Name == TEXT("nf.storesell"))
 {
  FGrpcNightfallV1SetPrivateStoreSellRequest Q; Q.IdempotencyKey = Key; Q.Title = A[0]; Q.PackageSale = A.Num() == 3;
  for (const auto& E : Entries(A.Last())) { if (E.Num() != 3) return false; const auto Id = Instance(FString::Printf(TEXT("template:%llu"), E[0])); if (!Id.IsSet()) return false; FGrpcNightfallV1StoreSellEntry S; S.InstanceId = *Id; S.TemplateId = uint32(E[0]); S.Count = E[1]; S.Price = E[2]; Q.Entries.Add(S); }
  M.SetPrivateStoreSell = Q; return true;
 }
 if (Name == TEXT("nf.storebuy"))
 {
  FGrpcNightfallV1SetPrivateStoreBuyRequest Q; Q.IdempotencyKey = Key; Q.Title = A[0];
  for (const auto& E : Entries(A[1])) { if (E.Num() != 4) return false; FGrpcNightfallV1StoreBuyEntry S; S.TemplateId = uint32(E[0]); S.Enchant = uint32(E[1]); S.Count = E[2]; S.Price = E[3]; Q.Entries.Add(S); }
  M.SetPrivateStoreBuy = Q; return true;
 }
 if (Name == TEXT("nf.storemanufacture"))
 {
  FGrpcNightfallV1SetPrivateManufactureRequest Q; Q.IdempotencyKey = Key; Q.Title = A[0];
  for (const auto& E : Entries(A[1])) { if (E.Num() != 2) return false; FGrpcNightfallV1ManufactureEntry S; S.RecipeId = uint32(E[0]); S.Cost = E[1]; Q.Entries.Add(S); }
  M.SetPrivateManufacture = Q; return true;
 }
 if (Name == TEXT("nf.storeclose")) { FGrpcNightfallV1ClosePrivateStoreRequest Q; Q.IdempotencyKey = Key; M.ClosePrivateStore = Q; return true; }
 if (Name == TEXT("nf.storebuyfrom"))
 {
  // P5-D9: the buyer binds `template:<id>:<count>:<price>` to the received store entry of that template with that price and enough count.
  const FString Owner = Role(A[0]); const FGrpcNightfallV1PrivateStoreChanged* S = Owner.IsEmpty() ? nullptr : Store(Owner); if (!S) return false;
  FGrpcNightfallV1PrivateStoreBuyRequest Q; Q.IdempotencyKey = Key; Q.StoreOwner = Owner; TSet<uint64> Used;
  for (const auto& E : Entries(A[1])) { if (E.Num() != 3) return false; const FGrpcNightfallV1StoreSellEntry* Match = nullptr; for (const auto& Row : S->Sell) if (!Used.Contains(Row.InstanceId.Value) && Row.TemplateId.Value == E[0] && Row.Price.Value == E[2] && Row.Count.Value >= E[1]) { Match = &Row; break; } if (!Match) return false; Used.Add(Match->InstanceId.Value); FGrpcNightfallV1StoreSellEntry R; R.InstanceId = Match->InstanceId; R.TemplateId = Match->TemplateId; R.Count = E[1]; R.Price = E[2]; Q.Entries.Add(R); }
  M.PrivateStoreBuy = Q; return true;
 }
 if (Name == TEXT("nf.storesellto"))
 {
  const FString Owner = Role(A[0]); if (Owner.IsEmpty()) return false; FGrpcNightfallV1PrivateStoreSellRequest Q; Q.IdempotencyKey = Key; Q.StoreOwner = Owner;
  for (const auto& E : Entries(A[1])) { if (E.Num() != 3) return false; const auto Id = Instance(FString::Printf(TEXT("template:%llu"), E[0])); if (!Id.IsSet()) return false; FGrpcNightfallV1PrivateStoreSellEntry S; S.InstanceId = *Id; S.TemplateId = uint32(E[0]); S.Count = E[1]; S.Price = E[2]; Q.Entries.Add(S); }
  M.PrivateStoreSell = Q; return true;
 }
 if (Name == TEXT("nf.partyinvite")) { const FString T = Role(A[0]); if (T.IsEmpty()) return false; FGrpcNightfallV1PartyInviteRequest Q; Q.IdempotencyKey = Key; Q.Target = T; if (A.Num() == 2 && !ParseEnum(TEXT("LOOT_MODE_"), A[1], Q.Mode)) return false; M.PartyInvite = Q; return true; }
 if (Name == TEXT("nf.partyanswer")) { FGrpcNightfallV1PartyAnswerRequest Q; Q.IdempotencyKey = Key; Q.Answer = FCString::Atoi(*A[0]); M.PartyAnswer = Q; return true; }
 if (Name == TEXT("nf.partyleave")) { FGrpcNightfallV1PartyLeaveRequest Q; Q.IdempotencyKey = Key; M.PartyLeave = Q; return true; }
 if (Name == TEXT("nf.partykick")) { const FString T = Role(A[0]); if (T.IsEmpty()) return false; FGrpcNightfallV1PartyKickRequest Q; Q.IdempotencyKey = Key; Q.Member = T; M.PartyKick = Q; return true; }
 if (Name == TEXT("nf.partyleader")) { const FString T = Role(A[0]); if (T.IsEmpty()) return false; FGrpcNightfallV1PartyChangeLeaderRequest Q; Q.IdempotencyKey = Key; Q.Member = T; M.PartyChangeLeader = Q; return true; }
 if (Name == TEXT("nf.partylootvote")) { FGrpcNightfallV1PartyLootChangeRequest Q; Q.IdempotencyKey = Key; if (!ParseEnum(TEXT("LOOT_MODE_"), A[0], Q.Mode)) return false; M.PartyLootChangeRequest = Q; return true; }
 if (Name == TEXT("nf.partylootanswer")) { FGrpcNightfallV1PartyLootChangeAnswer Q; Q.IdempotencyKey = Key; Q.Accept = A[0] == TEXT("accept"); M.PartyLootChangeAnswer = Q; return true; }
 if (Name == TEXT("nf.deposit")) { const FString Keeper = Npc(TEXT("warehouse_keeper")); if (Keeper.IsEmpty()) return false; FGrpcNightfallV1WarehouseDepositRequest Q; Q.IdempotencyKey = Key; Q.Keeper = Keeper; if (!Items(A[0], Q.Entries)) return false; M.WarehouseDeposit = Q; return true; }
 if (Name == TEXT("nf.withdraw")) { const FString Keeper = Npc(TEXT("warehouse_keeper")); if (Keeper.IsEmpty() || !bWarehouseKnown) return false; FGrpcNightfallV1WarehouseWithdrawRequest Q; Q.IdempotencyKey = Key; Q.Keeper = Keeper; if (!ContainerItems(A[0], WarehouseInstanceTemplates, Q.Entries)) return false; M.WarehouseWithdraw = Q; return true; }
 if (Name == TEXT("nf.freightsend")) { const FString Keeper = Npc(TEXT("warehouse_keeper")), T = Role(A[0]); if (Keeper.IsEmpty() || T.IsEmpty()) return false; FGrpcNightfallV1FreightSendRequest Q; Q.IdempotencyKey = Key; Q.Keeper = Keeper; Q.TargetCharacter = T; if (!Items(A[1], Q.Entries)) return false; M.FreightSend = Q; return true; }
 if (Name == TEXT("nf.freightwithdraw")) { const FString Keeper = Npc(TEXT("warehouse_keeper")); if (Keeper.IsEmpty() || !bFreightKnown) return false; FGrpcNightfallV1FreightWithdrawRequest Q; Q.IdempotencyKey = Key; Q.Keeper = Keeper; if (!ContainerItems(A[0], FreightInstanceTemplates, Q.Entries)) return false; M.FreightWithdraw = Q; return true; }
 if (Name == TEXT("nf.mailsend"))
 {
  const FString Receiver = RoleName(A[0].ToLower()); if (Receiver.IsEmpty()) return false;
  FGrpcNightfallV1MailSendRequest Q; Q.IdempotencyKey = Key; Q.ReceiverName = Receiver; Q.Subject = A[1]; int32 I = 2;
  if (A.IsValidIndex(I) && A[I] == TEXT("cod")) { Q.CodAdena = Num(A[I + 1]); I += 2; }
  if (A.IsValidIndex(I) && !Items(A[I], Q.Attachments)) return false;
  M.MailSend = Q; return true;
 }
 if (Name == TEXT("nf.mailtake") || Name == TEXT("nf.mailreject") || Name == TEXT("nf.mailread"))
 {
  FString Id; if (!Mail.IsEmpty()) Id = Mail.Last().Notice.MessageId; else if (InboxView.IsSet() && InboxView->Messages.Num()) Id = InboxView->Messages[0].MessageId; if (Id.IsEmpty()) return false;
  if (Name == TEXT("nf.mailtake")) { FGrpcNightfallV1MailTakeAttachmentsRequest Q; Q.IdempotencyKey = Key; Q.MessageId = Id; M.MailTakeAttachments = Q; }
  else if (Name == TEXT("nf.mailreject")) { FGrpcNightfallV1MailRejectRequest Q; Q.IdempotencyKey = Key; Q.MessageId = Id; M.MailReject = Q; }
  else { FGrpcNightfallV1MailMarkReadRequest Q; Q.IdempotencyKey = Key; Q.MessageId = Id; M.MailMarkRead = Q; }
  return true;
 }
 if (Name == TEXT("nf.mailcancel") || Name == TEXT("nf.maildelete"))
 {
  const bool bSent = Name == TEXT("nf.mailcancel") || A.Num() == 2; const TOptional<FGrpcNightfallV1ListMailResponse>& View = bSent ? OutboxView : InboxView;
  FString Id; if (!bSent && !Mail.IsEmpty()) Id = Mail.Last().Notice.MessageId; else if (View.IsSet() && View->Messages.Num()) Id = View->Messages[0].MessageId; if (Id.IsEmpty()) return false;
  if (Name == TEXT("nf.mailcancel")) { FGrpcNightfallV1MailCancelRequest Q; Q.IdempotencyKey = Key; Q.MessageId = Id; M.MailCancel = Q; }
  else { FGrpcNightfallV1MailDeleteRequest Q; Q.IdempotencyKey = Key; Q.MessageIds.Add(Id); Q.SentBox = bSent; M.MailDelete = Q; }
  return true;
 }
 if (Name == TEXT("nf.auctionbid")) { FGrpcNightfallV1ItemAuctionBidRequest Q; Q.IdempotencyKey = Key; Q.InstanceNpcId = uint32(Num(A[0])); Q.Bid = Num(A[1]); M.ItemAuctionBid = Q; return true; }
 if (Name == TEXT("nf.auctioncancel")) { const FString Auctioneer = Npc(TEXT("auctioneer")); if (Auctioneer.IsEmpty()) return false; FGrpcNightfallV1ItemAuctionCancelBidRequest Q; Q.IdempotencyKey = Key; Q.Auctioneer = Auctioneer; Q.InstanceNpcId = uint32(Num(A[0])); M.ItemAuctionCancelBid = Q; return true; }
 if (Name == TEXT("nf.manorbuyseed")) { const FString Manager = Npc(TEXT("manor_manager")); if (Manager.IsEmpty()) return false; FGrpcNightfallV1ManorBuySeedRequest Q; Q.IdempotencyKey = Key; Q.Manager = Manager; Q.ManorId = uint32(Num(A[1])); FGrpcNightfallV1TemplateCount T; T.TemplateId = uint32(Num(A[2])); T.Count = Num(A[3]); Q.Seeds.Add(T); M.ManorBuySeed = Q; return true; }
 if (Name == TEXT("nf.manorsellcrop"))
 {
  const FString Manager = Npc(TEXT("manor_manager")); const auto Id = Instance(A[1]); if (Manager.IsEmpty() || !Id.IsSet() || !Inventory) return false;
  const FGrpcNightfallV1OwnedItem* Item = Inventory->Find(*Id); if (!Item) return false;
  FGrpcNightfallV1ManorSellCropRequest Q; Q.IdempotencyKey = Key; Q.Manager = Manager; FGrpcNightfallV1CropSale C; C.InstanceId = *Id; C.CropId = uint32(Num(A[2])); C.ManorId = uint32(Num(A[3])); C.Count = A[4] == TEXT("all") ? Item->Count.Value : Num(A[4]); Q.Entries.Add(C); M.ManorSellCrop = Q; return true;
 }
 return false;
}
