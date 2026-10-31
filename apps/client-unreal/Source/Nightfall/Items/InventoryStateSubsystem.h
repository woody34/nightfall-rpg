#pragma once
#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "Skills/PresentationFence.h"
#include "Net/SessionClientSubsystem.h"
#include "InventoryStateSubsystem.generated.h"
class UNetClientSubsystem;
DECLARE_MULTICAST_DELEGATE(FOnInventoryStateChanged);
namespace NightfallItems
{
 NIGHTFALL_API FBytes UuidBytes(const FString& Id);
 NIGHTFALL_API FString UuidString(const FBytes& Id);
 NIGHTFALL_API FString RequestFingerprint(const FClientMessage& Body, const FString& Owner);
 NIGHTFALL_API EGrpcNightfallV1EquipSlot Slot(const FString& Name);
 NIGHTFALL_API FString SlotName(EGrpcNightfallV1EquipSlot Slot);
}
/** Owner projection from committed wire facts; detached RPC views never hydrate the world. */
UCLASS()
class NIGHTFALL_API UInventoryStateSubsystem : public UGameInstanceSubsystem
{
 GENERATED_BODY()
public:
 virtual void Initialize(FSubsystemCollectionBase& Collection) override;
 virtual void Deinitialize() override;
 UFUNCTION() void Reset();
 UFUNCTION() void Disconnected(const FString& Reason) { Reset(); }
 void ApplySpawn(const FEntitySpawn& S);
 void ApplyDespawn(const FEntityDespawn& S);
 void ApplyRespawn(const FEntityRespawned& S);
 void ApplySync(const FGrpcNightfallV1InventorySync& S);
 void ApplyChanged(const FGrpcNightfallV1InventoryChanged& S);
 void ApplyEquipment(const FGrpcNightfallV1EquipmentChanged& S);
 void ApplyWeight(const FGrpcNightfallV1WeightChanged& S);
 void ApplyPenalty(const FGrpcNightfallV1PenaltyChanged& S);
 void ApplyEnchant(const FGrpcNightfallV1EnchantResult& S);
 void ApplyRefine(const FGrpcNightfallV1RefineResult& S);
 void ApplyOperation(const FGrpcNightfallV1ItemOperationResult& S);
 void ApplyReceiptPayload(const FWorldEvent& E);
 void ApplyStats(const FStatsChanged& S);
 void ApplyRejected(const FIntentRejected& R);
 bool HasBaseline() const { return bBaseline && !bResync; }
 bool NeedsBaseline() const { return bResync; }
 uint64 Revision() const { return Inventory.InventoryRevision; }
 const FGrpcNightfallV1InventorySync& GetInventory() const { return Inventory; }
 const TMap<uint64,FGrpcNightfallV1OwnedItem>& GetItems() const { return Items; }
 const FGrpcNightfallV1OwnedItem* Find(uint64 Id) const { return Items.Find(Id); }
 const FGrpcNightfallV1OwnedItem* Equipped(EGrpcNightfallV1EquipSlot Slot) const;
 TOptional<uint64> Count(uint32 Template) const;
 TOptional<uint64> Resolve(const FString& Selector) const;
 bool AutoShot(uint32 Template) const;
 bool EnchantConsistent() const;
 TOptional<bool> RefineConsistent() const;
 bool MarkItemState();
 TOptional<uint64> RevisionSinceMark() const;
 TOptional<FString> OwnerStatDelta(const FString& Name) const;
 TOptional<bool> OwnerStatsMatchMark() const;
 const TOptional<FGrpcNightfallV1EnchantResult>& GetEnchant() const { return Enchant; }
 const TOptional<FGrpcNightfallV1RefineResult>& GetRefine() const { return Refine; }
 void SetHotbarConfigForTesting(const FString& Path) {HotbarConfigOverride=Path;Hotbars.Reset();}
 bool AssignHotbar(int32 Index, uint32 Template, bool Auto);
 uint32 ActivateHotbar(int32 Index);
 TArray<TPair<uint32,bool>> Hotbar() const;
 const TOptional<EGrpcNightfallV1ItemOperation>& LastOperation() const { return Operation; }
 const TArray<FGrpcNightfallV1VisibleEquipment>* Visible(const FString& Entity) const { return Equipment.Find(Entity.ToLower()); }
 FString Blacksmith() const;
 TOptional<FGrpcNightfallV1ItemTarget> Target(const FString& Selector, bool bNpc = false) const;
 bool Execute(const TArray<FString>& Tokens);
 using FCatalogProvider = TFunction<void(USessionClient::FGetItemCatalogCallback)>;
 void SetCatalogProviderForTesting(FCatalogProvider P) { CatalogProvider = MoveTemp(P); }
 void LoadCatalog();
 void ApplyCatalog(const FGrpcNightfallV1GetItemCatalogResponse& R);
 bool HasCatalog() const { return bCatalog; }
 const FGrpcNightfallV1ItemDefinition* Definition(uint32 Id) const;
 const FGrpcNightfallV1GetItemCatalogResponse& GetCatalog() const { return Catalog; }
 void LoadDetached(const FString& Character);
 void ApplyDetached(const FString& Character,const FGrpcNightfallV1GetInventoryResponse& R);
 const FGrpcNightfallV1InventorySync* Detached(const FString& Character) const;
 using FInventoryProvider = TFunction<void(const FGrpcNightfallV1GetInventoryRequest&,USessionClient::FGetInventoryCallback)>;
 void SetInventoryProviderForTesting(FInventoryProvider P) { InventoryProvider=MoveTemp(P); }
 void Grant(uint32 Template, uint64 Count, const FString& Key);
 using FGrantProvider = TFunction<void(const FGrpcNightfallV1GrantItemRequest&,USessionClient::FGrantItemCallback)>;
 void SetGrantProviderForTesting(FGrantProvider P) { GrantProvider=MoveTemp(P); }
 const FNetResult& RpcResult() const { return LastRpc; }
 bool RpcDone() const { return bRpcDone; }
 bool IsBusy() const { return bRpcBusy || !Outstanding.IsEmpty(); }
 TOptional<bool> ReceiptMatchesFirst() const { return ReceiptMatches; }
 const FString& Status() const { return StatusText; }
 FOnInventoryStateChanged OnChanged;
 /** nf.MarkItemState accepted: Phase 5 observers (adena deltas) sample their marks here. */
 FOnInventoryStateChanged OnMarked;
 /** Received count of a template at the newest accepted mark; unset without a mark for this character. */
 TOptional<uint64> MarkedCount(uint32 Template) const;
private:
 bool StatsSynchronized() const;
 bool Owner(const FBytes& Id) const;
 bool Accept(const FBytes& Id, uint32 Inc, uint64 Tick, uint64 Revision, bool bReceipt = false);
 void ClearOwner();
 void Gap();
 bool Validate(const TMap<uint64,FGrpcNightfallV1OwnedItem>& Candidate) const;
 void Publish(const TMap<uint64,FGrpcNightfallV1OwnedItem>& Candidate);
 void Remember(EGrpcNightfallV1ItemOperation Op, const FString& Key, const FString& Fingerprint, const TArray<uint8>& Payload);
 void FinishSend(uint32 Seq,const FString& Key,EGrpcNightfallV1ItemOperation Op,const FClientMessage& Body,const FString& Arguments,const FString& Fingerprint);
 FString RequestScope(EGrpcNightfallV1ItemOperation Op,const FString& Key) const;
 void AuthenticationChanged();
 UPROPERTY() TObjectPtr<UNetClientSubsystem> Net;
 UPROPERTY() TObjectPtr<USessionClient> Session;
 FPresentationFence Fence;
 TMap<FString,TArray<FGrpcNightfallV1VisibleEquipment>> Equipment;
 TMap<FString,uint64> EquipmentTicks;
 FGrpcNightfallV1InventorySync Inventory;
 TMap<uint64,FGrpcNightfallV1OwnedItem> Items;
 TMap<uint64,FGrpcNightfallV1ItemWeightState> PendingWeights;
 TMap<uint64,FGrpcNightfallV1ItemPenaltyState> PendingPenalties;
 uint64 WeightTick=0,PenaltyTick=0;
 uint64 OwnerTick = 0, CatalogSerial = 0, RpcSerial = 0, GrantSerial = 0;
 bool bBaseline = false, bResync = false, bCatalog = false, bRpcDone = false, bRpcBusy = false;
 TMap<FString,TSet<uint32>> Outstanding;
 enum class ERequestStatus : uint8 { Pending, Committed, Rejected };
 struct FSubmittedRequest { FString Scope,Arguments,Fingerprint; FClientMessage Body; ERequestStatus Status=ERequestStatus::Pending; };
 TMap<uint32,FSubmittedRequest> SequenceRequests;
 uint32 LastSendSeq=0;
 uint64 SyncTick=0;
 FGrpcNightfallV1GetItemCatalogResponse Catalog;
 FCatalogProvider CatalogProvider;
 struct FDetachedView { uint64 Generation=0, Serial=0; TOptional<FGrpcNightfallV1InventorySync> Inventory; };
 TMap<FString,FDetachedView> DetachedViews;
 FInventoryProvider InventoryProvider;
 FGrantProvider GrantProvider;
 TOptional<FGrpcNightfallV1EnchantResult> Enchant;
 TOptional<FGrpcNightfallV1RefineResult> Refine;
 TOptional<FStatsChanged> CurrentStats,MarkedStats;
 TOptional<uint64> MarkRevision;
 FString MarkOwner;
 TMap<uint32,uint64> MarkCounts;
 TMap<FString,FString> CommittedFingerprints;
 TMap<FString,TArray<TPair<uint32,bool>>> Hotbars;
 TOptional<EGrpcNightfallV1ItemOperation> Operation;
 TMap<FString,TArray<uint8>> Receipts;
 TOptional<bool> ReceiptMatches;
 FNetResult LastRpc;
 FString StatusText,HotbarConfigOverride;
 FString HotbarConfigFile() const;
};
