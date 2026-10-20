#pragma once

#include "CoreMinimal.h"

/**
 * Local estimate of the zone's current tick between received tick-bearing frames (D132, T22).
 *
 * The zone runs a fixed 100 ms tick, but it sends a frame to an observer only when something the
 * observer can see changes. On V6 an observer with full MP gets no regen updates, so the newest
 * received tick can stay 20+ ticks behind the zone. A deadline-relative predicate
 * ("flag deadline - current tick") read from the newest received tick then jumps from 22 to 0.
 *
 * The estimate is the newest received tick plus the whole 100 ms ticks of the local monotonic
 * clock since that tick arrived. Arrival is always after the zone produced the tick, so between
 * frames the estimate trails the zone. It is rebased on every newer received tick. If a frame
 * arrives with a tick lower than the projection (the zone ran slower than 100 ms per tick), the
 * estimate is clamped down to that received tick: it never claims more than received evidence
 * once that evidence arrives. Nothing is sent to the server; this only feeds local reads.
 */
struct FTickEstimator
{
	static constexpr double TICK_SECONDS = 0.1;

	/** Forget the base (new socket / zone epoch). */
	void Reset() { BaseTick = 0; BaseSeconds = 0.0; bHasBase = false; }

	/**
	 * The newest received tick is Tick at local monotonic time NowSeconds. A newer tick rebases the
	 * estimate on it: upward when it is ahead of the projection, and clamped down to it when the
	 * projection had run past it. A repeat of the current base tick (another frame of that tick,
	 * a message without a newer tick) carries no new information and keeps the base.
	 */
	void Observe(uint64 Tick, double NowSeconds)
	{
		if (!bHasBase || Tick > BaseTick)
		{
			BaseTick = Tick;
			BaseSeconds = NowSeconds;
			bHasBase = true;
		}
	}

	/** Estimated zone tick at local monotonic time NowSeconds (0 before the first tick). */
	uint64 Estimate(double NowSeconds) const { return bHasBase ? Project(NowSeconds) : 0; }

	bool HasBase() const { return bHasBase; }
	uint64 GetBaseTick() const { return BaseTick; }

private:
	uint64 Project(double NowSeconds) const
	{
		const double Elapsed = NowSeconds - BaseSeconds;
		if (!(Elapsed > 0.0)) return BaseTick;   // clock went backwards or NaN: hold the received tick
		// A tiny epsilon keeps exact multiples of 0.1 s (binary inexact) on the right side of floor.
		const double Whole = FMath::FloorToDouble(Elapsed / TICK_SECONDS + 1e-9);
		const uint64 Ahead = Whole >= double(MAX_uint64 - BaseTick) ? MAX_uint64 - BaseTick : uint64(Whole);
		return BaseTick + Ahead;
	}

	uint64 BaseTick = 0;
	double BaseSeconds = 0.0;
	bool bHasBase = false;
};
