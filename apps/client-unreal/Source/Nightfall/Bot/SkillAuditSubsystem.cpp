#include "SkillAuditSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "NightfallWire.h"
#include "Combat/CombatStateSubsystem.h"
#include "Engine/GameInstance.h"
#include "Skills/SkillStateSubsystem.h"
#include "BotFixtureData.h"
#include "SkillOracleData.h"

void USkillAuditSubsystem::Initialize(FSubsystemCollectionBase& C)
{
 C.InitializeDependency<UCombatStateSubsystem>();Super::Initialize(C);Net=C.InitializeDependency<UNetClientSubsystem>();
 // Typed delegates also exercise the audit in codec-independent automation fixtures.
#define AUDIT_EVENT(Delegate,Type,Field) {const auto H=Net->Delegate.AddWeakLambda(this,[this](const Type& V){FWorldEvent E;E.Field=V;Observe(E);});Unbinders.Add([this,H]{Net->Delegate.Remove(H);});}
 AUDIT_EVENT(OnEntitySpawn,FEntitySpawn,Spawn)
 AUDIT_EVENT(OnEntityDespawn,FEntityDespawn,Despawn)
 AUDIT_EVENT(OnEntityDied,FEntityDied,EntityDied)
 AUDIT_EVENT(OnEntityRespawned,FEntityRespawned,EntityRespawned)
 AUDIT_EVENT(OnStatsChanged,FStatsChanged,StatsChanged)
 AUDIT_EVENT(OnSkillStateSync,FSkillStateSync,SkillStateSync)
 AUDIT_EVENT(OnSkillReuse,FSkillReuse,SkillReuse)
 AUDIT_EVENT(OnPvpStateChanged,FPvpStateChanged,PvpStateChanged)
 AUDIT_EVENT(OnVitalsChanged,FVitalsChanged,VitalsChanged)
 AUDIT_EVENT(OnSkillCastStarted,FSkillCastStarted,SkillCastStarted)
 AUDIT_EVENT(OnSkillCastResult,FSkillCastResult,SkillCastResult)
 AUDIT_EVENT(OnSkillCastCancelled,FSkillCastCancelled,SkillCastCancelled)
 AUDIT_EVENT(OnAttackStarted,FAttackStarted,AttackStarted)
 AUDIT_EVENT(OnAttackCancelled,FAttackCancelled,AttackCancelled)
 AUDIT_EVENT(OnAttackResult,FAttackResult,AttackResult)
 AUDIT_EVENT(OnEffectApplied,FEffectApplied,EffectApplied)
 AUDIT_EVENT(OnEffectRemoved,FEffectRemoved,EffectRemoved)
 AUDIT_EVENT(OnResurrectOffer,FResurrectOffer,ResurrectOffer)
 AUDIT_EVENT(OnXpGained,FXpGained,XpGained)
#undef AUDIT_EVENT
 Net->OnIntentAck.AddUObject(this,&USkillAuditSubsystem::ObserveAck);
 Net->OnIntentRejected.AddUObject(this,&USkillAuditSubsystem::ObserveRejected);
 Net->OnWireSent.AddUObject(this,&USkillAuditSubsystem::ObserveSent);
 Net->OnConnected.AddDynamic(this,&USkillAuditSubsystem::Connected);
 Net->OnDisconnected.AddDynamic(this,&USkillAuditSubsystem::Disconnected);
}
void USkillAuditSubsystem::Deinitialize()
{
 for(const auto& U:Unbinders)U();Unbinders.Reset();
 if(Net){Net->OnIntentAck.RemoveAll(this);Net->OnIntentRejected.RemoveAll(this);Net->OnWireSent.RemoveAll(this);Net->OnConnected.RemoveAll(this);Net->OnDisconnected.RemoveAll(this);}
 Super::Deinitialize();
}
void USkillAuditSubsystem::Reset(){ClearTransport();PriorPenalty.Reset();PriorOwner.Reset();PriorEffects.Reset();bPriorEffectsKnown=false;}
void USkillAuditSubsystem::ClearTransport()
{
 Fence.Reset();Stats.Reset();BeforeDeath.Reset();DeadResources.Reset();AfterRevival.Reset();bSkillRevival=false;TownRequest.Reset();KarmaGainChange.Reset();KarmaLossChange.Reset();XpCredit.Reset();Deaths.Reset();SwingTargetsAtStart.Reset();ResourceSamples.Reset();Reuse.Reset();bSamplesComplete=bBaselineOk=true;Sync.Reset();Offer.Reset();Death.Reset();PublicPvp.Reset();PrivatePvp.Reset();Casts.Reset();Departed.Reset();Swings.Reset();SwingTicks.Reset();Vitals.Reset();Revivals.Reset();PairedRevivals.Reset();Effects.Reset();EffectTicks.Reset();RemovedEffects.Reset();DamageIds.Reset();PendingCp.Reset();LastImpact.Reset();DeathIncarnations.Reset();RefundedOffers.Reset();SentSkills.Reset();SentStartCounts.Reset();SentSwingCounts.Reset();OwnStartCount=OwnSwingStartCount=0;AcceptedSeqs.Reset();PeaceRejectSeq.Reset();
 OwnCp=PlayerCp=0;References=PublicPairs=RevivalPairs=PvpPairs=CpPairs=EffectSamples=0;
 bReferencesOk=bVitalsOk=bPvpOk=bCpOk=bEffectsOk=bDispositionOk=true;
 bOwnerStatsBaseline=bOwnerSyncBaseline=bObserverBaseline=false;
}
void USkillAuditSubsystem::Connected(){ClearTransport();}
void USkillAuditSubsystem::Disconnected(const FString& Reason)
{
 if(Stats.IsSet() && (!Sync.IsSet() || Stats->Tick>=Sync->Tick)){PriorPenalty=Stats->DeathPenaltyLevel;PriorOwner=Stats->Entity.ToLower();}
 else if(Sync.IsSet() && Net){PriorPenalty=Sync->DeathPenaltyLevel;PriorOwner=Net->GetOwnEntityId().ToLower();}
 PriorEffects.Reset();bPriorEffectsKnown=false;
 if(Net && Fence.Identities.Contains(Net->GetOwnEntityId().ToLower()))
 {
  if(const auto* Rows=Effects.Find(Net->GetOwnEntityId().ToLower()))for(const auto& Pair:*Rows)if(!Pair.Value.EndTick || Pair.Value.EndTick>Net->GetLatestReceivedTick())PriorEffects.Add(Pair.Key,Pair.Value);
  bPriorEffectsKnown=true;
 }
 ClearTransport();
}
FString USkillAuditSubsystem::CastKey(const FString& Id,uint32 Inc,uint64 Cast) const{return FString::Printf(TEXT("%s|%u|%llu"),*Id.ToLower(),Inc,Cast);}
bool USkillAuditSubsystem::Reference(const TOptional<FString>& Id,const TOptional<uint32>& Inc)
{
 ++References;const bool Ok=Id.IsSet()==Inc.IsSet() && (!Id.IsSet() || Fence.Visible(*Id,*Inc));bReferencesOk &= Ok;return Ok;
}
bool USkillAuditSubsystem::AuditSubject(const FString& Id,TOptional<uint32> Inc,uint64 Tick,bool Baseline,uint64 BaselineTick)
{
 const auto* Known=Fence.Identities.Find(Id.ToLower());if(!Known)Known=Fence.Tombstones.Find(Id.ToLower());
 // Historical attack frames have no subject incarnation; only an admitted identity can supply it.
 if(!Inc.IsSet() && Known)Inc=Known->LifeIncarnation;
 // Delayed old-life/old-baseline frames are stale, not evidence of invalid delivery ordering.
 if(Known && (*Inc<Known->LifeIncarnation || (*Inc==Known->LifeIncarnation && (Baseline?BaselineTick<Known->StateTick:Tick<=Known->StateTick))))return false;
 Reference(Id,Inc);return true;
}
void USkillAuditSubsystem::ComparePvp(const FString& Id)
{
 const auto* A=PublicPvp.Find(Id.ToLower());const auto* B=PrivatePvp.Find(Id.ToLower());
 if(!A || !B || A->Tick!=B->Tick || A->Incarnation!=B->Incarnation)return;
 ++PvpPairs;bPvpOk &= A->Karma==B->Karma && A->PkCount==B->PkCount && A->PvpCount==B->PvpCount && A->FlagUntilTick==B->FlagUntilTick;
}
void USkillAuditSubsystem::PvpCopy(const FPvpStateChanged& P,bool Private)
{
 if(!Fence.Accept(P.Entity,P.Incarnation,P.Tick,P.bBaseline,P.BaselineTick))return;
 auto& Map=Private?PrivatePvp:PublicPvp;const auto* Old=Map.Find(P.Entity.ToLower());if(Old && Old->Tick>P.Tick)return;
 if(!Private && Old && Old->Incarnation==P.Incarnation && Old->Tick<P.Tick && Net && Net->IsOwnEntity(P.Entity))
 {if(P.Karma>Old->Karma && uint64(P.PkCount)==uint64(Old->PkCount)+1)KarmaGainChange=FKarmaChange{*Old,P};if(P.Karma<Old->Karma)KarmaLossChange=FKarmaChange{*Old,P};}
 Map.Add(P.Entity.ToLower(),P);ComparePvp(P.Entity);Fence.Tick=FMath::Max(Fence.Tick,P.Tick);
 if(P.bBaseline && Net && !Net->IsOwnEntity(P.Entity))bObserverBaseline=true;
}
void USkillAuditSubsystem::StatsInput(const FStatsChanged& S)
{
 if(!Net || !Net->IsOwnEntity(S.Entity)) {bPvpOk=false;return;}
 if(!Fence.Accept(S.Entity,S.Incarnation,S.Tick,S.bBaseline,S.BaselineTick) || (Stats.IsSet() && S.Tick<=Stats->Tick))return;
 if(S.Hp==0)
 {
  if(!DeadResources.IsSet() || DeadResources->Incarnation!=S.Incarnation)
  {AfterRevival.Reset();bSkillRevival=false;TownRequest.Reset();if(Stats.IsSet() && Stats->Hp>0 && Stats->Incarnation==S.Incarnation)BeforeDeath=*Stats;}
  DeadResources=S;
 }
 else if(!AfterRevival.IsSet() && DeadResources.IsSet() && S.Incarnation>DeadResources->Incarnation)
 {AfterRevival=S;}
 CompleteResurrectionAudit();
 if(ResourceSamples.Num()<1024)ResourceSamples.Add(S);else bSamplesComplete=false;
 Stats=S;bOwnerStatsBaseline |= S.bBaseline;
 FPvpStateChanged P;P.Entity=S.Entity;P.Incarnation=S.Incarnation;P.Tick=S.Tick;P.bBaseline=S.bBaseline;P.BaselineTick=S.BaselineTick;P.FlagUntilTick=S.FlagUntilTick;P.Karma=S.Karma;P.PkCount=S.PkCount;P.PvpCount=S.PvpCount;PvpCopy(P,true);
}
bool USkillAuditSubsystem::RevivalTerminal(const FResurrectOffer& O) const
{
 // Only the retained corpse offer may cross the life fence. All other old-life frames stay stale.
 if(!Net || !Net->IsOwnEntity(O.Target) || O.State!=4 || O.bBaseline || !Offer.IsSet() || Offer->OfferId!=O.OfferId || Offer->TargetIncarnation!=O.TargetIncarnation || !Offer->Target.Equals(O.Target,ESearchCase::IgnoreCase) || Offer->State!=1 || O.Tick<Offer->Tick)return false;
 const auto* R=Revivals.Find(O.Target.ToLower());
 return R && R->Tick==O.Tick && R->Incarnation>O.TargetIncarnation && Fence.Visible(O.Target,R->Incarnation) && !Fence.Dead(O.Target);
}
void USkillAuditSubsystem::CompleteResurrectionAudit()
{
 if(!Offer.IsSet() || Offer->State!=4 || !DeadResources.IsSet() || !AfterRevival.IsSet())return;
 const auto& O=*Offer;const auto& D=*DeadResources;const auto& A=*AfterRevival;
 const auto* R=Revivals.Find(O.Target.ToLower());
 if(!R || R->Tick!=O.Tick || A.Tick!=O.Tick || R->Incarnation!=A.Incarnation || D.Incarnation!=O.TargetIncarnation || A.Incarnation<=D.Incarnation || D.Hp!=0 || A.Hp==0 || !D.Entity.Equals(O.Target,ESearchCase::IgnoreCase) || !A.Entity.Equals(O.Target,ESearchCase::IgnoreCase))return;
 bSkillRevival=true;
 // This counts a witnessed XP increase only. Exact stored-loss/formula proof belongs to replay (C11).
 if(A.Xp>D.Xp)RefundedOffers.Add(O.OfferId);
}
void USkillAuditSubsystem::Impact(const FString& Attacker,const FString& Target,uint32 Inc,uint64 Tick,uint64 Start,uint32 Hp,uint32 Cp,uint32 Removed)
{
 if(!Fence.Accept(Target,Inc,Tick))return;
 // Caller supplies a cast/swing-specific key before this function, so AoE rows are independent.
 const auto* T=Fence.Identities.Find(Target.ToLower());const auto* A=Fence.Identities.Find(Attacker.ToLower());
 if(!T || !A)return;
 FImpact I;I.Attacker=Attacker.ToLower();I.Target=Target.ToLower();I.Inc=Inc;I.Tick=Tick;I.StartTick=Start;I.HpBefore=Vitals.Contains(I.Target)?Vitals[I.Target].Hp:T->Hp;I.HpAfter=Hp;I.Cp=Cp;
 if(const auto* P=PublicPvp.Find(I.Attacker))I.AttackerBefore=*P;
 if(const auto* P=PublicPvp.Find(I.Target))I.TargetBefore=*P;
 if(A->Kind==1 && T->Kind==1)LastImpact=I;
 if(Net && Net->IsOwnEntity(Target) && A->Kind==1){OwnCp+=Cp;PendingCp.Add({Tick,Cp,Removed,Hp});}
 if(Net && Net->IsOwnEntity(Attacker) && T->Kind==1)PlayerCp+=Cp;

}
void USkillAuditSubsystem::Observe(const FWorldEvent& E)
{
 if(E.Spawn.IsSet())
 {
  const auto& S=*E.Spawn;
  if(const auto* Old=Fence.Identities.Find(S.EntityId.ToLower());Old && Net && Net->IsOwnEntity(S.EntityId) && !NightfallProto::IsStaleSpawn(*Old,S) && S.SessionGeneration>Old->SessionGeneration)Disconnected(TEXT("replacement"));
  if(!Fence.Spawn(S))return;
  Departed.Remove(S.EntityId.ToLower());PublicPvp.Remove(S.EntityId.ToLower());PrivatePvp.Remove(S.EntityId.ToLower());Effects.Remove(S.EntityId.ToLower());Swings.Remove(S.EntityId.ToLower());SwingTicks.Remove(S.EntityId.ToLower());
  FVitalsChanged V;V.Entity=S.EntityId;V.Incarnation=S.LifeIncarnation;V.Tick=S.StateTick;V.Hp=S.Hp;V.MaxHp=S.MaxHp;Vitals.Add(S.EntityId.ToLower(),V);
  FPvpStateChanged P;P.Entity=S.EntityId;P.Incarnation=S.LifeIncarnation;P.Tick=P.BaselineTick=S.StateTick;P.bBaseline=true;P.FlagUntilTick=S.FlagUntilTick;P.Karma=S.Karma;P.PkCount=S.PkCount;P.PvpCount=S.PvpCount;PublicPvp.Add(S.EntityId.ToLower(),P);
  // V5 identity frames must not contain reference-bearing legacy hydration.
  bReferencesOk &= S.ActiveEffects.IsEmpty() && !S.PendingCast.IsSet() && !S.PendingSwing.IsSet();
  return;
 }
 if(E.Despawn.IsSet()){const FString Id=E.Despawn->EntityId.ToLower();if(Fence.Identities.Contains(Id))Departed.Add(Id);Fence.Despawn(Id);Swings.Remove(Id);SwingTicks.Remove(Id);Effects.Remove(Id);PublicPvp.Remove(Id);PrivatePvp.Remove(Id);return;}
 if(E.EntityDied.IsSet())
 {
  const auto& D=*E.EntityDied;const auto* Old=Deaths.Find(D.Entity.ToLower());if(Old && Old->Incarnation==D.Incarnation && Old->Tick>=D.Tick)return;if(!Fence.Died(D))return;DeathIncarnations.Add(D.Entity.ToLower(),D.Incarnation);Swings.Remove(D.Entity.ToLower());
  Deaths.Add(D.Entity.ToLower(),D);if(Net && Net->IsOwnEntity(D.Entity)){Death=D;AfterRevival.Reset();bSkillRevival=false;TownRequest.Reset();if(Stats.IsSet() && Stats->Hp>0){BeforeDeath=*Stats;DeadResources.Reset();}}return;
 }
 if(E.EntityRespawned.IsSet()){const auto& R=*E.EntityRespawned;if(!Fence.Respawn(R))return;Revivals.Add(R.Entity.ToLower(),R);Swings.Remove(R.Entity.ToLower());Effects.Remove(R.Entity.ToLower());return;}
 if(E.StatsChanged.IsSet()){StatsInput(*E.StatsChanged);return;}
 if(E.SkillStateSync.IsSet())
 {
  const auto& S=*E.SkillStateSync;if(!Net || !Net->IsOwnEntity(S.Entity)){bPvpOk=false;return;}
  if(!Fence.Accept(S.Entity,S.Incarnation,S.Tick,S.bBaseline,S.BaselineTick) || (Sync.IsSet() && S.Tick<=Sync->Tick))return;
  Sync=S;bOwnerSyncBaseline |= S.bBaseline;
  FPvpStateChanged P;P.Entity=S.Entity;P.Incarnation=S.Incarnation;P.Tick=S.Tick;P.bBaseline=S.bBaseline;P.BaselineTick=S.BaselineTick;P.Karma=S.Karma;P.PkCount=S.PkCount;P.PvpCount=S.PvpCount;P.FlagUntilTick=S.FlagUntilTick;PvpCopy(P,true);return;
 }
 if(E.SkillReuse.IsSet())
 {
  const auto& R=*E.SkillReuse;if(Net && Net->IsOwnEntity(R.Entity) && Fence.Accept(R.Entity,R.Incarnation,R.Tick))
  {const auto* Old=Reuse.Find(R.SkillId);if(!Old || R.Tick>Old->Tick)Reuse.Add(R.SkillId,R);}return;
 }
 if(E.PvpStateChanged.IsSet()){PvpCopy(*E.PvpStateChanged,false);return;}
 if(E.VitalsChanged.IsSet())
 {
  const auto& V=*E.VitalsChanged;if(!Fence.Accept(V.Entity,V.Incarnation,V.Tick,V.bBaseline,V.BaselineTick))return;
  const FString Id=V.Entity.ToLower();if(const auto* Old=Vitals.Find(Id);Old && Old->Tick>V.Tick)return;
  ++PublicPairs;
  if(const auto* R=Revivals.Find(Id);R && R->Tick==V.Tick && R->Incarnation==V.Incarnation && V.bBaseline)
  {
   // Public vitals arrive before the old-corpse terminal. Retain the life transition for correlation.
   const FString Key=CastKey(Id,R->Incarnation,R->Tick);
   if(!PairedRevivals.Contains(Key)){++RevivalPairs;bVitalsOk &= V.MaxHp>0;PairedRevivals.Add(Key);}
  }
  Vitals.Add(Id,V);Fence.Tick=FMath::Max(Fence.Tick,V.Tick);return;
 }
 if(E.SkillCastStarted.IsSet())
 {
  const auto& S=*E.SkillCastStarted;
  if(!AuditSubject(S.Caster,S.CasterIncarnation,S.Tick,S.bBaseline,S.BaselineTick))return;
  const bool ValidReference=Reference(S.Target,S.TargetIncarnation);
  if(!Fence.Accept(S.Caster,S.CasterIncarnation,S.Tick,S.bBaseline,S.BaselineTick))return;
  const bool ValidBaseline=!S.bBaseline || (S.StartTick<S.BaselineTick && S.BaselineTick<S.CompleteTick);bBaselineOk &= ValidBaseline;const FString K=CastKey(S.Caster,S.CasterIncarnation,S.CastId);
  if(!Casts.Contains(K))
  {FCastEvidence C;
   if(S.Target.IsSet())
   {const auto* V=Vitals.Find(S.Target->ToLower());if(V && Fence.Visible(*S.Target,S.TargetIncarnation.Get(0)))C.HpAtStart=V->Hp;const auto* P=PublicPvp.Find(S.Target->ToLower());if(P)C.TargetFlagAtStart=P->FlagUntilTick;
    if(Net && Net->IsOwnEntity(*S.Target) && Stats.IsSet() && Stats->Incarnation==S.TargetIncarnation.Get(0))C.CpAtStart=Stats->Cp;}
   C.Start=S;C.SeenTick=S.Tick;C.bIdentityOk=ValidReference && ValidBaseline;if(S.bBaseline)C.Hydration=S;Casts.Add(K,C);if(!S.bBaseline && Net && Net->IsOwnEntity(S.Caster))++OwnStartCount;}
  else
  {auto& C=Casts[K];C.bIdentityOk &= ValidReference && ValidBaseline && C.Start.SkillId==S.SkillId && C.Start.SkillLevel==S.SkillLevel;C.SeenTick=FMath::Max(C.SeenTick,S.Tick);if(S.bBaseline)C.Hydration=S;}if(S.bBaseline && Net && !Net->IsOwnEntity(S.Caster))bObserverBaseline=true;return;
 }
 if(E.SkillCastResult.IsSet() || E.SkillCastCancelled.IsSet())
 {
  const bool Result=E.SkillCastResult.IsSet();const FString Caster=Result?E.SkillCastResult->Caster:E.SkillCastCancelled->Caster;
  const uint32 Inc=Result?E.SkillCastResult->CasterIncarnation:E.SkillCastCancelled->CasterIncarnation;
  const uint64 Tick=Result?E.SkillCastResult->Tick:E.SkillCastCancelled->Tick,Id=Result?E.SkillCastResult->CastId:E.SkillCastCancelled->CastId;
  if(!AuditSubject(Caster,Inc,Tick))return;
  // Audit delivered rows even when the subject or matching cast cannot be projected.
  if(Result)for(const auto& T:E.SkillCastResult->Targets)AuditSubject(T.Target,T.TargetIncarnation,T.Tick);
  if(!Fence.Accept(Caster,Inc,Tick))return;
  auto* C=Casts.Find(CastKey(Caster,Inc,Id));if(!C || Tick<C->Start.Tick)return;
  if(Result && (E.SkillCastResult->SkillId!=C->Start.SkillId || E.SkillCastResult->SkillLevel!=C->Start.SkillLevel)){C->bIdentityOk=false;bDispositionOk=false;return;}
  FString Fingerprint=FString::Printf(TEXT("%d|%llu|%u"),Result,Tick,Result?0:E.SkillCastCancelled->Reason);
  if(Result)Fingerprint+=FString::Printf(TEXT("|owner_effect:%d:%llu"),E.SkillCastResult->EffectId.IsSet(),E.SkillCastResult->EffectId.Get(0));
  if(Result)for(const auto& T:E.SkillCastResult->Targets)Fingerprint+=FString::Printf(TEXT("|%s|%u|%llu|%u|%u|%u|%u|%u|%u|%u"),*T.Target,T.TargetIncarnation,T.Tick,T.ResolutionKind,T.Outcome,T.Rolled,T.CpAbsorbed,T.HpRemoved,T.Heal,T.TargetHpAfter);
  if(Result)for(const auto& T:E.SkillCastResult->Targets)for(uint64 Effect:T.AppliedEffectIds)Fingerprint+=FString::Printf(TEXT("|effect:%s:%llu"),*T.Target,Effect);
  if(C->Terminals){C->bIdentityOk &= C->Terminal==Fingerprint;bDispositionOk &= C->Terminal==Fingerprint;return;}C->Terminal=Fingerprint;++C->Terminals;C->Reason=Result?0:E.SkillCastCancelled->Reason;C->TerminalTick=Tick;
  if(!Result && Net && Net->IsOwnEntity(Caster)){const auto* ReuseAtCancel=Reuse.Find(C->Start.SkillId);C->ReuseAtCancel=ReuseAtCancel?TOptional<FSkillReuse>(*ReuseAtCancel):TOptional<FSkillReuse>();}
  if(Result)for(const auto& T:E.SkillCastResult->Targets)
  {
   const FString K=CastKey(Caster,Inc,Id)+TEXT("|")+T.Target.ToLower();if(DamageIds.Contains(K))continue;DamageIds.Add(K);
   Impact(Caster,T.Target,T.TargetIncarnation,T.Tick,C->Start.Tick,T.TargetHpAfter,T.CpAbsorbed,T.HpRemoved);
   if(LastImpact.IsSet() && LastImpact->Tick==T.Tick && C->Start.Target.IsSet() && C->Start.Target->Equals(T.Target,ESearchCase::IgnoreCase))LastImpact->StartFlagDeadline=C->TargetFlagAtStart;
  }
  return;
 }
 if(E.AttackStarted.IsSet())
 {
  const auto& S=*E.AttackStarted;
  const uint64 SeenTick=S.bBaseline?S.BaselineTick:S.Tick;
  if(!AuditSubject(S.Attacker,S.AttackerIncarnation,SeenTick,S.bBaseline,S.BaselineTick))return;
  Reference(S.Target.IsEmpty()?TOptional<FString>():TOptional<FString>(S.Target),S.Target.IsEmpty() && !S.TargetIncarnation?TOptional<uint32>():TOptional<uint32>(S.TargetIncarnation));
  const auto* A=Fence.Identities.Find(S.Attacker.ToLower());if(!A || !Fence.Accept(S.Attacker,S.AttackerIncarnation.Get(A->LifeIncarnation),SeenTick,S.bBaseline,S.BaselineTick))return;
  const FString Key=S.Attacker.ToLower();if(const auto* Old=SwingTicks.Find(Key);Old && SeenTick<*Old)return;
  if(!S.bBaseline && Net && Net->IsOwnEntity(S.Attacker) && (!SwingTicks.Contains(Key) || SwingTicks[Key]<SeenTick))++OwnSwingStartCount;
  SwingTicks.Add(Key,SeenTick);Swings.Add(Key,S);if(const auto* P=PublicPvp.Find(S.Target.ToLower()))SwingTargetsAtStart.Add(S.Attacker.ToLower(),*P);return;
 }
 if(E.AttackCancelled.IsSet()){const auto& C=*E.AttackCancelled;if(!AuditSubject(C.Attacker,{},C.Tick))return;if(!C.Target.IsEmpty())AuditSubject(C.Target,{},C.Tick);const auto* A=Fence.Identities.Find(C.Attacker.ToLower());if(A && Fence.Accept(C.Attacker,A->LifeIncarnation,C.Tick)){const FString Key=C.Attacker.ToLower();if(const auto* Old=SwingTicks.Find(Key);Old && C.Tick<*Old)return;SwingTicks.Add(Key,C.Tick);Swings.Remove(Key);}return;}
 if(E.AttackResult.IsSet())
 {
  const auto& R=*E.AttackResult;const auto* S=Swings.Find(R.Attacker.ToLower());const FString K=FString::Printf(TEXT("swing|%s|%s|%u|%llu"),*R.Attacker.ToLower(),*R.Target.ToLower(),R.TargetIncarnation,R.Tick);
  if(!AuditSubject(R.Target,R.TargetIncarnation,R.Tick) || !AuditSubject(R.Attacker,{},R.Tick))return;
  if(DamageIds.Contains(K))return;if(!Fence.Accept(R.Target,R.TargetIncarnation,R.Tick))return;DamageIds.Add(K);Impact(R.Attacker,R.Target,R.TargetIncarnation,R.Tick,S?S->Tick:0,R.TargetHpAfter,R.CpDamage,R.Damage);if(LastImpact.IsSet() && LastImpact->Tick==R.Tick)if(const auto* P=SwingTargetsAtStart.Find(R.Attacker.ToLower()))LastImpact->StartFlagDeadline=P->FlagUntilTick;return;
 }
 if(E.EffectApplied.IsSet())
 {
  const auto& A=*E.EffectApplied;
  if(!AuditSubject(A.Target,A.TargetIncarnation,A.Tick,A.bBaseline,A.BaselineTick))return;
  Reference(A.Caster,A.CasterIncarnation);
  if(!A.EffectId || !Fence.Accept(A.Target,A.TargetIncarnation,A.Tick,A.bBaseline,A.BaselineTick))return;
  const FString K=CastKey(A.Target,A.TargetIncarnation,A.EffectId);if(const auto* Tick=EffectTicks.Find(K);Tick && A.Tick<=*Tick)return;
  if(RemovedEffects.Contains(K)){bEffectsOk=false;return;}
  auto& Rows=Effects.FindOrAdd(A.Target.ToLower());if(const auto* Old=Rows.Find(A.EffectId))bEffectsOk &= Old->SkillId==A.SkillId && Old->SkillLevel==A.SkillLevel && Old->Pool==A.Pool && Old->AppliedTick==A.AppliedTick;
  Rows.Add(A.EffectId,A);EffectTicks.Add(K,A.Tick);++EffectSamples;if(A.bBaseline && Net && !Net->IsOwnEntity(A.Target))bObserverBaseline=true;return;
 }
 if(E.EffectRemoved.IsSet())
 {
  const auto& R=*E.EffectRemoved;if(!AuditSubject(R.Target,R.TargetIncarnation,R.Tick))return;if(!R.EffectId || !Fence.Accept(R.Target,R.TargetIncarnation,R.Tick))return;const FString K=CastKey(R.Target,R.TargetIncarnation,R.EffectId);if(const auto* Old=EffectTicks.Find(K);Old && R.Tick<*Old)return;auto& Rows=Effects.FindOrAdd(R.Target.ToLower());Rows.Remove(R.EffectId);EffectTicks.Add(K,R.Tick);RemovedEffects.Add(K);return;
 }
 if(E.XpGained.IsSet())
 {
  const auto& G=*E.XpGained;if(Net && Net->IsOwnEntity(G.Entity))
  {const uint64 Tick=Net->GetLatestReceivedTick();const FStatsChanged* Before=nullptr;for(const auto& S:ResourceSamples)if(S.Tick<Tick && (!Before || S.Tick>Before->Tick))Before=&S;if(Before)XpCredit=FXpCredit{G.Amount,Tick,Before->Level};}return;
 }
 if(E.ResurrectOffer.IsSet())
 {
  const auto& O=*E.ResurrectOffer;
  const bool AcrossLife=RevivalTerminal(O);
  if(!AcrossLife && !AuditSubject(O.Target,O.TargetIncarnation,O.Tick,O.bBaseline,O.BaselineTick))return;
  Reference(O.Caster,O.CasterIncarnation);
  if(!Net || !Net->IsOwnEntity(O.Target) || (!AcrossLife && (!Fence.Accept(O.Target,O.TargetIncarnation,O.Tick,O.bBaseline,O.BaselineTick) || !Fence.Dead(O.Target))))return;
  if(!O.OfferId || (O.State!=1 && (!Offer.IsSet() || O.OfferId!=Offer->OfferId)))return;
  if(Offer.IsSet() && (O.Tick<Offer->Tick || (O.OfferId==Offer->OfferId && Offer->State!=1)))return;
  Offer=O;CompleteResurrectionAudit();return;
 }
}
void USkillAuditSubsystem::ObserveAck(const FAck& A){AcceptedSeqs.Add(A.Seq,A.Tick);Fence.Tick=FMath::Max(Fence.Tick,A.Tick);}
void USkillAuditSubsystem::ObserveRejected(const FIntentRejected& R)
{
 if(AcceptedSeqs.Contains(R.Seq))bDispositionOk=false;
 if(R.Reason==22 && (SentSkills.Contains(R.Seq) || SentSwingCounts.Contains(R.Seq)))PeaceRejectSeq=R.Seq;
}
void USkillAuditSubsystem::ObserveSent(const TArray<uint8>& Bytes)
{
 FGrpcNightfallV1ClientMessage M;if(!NightfallWire::DecodeClientMessage(Bytes.GetData(),Bytes.Num(),M))return;
 if(M.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::Respawn && M.Intent.Respawn.IsValid())TownRequest=M.Seq.Value;
 if(M.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::Attack && M.Intent.Attack.IsValid()){SentSwingCounts.Add(M.Seq.Value,OwnSwingStartCount);PeaceRejectSeq.Reset();}
 if(M.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::CastSkill && M.Intent.CastSkill.IsValid()){SentSkills.Add(M.Seq.Value,M.Intent.CastSkill->SkillId.Value);SentStartCounts.Add(M.Seq.Value,OwnStartCount);PeaceRejectSeq.Reset();}
}
TOptional<uint32> USkillAuditSubsystem::OfferState() const{return Offer.IsSet()?TOptional<uint32>(Offer->State):TOptional<uint32>();}
TOptional<uint64> USkillAuditSubsystem::Number(const FString& Name) const
{
 if(!Net || !Fence.Identities.Contains(Net->GetOwnEntityId().ToLower()))return {};
 if(Name==TEXT("own_cp_damage_total"))return OwnCp;if(Name==TEXT("player_cp_damage_total"))return PlayerCp;if(Name==TEXT("resurrection_refunds"))return uint64(RefundedOffers.Num());
 if(Name==TEXT("players_attacking_me"))
 {
  // A landed swing stays available for impact audits; StopAttack during recovery has no cancel frame.
  const uint64 Tick=Net->GetLatestReceivedTick();uint64 Count=0;
  for(const auto& S:Swings)
  {
   const auto* I=Fence.Identities.Find(S.Key);
   if(I && I->Kind==1 && !I->bDead && S.Value.ReadyTick>Tick
      && S.Value.Target.Equals(Net->GetOwnEntityId(),ESearchCase::IgnoreCase)
      && Fence.Visible(S.Value.Target,S.Value.TargetIncarnation) && !Fence.Dead(S.Value.Target))++Count;
  }
  return Count;
 }
 return {};
}
TOptional<bool> USkillAuditSubsystem::Check(const FString& Name) const
{
 if(Name==TEXT("cast_dispositions_once")){uint32 N=0;for(const auto& C:Casts)if(!C.Value.Start.bBaseline){++N;if(C.Value.Terminals!=1)return false;}return N?TOptional<bool>(bDispositionOk):TOptional<bool>();}
 if(Name==TEXT("spawn_before_skill_references"))return References?TOptional<bool>(bReferencesOk):TOptional<bool>();
 if(Name==TEXT("owner_skill_baseline_coherent"))return bOwnerStatsBaseline && bOwnerSyncBaseline?TOptional<bool>(bReferencesOk && bPvpOk && bBaselineOk):TOptional<bool>();
 if(Name==TEXT("observer_skill_baseline_coherent"))return bObserverBaseline?TOptional<bool>(bReferencesOk && bBaselineOk):TOptional<bool>();
 if(Name==TEXT("public_vitals_coherent") || Name==TEXT("public_revival_vitals_coherent"))
 {
  if(!(Name==TEXT("public_vitals_coherent")?PublicPairs:RevivalPairs) || !Net)return {};
  const auto* Combat=Net->GetGameInstance()->GetSubsystem<UCombatStateSubsystem>();if(!Combat)return {};
  for(const auto& Pair:Vitals)
  {
   const auto* P=Combat->FindEntity(Pair.Key);const auto& V=Pair.Value;
   if(!P || P->Incarnation!=V.Incarnation)continue;
   const uint64 ProjectedTick=FMath::Max(P->LastFactTick,P->IdentityBaselineTick);
   if(ProjectedTick>V.Tick)continue;
   if(ProjectedTick<V.Tick || P->Hp!=V.Hp || P->MaxHp!=V.MaxHp)return false;
  }
  return bVitalsOk;
 }
 if(Name==TEXT("pvp_private_stats_coherent"))return PvpPairs?TOptional<bool>(bPvpOk):TOptional<bool>();
 if(Name==TEXT("cp_absorption_matches_received"))
 {
  if(!bSamplesComplete)return {};
  // Owner StatsChanged precedes public results in the V5 projection. Join by tick,
  // not callback order; no received resource interval is evidence until it has damage.
  bool Witnessed=false;
  for(int32 Index=1;Index<ResourceSamples.Num();++Index)
  {
   const auto& Before=ResourceSamples[Index-1];const auto& After=ResourceSamples[Index];
   if(Before.Incarnation!=After.Incarnation)continue;
   uint64 Cp=0,Hp=0;uint32 FinalHp=0;
   for(const auto& D:PendingCp)if(D.Tick>Before.Tick && D.Tick<=After.Tick){Cp+=D.Cp;Hp+=D.Hp;FinalHp=D.HpAfter;}
   if(!Cp)continue;Witnessed=true;
   if(uint64(Before.Cp)<Cp || uint64(Before.Cp)-Cp!=After.Cp || After.Hp!=FinalHp || uint64(Before.Hp)<Hp || uint64(Before.Hp)-Hp!=After.Hp)return false;
  }
  return Witnessed?TOptional<bool>(true):TOptional<bool>();
 }
 if(Name==TEXT("effect_identity_coherent"))return EffectSamples?TOptional<bool>(bEffectsOk && bReferencesOk):TOptional<bool>();
 if(Name==TEXT("death_penalty_matches_before_disconnect"))
 {
  if(!PriorPenalty.IsSet() || !Net || PriorOwner!=Net->GetOwnEntityId().ToLower())return {};
  return Stats.IsSet()?TOptional<bool>(Stats->DeathPenaltyLevel==*PriorPenalty):Sync.IsSet()?TOptional<bool>(Sync->DeathPenaltyLevel==*PriorPenalty):TOptional<bool>();
 }
 if(Name==TEXT("resurrect_offer_matches_death"))return Offer.IsSet() && Death.IsSet()?TOptional<bool>(Offer->TargetIncarnation==Death->Incarnation && Offer->Target.Equals(Death->Entity,ESearchCase::IgnoreCase) && Offer->ExpiryTick>Offer->Tick):TOptional<bool>();
 if(Name==TEXT("own_flag_deadline_matches_received"))
 {
  if(!Stats.IsSet() || !Net)return {};
  const auto* Public=PublicPvp.Find(Net->GetOwnEntityId().ToLower());
  // StatsChanged can advance for StopAttack/resources while the public PvP state
  // stays unchanged. Compare the latest accepted copies of this life, not their
  // publication ticks. A differing deadline remains false; no local expiry math.
  if(!Public || Public->Incarnation!=Stats->Incarnation
   || !Fence.Accept(Stats->Entity,Stats->Incarnation,Stats->Tick,Stats->bBaseline,Stats->BaselineTick)
   || !Fence.Accept(Public->Entity,Public->Incarnation,Public->Tick,Public->bBaseline,Public->BaselineTick))return {};
  return Public->FlagUntilTick==Stats->FlagUntilTick;
 }
 if(Name==TEXT("npc_target_is_owner"))
 {
  if(!Net)return {};bool Witnessed=false;for(const auto& S:Swings){const auto* I=Fence.Identities.Find(S.Key);if(I && I->Kind==2 && !I->bDead){Witnessed=true;if(S.Value.Target.Equals(Net->GetOwnEntityId(),ESearchCase::IgnoreCase))return true;}}return Witnessed?TOptional<bool>(false):TOptional<bool>();
 }
 if(Name==TEXT("own_flag_expired_at_last_impact") || Name==TEXT("target_flag_expired_at_last_impact"))
 {
  if(!LastImpact.IsSet() || !Net)return {};const auto& I=*LastImpact;
  const bool Own=Name==TEXT("own_flag_expired_at_last_impact");if(Own?!Net->IsOwnEntity(I.Target):!Net->IsOwnEntity(I.Attacker))return {};
  const uint64 Deadline=I.StartFlagDeadline;return Deadline>0 && I.StartTick>0?TOptional<bool>(I.StartTick<Deadline && I.Tick>=Deadline):TOptional<bool>();
 }
 if(Name==TEXT("victim_flag_not_refreshed"))
 {
  if(!LastImpact.IsSet())return {};const auto* P=PublicPvp.Find(LastImpact->Target);return P && Net && Net->GetLatestReceivedTick()>LastImpact->Tick?TOptional<bool>(P->FlagUntilTick==LastImpact->TargetBefore.FlagUntilTick):TOptional<bool>();
 }

 if(Name==TEXT("impact_counter_uses_received_flag") || Name==TEXT("flagged_kill_matches_received"))
 {
  if(!LastImpact.IsSet())return {};const auto& I=*LastImpact;const auto* D=Deaths.Find(I.Target);const auto* After=PublicPvp.Find(I.Attacker);
  // PcInstance:4592-4647 updates kill counters only on lethal impacts. The
  // expiry scenario deliberately keeps this first victim alive, then tests a kill.
  if(I.HpAfter>0 && Name==TEXT("impact_counter_uses_received_flag"))
  {
   if(!After || !Net || Net->GetLatestReceivedTick()<=I.Tick || After->Incarnation!=I.AttackerBefore.Incarnation)return {};
   return After->PvpCount==I.AttackerBefore.PvpCount && After->PkCount==I.AttackerBefore.PkCount && After->Karma==I.AttackerBefore.Karma;
  }
  if(!D || !After || D->Tick!=I.Tick || D->Incarnation!=I.Inc || !D->Killer.Equals(I.Attacker,ESearchCase::IgnoreCase) || After->Tick!=I.Tick || After->Incarnation!=I.AttackerBefore.Incarnation)return {};
  const bool Flagged=I.TargetBefore.FlagUntilTick>I.Tick;
  if(Name==TEXT("flagged_kill_matches_received") && !Flagged)return {};
  // Compare the two received counter copies. Pinned PcInstance:4592-4612,4624,4647; D-R4 chaotic-unflagged precedence.
  if(Flagged)return uint64(After->PvpCount)==uint64(I.AttackerBefore.PvpCount)+1 && After->PkCount==I.AttackerBefore.PkCount && After->Karma==I.AttackerBefore.Karma;
  if(I.TargetBefore.Karma>0)return After->PvpCount==I.AttackerBefore.PvpCount && After->PkCount==I.AttackerBefore.PkCount;
  return uint64(After->PkCount)==uint64(I.AttackerBefore.PkCount)+1 && After->PvpCount==I.AttackerBefore.PvpCount;
 }
 if(Name==TEXT("karma_gain_matches_oracle"))
 {
  if(!KarmaGainChange.IsSet() || !LastImpact.IsSet() || !Check(TEXT("impact_counter_uses_received_flag")).Get(false))return {};
  const auto& G=*KarmaGainChange;if(G.After.Tick!=LastImpact->Tick || LastImpact->TargetBefore.Karma || LastImpact->TargetBefore.FlagUntilTick>LastImpact->Tick)return {};
  const auto Expected=FSkillOracleData::Get().KarmaGain(G.Before.PkCount);return Expected.IsSet()?TOptional<bool>(uint64(G.After.Karma)-G.Before.Karma==*Expected):TOptional<bool>();
 }
 if(Name==TEXT("karma_decay_coherent"))
 {
  if(!bSamplesComplete || !KarmaLossChange.IsSet() || !XpCredit.IsSet() || KarmaLossChange->After.Tick!=XpCredit->Tick || !XpCredit->Amount)return {};
  // C11/C15: exact decay uses actual replay XP and pre-award level.
  const auto& G=*KarmaLossChange;return G.After.Karma<G.Before.Karma;
 }
 if(Name==TEXT("death_xp_matches_fixture"))
 {
  if(!BeforeDeath.IsSet() || !DeadResources.IsSet() || !Death.IsSet() || BeforeDeath->Incarnation!=Death->Incarnation || DeadResources->Incarnation!=Death->Incarnation)return {};
  const auto& D=FBotFixtureData::Get();const auto Loss=D.IsValid()?D.DeathXpLoss(BeforeDeath->Level):TOptional<uint64>();if(!Loss.IsSet())return {};
  return BeforeDeath->Xp>=DeadResources->Xp && BeforeDeath->Xp-DeadResources->Xp==FMath::Min(*Loss,BeforeDeath->Xp);
 }
 if(Name==TEXT("town_respawn_vitals_match_fixture"))
 {
  if(!TownRequest.IsSet() || !AcceptedSeqs.Contains(*TownRequest) || !DeadResources.IsSet() || !AfterRevival.IsSet() || bSkillRevival)return {};
  const auto& D=FBotFixtureData::Get();if(!D.IsValid())return {};
  return AfterRevival->Hp==D.RespawnHp(AfterRevival->MaxHp) && AfterRevival->Mp==D.RespawnMp(AfterRevival->MaxMp) && AfterRevival->Cp==DeadResources->Cp;
 }
 if(Name==TEXT("resurrection_vitals_coherent"))
 {
  if(!DeadResources.IsSet() || !AfterRevival.IsSet() || !bSkillRevival)return {};
  // C11: exact doRevive HP is verified from replay with actual inputs.
  return AfterRevival->Hp>0 && AfterRevival->Mp==DeadResources->Mp && AfterRevival->Cp==DeadResources->Cp;
 }
 if(Name==TEXT("retained_effect_identity_coherent"))
 {
  if(!bPriorEffectsKnown || PriorEffects.IsEmpty() || !Net || PriorOwner!=Net->GetOwnEntityId().ToLower() || !bOwnerSyncBaseline)return {};
  const auto* Current=Effects.Find(Net->GetOwnEntityId().ToLower());if(!Current)return {};
  for(const auto& Pair:PriorEffects)
  {
   const auto& P=Pair.Value;const FEffectApplied* E=nullptr;
   // Logout restoration allocates a fresh identity and restarts paused deadlines.
   // Match the unique retained skill/source instead of the old transport EffectId.
   for(const auto& Candidate:*Current)if(Candidate.Value.SkillId==P.SkillId && Candidate.Value.SkillLevel==P.SkillLevel && Candidate.Value.bBaseline)
   {if(E)return false;E=&Candidate.Value;}
   if(!E)return {};
   if(bool(E->EndTick)!=bool(P.EndTick) || (E->EndTick && (E->EndTick<E->AppliedTick || P.EndTick<P.AppliedTick || E->EndTick-E->AppliedTick>P.EndTick-P.AppliedTick)))return false;
   if(E->SkillId!=P.SkillId || E->SkillLevel!=P.SkillLevel || E->Pool!=P.Pool || E->AbnormalType!=P.AbnormalType || E->AbnormalLevel!=P.AbnormalLevel || E->StatusFlags!=P.StatusFlags)return false;
   if(P.Caster.IsSet() && P.CasterIncarnation.IsSet() && Fence.Visible(*P.Caster,*P.CasterIncarnation))
   {if(!E->Caster.IsSet() || !E->CasterIncarnation.IsSet() || !E->Caster->Equals(*P.Caster,ESearchCase::IgnoreCase) || E->CasterIncarnation!=P.CasterIncarnation)return false;}
  }
  return bReferencesOk && bEffectsOk;
 }
 if(Name==TEXT("peace_impact_has_no_damage"))
 {
  const FCastEvidence* Last=nullptr;for(const auto& Pair:Casts)if(Pair.Value.Reason==9 && Pair.Value.Terminals && (!Last || Pair.Value.TerminalTick>Last->TerminalTick))Last=&Pair.Value;
  if(!Last || !Net || Net->GetLatestReceivedTick()<=Last->TerminalTick || !Fence.Visible(Last->Start.Caster,Last->Start.CasterIncarnation) || !Last->bIdentityOk || !Last->Start.Target.IsSet() || !Last->Start.TargetIncarnation.IsSet() || !Last->HpAtStart.IsSet())return {};
  const auto* V=Vitals.Find(Last->Start.Target->ToLower());if(!V || V->Incarnation!=*Last->Start.TargetIncarnation || !Fence.Visible(*Last->Start.Target,*Last->Start.TargetIncarnation))return {};
  // Public-only observers cannot assert the victim's private CP. Exact CP proof is owner-scoped.
  if(!Net || !Net->IsOwnEntity(*Last->Start.Target) || !Last->CpAtStart.IsSet() || !Stats.IsSet() || Stats->Incarnation!=*Last->Start.TargetIncarnation)return {};
  return Last->Terminals==1 && V->Hp==*Last->HpAtStart && Stats->Cp==*Last->CpAtStart;
 }
 if(Name==TEXT("peace_rejection_has_no_start"))
 {
  if(!PeaceRejectSeq.IsSet())return {};if(AcceptedSeqs.Contains(*PeaceRejectSeq))return false;
  if(const auto* Count=SentSwingCounts.Find(*PeaceRejectSeq))return OwnSwingStartCount==*Count;
  const auto* Count=SentStartCounts.Find(*PeaceRejectSeq);return Count?TOptional<bool>(OwnStartCount==*Count):TOptional<bool>();
 }
 return {};
}

