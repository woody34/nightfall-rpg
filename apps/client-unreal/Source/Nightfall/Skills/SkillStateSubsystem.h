#pragma once
#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "PresentationFence.h"
#include "SkillStateSubsystem.generated.h"
class UNetClientSubsystem;
DECLARE_MULTICAST_DELEGATE(FOnSkillStateChanged);

struct FSkillReuseView { uint64 ReadyTick = 0; uint64 Tick = 0; };
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
	uint64 GetTick() const;
	bool HasOwnerState() const { return bOwnerKnown; }
	const TMap<uint32, uint32>& GetLearned() const { return Learned; }
	const TOptional<uint64>& GetSp() const { return Sp; }
	uint64 GetSpGainedTotal() const { return SpGainedTotal; }
	uint32 GetDeathPenalty() const { return DeathPenalty; }
	TOptional<uint64> ReuseRemaining(uint32 Id) const;
	const FCastView* FindCast(const FString& Entity) const { return Casts.Find(Entity.ToLower()); }
	const FPvpStateChanged* FindPvp(const FString& Entity) const { return Pvp.Find(Entity.ToLower()); }
	FString Flag(const FString& Entity) const;
	const TOptional<FResurrectOffer>& GetOffer() const { return Offer; }
	bool OfferPending() const;
	int32 GetCastTransitions() const { return CastTransitions; }
	FOnSkillStateChanged OnChanged;
private:
	bool IsOwn(const FString& Id) const;
	void ClearOwner();
	void FinishCast(const FString& Caster, uint32 Inc, uint64 Id, uint64 Tick, uint32 Reason);
	UPROPERTY() TObjectPtr<UNetClientSubsystem> Net;
	FPresentationFence Fence;
	TMap<uint32, uint32> Learned;
	TMap<uint32, FSkillReuseView> Reuse;
	TMap<FString, FPvpStateChanged> Pvp;
	TMap<FString, FCastView> Casts;
	TOptional<FResurrectOffer> Offer;
	TOptional<uint64> Sp;
	uint64 OwnerTick = 0;
	TSet<FString> SpGainEvents;
	uint64 SyncTick = 0, StatsTick = 0, SpTick = 0, LastSpGainTick = 0, SpGainedTotal = 0, OfferTick = 0;
	TSet<FString> LearnedEvents;
	bool bOwnerKnown = false, bSyncKnown = false, bStatsKnown = false, bSpGainKnown = false;
	uint32 DeathPenalty = 0;
	int32 CastTransitions = 0;
};
