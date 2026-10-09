#include "BotFixtureData.h"
#include "Misc/CommandLine.h"
#include "Misc/FileHelper.h"
#include "Misc/Parse.h"
#include "Misc/Paths.h"

namespace
{
	FString StripTomlComment(const FString& Line)
	{
		// The tables keep '#' out of quoted values, so the first '#' starts the comment.
		int32 Hash = INDEX_NONE;
		FString Text = Line.FindChar(TEXT('#'), Hash) ? Line.Left(Hash) : Line;
		Text.TrimStartAndEndInline();
		return Text;
	}

	FString Unquote(FString Value)
	{
		Value.TrimStartAndEndInline();
		if (Value.Len() >= 2 && Value.StartsWith(TEXT("\"")) && Value.EndsWith(TEXT("\""))) Value = Value.Mid(1, Value.Len() - 2);
		return Value;
	}

	/** `{ a = 1, b = "x" }` -> {a: 1, b: x}. */
	TMap<FString, FString> ParseInlineTable(const FString& Text)
	{
		TMap<FString, FString> Out;
		int32 Open = INDEX_NONE, Close = INDEX_NONE;
		if (!Text.FindChar(TEXT('{'), Open) || !Text.FindLastChar(TEXT('}'), Close) || Close < Open) return Out;
		TArray<FString> Pairs;
		Text.Mid(Open + 1, Close - Open - 1).ParseIntoArray(Pairs, TEXT(","));
		for (const FString& Pair : Pairs)
		{
			FString Key, Value;
			if (Pair.Split(TEXT("="), &Key, &Value)) Out.Add(Key.TrimStartAndEnd(), Unquote(Value));
		}
		return Out;
	}

	/** `[100, 102]` -> (100, 102). */
	bool ParsePair(const FString& Value, FVector2D& Out)
	{
		FString Inner = Value.TrimStartAndEnd();
		if (!Inner.RemoveFromStart(TEXT("[")) || !Inner.RemoveFromEnd(TEXT("]"))) return false;
		FString X, Y;
		if (!Inner.Split(TEXT(","), &X, &Y)) return false;
		X.TrimStartAndEndInline();
		Y.TrimStartAndEndInline();
		if (!X.IsNumeric() || !Y.IsNumeric()) return false;
		Out = FVector2D(FCString::Atod(*X), FCString::Atod(*Y));
		return true;
	}

	/** Calls Visit(section, key, value) for every `key = value` line; `{...}` rows are passed with key "". */
	void ForEachEntry(const FString& Text, TFunctionRef<void(const FString& Section, const FString& Key, const FString& Value)> Visit)
	{
		TArray<FString> Lines;
		Text.ParseIntoArrayLines(Lines);
		FString Section;
		for (const FString& Raw : Lines)
		{
			const FString Line = StripTomlComment(Raw);
			if (Line.IsEmpty()) continue;
			if (Line.StartsWith(TEXT("[[")) && Line.EndsWith(TEXT("]]")))
			{
				Section = Line.Mid(2, Line.Len() - 4).TrimStartAndEnd();
				Visit(Section, TEXT("[[]]"), FString());   // a new array element starts
				continue;
			}
			if (Line.StartsWith(TEXT("[")) && Line.EndsWith(TEXT("]")) && !Line.Contains(TEXT("=")))
			{
				Section = Line.Mid(1, Line.Len() - 2).TrimStartAndEnd();
				continue;
			}
			if (Line.StartsWith(TEXT("{")))
			{
				Visit(Section, FString(), Line);
				continue;
			}
			FString Key, Value;
			if (Line.Split(TEXT("="), &Key, &Value)) Visit(Section, Key.TrimStartAndEnd(), Value.TrimStartAndEnd());
		}
	}

	/** A whole TOML integer (digits, `_` separators allowed); false for anything else (signs, decimals, text). */
	bool ParseU64(const FString& Text, uint64& Out)
	{
		const FString Digits = Text.TrimStartAndEnd().Replace(TEXT("_"), TEXT(""));
		if (Digits.IsEmpty() || Digits.Len() > 19) return false;
		for (const TCHAR C : Digits) if (!FChar::IsDigit(C)) return false;
		Out = FCString::Strtoui64(*Digits, nullptr, 10);
		return true;
	}
}

