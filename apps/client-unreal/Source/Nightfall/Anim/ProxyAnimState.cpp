#include "ProxyAnimState.h"

const TCHAR* LexToString(EProxyAnimState State)
{
	switch (State)
	{
	case EProxyAnimState::Idle: return TEXT("Idle");
	case EProxyAnimState::Walk: return TEXT("Walk");
	case EProxyAnimState::Run: return TEXT("Run");
	case EProxyAnimState::Attack: return TEXT("Attack");
	case EProxyAnimState::Flinch: return TEXT("Flinch");
	case EProxyAnimState::Dying: return TEXT("Dying");
	case EProxyAnimState::Corpse: return TEXT("Corpse");
	}
	return TEXT("?");
}

namespace
{
	void InsertSorted(TArray<int64>& Times, int64 T)
	{
		if (Times.Contains(T)) return;   // a replayed event (same impact tick) changes nothing
		int32 Index = Times.Num();
		while (Index > 0 && Times[Index - 1] > T) --Index;
		Times.Insert(T, Index);
	}
}

void FProxyAnimStateMachine::NoteMove(int64 ServerTimeMs, float SpeedTilesPerSecond)
{
	int32 Index = Moves.Num();
	while (Index > 0 && Moves[Index - 1].TimeMs > ServerTimeMs) --Index;
	Moves.Insert({ ServerTimeMs, FMath::Max(0.f, SpeedTilesPerSecond) }, Index);
}

void FProxyAnimStateMachine::NoteAttack(int64 ImpactServerTimeMs)
{
	InsertSorted(AttackImpacts, ImpactServerTimeMs);
}

void FProxyAnimStateMachine::NoteHit(int64 ImpactServerTimeMs)
{
	InsertSorted(HitImpacts, ImpactServerTimeMs);
}

void FProxyAnimStateMachine::NoteDied(int64 DeathServerTimeMs)
{
	if (AliveMs.IsSet() && DeathServerTimeMs >= *AliveMs)
	{
		// Died again after a pending respawn: the new death is the one that counts.
		DeathMs = DeathServerTimeMs;
		AliveMs.Reset();
	}
	else if (!DeathMs.IsSet())
	{
		DeathMs = DeathServerTimeMs;
	}
}

void FProxyAnimStateMachine::NoteSpawnedDead()
{
	DeathMs = TNumericLimits<int64>::Lowest();
	AliveMs.Reset();
}

bool FProxyAnimStateMachine::IsDeadAt(int64 RenderTimeMs) const
{
	return DeathMs.IsSet() && RenderTimeMs >= *DeathMs && !(AliveMs.IsSet() && RenderTimeMs >= *AliveMs);
}

void FProxyAnimStateMachine::NoteAlive(int64 ServerTimeMs)
{
	if (!DeathMs.IsSet()) return;   // already alive
	if (ServerTimeMs < *DeathMs) return;   // an older life's alive fact
	AliveMs = ServerTimeMs;
	// Swings and flinches of the previous life must not replay on the new one.
	AttackImpacts.RemoveAll([&](int64 T) { return T <= ServerTimeMs; });
	HitImpacts.RemoveAll([&](int64 T) { return T <= ServerTimeMs; });
}

int64 FProxyAnimStateMachine::AttackStartMs(int64 ImpactMs) const
{
	const double Rate = FMath::Max(Timing.AttackPlayRate, 0.01);
	return ImpactMs - FMath::RoundToInt64(Timing.AttackImpactSeconds / Rate * 1000.0);
}

float FProxyAnimStateMachine::SpeedAt(int64 RenderTimeMs) const
{
	if (SpeedOverride >= 0.f) return SpeedOverride;
	float Speed = 0.f;
	for (const FMoveSample& M : Moves)
	{
		if (M.TimeMs > RenderTimeMs) break;
		Speed = M.Speed;
	}
	return Speed;
}

