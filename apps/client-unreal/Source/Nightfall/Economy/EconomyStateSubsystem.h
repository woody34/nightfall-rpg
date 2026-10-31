#pragma once
#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "Skills/PresentationFence.h"
#include "Net/SessionClientSubsystem.h"
#include "EconomyStateSubsystem.generated.h"
class UNetClientSubsystem;
class UInventoryStateSubsystem;
DECLARE_MULTICAST_DELEGATE(FOnEconomyStateChanged);

/** A role of a Phase 5 multi-client unit (`# role: a|b|c`): the character the bot addresses by that letter. */
struct FEconomyRoleIdentity { FString EntityId; FString Name; };

/**
 * Phase 5 E5 skeleton (stories 5.1/5.2 + 5.6 registration): the owner projection of the economy wire
 * facts (WorldEvent 34..45, EntitySpawn.ground_item) and the detached read-RPC views, with the D43
 * keyed-replay rules of Phase 4 for every economy intent. Received facts only: the client never
 * predicts counts, prices, chances, protection or party membership. No UI (5.7).
 *
 * Reset rules (phase5-grammar.py): the transport reset clears everything; the own EntitySpawn (a new
 * identity baseline) clears the generation/identity-scoped facts; the own EntityRespawned (a new owner
 * incarnation) clears the owner-incarnation facts; nf.MarkItemState re-samples the mark facts.
 */
UCLASS()
class NIGHTFALL_API UEconomyStateSubsystem : public UGameInstanceSubsystem
{
 GENERATED_BODY()
public:
 virtual void Initialize(FSubsystemCollectionBase& Collection) override;
 virtual void Deinitialize() override;
 UFUNCTION() void Reset();
 UFUNCTION() void Disconnected(const FString& Reason) { Reset(); }

 // --- Reducers (bound to UNetClientSubsystem; tests call them directly) ---
 void ApplySpawn(const FEntitySpawn& S);
 void ApplyDespawn(const FEntityDespawn& S);
 void ApplyRespawn(const FEntityRespawned& S);
 void ApplyDied(const FEntityDied& S);
 void ApplyStats(const FStatsChanged& S);
 void ApplyXp(const FXpGained& S);
 void ApplySp(const FSpGained& S);
 void ApplyEffectApplied(const FEffectApplied& S);
 void ApplyCastResult(const FSkillCastResult& S);
 void ApplyInventorySync(const FGrpcNightfallV1InventorySync& S);
 void ApplyInventoryChanged(const FGrpcNightfallV1InventoryChanged& S);
 void ApplyEquipmentChanged(const FGrpcNightfallV1EquipmentChanged& S);
 void ApplyEconomyReceipt(const FWorldEvent& E);
 void ApplyCorpse(const FGrpcNightfallV1CorpseStateChanged& S);
 void ApplyParty(const FGrpcNightfallV1PartyStateChanged& S);
 void ApplyLootVote(const FGrpcNightfallV1PartyLootVote& S);
 void ApplyInvite(const FGrpcNightfallV1PartyInviteReceived& S);
 void ApplyTradeRequest(const FGrpcNightfallV1TradeRequestReceived& S);
 void ApplyTrade(const FGrpcNightfallV1TradeSessionChanged& S);
 void ApplyStore(const FGrpcNightfallV1PrivateStoreChanged& S);
 void ApplyMail(const FGrpcNightfallV1MailNotification& S);
 void ApplyRecipeBook(const FGrpcNightfallV1RecipeBookSync& S);
 void ApplyAuction(const FGrpcNightfallV1ItemAuctionStatusChanged& S);
 void ApplyLootDistributed(const FGrpcNightfallV1LootDistributed& S);
 void ApplyAck(const FAck& A);
 void ApplyRejected(const FIntentRejected& R);
 void Marked();
 void TrackAdena();

 // --- Ground items (EntitySpawn kind 3) ---
 struct FGroundItem { FString EntityId; FGrpcNightfallV1GroundItemDetail Detail; uint64 SpawnTick = 0; FNetVec2 Position; bool bPkDrop = false; };
 struct FGoneGroundItem { FGrpcNightfallV1GroundItemDetail Detail; uint64 DespawnTick = 0; bool bPkDrop = false; };
 const TMap<FString, FGroundItem>& GroundItems() const { return Ground; }
 const TMap<FString, FGoneGroundItem>& GoneGroundItems() const { return GoneGround; }
 /** Known ground entities of a template, newest spawn first. */
 TArray<const FGroundItem*> GroundOf(uint32 Template) const;
 int32 PkDropsReceived() const { return PkDropSpawns; }
 bool PkDropsAllUnowned() const { return bPkDropsAllUnowned; }
 bool PkDropDespawned() const { return bPkDropDespawned; }

