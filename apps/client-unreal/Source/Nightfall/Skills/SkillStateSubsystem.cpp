#include "SkillStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "Engine/GameInstance.h"
#include "Misc/Guid.h"

void USkillStateSubsystem::Initialize(FSubsystemCollectionBase& C)
{
	Session = C.InitializeDependency<USessionClient>();
	Super::Initialize(C); Net = C.InitializeDependency<UNetClientSubsystem>();
	Net->OnEntityDied.AddUObject(this, &USkillStateSubsystem::ApplyDied);
	Net->OnEntitySpawn.AddUObject(this, &USkillStateSubsystem::ApplySpawn);
	Net->OnEntityDespawn.AddUObject(this, &USkillStateSubsystem::ApplyDespawn);
	Net->OnEntityRespawned.AddUObject(this, &USkillStateSubsystem::ApplyRespawn);
	Net->OnSkillStateSync.AddUObject(this, &USkillStateSubsystem::ApplySync);
	Net->OnSkillReuse.AddUObject(this, &USkillStateSubsystem::ApplyReuse);
	Net->OnSkillLearned.AddUObject(this, &USkillStateSubsystem::ApplyLearned);
	Net->OnStatsChanged.AddUObject(this, &USkillStateSubsystem::ApplyStats);
	Net->OnSpGained.AddUObject(this, &USkillStateSubsystem::ApplySp);
	Net->OnPvpStateChanged.AddUObject(this, &USkillStateSubsystem::ApplyPvp);
	Net->OnResurrectOffer.AddUObject(this, &USkillStateSubsystem::ApplyOffer);
	Net->OnSkillCastStarted.AddUObject(this, &USkillStateSubsystem::ApplyCast);
	Net->OnSkillCastResult.AddUObject(this, &USkillStateSubsystem::ApplyResult);
	Net->OnSkillCastCancelled.AddUObject(this, &USkillStateSubsystem::ApplyCancel);
 Net->OnAttackStarted.AddUObject(this,&USkillStateSubsystem::ApplySwing);
 Net->OnAttackCancelled.AddUObject(this,&USkillStateSubsystem::ApplySwingCancel);
	Net->OnIntentAck.AddWeakLambda(this, [this](const FAck& A) { Fence.Tick = FMath::Max(Fence.Tick, A.Tick); });
	Net->OnEntityMove.AddWeakLambda(this, [this](const FEntityMove& M) { Fence.Tick = FMath::Max(Fence.Tick, M.Tick); });
	Net->OnVitalsChanged.AddWeakLambda(this, [this](const FVitalsChanged& V) { if (Fence.Visible(V.Entity, V.Incarnation)) Fence.Tick = FMath::Max(Fence.Tick, V.Tick); });
	Net->OnConnected.AddDynamic(this, &USkillStateSubsystem::Reset); Net->OnDisconnected.AddDynamic(this, &USkillStateSubsystem::Disconnected);
}
void USkillStateSubsystem::Deinitialize()
{
	if (Net)
	{
		Net->OnEntityDied.RemoveAll(this); Net->OnAttackStarted.RemoveAll(this); Net->OnAttackCancelled.RemoveAll(this); Net->OnEntitySpawn.RemoveAll(this); Net->OnEntityDespawn.RemoveAll(this); Net->OnEntityRespawned.RemoveAll(this); Net->OnSkillStateSync.RemoveAll(this); Net->OnSkillReuse.RemoveAll(this); Net->OnSkillLearned.RemoveAll(this); Net->OnStatsChanged.RemoveAll(this); Net->OnSpGained.RemoveAll(this); Net->OnPvpStateChanged.RemoveAll(this); Net->OnResurrectOffer.RemoveAll(this); Net->OnSkillCastStarted.RemoveAll(this); Net->OnSkillCastResult.RemoveAll(this); Net->OnSkillCastCancelled.RemoveAll(this); Net->OnIntentAck.RemoveAll(this); Net->OnEntityMove.RemoveAll(this); Net->OnVitalsChanged.RemoveAll(this); Net->OnConnected.RemoveAll(this); Net->OnDisconnected.RemoveAll(this);
	}
	Super::Deinitialize();
}
bool USkillStateSubsystem::IsOwn(const FString& Id) const { return Net && Net->IsOwnEntity(Id); }
void USkillStateSubsystem::ClearOwner()
{
	++OperationSerial; bBusy = false; bRpcDone = false; FirstReceipts.Reset(); bReceiptMatches.Reset(); Counts.Reset(); bLearnDone = false; bLearnableKnown = false; Learnable = {}; LearnReceipt = {};
	LastOfferId=0; Learned.Reset(); Reuse.Reset(); Sp.Reset(); Offer.Reset(); LearnedEvents.Reset(); SpGainEvents.Reset(); OwnerTick = 0;
	SyncTick = StatsTick = SpTick = LastSpGainTick = OfferTick = SpGainedTotal = 0; DeathPenalty = 0;
	bOwnerKnown = bSyncKnown = bStatsKnown = bSpGainKnown = false;
}
void USkillStateSubsystem::Reset() { Fence.Reset(); Pvp.Reset(); Casts.Reset(); Swings.Reset(); ClearOwner(); CastTransitions = 0; OnChanged.Broadcast(); }
void USkillStateSubsystem::ApplySpawn(const FEntitySpawn& S)
{
	if (!Fence.Spawn(S)) return;
	Casts.Remove(S.EntityId.ToLower()); Swings.Remove(S.EntityId.ToLower());
	if (IsOwn(S.EntityId)) ClearOwner();
	FPvpStateChanged P; P.Entity = S.EntityId; P.Incarnation = S.LifeIncarnation; P.Karma = S.Karma; P.PkCount = S.PkCount; P.PvpCount = S.PvpCount; P.FlagUntilTick = S.FlagUntilTick; P.Tick = S.StateTick;
	Pvp.Add(S.EntityId.ToLower(), P);
	if(S.PendingSwing.IsSet()){auto Swing=*S.PendingSwing;Swing.bBaseline=true;Swing.Tick=Swing.BaselineTick=S.StateTick;ApplySwing(Swing);}
	if (S.PendingCast.IsSet()) { auto Cast = *S.PendingCast; Cast.bBaseline = true; Cast.BaselineTick = S.StateTick; Cast.Tick = S.StateTick; ApplyCast(Cast); }
	OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyDespawn(const FEntityDespawn& D)
{
	Fence.Despawn(D.EntityId); Casts.Remove(D.EntityId.ToLower()); Swings.Remove(D.EntityId.ToLower()); Pvp.Remove(D.EntityId.ToLower()); if (IsOwn(D.EntityId)) ClearOwner(); OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyDied(const FEntityDied& D)
{
	if (!Fence.Died(D)) return; Casts.Remove(D.Entity.ToLower()); Swings.Remove(D.Entity.ToLower()); OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyRespawn(const FEntityRespawned& R)
{
	if (!Fence.Respawn(R)) return;
	Casts.Remove(R.Entity.ToLower()); Swings.Remove(R.Entity.ToLower()); if (auto* P = Pvp.Find(R.Entity.ToLower())) { P->Incarnation = R.Incarnation; P->Tick = R.Tick; }
	if (IsOwn(R.Entity)) { Offer.Reset(); OfferTick = R.Tick; }
	OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplySync(const FSkillStateSync& S)
{
	if (!IsOwn(S.Entity) || !Fence.Accept(S.Entity, S.Incarnation, S.Tick, S.bBaseline, S.BaselineTick) || (S.Tick < OwnerTick || (bSyncKnown && S.Tick <= SyncTick))) return;
	Learned.Reset(); Reuse.Reset(); for (const auto& L : S.Learned) Learned.Add(L.SkillId, L.SkillLevel);
	for (const auto& R : S.Reuse) Reuse.Add(R.SkillId, { R.ReadyTick, S.Tick });
	Sp = S.Sp; SpTick = S.Tick; DeathPenalty = S.DeathPenaltyLevel; SyncTick = S.Tick; OwnerTick = S.Tick; bSyncKnown = bOwnerKnown = true;
	FPvpStateChanged P; P.Entity = S.Entity; P.Incarnation = S.Incarnation; P.Tick = S.Tick; P.Karma = S.Karma; P.PkCount = S.PkCount; P.PvpCount = S.PvpCount; P.FlagUntilTick = S.FlagUntilTick; P.bBaseline = S.bBaseline; P.BaselineTick = S.BaselineTick; ApplyPvp(P);
	Fence.Tick = FMath::Max(Fence.Tick, S.Tick); OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyReuse(const FSkillReuse& R)
{
	if (!IsOwn(R.Entity) || !Fence.Accept(R.Entity, R.Incarnation, R.Tick)) return;
	if (const auto* Old = Reuse.Find(R.SkillId); Old && R.Tick <= Old->Tick) return;
	OwnerTick = FMath::Max(OwnerTick, R.Tick); Reuse.Add(R.SkillId, { R.ReadyTick, R.Tick }); Fence.Tick = FMath::Max(Fence.Tick, R.Tick); OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyLearned(const FSkillLearned& L)
{
	if (!IsOwn(L.Entity) || !Fence.Accept(L.Entity, L.Incarnation, L.Tick) || L.Tick < SyncTick) return;
	const FString Key = FString::Printf(TEXT("%u|%u|%llu|%s"), L.SkillId, L.SkillLevel, L.Tick, *L.IdempotencyKey);
	if (LearnedEvents.Contains(Key)) return; LearnedEvents.Add(Key); ++Counts.FindOrAdd(TEXT("skill_learned_events")); OwnerTick = FMath::Max(OwnerTick, L.Tick);
	Learned.FindOrAdd(L.SkillId) = FMath::Max(Learned.FindRef(L.SkillId), L.SkillLevel);
	if (!Sp.IsSet() || L.Tick >= SpTick) { Sp = L.SpRemaining; SpTick = L.Tick; }
	Fence.Tick = FMath::Max(Fence.Tick, L.Tick); OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyStats(const FStatsChanged& S)
{
	if (!IsOwn(S.Entity) || !Fence.Accept(S.Entity, S.Incarnation, S.Tick, S.bBaseline, S.BaselineTick) || (bStatsKnown && S.Tick <= StatsTick)) return;
	if (!Sp.IsSet() || S.Tick >= SpTick) { Sp = S.Sp; SpTick = S.Tick; }
	DeathPenalty = S.DeathPenaltyLevel; StatsTick = S.Tick; OwnerTick = FMath::Max(OwnerTick, S.Tick); bStatsKnown = bOwnerKnown = true;
	FPvpStateChanged P; P.Entity = S.Entity; P.Incarnation = S.Incarnation; P.Tick = S.Tick; P.Karma = S.Karma; P.PkCount = S.PkCount; P.PvpCount = S.PvpCount; P.FlagUntilTick = S.FlagUntilTick; P.bBaseline = S.bBaseline; P.BaselineTick = S.BaselineTick; ApplyPvp(P);
	Fence.Tick = FMath::Max(Fence.Tick, S.Tick); OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplySp(const FSpGained& S)
{
	if (!IsOwn(S.Entity) || !Fence.Accept(S.Entity, S.Incarnation, S.Tick) || (bSpGainKnown && S.Tick < LastSpGainTick)) return;
	const FString Key = FString::Printf(TEXT("%llu|%llu|%llu"), S.Tick, S.Amount, S.Total);
	if (SpGainEvents.Contains(Key)) return; SpGainEvents.Add(Key); ++Counts.FindOrAdd(TEXT("sp_gained_events")); OwnerTick = FMath::Max(OwnerTick, S.Tick);
	SpGainedTotal = S.Amount > MAX_uint64 - SpGainedTotal ? MAX_uint64 : SpGainedTotal + S.Amount;
	if (!Sp.IsSet() || S.Tick >= SpTick) { Sp = S.Total; SpTick = S.Tick; }
	LastSpGainTick = S.Tick; bSpGainKnown = true; Fence.Tick = FMath::Max(Fence.Tick, S.Tick); OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyPvp(const FPvpStateChanged& P)
{
	if (!Fence.Accept(P.Entity, P.Incarnation, P.Tick, P.bBaseline, P.BaselineTick)) return;
	if (const auto* Old = Pvp.Find(P.Entity.ToLower()); Old && P.Tick < Old->Tick) return;
	Pvp.Add(P.Entity.ToLower(), P); Fence.Tick = FMath::Max(Fence.Tick, P.Tick); OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyOffer(const FResurrectOffer& O)
{
	if (!IsOwn(O.Target) || !Fence.Accept(O.Target, O.TargetIncarnation, O.Tick) || O.Tick < OfferTick || !O.OfferId || !Fence.Dead(O.Target)) return;
	if (Offer.IsSet() && ((O.OfferId != Offer->OfferId && O.State != 1) || (O.OfferId == Offer->OfferId && Offer->State != 1))) return;
	Offer = O; LastOfferId=O.OfferId; OfferTick = O.Tick;
	if (!Offer->Caster.IsSet() || !Offer->CasterIncarnation.IsSet() || !Fence.Visible(*Offer->Caster, *Offer->CasterIncarnation)) { Offer->Caster.Reset(); Offer->CasterIncarnation.Reset(); }
	Fence.Tick = FMath::Max(Fence.Tick, O.Tick); OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyCast(const FSkillCastStarted& S)
{
	if (!S.CastId || !Fence.Accept(S.Caster, S.CasterIncarnation, S.Tick, S.bBaseline, S.BaselineTick)) return;
	auto& V = Casts.FindOrAdd(S.Caster.ToLower());
	if (S.Tick < V.Tick || S.CastId <= V.TerminalId || (V.Cast.IsSet() && S.CastId <= V.Cast->CastId)) return;
	V.Cast = S; V.Tick = S.Tick; V.CancelReason = 0;
	if (!V.Cast->Target.IsSet() || !V.Cast->TargetIncarnation.IsSet() || !Fence.Visible(*V.Cast->Target, *V.Cast->TargetIncarnation)) { V.Cast->Target.Reset(); V.Cast->TargetIncarnation.Reset(); }
	Fence.Tick = FMath::Max(Fence.Tick, S.Tick); if (!S.bBaseline) { ++CastTransitions; if(IsOwn(S.Caster))++Counts.FindOrAdd(TEXT("cast_starts")); } OnChanged.Broadcast();
}
void USkillStateSubsystem::FinishCast(const FString& Caster, uint32 Inc, uint64 Id, uint64 At, uint32 Reason)
{
	if (!Fence.Accept(Caster, Inc, At)) return;
	auto& V = Casts.FindOrAdd(Caster.ToLower());
	if (At < V.Tick || Id <= V.TerminalId || (V.Cast.IsSet() && Id != V.Cast->CastId)) return;
	V.Cast.Reset(); V.TerminalId = Id; V.Tick = At; V.CancelReason = Reason; ++CastTransitions; Fence.Tick = FMath::Max(Fence.Tick, At); OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyResult(const FSkillCastResult& R)
{
 const int32 Before=CastTransitions; FinishCast(R.Caster,R.CasterIncarnation,R.CastId,R.Tick,0);
 if(CastTransitions==Before)return;
 ++Counts.FindOrAdd(TEXT("observed_cast_results"));
 if(IsOwn(R.Caster))
 {
  ++Counts.FindOrAdd(TEXT("cast_results"));
  if(R.Targets.ContainsByPredicate([](const auto& T){return T.ResolutionKind==1 || T.ResolutionKind==2;}))++Counts.FindOrAdd(TEXT("cast_damage_results"));
 }
}
void USkillStateSubsystem::ApplyCancel(const FSkillCastCancelled& C)
{
 const int32 Before=CastTransitions;FinishCast(C.Caster,C.CasterIncarnation,C.CastId,C.Tick,C.Reason);
 if(CastTransitions!=Before && IsOwn(C.Caster))++Counts.FindOrAdd(TEXT("cast_cancellations"));
}
TOptional<uint64> USkillStateSubsystem::ReuseRemaining(uint32 Id) const
{
	if (!bSyncKnown || !Learned.Contains(Id)) return {};
	const auto* R = Reuse.Find(Id); return R && R->ReadyTick > GetTick() ? R->ReadyTick - GetTick() : uint64(0);
}
FString USkillStateSubsystem::Flag(const FString& Entity) const
{
	const auto* P = FindPvp(Entity); return !P ? TEXT("unknown") : P->Karma ? TEXT("chaotic") : P->FlagUntilTick > GetTick() ? TEXT("pvp") : TEXT("none");
}
bool USkillStateSubsystem::OfferPending() const { return Net && Fence.Dead(Net->GetOwnEntityId()) && Offer.IsSet() && Offer->State == 1 && Offer->ExpiryTick > GetTick(); }

uint64 USkillStateSubsystem::GetTick() const { return FMath::Max(Fence.Tick, Net ? Net->GetLatestReceivedTick() : uint64(0)); }

void USkillStateSubsystem::ApplyCatalog(const FGrpcNightfallV1GetSkillCatalogResponse& R) { Catalog = R; bCatalogKnown = true; OnChanged.Broadcast(); }
void USkillStateSubsystem::ApplyLearnable(const FGrpcNightfallV1ListLearnableSkillsResponse& R) { Learnable = R; bLearnableKnown = true; OnChanged.Broadcast(); }
void USkillStateSubsystem::LoadCatalog(FDone Done)
{
	if (!Session || bBusy) return;
	bBusy = true; bRpcDone = false; bCatalogKnown = false; const uint64 Serial = ++OperationSerial;
	Session->GetSkillCatalog({}, [Weak = TWeakObjectPtr<USkillStateSubsystem>(this), Serial, Done](const FNetResult& Result, const FGrpcNightfallV1GetSkillCatalogResponse& R)
	{
		if (!Weak.IsValid() || Serial != Weak->OperationSerial) return;
		Weak->bBusy = false; Weak->bRpcDone = true; Weak->LastResult = Result; if (Result.IsOk()) Weak->ApplyCatalog(R); Weak->OnChanged.Broadcast(); if (Done) Done(Result);
	});
}
void USkillStateSubsystem::LoadLearnable(FDone Done)
{
	if (!Session || !Net || bBusy) return;
	bBusy = true; bRpcDone = false; bLearnableKnown = false; const uint64 Serial = ++OperationSerial;
	FGrpcNightfallV1ListLearnableSkillsRequest Req; Req.CharacterId = Net->GetOwnEntityId();
	Session->ListLearnableSkills(Req, [Weak = TWeakObjectPtr<USkillStateSubsystem>(this), Serial, Done](const FNetResult& Result, const FGrpcNightfallV1ListLearnableSkillsResponse& R)
	{
		if (!Weak.IsValid() || Serial != Weak->OperationSerial) return;
		Weak->bBusy = false; Weak->bRpcDone = true; Weak->LastResult = Result; if (Result.IsOk()) Weak->ApplyLearnable(R); Weak->OnChanged.Broadcast(); if (Done) Done(Result);
	});
}
void USkillStateSubsystem::Learn(uint32 Id, uint32 Rank, FDone Done, const FString& Key)
{
	if (!Session || !Net || bBusy) return;
	bBusy = true; bRpcDone = false; bLearnDone = false; const uint64 Serial = ++OperationSerial;
	FGrpcNightfallV1LearnSkillRequest Req; Req.CharacterId = Net->GetOwnEntityId(); Req.SkillId = Id; Req.SkillLevel = Rank;
	Req.IdempotencyKey = Key.IsEmpty() ? FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower) : Key;
	LastLearnKey = Req.IdempotencyKey;
	Session->LearnSkill(Req, [Weak = TWeakObjectPtr<USkillStateSubsystem>(this), Serial, Done](const FNetResult& Result, const FGrpcNightfallV1LearnSkillResponse& R)
	{
		if (Weak.IsValid()) Weak->CompleteLearn(Serial, Result, R, Done);
	});
}
void USkillStateSubsystem::CompleteLearn(uint64 Serial, const FNetResult& Result, const FGrpcNightfallV1LearnSkillResponse& R, FDone Done)
{
	if (Serial != OperationSerial) return;
	bBusy = false; bRpcDone = true; bLearnDone = true; LastResult = LearnResult = Result;
	// Frozen receipts may be historical. World facts remain the only live state inputs.
	if (Result.IsOk()) { LearnReceipt = R; bLearnableKnown = false;
  auto Safe=R;const bool Present=Safe.EffectiveStats.IsValid();if(!Present)Safe.EffectiveStats=MakeShared<FGrpcNightfallV1EffectiveStats>();
  const FString Json=(Present?TEXT("present:"):TEXT("absent:"))+Safe.ToJsonString(false);
  if(const auto* First=FirstReceipts.Find(LastLearnKey))bReceiptMatches=(*First==Json);
  else {if(FirstReceipts.Num()>=128)FirstReceipts.Reset();FirstReceipts.Add(LastLearnKey,Json);bReceiptMatches.Reset();} }
	OnChanged.Broadcast(); if (Done) Done(Result);
}
const FGrpcNightfallV1SkillCatalogEntry* USkillStateSubsystem::FindSkill(uint32 Id) const
{
	if (bCatalogKnown) for (const auto& Skill : Catalog.Skills) if (Skill.SkillId.Value == Id) return &Skill; return nullptr;
}
TArray<uint32> USkillStateSubsystem::Hotbar() const
{
	TArray<uint32> Out; for (const auto& Pair : Learned) { const auto* Skill = FindSkill(Pair.Key); if (Skill && Skill->EffectImplemented) Out.Add(Pair.Key); }
	Out.Sort(); if (Out.Num() > 9) Out.SetNum(9); return Out;
}
uint32 USkillStateSubsystem::CastSlot(int32 Index)
{
	const auto Slots = Hotbar(); if (!Net || !Slots.IsValidIndex(Index)) return 0;
	return Net->SendCastSkill(Slots[Index]);
}

TOptional<uint64> USkillStateSubsystem::History(const FString& Name) const { return bOwnerKnown ? TOptional<uint64>(Counts.FindRef(Name)) : TOptional<uint64>(); }
void USkillStateSubsystem::ApplySwing(const FAttackStarted& S)
{
 const auto* Identity=Fence.Identities.Find(S.Attacker.ToLower());if(!Identity)return;
 const uint32 Inc=S.AttackerIncarnation.IsSet()?*S.AttackerIncarnation:Identity->LifeIncarnation;
 if(!Fence.Accept(S.Attacker,Inc,S.Tick,S.bBaseline,S.BaselineTick))return;
 auto& V=Swings.FindOrAdd(S.Attacker.ToLower());if(S.Tick<V.Tick)return;
 V.Swing=S;V.Tick=S.Tick;
 if(S.Target.IsEmpty() || !Fence.Visible(S.Target,S.TargetIncarnation)){V.Swing->Target.Reset();V.Swing->TargetIncarnation=0;}
 Fence.Tick=FMath::Max(Fence.Tick,S.Tick);
}
void USkillStateSubsystem::ApplySwingCancel(const FAttackCancelled& C)
{
 const auto* Identity=Fence.Identities.Find(C.Attacker.ToLower());if(!Identity || !Fence.Accept(C.Attacker,Identity->LifeIncarnation,C.Tick))return;
 auto& V=Swings.FindOrAdd(C.Attacker.ToLower());if(C.Tick<V.Tick)return;V.Swing.Reset();V.Tick=C.Tick;
}

float USkillStateSubsystem::CooldownFraction(uint32 Id) const
{
 const auto* R=Reuse.Find(Id);if(!R || R->ReadyTick<=R->Tick || GetTick()>=R->ReadyTick)return 0.f;
 return FMath::Clamp(float(R->ReadyTick-GetTick())/float(R->ReadyTick-R->Tick),0.f,1.f);
}