FProxyAnimPose FProxyAnimStateMachine::Evaluate(int64 RenderTimeMs) const
{
	FProxyAnimPose Pose;

	if (IsDeadAt(RenderTimeMs))
	{
		const double Elapsed = *DeathMs == TNumericLimits<int64>::Lowest()
			? Timing.DeathLengthSeconds
			: (RenderTimeMs - *DeathMs) / 1000.0;
		Pose.Clip = EProxyClip::Death;
		Pose.bLoop = false;
		Pose.State = Elapsed < Timing.DeathLengthSeconds ? EProxyAnimState::Dying : EProxyAnimState::Corpse;
		Pose.ClipTime = FMath::Min(Elapsed, Timing.DeathLengthSeconds);
		return Pose;
	}

	// The newest swing that has started by now and is still playing.
	const double Rate = FMath::Max(Timing.AttackPlayRate, 0.01);
	for (int32 I = AttackImpacts.Num() - 1; I >= 0; --I)
	{
		const int64 Start = AttackStartMs(AttackImpacts[I]);
		if (Start > RenderTimeMs) continue;
		const double ClipTime = (RenderTimeMs - Start) / 1000.0 * Rate;
		if (ClipTime < Timing.AttackLengthSeconds)
		{
			Pose.State = EProxyAnimState::Attack;
			Pose.Clip = EProxyClip::Attack;
			Pose.ClipTime = ClipTime;
			Pose.bLoop = false;
			Pose.PlayRate = Rate;
			return Pose;
		}
		break;
	}

	if (Timing.FlinchSeconds > 0.0)
	{
		for (int32 I = HitImpacts.Num() - 1; I >= 0; --I)
		{
			if (HitImpacts[I] > RenderTimeMs) continue;
			const double Elapsed = (RenderTimeMs - HitImpacts[I]) / 1000.0;
			if (Elapsed < Timing.FlinchSeconds)
			{
				Pose.State = EProxyAnimState::Flinch;
				Pose.Clip = EProxyClip::Flinch;
				Pose.ClipTime = Elapsed;
				Pose.bLoop = false;
				return Pose;
			}
			break;
		}
	}

	const float Speed = SpeedAt(RenderTimeMs);
	// Loops are phased on absolute render time so a state re-entered later does not restart its cycle.
	Pose.ClipTime = RenderTimeMs / 1000.0;
	Pose.bLoop = true;
	if (Speed <= KINDA_SMALL_NUMBER)
	{
		Pose.State = EProxyAnimState::Idle;
		Pose.Clip = EProxyClip::Idle;
	}
	else if (Timing.RunSpeedTilesPerSecond > 0.f && Speed >= Timing.RunSpeedTilesPerSecond)
	{
		Pose.State = EProxyAnimState::Run;
		Pose.Clip = EProxyClip::Run;
	}
	else
	{
		Pose.State = EProxyAnimState::Walk;
		Pose.Clip = EProxyClip::Walk;
	}
	return Pose;
}

void FProxyAnimStateMachine::Prune(int64 RenderTimeMs)
{
	if (AliveMs.IsSet() && RenderTimeMs >= *AliveMs)
	{
		DeathMs.Reset();
		AliveMs.Reset();
	}

	// Keep the newest sample at or before RenderTimeMs: it is the current speed.
	int32 Current = INDEX_NONE;
	for (int32 I = 0; I < Moves.Num() && Moves[I].TimeMs <= RenderTimeMs; ++I) Current = I;
	if (Current > 0) Moves.RemoveAt(0, Current);

	const double Rate = FMath::Max(Timing.AttackPlayRate, 0.01);
	const int64 AttackSpanMs = FMath::RoundToInt64(Timing.AttackLengthSeconds / Rate * 1000.0);
	AttackImpacts.RemoveAll([&](int64 T) { return AttackStartMs(T) + AttackSpanMs < RenderTimeMs - 1000; });
	const int64 FlinchSpanMs = FMath::RoundToInt64(Timing.FlinchSeconds * 1000.0);
	HitImpacts.RemoveAll([&](int64 T) { return T + FlinchSpanMs < RenderTimeMs - 1000; });
}