FBotFixtureData FBotFixtureData::Parse(const FString& Experience, const FString& Penalties, const FString& Formulas, const FString& Zone)
{
	FBotFixtureData D;
	TArray<FString> Bad;   // values that are not whole integers
	auto Number = [&Bad](const FString& What, const FString& Text, uint64& Out)
	{
		if (ParseU64(Text, Out)) return true;
		Bad.AddUnique(FString::Printf(TEXT("%s = %s"), *What, *Text));
		return false;
	};
	auto Rows = [&Number](const FString& Text, const TCHAR* ValueKey, const TCHAR* File, TMap<uint32, uint64>& Out)
	{
		ForEachEntry(Text, [&](const FString&, const FString& Key, const FString& Value)
		{
			if (!Key.IsEmpty()) return;
			const TMap<FString, FString> Row = ParseInlineTable(Value);
			uint64 Level = 0, V = 0;
			if (Row.Contains(TEXT("level")) && Row.Contains(ValueKey)
				&& Number(FString::Printf(TEXT("%s level"), File), Row[TEXT("level")], Level)
				&& Number(FString::Printf(TEXT("%s %s"), File, ValueKey), Row[ValueKey], V))
			{
				Out.Add(static_cast<uint32>(Level), V);
			}
		});
	};
	Rows(Experience, TEXT("xp"), TEXT("experience.toml"), D.XpForLevel);
	Rows(Penalties, TEXT("fraction_q"), TEXT("penalties.toml"), D.DeathLossQ);
	bool bHpQ = false;
	ForEachEntry(Formulas, [&D, &bHpQ, &Number](const FString& Section, const FString& Key, const FString& Value)
	{
		if (Section != TEXT("formulas.town_respawn")) return;
		uint64 V = 0;
		if (Key == TEXT("restore_hp_q")) bHpQ = Number(Key, Value, D.RespawnHpQ);
		else if (Key == TEXT("restore_mp_q")) Number(Key, Value, D.RespawnMpQ);
		else if (Key == TEXT("spawn_protection_seconds") && Number(Key, Value, V)) D.SpawnProtectionSeconds = static_cast<uint32>(V);
	});
	ForEachEntry(Zone, [&D](const FString& Section, const FString& Key, const FString& Value)
	{
		FVector2D Pos;
		if (Section == TEXT("safe_point") && Key == TEXT("pos") && ParsePair(Value, Pos)) D.SafePoint = Pos;
		if (Section != TEXT("spawn_slots")) return;
		if (Key == TEXT("[[]]")) D.SpawnSlots.AddDefaulted();
		else if (D.SpawnSlots.IsEmpty()) return;
		else if (Key == TEXT("id")) D.SpawnSlots.Last().Id = Unquote(Value);
		else if (Key == TEXT("template")) D.SpawnSlots.Last().Template = Unquote(Value);
		else if (Key == TEXT("home") && ParsePair(Value, Pos)) D.SpawnSlots.Last().Home = Pos;
	});

	TArray<FString> Missing;
	if (D.XpForLevel.IsEmpty()) Missing.Add(TEXT("tables/experience.toml to_level"));
	if (D.DeathLossQ.IsEmpty()) Missing.Add(TEXT("tables/penalties.toml death_xp_loss"));
	if (!bHpQ) Missing.Add(TEXT("tables/formulas.toml [formulas.town_respawn]"));
	if (!D.SafePoint.IsSet()) Missing.Add(TEXT("zones/test_zone.toml [safe_point]"));
	if (D.SpawnSlots.IsEmpty()) Missing.Add(TEXT("zones/test_zone.toml [[spawn_slots]]"));
	if (!Bad.IsEmpty()) Missing.Add(FString::Printf(TEXT("not whole integers: %s"), *FString::Join(Bad, TEXT("; "))));
	if (!Missing.IsEmpty()) D.Error = FString::Printf(TEXT("fixture data incomplete: %s"), *FString::Join(Missing, TEXT(", ")));
	return D;
}

FBotFixtureData FBotFixtureData::Load(const FString& DataDir)
{
	auto Read = [&DataDir](const TCHAR* Relative)
	{
		FString Text;
		FFileHelper::LoadFileToString(Text, *FPaths::Combine(DataDir, Relative));
		return Text;
	};
	FBotFixtureData D = Parse(Read(TEXT("tables/experience.toml")), Read(TEXT("tables/penalties.toml")),
		Read(TEXT("tables/formulas.toml")), Read(TEXT("zones/test_zone.toml")));
	ForEachEntry(Read(TEXT("npcs/keltir.toml")), [&D](const FString&, const FString& Key, const FString& Value)
	{
		uint64 Reward = 0;
		if (Key == TEXT("xp_reward") && ParseU64(Value, Reward)) D.KeltirXpReward = Reward;
	});
	if (!D.KeltirXpReward.IsSet()) D.Error += TEXT(" missing npcs/keltir.toml xp_reward");
	if (!D.IsValid()) D.Error = FString::Printf(TEXT("%s (data dir %s; set -BotDataDir=)"), *D.Error, *DataDir);
	return D;
}

const FBotFixtureData& FBotFixtureData::Get()
{
	static const FBotFixtureData Data = []
	{
		FString Dir;
		if (!FParse::Value(FCommandLine::Get(), TEXT("-BotDataDir="), Dir))
		{
			Dir = FPaths::Combine(FPaths::ProjectDir(), TEXT("../../packages/data"));
		}
		FPaths::CollapseRelativeDirectories(Dir);
		return Load(FPaths::ConvertRelativePathToFull(Dir));
	}();
	return Data;
}

uint32 FBotFixtureData::LevelForXp(uint64 Xp) const
{
	uint32 Level = 0;
	for (const TPair<uint32, uint64>& Row : XpForLevel)
	{
		if (Row.Value <= Xp && Row.Key > Level) Level = Row.Key;
	}
	return Level;
}

TOptional<uint64> FBotFixtureData::DeathXpLoss(uint32 Level) const
{
	const uint64* Low = XpForLevel.Find(Level);
	const uint64* High = XpForLevel.Find(Level + 1);
	const uint64* LossQ = DeathLossQ.Find(Level);
	if (!Low || !High || !LossQ || *High < *Low) return {};
	return ((*High - *Low) * *LossQ + Q / 2) / Q;
}

uint32 FBotFixtureData::RespawnHp(uint32 MaxHp) const
{
	return FMath::Max<uint32>(1, static_cast<uint32>(static_cast<uint64>(MaxHp) * RespawnHpQ / Q));
}

uint32 FBotFixtureData::RespawnMp(uint32 MaxMp) const
{
	return static_cast<uint32>(static_cast<uint64>(MaxMp) * RespawnMpQ / Q);
}

TOptional<double> FBotFixtureData::NearestHomeDistance(const FVector2D& Pos, const FString& Template) const
{
	TOptional<double> Best;
	for (const FSpawnSlot& Slot : SpawnSlots)
	{
		if (!Template.IsEmpty() && !Slot.Template.Equals(Template, ESearchCase::IgnoreCase)) continue;
		const double Distance = FVector2D::Distance(Pos, Slot.Home);
		if (!Best.IsSet() || Distance < Best.GetValue()) Best = Distance;
	}
	return Best;
}