namespace SkillCostAudit
{
 const FGrpcNightfallV1SkillCatalogEntry* CostRow(const UNetClientSubsystem* Net,uint32 Skill,uint32 Rank)
 {
  if(!Net || !Rank)return nullptr;
  const auto* S=Net->GetGameInstance()->GetSubsystem<USkillStateSubsystem>();const auto* Row=S?S->FindSkill(Skill):nullptr;
  return Row && Rank<=Row->MaxLevel.Value && Row->MpConsumeStart.Num()==int64(Row->MaxLevel.Value) && Row->MpConsumeEnd.Num()==int64(Row->MaxLevel.Value)?Row:nullptr;
 }
 const FStatsChanged* Before(const TArray<FStatsChanged>& Samples,uint32 Inc,uint64 Tick)
 {
  const FStatsChanged* Out=nullptr;for(const auto& S:Samples)if(S.Incarnation==Inc && S.Tick<Tick && (!Out || S.Tick>Out->Tick))Out=&S;return Out;
 }
 const FStatsChanged* At(const TArray<FStatsChanged>& Samples,uint32 Inc,uint64 Tick)
 {
  for(const auto& S:Samples)if(S.Incarnation==Inc && S.Tick==Tick)return &S;return nullptr;
 }
 bool Delta(const FStatsChanged& Before,const FStatsChanged& After,uint32 Cost){return Before.Mp>=After.Mp && uint64(Before.Mp)-After.Mp==Cost;}
}
TOptional<bool> USkillAuditSubsystem::MpCostMatches(uint32 Skill) const
{
 using namespace SkillCostAudit;if(!Net || !bSamplesComplete)return {};
 const FCastEvidence* C=nullptr;
 for(const auto& Pair:Casts)if(!Pair.Value.Start.bBaseline && Net->IsOwnEntity(Pair.Value.Start.Caster) && Pair.Value.Start.SkillId==Skill && Pair.Value.Terminals && Pair.Value.Reason==0 && (!C || Pair.Value.Start.Tick>C->Start.Tick))C=&Pair.Value;
 if(!C)return {};if(!C->bIdentityOk)return false;const auto* Row=CostRow(Net,Skill,C->Start.SkillLevel);if(!Row)return {};
 const uint32 Mp1=Row->MpConsumeStart[C->Start.SkillLevel-1].Value,Mp2=Row->MpConsumeEnd[C->Start.SkillLevel-1].Value;
 const auto* BeforeStart=Before(ResourceSamples,C->Start.CasterIncarnation,C->Start.Tick);
 const auto* Start=At(ResourceSamples,C->Start.CasterIncarnation,C->Start.Tick);
 const auto* BeforeEnd=Before(ResourceSamples,C->Start.CasterIncarnation,C->TerminalTick);
 const auto* End=At(ResourceSamples,C->Start.CasterIncarnation,C->TerminalTick);
 if(!BeforeStart || !BeforeEnd || !End || (!Start && Mp1))return {};
 // Zero MP1 may leave no dirty StatsChanged at start. Explicit catalogue zero is required.
 if(Start && !Delta(*BeforeStart,*Start,Mp1))return false;
 // Stage deltas permit separately received positive MP changes between start and completion.
 return Delta(*BeforeEnd,*End,Mp2);
}
TOptional<bool> USkillAuditSubsystem::CancelledCostsRetained() const
{
 using namespace SkillCostAudit;if(!Net || !bSamplesComplete)return {};
 const FCastEvidence* C=nullptr;
 for(const auto& Pair:Casts)if(!Pair.Value.Start.bBaseline && Net->IsOwnEntity(Pair.Value.Start.Caster) && Pair.Value.Terminals && Pair.Value.Reason!=0 && (!C || Pair.Value.Start.Tick>C->Start.Tick))C=&Pair.Value;
 if(!C || Net->GetLatestReceivedTick()<=C->TerminalTick || !Fence.Visible(C->Start.Caster,C->Start.CasterIncarnation))return {};if(!C->bIdentityOk)return false;const auto* Row=CostRow(Net,C->Start.SkillId,C->Start.SkillLevel);if(!Row)return {};
 const uint32 Mp1=Row->MpConsumeStart[C->Start.SkillLevel-1].Value;
 const auto* BeforeStart=Before(ResourceSamples,C->Start.CasterIncarnation,C->Start.Tick);const auto* Start=At(ResourceSamples,C->Start.CasterIncarnation,C->Start.Tick);
 if(!BeforeStart || (!Start && Mp1))return {};
 const auto* Current=Stats.IsSet() && Stats->Incarnation==C->Start.CasterIncarnation?&*Stats:nullptr;if(!Current)return {};
 if(Start && !Delta(*BeforeStart,*Start,Mp1))return false;
 const auto* CurrentReuse=Reuse.Find(C->Start.SkillId);
 if(!Row->ReuseMs.IsValidIndex(C->Start.SkillLevel-1))return {};
 if(Row->ReuseMs[C->Start.SkillLevel-1].Value==0)
 {
  // No SkillReuse event exists for explicit catalogue zero. A rejected intent has
  // no cast/terminal pair and can never reach this branch.
  return !CurrentReuse && !C->ReuseAtCancel.IsSet() && Current->Mp==(Start?Start->Mp:BeforeStart->Mp);
 }
 if(!CurrentReuse || !C->ReuseAtCancel.IsSet())return {};
 const auto& R=*CurrentReuse;const auto& Expected=*C->ReuseAtCancel;
 // This tests retained received costs, not a locally reconstructed reuse duration.
 return R.SkillId==C->Start.SkillId && R.Incarnation==C->Start.CasterIncarnation && R.Tick>=C->Start.Tick && R.ReadyTick==Expected.ReadyTick && R.ReadyTick>C->TerminalTick && R.ReadyTick>Net->GetLatestReceivedTick() && Current->Mp==(Start?Start->Mp:BeforeStart->Mp);
}

