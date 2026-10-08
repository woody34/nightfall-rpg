#include "SnapshotBuffer.h"

void FSnapshotBuffer::Push(const FString& EntityId, const FNetVec2& Position, int64 ServerTimeMs)
{
	TArray<FSample>& Ring = Samples.FindOrAdd(EntityId);
	// Out-of-order packets: keep the ring sorted by time.
	int32 Index = Ring.Num();
	while (Index > 0 && Ring[Index - 1].TimeMs > ServerTimeMs) --Index;
	Ring.Insert({ ServerTimeMs, Position }, Index);
}

void FSnapshotBuffer::Remove(const FString& EntityId)
{
	Samples.Remove(EntityId);
}

bool FSnapshotBuffer::Sample(const FString& EntityId, int64 RenderTimeMs, FNetVec2& Out) const
{
	const TArray<FSample>* Ring = Samples.Find(EntityId);
	if (!Ring || Ring->Num() == 0) return false;

	const int64 T = RenderTimeMs - INTERP_DELAY_MS;
	if (T <= (*Ring)[0].TimeMs) { Out = (*Ring)[0].Pos; return true; }
	const FSample& Last = Ring->Last();
	if (T >= Last.TimeMs) { Out = Last.Pos; return true; } // hold, do not extrapolate

	for (int32 i = 1; i < Ring->Num(); ++i)
	{
		const FSample& A = (*Ring)[i - 1];
		const FSample& B = (*Ring)[i];
		if (T <= B.TimeMs)
		{
			const float Alpha = (B.TimeMs == A.TimeMs) ? 1.f
				: static_cast<float>(T - A.TimeMs) / static_cast<float>(B.TimeMs - A.TimeMs);
			Out.X = FMath::Lerp(A.Pos.X, B.Pos.X, Alpha);
			Out.Y = FMath::Lerp(A.Pos.Y, B.Pos.Y, Alpha);
			return true;
		}
	}
	Out = Last.Pos;
	return true;
}

void FSnapshotBuffer::Trim(int64 RenderTimeMs)
{
	const int64 Horizon = RenderTimeMs - INTERP_DELAY_MS * 4;
	for (auto& Pair : Samples)
	{
		TArray<FSample>& Ring = Pair.Value;
		int32 Keep = 0;
		while (Keep + 1 < Ring.Num() && Ring[Keep + 1].TimeMs < Horizon) ++Keep;
		if (Keep > 0) Ring.RemoveAt(0, Keep);
	}
}