 // --- Corpses (CorpseStateChanged by template `npc_<id>`) ---
 struct FCorpse { FString EntityId; FString Template; FGrpcNightfallV1CorpseStateChanged State; uint64 FirstTick = 0; TOptional<uint64> DespawnTick; };
 const FCorpse* CorpseOf(const FString& Template) const;
 FString TemplateOf(const FString& EntityId) const;
 bool EntityGone(const FString& Template) const;

 // --- Party (baseline/delta fenced), votes, invites, loot log ---
 const TOptional<FGrpcNightfallV1PartyStateChanged>& Party() const { return PartyState; }
 const TOptional<FGrpcNightfallV1PartyLootVote>& LootVote() const { return Vote; }
 const TOptional<FGrpcNightfallV1PartyInviteReceived>& Invite() const { return PendingInvite; }
 const TArray<FGrpcNightfallV1LootDistributed>& LootLog() const { return Loot; }
 bool InParty(const FString& EntityId) const;
 FString PartyLeader() const;

 // --- Trade, stores, mail, recipes, auctions ---
 const TOptional<FGrpcNightfallV1TradeSessionChanged>& Trade() const { return TradeState; }
 const TOptional<FGrpcNightfallV1TradeRequestReceived>& TradeRequest() const { return PendingTrade; }
 const FGrpcNightfallV1PrivateStoreChanged* Store(const FString& Owner) const { return Stores.Find(Owner.ToLower()); }
 struct FMailNotice { FGrpcNightfallV1MailNotification Notice; int64 ReceivedUnixMs = 0; };
 const TArray<FMailNotice>& MailNotices() const { return Mail; }
 const TOptional<FGrpcNightfallV1ListMailResponse>& Inbox() const { return InboxView; }
 const TOptional<FGrpcNightfallV1ListMailResponse>& Outbox() const { return OutboxView; }
 const TOptional<FGrpcNightfallV1RecipeBookSync>& RecipeBook() const { return Recipes; }
 const FGrpcNightfallV1ItemAuctionStatusChanged* Auction(uint32 InstanceNpcId) const { return Auctions.Find(InstanceNpcId); }
 /** The newest ItemAuctionStatusChanged received (any auctioneer); null before the first. */
 const FGrpcNightfallV1ItemAuctionStatusChanged* LastAuction() const { return Auctions.Find(LastAuctionNpc); }
 const TOptional<FGrpcNightfallV1ListItemAuctionsResponse>& AuctionView() const { return AuctionsView; }

 // --- Economy receipts (owner-only EconomyResult) ---
 struct FReceipt { FGrpcNightfallV1EconomyResult Result; int32 Order = 0; };
 const TArray<FReceipt>& Receipts() const { return Results; }
 const FReceipt* LastReceipt() const { return Results.IsEmpty() ? nullptr : &Results.Last(); }
 const FReceipt* LastReceiptOf(EGrpcNightfallV1ItemOperation Op) const;
 int32 ReceiptCount(EGrpcNightfallV1ItemOperation Op) const;
 TOptional<bool> ReceiptMatchesFirst() const { return ReceiptMatches; }
 bool IsBusy() const { return !Outstanding.IsEmpty(); }
 const FString& Status() const { return StatusText; }
 /** Ack tick difference between the two newest accepted PrivateManufactureOrder intents. */
 TOptional<uint64> ManufactureGateTicks() const;
 bool ContainerBusyRetryObserved() const { return bContainerBusyRetry; }
 bool EconomyExportMarker() const { return !Results.IsEmpty() || bOwnKillDied; }

 // --- Owner facts ---
 const TOptional<FStatsChanged>& OwnStats() const { return Stats; }
 int32 XpEvents() const { return XpGainedEvents; }
 const TOptional<uint64>& LastXpAmount() const { return LastXp; }
 const TOptional<uint64>& LastSpAmount() const { return LastSp; }
 int32 SpEventsAfterKill(const FString& Template) const { const int32* N = SpAfterKill.Find(Template); return N ? *N : 0; }
 /** Adena (template 57) count minus the nf.MarkItemState sample; unset without a mark or baseline. */
 TOptional<int64> AdenaDelta() const;
 const TArray<uint64>& AdenaIncrementsSinceMark() const { return AdenaIncrements; }
 TOptional<int32> PkDropCount() const { return DeathRemovedIds; }
 TOptional<bool> PkUnequippedBeforeRoll() const;
 TOptional<uint32> HerbPickupTemplate() const { return HerbTemplate; }
 int32 OwnEffectsAfterHerb() const { return EffectsAfterHerb; }
 int32 CastResults(uint32 SkillId) const { const int32* N = OwnCastResults.Find(SkillId); return N ? *N : 0; }
 TOptional<uint64> LastCastResultTick(uint32 SkillId) const { const uint64* T = OwnCastResultTicks.Find(SkillId); return T ? TOptional<uint64>(*T) : TOptional<uint64>(); }
 uint64 InventoryTick() const { return OwnInventoryTick; }
 TOptional<bool> OwnOutOfPartyRange() const;

