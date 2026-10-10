#include "EffectStateSubsystem.h"
#include "Net/NetClientSubsystem.h"

void UEffectStateSubsystem::Initialize(FSubsystemCollectionBase& C)
{
	Super::Initialize(C); Net = C.InitializeDependency<UNetClientSubsystem>();
	Net->OnEntitySpawn.AddUObject(this, &UEffectStateSubsystem::ApplySpawn);
	Net->OnEntityRespawned.AddUObject(this, &UEffectStateSubsystem::ApplyRespawn);
	Net->OnEntityDespawn.AddUObject(this, &UEffectStateSubsystem::ApplyDespawn);
	Net->OnEffectApplied.AddUObject(this, &UEffectStateSubsystem::ApplyEffect);
	Net->OnEffectRemoved.AddUObject(this, &UEffectStateSubsystem::ApplyRemoved);
	Net->OnIntentAck.AddWeakLambda(this, [this](const FAck& A) { Fence.Tick = FMath::Max(Fence.Tick, A.Tick); });
	Net->OnEntityMove.AddWeakLambda(this, [this](const FEntityMove& M) { Fence.Tick = FMath::Max(Fence.Tick, M.Tick); });
	Net->OnVitalsChanged.AddWeakLambda(this, [this](const FVitalsChanged& V) { if (Fence.Visible(V.Entity, V.Incarnation)) Fence.Tick = FMath::Max(Fence.Tick, V.Tick); });
	Net->OnConnected.AddDynamic(this, &UEffectStateSubsystem::Reset);
	Net->OnDisconnected.AddDynamic(this, &UEffectStateSubsystem::Disconnected);
}
void UEffectStateSubsystem::Deinitialize()
{
	if (Net) { Net->OnEntitySpawn.RemoveAll(this); Net->OnEntityRespawned.RemoveAll(this); Net->OnEntityDespawn.RemoveAll(this); Net->OnEffectApplied.RemoveAll(this); Net->OnEffectRemoved.RemoveAll(this); Net->OnIntentAck.RemoveAll(this); Net->OnEntityMove.RemoveAll(this); Net->OnVitalsChanged.RemoveAll(this); Net->OnConnected.RemoveAll(this); Net->OnDisconnected.RemoveAll(this); }
	Super::Deinitialize();
}
void UEffectStateSubsystem::Reset() { Fence.Reset(); Effects.Reset(); Changes.Reset(); PendingReplacements.Reset(); ReplacementCounts.Reset(); LastRemoveReason.Reset(); bReplacementOrdered.Reset(); TransitionCount = 0; }
void UEffectStateSubsystem::ApplySpawn(const FEntitySpawn& S)
{
	if (!Fence.Spawn(S)) return;
	Effects.Remove(S.EntityId.ToLower()); Changes.Remove(S.EntityId.ToLower()); PendingReplacements.Remove(S.EntityId.ToLower()); ReplacementCounts.Remove(S.EntityId.ToLower());
	for (const auto& A : S.ActiveEffects)
	{
		FEffectApplied E; E.Target = S.EntityId; E.TargetIncarnation = S.LifeIncarnation; E.EffectId = A.EffectId; E.SkillId = A.SkillId; E.SkillLevel = A.SkillLevel; E.Pool = A.bIsDebuff ? 3 : 1;
		E.Caster = A.Caster; E.CasterIncarnation = A.CasterIncarnation; E.Tick = S.StateTick;
		// Reserved legacy baseline summaries are interpreted without any stack/capacity rules.
		if (A.RemainingTicks > MAX_uint64 - S.StateTick) continue;
		E.EndTick = A.RemainingTicks ? S.StateTick + A.RemainingTicks : 0; E.bBaseline = true; E.BaselineTick = S.StateTick; ApplyEffect(E);
	}
}
void UEffectStateSubsystem::ApplyRespawn(const FEntityRespawned& R) { if (Fence.Respawn(R)) { Effects.Remove(R.Entity.ToLower()); Changes.Remove(R.Entity.ToLower()); PendingReplacements.Remove(R.Entity.ToLower()); ReplacementCounts.Remove(R.Entity.ToLower()); } }
void UEffectStateSubsystem::ApplyDespawn(const FEntityDespawn& D) { Fence.Despawn(D.EntityId); Effects.Remove(D.EntityId.ToLower()); Changes.Remove(D.EntityId.ToLower()); PendingReplacements.Remove(D.EntityId.ToLower()); ReplacementCounts.Remove(D.EntityId.ToLower()); }
void UEffectStateSubsystem::ApplyEffect(const FEffectApplied& E)
{
	if (!E.EffectId || !Fence.Accept(E.Target, E.TargetIncarnation, E.Tick, E.bBaseline, E.BaselineTick)) return;
	auto& Changed = Changes.FindOrAdd(E.Target.ToLower());
	if (const auto* At = Changed.Find(E.EffectId); At && E.Tick <= *At) return;
	FEffectApplied Copy = E;
	if (!Copy.Caster.IsSet() || !Copy.CasterIncarnation.IsSet() || !Fence.Visible(*Copy.Caster, *Copy.CasterIncarnation)) { Copy.Caster.Reset(); Copy.CasterIncarnation.Reset(); }
	if(!E.bBaseline)
 {
  if(auto* Pending=PendingReplacements.Find(E.Target.ToLower()))
   for(auto It=Pending->CreateIterator();It;++It)
    if(It.Value().SkillId==E.SkillId && It.Value().Pool==E.Pool && It.Key()!=E.EffectId && E.Tick>=It.Value().Tick)
    {bReplacementOrdered=true;++ReplacementCounts.FindOrAdd(E.Target.ToLower());It.RemoveCurrent();break;}
 }
 Effects.FindOrAdd(E.Target.ToLower()).Add(E.EffectId, Copy); Changed.Add(E.EffectId, E.Tick);
	Fence.Tick = FMath::Max(Fence.Tick, E.Tick); if (!E.bBaseline) ++TransitionCount;
}
void UEffectStateSubsystem::ApplyRemoved(const FEffectRemoved& E)
{
	if (!Fence.Accept(E.Target, E.TargetIncarnation, E.Tick)) return;
	auto& Changed = Changes.FindOrAdd(E.Target.ToLower());
	if (const auto* At = Changed.Find(E.EffectId); At && E.Tick < *At) return;
	// Removal may legitimately follow application in the same tick; duplicate removals are no-ops.
	auto& Rows = Effects.FindOrAdd(E.Target.ToLower());
 if(const auto* Old=Rows.Find(E.EffectId))
 {
  LastRemoveReason=E.Reason;
  if(E.Reason==2 && (Old->Pool==1 || Old->Pool==2)){auto Copy=*Old;Copy.Tick=E.Tick;PendingReplacements.FindOrAdd(E.Target.ToLower()).Add(E.EffectId,Copy);bReplacementOrdered=false;}
  Rows.Remove(E.EffectId);++TransitionCount;
 }
	Changed.Add(E.EffectId, E.Tick); Fence.Tick = FMath::Max(Fence.Tick, E.Tick);
}
TArray<FEffectApplied> UEffectStateSubsystem::Active(const FString& Entity) const
{
	TArray<FEffectApplied> Out;
	if (const auto* Rows = Effects.Find(Entity.ToLower())) for (const auto& Pair : *Rows) if (!Pair.Value.EndTick || Pair.Value.EndTick > GetTick()) Out.Add(Pair.Value);
	Out.Sort([](const auto& A, const auto& B) { return A.Pool != B.Pool ? A.Pool < B.Pool : A.EffectId < B.EffectId; }); return Out;
}
bool UEffectStateSubsystem::Has(const FString& Entity, uint32 SkillId, TOptional<uint32> Pool) const
{
	for (const auto& E : Active(Entity)) if (E.SkillId == SkillId && (!Pool.IsSet() || E.Pool == *Pool)) return true; return false;
}

uint64 UEffectStateSubsystem::GetTick() const { return FMath::Max(Fence.Tick, Net ? Net->GetLatestReceivedTick() : uint64(0)); }
