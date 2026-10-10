#pragma once
#include "Net/ProtoCodec.h"

/** Identity and hydration fence, scoped to one transport. No gameplay calculations. */
struct FPresentationFence
{
	TMap<FString, FEntitySpawn> Identities;
	TMap<FString, FEntitySpawn> Tombstones;
	uint64 Tick = 0;
	void Reset() { Identities.Reset(); Tombstones.Reset(); Tick = 0; }
	bool Spawn(const FEntitySpawn& S)
	{
		const FString K = S.EntityId.ToLower();
		const auto* Held = Identities.Find(K); if (!Held) Held = Tombstones.Find(K);
		if (Held && (NightfallProto::IsStaleSpawn(*Held, S) || (Held->SessionGeneration == S.SessionGeneration && Held->LifeIncarnation == S.LifeIncarnation && Held->StateTick == S.StateTick && Identities.Contains(K)))) return false;
		Identities.Add(K, S); Tombstones.Remove(K); Tick = FMath::Max(Tick, S.StateTick); return true;
	}
	void Despawn(const FString& Id)
	{
		const FString K = Id.ToLower(); if (const auto* S = Identities.Find(K)) Tombstones.Add(K, *S); Identities.Remove(K);
	}
	bool Respawn(const FEntityRespawned& R)
	{
		auto* S = Identities.Find(R.Entity.ToLower());
		if (!S || R.Incarnation <= S->LifeIncarnation || R.Tick < S->StateTick) return false;
		S->LifeIncarnation = R.Incarnation; S->StateTick = R.Tick; S->bDead = false; S->Hp = R.Hp;
		Tick = FMath::Max(Tick, R.Tick); return true;
	}
	bool Accept(const FString& Id, uint32 Inc, uint64 At, bool bBaseline = false, uint64 BaselineTick = 0) const
	{
		const auto* S = Identities.Find(Id.ToLower());
		return S && Inc == S->LifeIncarnation && (bBaseline ? BaselineTick == S->StateTick && At >= BaselineTick : At > S->StateTick);
	}
	bool Visible(const FString& Id, uint32 Inc) const
	{
		const auto* S = Identities.Find(Id.ToLower()); return S && Inc == S->LifeIncarnation;
	}
};
