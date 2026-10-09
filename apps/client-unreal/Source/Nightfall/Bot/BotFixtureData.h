#pragma once

#include "CoreMinimal.h"

/**
 * The server's data tables, read from `packages/data` so a scenario asserts against the same
 * numbers the server loads (plan E3.2: "computed from experience.toml, not hard-coded"; E3.5: "a
 * changed penalties.toml row fails it"). Test-only: the bot predicates use it, the game never does.
 *
 * Only the fields the Phase 1 scenarios need are parsed, with a line reader for the generated
 * table layout (`{ level = 2, xp = 68 }`, `key = value`, `[section]`, `[[array]]`), not a TOML parser.
 * The directory is `-BotDataDir=<dir>`, else `<project>/../../packages/data`.
 */
struct NIGHTFALL_API FBotFixtureData
{
	struct FSpawnSlot
	{
		FString Id;
		FString Template;
		FVector2D Home = FVector2D::ZeroVector;   // tiles
	};

	TMap<uint32, uint64> XpForLevel;        // tables/experience.toml to_level: level -> cumulative XP
	TMap<uint32, uint64> DeathLossQ;        // tables/penalties.toml death_xp_loss: level -> fraction_q (Q = 1_000_000)
	uint64 RespawnHpQ = 0;                  // tables/formulas.toml [formulas.town_respawn] restore_hp_q
	uint64 RespawnMpQ = 0;                  // ... restore_mp_q
	uint32 SpawnProtectionSeconds = 0;      // ... spawn_protection_seconds
	TOptional<FVector2D> SafePoint;         // zones/test_zone.toml [safe_point] pos
	TArray<FSpawnSlot> SpawnSlots;          // zones/test_zone.toml [[spawn_slots]]
	FString Error;                          // why loading failed; empty when every table was read

	static constexpr uint64 Q = 1000000;

	/** The process-wide copy, loaded on first use. */
	static const FBotFixtureData& Get();

	/** Loads from a `packages/data` directory. */
	static FBotFixtureData Load(const FString& DataDir);

	/** Parses already-read file contents (tests). Missing files are empty strings. */
	static FBotFixtureData Parse(const FString& Experience, const FString& Penalties, const FString& Formulas, const FString& Zone);

	bool IsValid() const { return Error.IsEmpty(); }

	/** Highest level whose threshold is <= Xp (HF ExperienceData). 0 when the table is empty. */
	uint32 LevelForXp(uint64 Xp) const;

	/** XP lost dying at Level: round((X[L+1] - X[L]) * loss_q[L] / Q), Math.round (half up). Unset when a row is missing. */
	TOptional<uint64> DeathXpLoss(uint32 Level) const;

	/** HP after a town respawn: max(1, floor(MaxHp * restore_hp_q / Q)). */
	uint32 RespawnHp(uint32 MaxHp) const;

	/** MP after a town respawn: floor(MaxMp * restore_mp_q / Q). */
	uint32 RespawnMp(uint32 MaxMp) const;

	/** Distance in tiles from Pos to the nearest spawn-slot home of Template (any template when empty). Unset without slots. */
	TOptional<double> NearestHomeDistance(const FVector2D& Pos, const FString& Template) const;
};