 // --- Detached read-RPC views (design §13.4): never world state ---
 const TOptional<FGrpcNightfallV1ListBuylistResponse>& Buylist() const { return BuylistView; }
 TOptional<uint64> WarehouseCount(uint32 Template) const;
 TOptional<uint64> FreightCount(uint32 Template) const;
 TOptional<uint32> FreightCapacity() const { return bFreightKnown ? TOptional<uint32>(FreightCapacityValue) : TOptional<uint32>(); }
 /** Requests a fresh view (throttled); predicates call these when their carrier is missing or stale. */
 void RefreshBuylist(); void RefreshWarehouse(); void RefreshFreight(); void RefreshMail(); void RefreshAuctions();
 const TArray<uint64>& SoldInstances() const { return Sold; }

 // --- Commands (phase5-grammar.py COMMANDS): parse, resolve received bindings, send with D43 keys ---
 bool Execute(const TArray<FString>& Tokens);
 static TOptional<EGrpcNightfallV1ItemOperation> OperationOf(const FString& CommandName);
 static FString OperationName(EGrpcNightfallV1ItemOperation Op);
 static TOptional<EGrpcNightfallV1ItemOperation> ParseOperation(const FString& Name);
 FString ResolveRole(const FString& Role) const { const auto* R = Roles.Find(Role); return R ? R->EntityId : FString(); }
 FString RoleName(const FString& Role) const { const auto* R = Roles.Find(Role); return R ? R->Name : FString(); }
 FString NpcNamed(const FString& Name) const;

 // --- Test seams ---
 void SetRolesForTesting(const TMap<FString, FEconomyRoleIdentity>& InRoles) { Roles = InRoles; }
 using FBuylistProvider = TFunction<void(const FGrpcNightfallV1ListBuylistRequest&, USessionClient::FListBuylistCallback)>;
 using FWarehouseProvider = TFunction<void(const FGrpcNightfallV1GetWarehouseRequest&, USessionClient::FGetWarehouseCallback)>;
 using FFreightProvider = TFunction<void(const FGrpcNightfallV1GetFreightRequest&, USessionClient::FGetFreightCallback)>;
 using FMailProvider = TFunction<void(const FGrpcNightfallV1ListMailRequest&, USessionClient::FListMailCallback)>;
 using FAuctionsProvider = TFunction<void(const FGrpcNightfallV1ListItemAuctionsRequest&, USessionClient::FListItemAuctionsCallback)>;
 void SetBuylistProviderForTesting(FBuylistProvider P) { BuylistProvider = MoveTemp(P); }
 void SetWarehouseProviderForTesting(FWarehouseProvider P) { WarehouseProvider = MoveTemp(P); }
 void SetFreightProviderForTesting(FFreightProvider P) { FreightProvider = MoveTemp(P); }
 void SetMailProviderForTesting(FMailProvider P) { MailProvider = MoveTemp(P); }
 void SetAuctionsProviderForTesting(FAuctionsProvider P) { AuctionsProvider = MoveTemp(P); }
 void SetClockForTesting(TFunction<double()> C) { Clock = MoveTemp(C); }
 void ApplyBuylist(const FGrpcNightfallV1ListBuylistResponse& R);
 void ApplyWarehouse(const FGrpcNightfallV1GetWarehouseResponse& R);
 void ApplyFreight(const FGrpcNightfallV1GetFreightResponse& R);
 void ApplyMailList(EGrpcNightfallV1MailBox Box, const FGrpcNightfallV1ListMailResponse& R);
 void ApplyAuctions(const FGrpcNightfallV1ListItemAuctionsResponse& R);

 FOnEconomyStateChanged OnChanged;

private:
 bool OwnerVisible(const FBytes& Id, uint32 Inc) const;
 bool Own(const FString& Id) const;
 void ResetIdentityScoped();
 void ResetIncarnationScoped();
 FString RequestScope(EGrpcNightfallV1ItemOperation Op, const FString& Key) const;
 void Remember(EGrpcNightfallV1ItemOperation Op, const FString& Key, const FString& Fingerprint, const TArray<uint8>& Payload);
 void FinishSend(uint32 Seq, const FString& Scope, EGrpcNightfallV1ItemOperation Op, const FClientMessage& Body, const FString& Arguments);
 bool Build(const FString& Name, const TArray<FString>& A, const FString& Key, FClientMessage& Out) const;
 TOptional<uint32> TemplateOfInstance(uint64 Instance) const;
 FBytes OwnCharacterId() const;
 double Now() const { return Clock ? Clock() : FPlatformTime::Seconds(); }
 bool Throttle(double& Last);