TOptional<bool> USkillAuditSubsystem::NotInView(const FString& Entity) const
{
 const FString Id=Entity.ToLower();if(Fence.Identities.Contains(Id))return false;return Departed.Contains(Id)?TOptional<bool>(true):TOptional<bool>();
}
TOptional<bool> USkillAuditSubsystem::CastFor(const FString& Entity,uint32 Skill,const FString& Predicate) const
{
 const auto* Identity=Fence.Identities.Find(Entity.ToLower());if(!Identity)return {};
 const FCastEvidence* C=nullptr;
 for(const auto& Pair:Casts)
 {
  const auto& Candidate=Pair.Value;
  if(Candidate.Start.Caster.Equals(Entity,ESearchCase::IgnoreCase) && Candidate.Start.CasterIncarnation==Identity->LifeIncarnation && Candidate.Start.SkillId==Skill && Candidate.SeenTick>=Identity->StateTick && (!C || Candidate.SeenTick>C->SeenTick || (Candidate.SeenTick==C->SeenTick && Candidate.Start.CastId>C->Start.CastId)))C=&Candidate;
 }
 if(!C)return {};
 if(Predicate==TEXT("observed_cast_baseline_for"))
 {
  if(!C->Hydration.IsSet())return {};const auto& H=*C->Hydration;
  return C->bIdentityOk && H.bBaseline && H.StartTick<H.BaselineTick && H.BaselineTick<H.CompleteTick && H.BaselineTick==H.Tick;
 }
 if(!C->Terminals)return {};
 return C->bIdentityOk && C->Terminals==1 && C->Reason==0;
}
