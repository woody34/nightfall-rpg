#include "SkillStateSubsystem.h"
#include "Net/NetClientSubsystem.h"

void USkillStateSubsystem::Initialize(FSubsystemCollectionBase& C)
{
	Super::Initialize(C); Net = C.InitializeDependency<UNetClientSubsystem>();
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
	Net->OnIntentAck.AddWeakLambda(this, [this](const FAck& A) { Fence.Tick = FMath::Max(Fence.Tick, A.Tick); });
	Net->OnEntityMove.AddWeakLambda(this, [this](const FEntityMove& M) { Fence.Tick = FMath::Max(Fence.Tick, M.Tick); });
	Net->OnVitalsChanged.AddWeakLambda(this, [this](const FVitalsChanged& V) { if (Fence.Visible(V.Entity, V.Incarnation)) Fence.Tick = FMath::Max(Fence.Tick, V.Tick); });
	Net->OnConnected.AddDynamic(this, &USkillStateSubsystem::Reset); Net->OnDisconnected.AddDynamic(this, &USkillStateSubsystem::Disconnected);
}
void USkillStateSubsystem::Deinitialize()
{
	if (Net)
	{
		Net->OnEntitySpawn.RemoveAll(this); Net->OnEntityDespawn.RemoveAll(this); Net->OnEntityRespawned.RemoveAll(this); Net->OnSkillStateSync.RemoveAll(this); Net->OnSkillReuse.RemoveAll(this); Net->OnSkillLearned.RemoveAll(this); Net->OnStatsChanged.RemoveAll(this); Net->OnSpGained.RemoveAll(this); Net->OnPvpStateChanged.RemoveAll(this); Net->OnResurrectOffer.RemoveAll(this); Net->OnSkillCastStarted.RemoveAll(this); Net->OnSkillCastResult.RemoveAll(this); Net->OnSkillCastCancelled.RemoveAll(this); Net->OnIntentAck.RemoveAll(this); Net->OnEntityMove.RemoveAll(this); Net->OnVitalsChanged.RemoveAll(this); Net->OnConnected.RemoveAll(this); Net->OnDisconnected.RemoveAll(this);
	}
	Super::Deinitialize();
}
bool USkillStateSubsystem::IsOwn(const FString& Id) const { return Net && Net->IsOwnEntity(Id); }
void USkillStateSubsystem::ClearOwner()
{
	Learned.Reset(); Reuse.Reset(); Sp.Reset(); Offer.Reset(); LearnedEvents.Reset(); SpGainEvents.Reset(); OwnerTick = 0;
	SyncTick = StatsTick = SpTick = LastSpGainTick = OfferTick = SpGainedTotal = 0; DeathPenalty = 0;
	bOwnerKnown = bSyncKnown = bStatsKnown = bSpGainKnown = false;
}
void USkillStateSubsystem::Reset() { Fence.Reset(); Pvp.Reset(); Casts.Reset(); ClearOwner(); CastTransitions = 0; OnChanged.Broadcast(); }
void USkillStateSubsystem::ApplySpawn(const FEntitySpawn& S)
{
	if (!Fence.Spawn(S)) return;
	Casts.Remove(S.EntityId.ToLower());
	if (IsOwn(S.EntityId)) ClearOwner();
	FPvpStateChanged P; P.Entity = S.EntityId; P.Incarnation = S.LifeIncarnation; P.Karma = S.Karma; P.PkCount = S.PkCount; P.PvpCount = S.PvpCount; P.FlagUntilTick = S.FlagUntilTick; P.Tick = S.StateTick;
	Pvp.Add(S.EntityId.ToLower(), P);
	if (S.PendingCast.IsSet()) { auto Cast = *S.PendingCast; Cast.bBaseline = true; Cast.BaselineTick = S.StateTick; Cast.Tick = S.StateTick; ApplyCast(Cast); }
	OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyDespawn(const FEntityDespawn& D)
{
	Fence.Despawn(D.EntityId); Casts.Remove(D.EntityId.ToLower()); Pvp.Remove(D.EntityId.ToLower()); if (IsOwn(D.EntityId)) ClearOwner(); OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyRespawn(const FEntityRespawned& R)
{
	if (!Fence.Respawn(R)) return;
	Casts.Remove(R.Entity.ToLower()); if (auto* P = Pvp.Find(R.Entity.ToLower())) { P->Incarnation = R.Incarnation; P->Tick = R.Tick; }
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
	if (LearnedEvents.Contains(Key)) return; LearnedEvents.Add(Key); OwnerTick = FMath::Max(OwnerTick, L.Tick);
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
	if (SpGainEvents.Contains(Key)) return; SpGainEvents.Add(Key); OwnerTick = FMath::Max(OwnerTick, S.Tick);
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
	if (!IsOwn(O.Target) || !Fence.Accept(O.Target, O.TargetIncarnation, O.Tick) || O.Tick < OfferTick || !O.OfferId) return;
	if (Offer.IsSet() && O.OfferId != Offer->OfferId && O.State != 1) return;
	Offer = O; OfferTick = O.Tick;
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
	Fence.Tick = FMath::Max(Fence.Tick, S.Tick); if (!S.bBaseline) ++CastTransitions; OnChanged.Broadcast();
}
void USkillStateSubsystem::FinishCast(const FString& Caster, uint32 Inc, uint64 Id, uint64 At, uint32 Reason)
{
	if (!Fence.Accept(Caster, Inc, At)) return;
	auto& V = Casts.FindOrAdd(Caster.ToLower());
	if (At < V.Tick || Id <= V.TerminalId || (V.Cast.IsSet() && Id != V.Cast->CastId)) return;
	V.Cast.Reset(); V.TerminalId = Id; V.Tick = At; V.CancelReason = Reason; ++CastTransitions; Fence.Tick = FMath::Max(Fence.Tick, At); OnChanged.Broadcast();
}
void USkillStateSubsystem::ApplyResult(const FSkillCastResult& R) { FinishCast(R.Caster, R.CasterIncarnation, R.CastId, R.Tick, 0); }
void USkillStateSubsystem::ApplyCancel(const FSkillCastCancelled& C) { FinishCast(C.Caster, C.CasterIncarnation, C.CastId, C.Tick, C.Reason); }
TOptional<uint64> USkillStateSubsystem::ReuseRemaining(uint32 Id) const
{
	if (!bSyncKnown || !Learned.Contains(Id)) return {};
	const auto* R = Reuse.Find(Id); return R && R->ReadyTick > GetTick() ? R->ReadyTick - GetTick() : uint64(0);
}
FString USkillStateSubsystem::Flag(const FString& Entity) const
{
	const auto* P = FindPvp(Entity); return !P ? TEXT("unknown") : P->Karma ? TEXT("chaotic") : P->FlagUntilTick > GetTick() ? TEXT("pvp") : TEXT("none");
}
bool USkillStateSubsystem::OfferPending() const { return Offer.IsSet() && Offer->State == 1 && Offer->ExpiryTick > GetTick(); }

uint64 USkillStateSubsystem::GetTick() const { return FMath::Max(Fence.Tick, Net ? Net->GetLatestReceivedTick() : uint64(0)); }