 UPROPERTY() TObjectPtr<UNetClientSubsystem> Net;
 UPROPERTY() TObjectPtr<USessionClient> Session;
 UPROPERTY() TObjectPtr<UInventoryStateSubsystem> Inventory;
 FPresentationFence Fence;
 TMap<FString, FEconomyRoleIdentity> Roles;
 TFunction<double()> Clock;

 // identity-scoped
 TMap<FString, FGroundItem> Ground;
 TMap<FString, FGoneGroundItem> GoneGround;
 int32 PkDropSpawns = 0; bool bPkDropsAllUnowned = true, bPkDropDespawned = false;
 TMap<FString, FCorpse> Corpses;
 TOptional<FGrpcNightfallV1PartyStateChanged> PartyState; uint64 PartyTick = 0;
 TOptional<FGrpcNightfallV1PartyLootVote> Vote;
 TOptional<FGrpcNightfallV1PartyInviteReceived> PendingInvite;
 TArray<FGrpcNightfallV1LootDistributed> Loot;
 TOptional<FGrpcNightfallV1TradeSessionChanged> TradeState;
 TOptional<FGrpcNightfallV1TradeRequestReceived> PendingTrade;
 TMap<FString, FGrpcNightfallV1PrivateStoreChanged> Stores;
 TMap<uint32, FGrpcNightfallV1ItemAuctionStatusChanged> Auctions;
 TArray<FMailNotice> Mail;
 bool bOwnKillDied = false; FString LastOwnKillTemplate;
 TOptional<uint64> OwnDeathTick; TOptional<int32> DeathRemovedIds; TOptional<int32> DeathEquipClearOrder, DeathRemovedOrder; int32 DeathEventOrder = 0;
 // owner-incarnation-scoped
 TArray<FReceipt> Results; int32 ReceiptOrder = 0;
 TOptional<FGrpcNightfallV1RecipeBookSync> Recipes;
 TOptional<FStatsChanged> Stats;
 int32 XpGainedEvents = 0; TOptional<uint64> LastXp, LastSp; TMap<FString, int32> SpAfterKill;
 TOptional<uint32> HerbTemplate; TOptional<uint64> HerbTick; int32 EffectsAfterHerb = 0;
 TMap<uint32, int32> OwnCastResults; TMap<uint32, uint64> OwnCastResultTicks;
 TArray<uint64> ManufactureAckTicks;
 bool bContainerBusyRetry = false; TSet<FString> BusyScopes;
 // mark-scoped
 TArray<uint64> AdenaIncrements; TOptional<uint64> LastAdena;
 // inventory mirror facts
 TMap<uint64, uint32> InstanceTemplates; uint64 OwnInventoryTick = 0;
 // D43 keyed replay
 enum class ERequestStatus : uint8 { Pending, Committed, Rejected };
 struct FSubmittedRequest { FString Scope, Arguments; EGrpcNightfallV1ItemOperation Op = EGrpcNightfallV1ItemOperation::ITEM_OPERATION_UNSPECIFIED; FClientMessage Body; ERequestStatus Status = ERequestStatus::Pending; };
 TMap<uint32, FSubmittedRequest> SequenceRequests;
 TMap<FString, TSet<uint32>> Outstanding;
 TMap<FString, FString> CommittedFingerprints;
 TMap<FString, TArray<uint8>> ReceiptBytes;
 TOptional<bool> ReceiptMatches;
 FString StatusText;
 // detached views
 TOptional<FGrpcNightfallV1ListBuylistResponse> BuylistView; FString LastMerchant; TArray<uint64> Sold;
 TMap<uint32, uint64> WarehouseCounts, FreightCounts; TMap<uint64, uint32> WarehouseInstanceTemplates, FreightInstanceTemplates;
 bool bWarehouseKnown = false, bFreightKnown = false; uint32 FreightCapacityValue = 0;
 TOptional<FGrpcNightfallV1ListMailResponse> InboxView, OutboxView; TMap<FString, FString> MailSubjects; // message id -> subject, every header ever seen
 TOptional<FGrpcNightfallV1ListItemAuctionsResponse> AuctionsView; uint32 LastAuctionNpc = 0;
 FBuylistProvider BuylistProvider; FWarehouseProvider WarehouseProvider; FFreightProvider FreightProvider; FMailProvider MailProvider; FAuctionsProvider AuctionsProvider;
 double LastBuylistRequest = -1e9, LastWarehouseRequest = -1e9, LastFreightRequest = -1e9, LastMailRequest = -1e9, LastAuctionRequest = -1e9;
 uint64 ViewSerial = 0;
};
