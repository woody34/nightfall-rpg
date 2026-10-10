#pragma once
#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "PresentationFence.h"
#include "Net/SessionClientSubsystem.h"
#include "SkillStateSubsystem.generated.h"
class UNetClientSubsystem;
DECLARE_MULTICAST_DELEGATE(FOnSkillStateChanged);

struct FSkillReuseView { uint64 ReadyTick = 0; uint64 Tick = 0; };
struct FSwingView { TOptional<FAttackStarted> Swing; uint64 Tick = 0; };
struct FCastView { TOptional<FSkillCastStarted> Cast; uint64 Tick = 0; uint64 TerminalId = 0; uint32 CancelReason = 0; };

/** Received owner skills/resources and public casts/PvP. Never computes gameplay stats or costs. */
UCLASS()
class NIGHTFALL_API USkillStateSubsystem : public UGameInstanceSubsystem
{
	GENERATED_BODY()
public:
	virtual void Initialize(FSubsystemCollectionBase& Collection) override;
	virtual void Deinitialize() override;
	UFUNCTION() void Reset();
	UFUNCTION() void Disconnected(const FString& Reason) { Reset(); }
	void ApplySpawn(const FEntitySpawn& S);
	void ApplyDespawn(const FEntityDespawn& D);

	void ApplyRespawn(const FEntityRespawned& R);
	void ApplySync(const FSkillStateSync& S);
	void ApplyReuse(const FSkillReuse& R);
	void ApplyLearned(const FSkillLearned& L);
	void ApplyStats(const FStatsChanged& S);
	void ApplySp(const FSpGained& S);
	void ApplyPvp(const FPvpStateChanged& P);
	void ApplyOffer(const FResurrectOffer& O);
	void ApplyCast(const FSkillCastStarted& S);
	void ApplyResult(const FSkillCastResult& R);
	void ApplyCancel(const FSkillCastCancelled& C);
 void ApplySwing(const FAttackStarted& S);
 void ApplySwingCancel(const FAttackCancelled& C);
 void ApplyDied(const FEntityDied& D);
 const FSwingView* FindSwing(const FString& Id) const { return Swings.Find(Id.ToLower()); }
 TOptional<uint64> History(const FString& Name) const;
 TOptional<bool> ReceiptMatchesFirst() const { return bReceiptMatches; }
 const TOptional<uint32>& GetInitialHp() const { return InitialHp; }
 const TOptional<uint32>& GetInitialMp() const { return InitialMp; }
 const TOptional<uint32>& GetInitialCp() const { return InitialCp; }
 const TOptional<FSkillCastCancelled>& GetObservedCancel() const { return ObservedCancel; }
 uint64 GetOperationSerial() const { return OperationSerial; }
 bool RpcDone() const { return bRpcDone && !IsBusy(); }
	uint64 GetTick() const;
	bool IsHydrated(const FString& Entity) const { return Fence.Hydrated(Entity, GetTick()); }
	bool HasOwnerState() const { return bOwnerKnown; }
 bool HasSkillBaseline() const { return bSyncKnown; }
	const TMap<uint32, uint32>& GetLearned() const { return Learned; }
	const TOptional<uint64>& GetSp() const { return Sp; }
	uint64 GetSpGainedTotal() const { return SpGainedTotal; }
	uint32 GetDeathPenalty() const { return DeathPenalty; }
	TOptional<uint64> ReuseRemaining(uint32 Id) const;
 float CooldownFraction(uint32 Id) const;
	const FCastView* FindCast(const FString& Entity) const { return Casts.Find(Entity.ToLower()); }
	const FPvpStateChanged* FindPvp(const FString& Entity) const { return Pvp.Find(Entity.ToLower()); }
	FString Flag(const FString& Entity) const;
	const TOptional<FResurrectOffer>& GetOffer() const { return Offer; }
	bool OfferPending() const;
 uint64 GetLastOfferId() const { return LastOfferId; }
	int32 GetCastTransitions() const { return CastTransitions; }
	using FDone = TFunction<void(const FNetResult&)>;
	using FCatalogProvider = TFunction<void(USessionClient::FSkillCatalogCallback)>;
	void SetCatalogProviderForTesting(FCatalogProvider Provider) { CatalogProvider = MoveTemp(Provider); }
	void LoadCatalog(FDone Done = nullptr);
	void LoadLearnable(FDone Done = nullptr);
	void Learn(uint32 Id, uint32 Rank, FDone Done = nullptr, const FString& Key = FString());
	void ApplyCatalog(const FGrpcNightfallV1GetSkillCatalogResponse& R);
	void ApplyLearnable(const FGrpcNightfallV1ListLearnableSkillsResponse& R);
	const FGrpcNightfallV1GetSkillCatalogResponse& GetCatalog() const { return Catalog; }
	const FGrpcNightfallV1ListLearnableSkillsResponse& GetLearnable() const { return Learnable; }
	const FGrpcNightfallV1LearnSkillResponse& GetLearnReceipt() const { return LearnReceipt; }
	const FNetResult& GetLastResult() const { return LastResult; }
	const FNetResult& GetLearnResult() const { return LearnResult; }
	bool IsBusy() const { return bBusy || bCatalogBusy; }
	bool HasCatalog() const { return bCatalogKnown; }
	bool HasLearnable() const { return bLearnableKnown; }
	bool HasLearnResult() const { return bLearnDone && !bBusy; }
	const FGrpcNightfallV1SkillCatalogEntry* FindSkill(uint32 Id) const;
	TArray<uint32> Hotbar() const;
	uint32 CastSlot(int32 Index);
	bool SelectCorpse(const FString& Entity);
	void ClearCorpseSelection() { SelectedCorpse.Reset(); }
	TOptional<FString> CorpseTarget() const;
	const FString& GetLastLearnKey() const { return LastLearnKey; }
	void CompleteLearn(uint64 Serial, const FNetResult& Result, const FGrpcNightfallV1LearnSkillResponse& R, FDone Done);
	FOnSkillStateChanged OnChanged;
private:
	bool IsOwn(const FString& Id) const;
	void ClearOwner();
	void FinishCast(const FString& Caster, uint32 Inc, uint64 Id, uint64 Tick, uint32 Reason);
	UPROPERTY() TObjectPtr<UNetClientSubsystem> Net;
	UPROPERTY() TObjectPtr<USessionClient> Session;
	uint64 OperationSerial = 0;
	uint64 CatalogSerial = 0;
	bool bCatalogBusy = false;
	FCatalogProvider CatalogProvider;
	TOptional<FEntitySpawn> SelectedCorpse;
 bool bRpcDone = false;
 TMap<FString,FString> FirstReceipts;
 TOptional<bool> bReceiptMatches;
 TMap<FString,uint64> Counts;
 TOptional<uint32> InitialHp, InitialMp, InitialCp;
 TOptional<FSkillCastCancelled> ObservedCancel;
 TMap<FString,FSwingView> Swings;
	bool bBusy = false, bCatalogKnown = false, bLearnableKnown = false, bLearnDone = false;
	FGrpcNightfallV1GetSkillCatalogResponse Catalog;
	FGrpcNightfallV1ListLearnableSkillsResponse Learnable;
	FGrpcNightfallV1LearnSkillResponse LearnReceipt;
	FNetResult LastResult, LearnResult;
	FString LastLearnKey;
	FPresentationFence Fence;
	TMap<uint32, uint32> Learned;
	TMap<uint32, FSkillReuseView> Reuse;
	TMap<FString, FPvpStateChanged> Pvp;
	TMap<FString, FCastView> Casts;
	TOptional<FResurrectOffer> Offer;
 uint64 LastOfferId=0;
	TOptional<uint64> Sp;
	uint64 OwnerTick = 0;
	TSet<FString> SpGainEvents;
	uint64 SyncTick = 0, StatsTick = 0, SpTick = 0, LastSpGainTick = 0, SpGainedTotal = 0, OfferTick = 0;
	TSet<FString> LearnedEvents;
	bool bOwnerKnown = false, bSyncKnown = false, bStatsKnown = false, bSpGainKnown = false;
	uint32 DeathPenalty = 0;
	int32 CastTransitions = 0;
};
