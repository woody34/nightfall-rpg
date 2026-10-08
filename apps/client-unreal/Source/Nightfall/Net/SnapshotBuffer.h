#pragma once

#include "CoreMinimal.h"
#include "ProtoCodec.h"

// Per-entity ring of server positions, rendered INTERP_DELAY_MS behind server time so that
// movement between 10 Hz updates is interpolated, not extrapolated (Phase 0 §3.2, Phase 8 §3).
class FSnapshotBuffer
{
public:
	static constexpr int64 INTERP_DELAY_MS = 150;

	void Push(const FString& EntityId, const FNetVec2& Position, int64 ServerTimeMs);
	void Remove(const FString& EntityId);

	// Position to render now. Returns false if no sample exists for the entity.
	bool Sample(const FString& EntityId, int64 RenderTimeMs, FNetVec2& Out) const;

	// Drop samples older than the render horizon to bound memory.
	void Trim(int64 RenderTimeMs);

private:
	struct FSample { int64 TimeMs; FNetVec2 Pos; };
	TMap<FString, TArray<FSample>> Samples;
};
