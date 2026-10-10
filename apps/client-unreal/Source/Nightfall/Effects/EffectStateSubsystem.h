#pragma once
#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "Skills/PresentationFence.h"
#include "EffectStateSubsystem.generated.h"

class UNetClientSubsystem;
/** Public effect summaries; pools, identities and deadlines come exclusively from wire facts. */
UCLASS()
class NIGHTFALL_API UEffectStateSubsystem : public UGameInstanceSubsystem
{
	GENERATED_BODY()
public:
	virtual void Initialize(FSubsystemCollectionBase& Collection) override;
	virtual void Deinitialize() override;
	UFUNCTION() void Reset();
	UFUNCTION() void Disconnected(const FString& Reason) { Reset(); }
	void ApplySpawn(const FEntitySpawn& S);
	void ApplyRespawn(const FEntityRespawned& R);
	void ApplyDespawn(const FEntityDespawn& D);
	void ApplyEffect(const FEffectApplied& E);
	void ApplyRemoved(const FEffectRemoved& E);
	TArray<FEffectApplied> Active(const FString& Entity) const;
	bool Has(const FString& Entity, uint32 SkillId, TOptional<uint32> Pool = {}) const;
	uint64 GetTick() const;
 bool IsKnown(const FString& Entity) const { return Fence.Identities.Contains(Entity.ToLower()); }
	TOptional<uint64> Replacements(const FString& Entity) const { return IsKnown(Entity)?TOptional<uint64>(ReplacementCounts.FindRef(Entity.ToLower())):TOptional<uint64>(); }
 const TOptional<uint32>& GetRemoveReason() const { return LastRemoveReason; }
 const TOptional<bool>& ReplacementOrdered() const { return bReplacementOrdered; }
 int32 GetTransitionCount() const { return TransitionCount; }
private:
	FPresentationFence Fence;
	TMap<FString, TMap<uint64, FEffectApplied>> Effects;
	TMap<FString, TMap<uint64, uint64>> Changes;
	int32 TransitionCount = 0;
 TMap<FString,TMap<uint64,FEffectApplied>> PendingReplacements;
 TMap<FString,uint64> ReplacementCounts;
 TOptional<uint32> LastRemoveReason;
 TOptional<bool> bReplacementOrdered;
	UPROPERTY() TObjectPtr<UNetClientSubsystem> Net;
};
