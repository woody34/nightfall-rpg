#pragma once
#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "Skills/PresentationFence.h"
#include "SkillAuditSubsystem.generated.h"
class UNetClientSubsystem;

/** Scenario evidence from received facts. Checks transport/identity/order, never gameplay formulas. */
UCLASS()
class NIGHTFALL_API USkillAuditSubsystem : public UGameInstanceSubsystem
{
 GENERATED_BODY()
public:
 virtual void Initialize(FSubsystemCollectionBase& C) override;
 virtual void Deinitialize() override;
 UFUNCTION() void Connected();
 UFUNCTION() void Disconnected(const FString& Reason);
 void Reset();
 void Observe(const FWorldEvent& E);
 void ObserveAck(const FAck& A);
 void ObserveRejected(const FIntentRejected& R);
 void ObserveSent(const TArray<uint8>& Bytes);
 TOptional<bool> Check(const FString& Name) const;
 TOptional<uint64> Number(const FString& Name) const;
 TOptional<uint32> OfferState() const;
 TOptional<bool> MpCostMatches(uint32 Skill) const;
 TOptional<bool> CancelledCostsRetained() const;
 TOptional<bool> CastFor(const FString& Entity,uint32 Skill,const FString& Predicate) const;
 TOptional<bool> NotInView(const FString& Entity) const;
 const TOptional<FStatsChanged>& OwnerStats() const { return Stats; }
 const FAttackStarted* SwingFor(const FString& Id) const { return Swings.Find(Id.ToLower()); }
private:
 struct FCastEvidence
 {
  FSkillCastStarted Start;
  TOptional<FSkillCastStarted> Hydration;
  uint32 Terminals=0,Reason=0;
  FString Terminal;
  uint64 TerminalTick=0,SeenTick=0;
  bool bIdentityOk=true;
  TOptional<uint32> HpAtStart,CpAtStart;
  TOptional<FSkillReuse> ReuseAtCancel;
  uint64 TargetFlagAtStart=0;
 };
 struct FImpact { FString Attacker,Target; uint32 Inc=0; uint64 Tick=0; uint64 StartTick=0; FPvpStateChanged AttackerBefore,TargetBefore; uint32 HpBefore=0,HpAfter=0,Cp=0; uint64 StartFlagDeadline=0; };
 struct FResourceDamage { uint64 Tick=0; uint32 Cp=0; uint32 Hp=0; uint32 HpAfter=0; };
 void ClearTransport();
 bool AuditSubject(const FString& Id,TOptional<uint32> Inc,uint64 Tick,bool Baseline=false,uint64 BaselineTick=0);
 bool Reference(const TOptional<FString>& Id,const TOptional<uint32>& Inc);
 void Impact(const FString& Attacker,const FString& Target,uint32 Inc,uint64 Tick,uint64 Start,uint32 Hp,uint32 Cp,uint32 Removed);
 void PvpCopy(const FPvpStateChanged& P,bool Private);
 void ComparePvp(const FString& Id);
 void StatsInput(const FStatsChanged& S);
 bool RevivalTerminal(const FResurrectOffer& O) const;
 void CompleteResurrectionAudit();
 FString CastKey(const FString& Id,uint32 Inc,uint64 Cast) const;
 UPROPERTY() TObjectPtr<UNetClientSubsystem> Net;
 TArray<TFunction<void()>> Unbinders;
 FPresentationFence Fence;
 TOptional<FStatsChanged> Stats;
 TOptional<FStatsChanged> BeforeDeath,DeadResources,AfterRevival;
 bool bSkillRevival=false;
 TOptional<uint32> TownRequest;
 struct FKarmaChange { FPvpStateChanged Before,After; };
 TOptional<FKarmaChange> KarmaGainChange,KarmaLossChange;
 struct FXpCredit { uint64 Amount=0,Tick=0; uint32 Level=0; };
 TOptional<FXpCredit> XpCredit;
 TMap<FString,FEntityDied> Deaths;
 TMap<FString,FPvpStateChanged> SwingTargetsAtStart;
 TMap<uint64,FEffectApplied> PriorEffects;
 bool bPriorEffectsKnown=false;
 TArray<FStatsChanged> ResourceSamples;
 TMap<uint32,FSkillReuse> Reuse;
 TOptional<FSkillStateSync> Sync;
 TOptional<FResurrectOffer> Offer;
 TOptional<FEntityDied> Death;
 TOptional<uint32> PriorPenalty;
 FString PriorOwner;
 TMap<FString,FPvpStateChanged> PublicPvp,PrivatePvp;
 TMap<FString,FCastEvidence> Casts;
 TSet<FString> Departed;
 TMap<FString,FAttackStarted> Swings;
 TMap<FString,uint64> SwingTicks;
 TMap<FString,FVitalsChanged> Vitals;
 TMap<FString,FEntityRespawned> Revivals;
 TSet<FString> PairedRevivals;
 TMap<FString,TMap<uint64,FEffectApplied>> Effects;
 TMap<FString,uint64> EffectTicks;
 TSet<FString> RemovedEffects;
 TSet<FString> DamageIds;
 TArray<FResourceDamage> PendingCp;
 TOptional<FImpact> LastImpact;
 TMap<FString,uint32> DeathIncarnations;
 TSet<uint64> RefundedOffers;
 TMap<uint32,uint32> SentSkills;
 TMap<uint32,uint64> AcceptedSeqs;
 TOptional<uint32> PeaceRejectSeq;
 TMap<uint32,uint32> SentStartCounts, SentSwingCounts;
 uint32 OwnStartCount=0, OwnSwingStartCount=0;
 uint64 OwnCp=0,PlayerCp=0;
 uint32 References=0,PublicPairs=0,RevivalPairs=0,PvpPairs=0,CpPairs=0,EffectSamples=0;
 bool bReferencesOk=true,bVitalsOk=true,bPvpOk=true,bCpOk=true,bEffectsOk=true,bDispositionOk=true;
 bool bSamplesComplete=true,bBaselineOk=true;
 bool bOwnerStatsBaseline=false,bOwnerSyncBaseline=false,bObserverBaseline=false;
};
