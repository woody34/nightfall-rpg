#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "Character/CharacterCreation.h"
#include "Character/ClassStateSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Game/LoginFlowSubsystem.h"
#include "World/RemoteEntityActor.h"
#include "World/WorldProxySubsystem.h"
#include "World/ClassMasterActor.h"
#include "NightfallWire.h"
#include "UI/NightfallClassDialog.h"
#include "UI/NightfallLoginScreen.h"
#include "Blueprint/WidgetTree.h"
#include "Blueprint/UserWidget.h"
#include "Bot/BotSteps.h"
#include "Bot/BotPredicates.h"
#include "HAL/IConsoleManager.h"
#include "HAL/FileManager.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"

#if WITH_DEV_AUTOMATION_TESTS
namespace
{
	constexpr EAutomationTestFlags Flags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;
	FGrpcNightfallV1ListClassesResponse Catalogue()
	{
		FGrpcNightfallV1ListClassesResponse D;
		D.DataVersion = TEXT("test-catalogue"); D.MaxTransferTier = 2; D.PlayableLevelCap = 85;
		D.ClassMaster.Name = TEXT("Guide"); D.ClassMaster.Position.X = 126; D.ClassMaster.Position.Y = 128; D.ClassMaster.InteractionRadius = 3;
		for (uint32 Id : { 0u, 1u, 2u, 88u, 53u })
		{
			auto& C = D.Classes.AddDefaulted_GetRef(); C.ClassId = Id;
			C.DisplayName = FString::Printf(TEXT("Class %u"), Id);
			C.Race = Id == 53 ? EGrpcNightfallV1Race::RACE_DWARF : EGrpcNightfallV1Race::RACE_HUMAN;
			C.Tier = Id == 0 || Id == 53 ? 0 : Id == 1 ? 1 : Id == 2 ? 2 : 3;
			C.ParentClassId = Id == 0 || Id == 53 ? 65535 : Id == 1 ? 0 : Id == 2 ? 1 : 2;
			C.BaseStats.Str = 40; C.BaseStats.Dex = 30; C.BaseStats.Con = 43; C.BaseStats.Int = 21; C.BaseStats.Wit = 11; C.BaseStats.Men = 25;
		}
		for (auto Race : { EGrpcNightfallV1Race::RACE_HUMAN, EGrpcNightfallV1Race::RACE_DWARF })
		{
			auto& R = D.Races.AddDefaulted_GetRef(); R.Race = Race;
			R.DisplayName = Race == EGrpcNightfallV1Race::RACE_HUMAN ? TEXT("Human") : TEXT("Dwarf");
			R.BaseClassIds.Add(Race == EGrpcNightfallV1Race::RACE_HUMAN ? 0u : 53u);
			R.HairStyleCount = 1; R.HairColorCount = 1; R.FaceCount = 1;
		}
		return D;
	}
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FCreationPresenceTest, "Nightfall.Class.Creation.OptionalPresence", Flags)
bool FCreationPresenceTest::RunTest(const FString& Parameters)
{
	FGrpcNightfallV1CreateCharacterRequest Legacy;
	Legacy.Name = TEXT("Abc"); Legacy.Race = EGrpcNightfallV1Race::RACE_ELF;
	TArray<uint8> Bytes;
	NightfallWire::EncodeCreateCharacterRequest(Legacy, Bytes);
	TestTrue(TEXT("legacy body omits class tag entirely"), Bytes == TArray<uint8>({ 0x1a, 3, 'A', 'b', 'c', 0x20, 2 }));
	const auto Human = NightfallCreation::Request(TEXT("Abc"), 1, 0, 2);
	NightfallWire::EncodeCreateCharacterRequest(Human, Bytes);
	TestTrue(TEXT("explicit fighter zero is present, female sex preserved"), Bytes == TArray<uint8>({ 0x1a, 3, 'A', 'b', 'c', 0x20, 1, 0x28, 0, 0x30, 2 }));
	const auto Mystic = NightfallCreation::Request(TEXT("Abc"), 2, 25, 1);
	NightfallWire::EncodeCreateCharacterRequest(Mystic, Bytes);
	TestTrue(TEXT("nonzero explicit class stable"), Bytes == TArray<uint8>({ 0x1a, 3, 'A', 'b', 'c', 0x20, 2, 0x28, 25, 0x30, 1 }));
	FGrpcNightfallV1CreateCharacterRequest RoundTrip;
	TestTrue(TEXT("generated optional decode succeeds"), NightfallWire::DecodeCreateCharacterRequest(Bytes.GetData(), Bytes.Num(), RoundTrip));
	TestEqual(TEXT("nonzero optional decode presence retained"), RoundTrip._base_class_id._base_class_idCase, EGrpcNightfallV1CreateCharacterRequest_base_class_id::BaseClassId);
	TestEqual(TEXT("nonzero optional decode value retained"), RoundTrip._base_class_id.BaseClassId.Value, 25u);
	NightfallWire::EncodeCreateCharacterRequest(Human, Bytes);
	NightfallWire::DecodeCreateCharacterRequest(Bytes.GetData(), Bytes.Num(), RoundTrip);
	TestEqual(TEXT("zero optional decode retains presence"), RoundTrip._base_class_id._base_class_idCase, EGrpcNightfallV1CreateCharacterRequest_base_class_id::BaseClassId);
	TestEqual(TEXT("zero optional decode value retained"), RoundTrip._base_class_id.BaseClassId.Value, 0u);
	NightfallWire::EncodeCreateCharacterRequest(Legacy, Bytes);
	NightfallWire::DecodeCreateCharacterRequest(Bytes.GetData(), Bytes.Num(), RoundTrip);
	TestEqual(TEXT("reuse with absent value clears optional presence"), RoundTrip._base_class_id._base_class_idCase, EGrpcNightfallV1CreateCharacterRequest_base_class_id::NotSet);
	const uint8 UnknownSex[] = { 0x30, 9 };
	NightfallWire::DecodeCreateCharacterRequest(UnknownSex, UE_ARRAY_COUNT(UnknownSex), RoundTrip);
	TestEqual(TEXT("unknown sex preserved for server validation, never coerced"), static_cast<uint32>(RoundTrip.Sex), 9u);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FClassProjectionTest, "Nightfall.Class.State.WireAndFences", Flags)
bool FClassProjectionTest::RunTest(const FString& Parameters)
{
	FScopedTestGameInstance I;
	auto* Net = I.Get<UNetClientSubsystem>(); auto* State = I.Get<UClassStateSubsystem>(); auto* Combat = I.Get<UCombatStateSubsystem>();
	State->ApplyCatalogue(Catalogue()); Net->SetOwnEntityId(TEXT("own"));
	FEntitySpawn Spawn; Spawn.EntityId = TEXT("own"); Spawn.SessionGeneration = 2; Spawn.StateTick = 10; Spawn.ClassId = 0;
	FServerMessage M; FWorldEvent E; E.Spawn = Spawn; M.Event = E; Net->DispatchServerMessage(M);
	TestEqual(TEXT("spawn class zero is known"), State->OwnClassLabel(), FString(TEXT("Class 0")));
	const uint8 Bytes[] = { 0x12, 0x0d, 0x6a, 0x0b, 0x0a, 3, 'o', 'w', 'n', 0x10, 1, 0x18, 15, 0x20, 2 };
	FBotObservations WireObservations; WireObservations.Bind(I.GameInstance);
	TArray<uint8> Envelope(Bytes, UE_ARRAY_COUNT(Bytes));
	Net->OnWireReceived.Broadcast(Envelope); Net->OnWireReceived.Broadcast(Envelope);
	TestEqual(TEXT("strict receipt diagnostics count duplicate raw owner events before admission filtering"), WireObservations.OwnClassWireEvents, 2);
	WireObservations.Unbind();
	TestTrue(TEXT("new wire event decodes"), NightfallProto::Decode(Bytes, UE_ARRAY_COUNT(Bytes), M)); Net->DispatchServerMessage(M);
	TestEqual(TEXT("class event updates projection"), State->OwnClassLabel(), FString(TEXT("Class 1")));
	TestEqual(TEXT("net admission identity updated"), Net->GetKnownEntities()[TEXT("own")].ClassId, 1u);
	E = FWorldEvent(); E.ClassChanged = FClassChanged{ TEXT("own"), 2, 14, 2 }; M.Event = E; Net->DispatchServerMessage(M);
	TestEqual(TEXT("older class fact rejected"), State->OwnClassLabel(), FString(TEXT("Class 1")));
	E = FWorldEvent(); Spawn.StateTick = 11; Spawn.ClassId = 0; E.Spawn = Spawn; M.Event = E; Net->DispatchServerMessage(M);
	TestEqual(TEXT("same-generation older spawn cannot rewind admission fence"), Net->GetKnownEntities()[TEXT("own")].StateTick, uint64(15));
	TestEqual(TEXT("same-generation older spawn cannot rewind class"), State->OwnClassLabel(), FString(TEXT("Class 1")));
	E.ClassChanged = FClassChanged{ TEXT("own"), 2, 20, 1 }; M.Event = E; Net->DispatchServerMessage(M);
	TestEqual(TEXT("older generation rejected"), State->OwnClassLabel(), FString(TEXT("Class 1")));
	E.ClassChanged = FClassChanged{ TEXT("unknown"), 2, 20, 2 }; M.Event = E; Net->DispatchServerMessage(M);
	TestEqual(TEXT("unadmitted identity ignored"), State->OwnClassLabel(), FString(TEXT("Class 1")));
	FStatsChanged Stats; Stats.Entity = TEXT("own"); Stats.Hp = 80; Stats.MaxHp = 100; Stats.Mp = 30; Stats.MaxMp = 40; Stats.Cp = 17; Stats.MaxCp = 25; Stats.ClassId = 1; Stats.Tick = 20;
	Combat->ApplyStats(Stats);
	TestEqual(TEXT("authoritative CP shown"), Combat->BuildHudModel().OwnCpText, FString(TEXT("CP 17 / 25")));
	Stats.Cp = 0; Stats.Tick = 19; Combat->ApplyStats(Stats);
	TestEqual(TEXT("stale owner resources ignored"), Combat->GetOwn().Cp, 17u);
	Stats.Entity = TEXT("other"); Stats.Tick = 30; Combat->ApplyStats(Stats);
	TestEqual(TEXT("other owner resources private"), Combat->GetOwn().Cp, 17u);
	State->ResetWorld(TEXT("drop")); Combat->Reset();
	TestFalse(TEXT("disconnect removes known class"), State->GetOwnClassId().IsSet());
	TestEqual(TEXT("disconnect removes known CP"), Combat->BuildHudModel().OwnCpText, FString(TEXT("CP --")));
	Spawn.ClassId = 1; Spawn.SessionGeneration = 3; Spawn.StateTick = 25; State->ApplySpawn(Spawn);
	TestEqual(TEXT("reconnect class restored by spawn"), State->OwnClassLabel(), FString(TEXT("Class 1")));
	TestEqual(TEXT("catalogue preserved across disconnect"), State->GetCatalogue().DataVersion, FString(TEXT("test-catalogue")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FClassTokenProjectionTest, "Nightfall.Class.State.TokenProjection", Flags)
bool FClassTokenProjectionTest::RunTest(const FString& Parameters)
{
	FScopedTestGameInstance I;
	auto* Net = I.Get<UNetClientSubsystem>();
	auto* Combat = I.Get<UCombatStateSubsystem>();
	auto* State = I.Get<UClassStateSubsystem>();
	Net->SetOwnEntityId(TEXT("own"));

	FBotPredicateRegistry Registry;
	Registry.RegisterBuiltins();
	FBotContext Context{ I.GameInstance, nullptr };

	FString Error;
	auto T1Eq0 = Registry.Parse({ TEXT("own_tier1_tokens"), TEXT("=="), TEXT("0") }, Error);
	TestTrue(TEXT("own_tier1_tokens == 0 parsed successfully"), T1Eq0.IsSet());
	auto T1Eq1 = Registry.Parse({ TEXT("own_tier1_tokens"), TEXT("=="), TEXT("1") }, Error);
	TestTrue(TEXT("own_tier1_tokens == 1 parsed successfully"), T1Eq1.IsSet());
	auto T1Neq0 = Registry.Parse({ TEXT("own_tier1_tokens"), TEXT("!="), TEXT("0") }, Error);
	TestTrue(TEXT("own_tier1_tokens != 0 parsed successfully"), T1Neq0.IsSet());
	auto T1Gt0 = Registry.Parse({ TEXT("own_tier1_tokens"), TEXT(">"), TEXT("0") }, Error);
	TestTrue(TEXT("own_tier1_tokens > 0 parsed successfully"), T1Gt0.IsSet());

	auto T2Eq0 = Registry.Parse({ TEXT("own_tier2_tokens"), TEXT("=="), TEXT("0") }, Error);
	TestTrue(TEXT("own_tier2_tokens == 0 parsed successfully"), T2Eq0.IsSet());
	auto T2Eq1 = Registry.Parse({ TEXT("own_tier2_tokens"), TEXT("=="), TEXT("1") }, Error);
	TestTrue(TEXT("own_tier2_tokens == 1 parsed successfully"), T2Eq1.IsSet());
	auto T2Eq2 = Registry.Parse({ TEXT("own_tier2_tokens"), TEXT("=="), TEXT("2") }, Error);
	TestTrue(TEXT("own_tier2_tokens == 2 parsed successfully"), T2Eq2.IsSet());
	auto T2Neq0 = Registry.Parse({ TEXT("own_tier2_tokens"), TEXT("!="), TEXT("0") }, Error);
	TestTrue(TEXT("own_tier2_tokens != 0 parsed successfully"), T2Neq0.IsSet());
	auto T2Gt0 = Registry.Parse({ TEXT("own_tier2_tokens"), TEXT(">"), TEXT("0") }, Error);
	TestTrue(TEXT("own_tier2_tokens > 0 parsed successfully"), T2Gt0.IsSet());

	if (!T1Eq0.IsSet() || !T1Eq1.IsSet() || !T1Neq0.IsSet() || !T1Gt0.IsSet() ||
		!T2Eq0.IsSet() || !T2Eq1.IsSet() || !T2Eq2.IsSet() || !T2Neq0.IsSet() || !T2Gt0.IsSet())
	{
		return false;
	}

	// 1. Unknown before stats: entity spawn arrives, but private StatsChanged not yet received
	FEntitySpawn Spawn;
	Spawn.EntityId = TEXT("own");
	Spawn.Kind = 1;
	Spawn.SessionGeneration = 1;
	Spawn.StateTick = 5;
	Combat->ApplySpawn(Spawn);

	TestFalse(TEXT("owner private stats unknown before stats"), Combat->GetOwn().bCpKnown);
	TestFalse(TEXT("own_tier1_tokens == 0 is false when unknown"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens observed unknown before stats"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("own_tier1_tokens != 0 is also false when unknown"), T1Neq0(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens != 0 observed unknown before stats"), T1Neq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("own_tier1_tokens > 0 is false when unknown"), T1Gt0(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens > 0 observed unknown before stats"), T1Gt0(Context).Observed, FString(TEXT("unknown")));

	TestFalse(TEXT("own_tier2_tokens == 0 is false when unknown"), T2Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens observed unknown before stats"), T2Eq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("own_tier2_tokens != 0 is also false when unknown"), T2Neq0(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens != 0 observed unknown before stats"), T2Neq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("own_tier2_tokens > 0 is false when unknown"), T2Gt0(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens > 0 observed unknown before stats"), T2Gt0(Context).Observed, FString(TEXT("unknown")));

	// 2. Explicit known zero: authoritative StatsChanged arrives with 0 tokens for both tiers
	FStatsChanged Stats;
	Stats.Entity = TEXT("own");
	Stats.Tick = 10;
	Stats.Hp = 100;
	Stats.MaxHp = 100;
	Stats.Cp = 20;
	Stats.MaxCp = 20;
	Stats.TokenTier1Count = 0;
	Stats.TokenTier2Count = 0;
	Combat->ApplyStats(Stats);

	TestTrue(TEXT("bCpKnown is true after authoritative stats"), Combat->GetOwn().bCpKnown);
	TestEqual(TEXT("subsystem tier1 count is 0"), Combat->GetOwn().TokenTier1Count, 0u);
	TestEqual(TEXT("subsystem tier2 count is 0"), Combat->GetOwn().TokenTier2Count, 0u);
	TestTrue(TEXT("own_tier1_tokens == 0 holds for explicit zero"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens observed 0"), T1Eq0(Context).Observed, FString(TEXT("0")));
	TestFalse(TEXT("own_tier1_tokens > 0 is false for zero"), T1Gt0(Context).bTrue);
	TestFalse(TEXT("own_tier1_tokens != 0 is false for known zero"), T1Neq0(Context).bTrue);
	TestTrue(TEXT("own_tier2_tokens == 0 holds for explicit zero"), T2Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens observed 0"), T2Eq0(Context).Observed, FString(TEXT("0")));
	TestFalse(TEXT("own_tier2_tokens > 0 is false for zero"), T2Gt0(Context).bTrue);
	TestFalse(TEXT("own_tier2_tokens != 0 is false for known zero"), T2Neq0(Context).bTrue);

	// 3. Independent tier updates: tier 1 updates while tier 2 remains unchanged
	Stats.Tick = 15;
	Stats.TokenTier1Count = 1;
	Stats.TokenTier2Count = 0;
	Combat->ApplyStats(Stats);

	TestEqual(TEXT("subsystem tier1 count updated to 1"), Combat->GetOwn().TokenTier1Count, 1u);
	TestEqual(TEXT("subsystem tier2 count remains 0"), Combat->GetOwn().TokenTier2Count, 0u);
	TestTrue(TEXT("own_tier1_tokens == 1 holds"), T1Eq1(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens observed 1"), T1Eq1(Context).Observed, FString(TEXT("1")));
	TestTrue(TEXT("own_tier1_tokens > 0 holds"), T1Gt0(Context).bTrue);
	TestFalse(TEXT("own_tier1_tokens == 0 is now false"), T1Eq0(Context).bTrue);
	TestTrue(TEXT("own_tier2_tokens == 0 remains true independently"), T2Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens observed 0"), T2Eq0(Context).Observed, FString(TEXT("0")));

	// Tier 2 updates independently to 2 while tier 1 remains 1
	Stats.Tick = 20;
	Stats.TokenTier1Count = 1;
	Stats.TokenTier2Count = 2;
	Combat->ApplyStats(Stats);

	TestEqual(TEXT("subsystem tier1 count remains 1"), Combat->GetOwn().TokenTier1Count, 1u);
	TestEqual(TEXT("subsystem tier2 count updated to 2"), Combat->GetOwn().TokenTier2Count, 2u);
	TestTrue(TEXT("own_tier1_tokens == 1 still holds"), T1Eq1(Context).bTrue);
	TestTrue(TEXT("own_tier2_tokens == 2 holds"), T2Eq2(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens observed 2"), T2Eq2(Context).Observed, FString(TEXT("2")));
	TestTrue(TEXT("own_tier2_tokens > 0 holds"), T2Gt0(Context).bTrue);
	TestFalse(TEXT("own_tier2_tokens == 0 is now false"), T2Eq0(Context).bTrue);

	// Tier 1 consumed (1 -> 0) while tier 2 remains 2
	Stats.Tick = 25;
	Stats.TokenTier1Count = 0;
	Stats.TokenTier2Count = 2;
	Combat->ApplyStats(Stats);

	TestEqual(TEXT("subsystem tier1 count consumed to 0"), Combat->GetOwn().TokenTier1Count, 0u);
	TestEqual(TEXT("subsystem tier2 count remains 2"), Combat->GetOwn().TokenTier2Count, 2u);
	TestTrue(TEXT("own_tier1_tokens == 0 holds again"), T1Eq0(Context).bTrue);
	TestTrue(TEXT("own_tier2_tokens == 2 continues to hold"), T2Eq2(Context).bTrue);

	// Verify no formula or receipt: class transfer options or RPC receipts do not alter authoritative tokens
	FGrpcNightfallV1TransferOptionsResponse Options;
	Options.TokenTier1Count = 99;
	Options.TokenTier2Count = 99;
	State->ApplyOptions(Options);
	TestEqual(TEXT("options RPC response does not mutate authoritative tier1 count"), Combat->GetOwn().TokenTier1Count, 0u);
	TestEqual(TEXT("options RPC response does not mutate authoritative tier2 count"), Combat->GetOwn().TokenTier2Count, 2u);
	TestTrue(TEXT("own_tier1_tokens still 0 despite options"), T1Eq0(Context).bTrue);
	TestTrue(TEXT("own_tier2_tokens still 2 despite options"), T2Eq2(Context).bTrue);

	// 4. Ignored stale stats: earlier tick stats must be rejected
	FStatsChanged StaleStats;
	StaleStats.Entity = TEXT("own");
	StaleStats.Tick = 24; // older than current tick 25
	StaleStats.TokenTier1Count = 5;
	StaleStats.TokenTier2Count = 5;
	Combat->ApplyStats(StaleStats);

	TestEqual(TEXT("stale stats ignored: tier1 remains 0"), Combat->GetOwn().TokenTier1Count, 0u);
	TestEqual(TEXT("stale stats ignored: tier2 remains 2"), Combat->GetOwn().TokenTier2Count, 2u);
	TestTrue(TEXT("own_tier1_tokens == 0 holds after stale stats"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens observed 0 after stale stats"), T1Eq0(Context).Observed, FString(TEXT("0")));
	TestTrue(TEXT("own_tier2_tokens == 2 holds after stale stats"), T2Eq2(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens observed 2 after stale stats"), T2Eq2(Context).Observed, FString(TEXT("2")));

	// 5. Ignored other entity stats: stats for another entity must not leak into own token projection
	FStatsChanged OtherStats;
	OtherStats.Entity = TEXT("other");
	OtherStats.Tick = 30;
	OtherStats.TokenTier1Count = 8;
	OtherStats.TokenTier2Count = 9;
	Combat->ApplyStats(OtherStats);

	TestEqual(TEXT("other entity stats ignored: tier1 remains 0"), Combat->GetOwn().TokenTier1Count, 0u);
	TestEqual(TEXT("other entity stats ignored: tier2 remains 2"), Combat->GetOwn().TokenTier2Count, 2u);
	TestTrue(TEXT("own_tier1_tokens == 0 holds after other entity stats"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens observed 0 after other entity stats"), T1Eq0(Context).Observed, FString(TEXT("0")));
	TestTrue(TEXT("own_tier2_tokens == 2 holds after other entity stats"), T2Eq2(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens observed 2 after other entity stats"), T2Eq2(Context).Observed, FString(TEXT("2")));

	// 6. Reset/disconnect unknown: network disconnect resets combat projection to unknown
	Net->OnDisconnected.Broadcast(TEXT("connection lost"));

	TestFalse(TEXT("bCpKnown reset to false on disconnect"), Combat->GetOwn().bCpKnown);
	TestFalse(TEXT("own_tier1_tokens == 0 is false after disconnect"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens observed unknown after disconnect"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("own_tier1_tokens != 0 is false after disconnect"), T1Neq0(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens != 0 observed unknown after disconnect"), T1Neq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("own_tier2_tokens == 2 is false after disconnect"), T2Eq2(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens observed unknown after disconnect"), T2Eq2(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("own_tier2_tokens != 0 is false after disconnect"), T2Neq0(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens != 0 observed unknown after disconnect"), T2Neq0(Context).Observed, FString(TEXT("unknown")));

	// Direct Combat->Reset() also clears private stats
	Combat->ApplyStats(Stats);
	TestTrue(TEXT("stats re-applied before direct reset"), Combat->GetOwn().bCpKnown);
	Combat->Reset();
	TestFalse(TEXT("bCpKnown reset after direct Reset"), Combat->GetOwn().bCpKnown);
	TestFalse(TEXT("own_tier1_tokens is unknown after direct Reset"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens observed unknown after direct Reset"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("own_tier2_tokens is unknown after direct Reset"), T2Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens observed unknown after direct Reset"), T2Eq0(Context).Observed, FString(TEXT("unknown")));

	// 7. Fresh reconnect restoration: seed known token stats immediately before OnConnected to verify connected reset binding
	FStatsChanged PreConnectStats;
	PreConnectStats.Entity = TEXT("own");
	PreConnectStats.Tick = 25;
	PreConnectStats.Hp = 100;
	PreConnectStats.MaxHp = 100;
	PreConnectStats.Cp = 20;
	PreConnectStats.MaxCp = 20;
	PreConnectStats.TokenTier1Count = 1;
	PreConnectStats.TokenTier2Count = 2;
	Combat->ApplyStats(PreConnectStats);

	TestTrue(TEXT("known stats populated immediately before OnConnected"), Combat->GetOwn().bCpKnown);
	TestEqual(TEXT("tier1 count populated before OnConnected"), Combat->GetOwn().TokenTier1Count, 1u);
	TestEqual(TEXT("tier2 count populated before OnConnected"), Combat->GetOwn().TokenTier2Count, 2u);
	TestTrue(TEXT("own_tier1_tokens == 1 before reconnect"), T1Eq1(Context).bTrue);
	TestTrue(TEXT("own_tier2_tokens == 2 before reconnect"), T2Eq2(Context).bTrue);

	// OnConnected must reset combat projection and clear stats fence
	Net->OnConnected.Broadcast();

	TestFalse(TEXT("bCpKnown reset to false on reconnect broadcast"), Combat->GetOwn().bCpKnown);
	TestFalse(TEXT("own_tier1_tokens == 1 is false after reconnect before stats"), T1Eq1(Context).bTrue);
	TestFalse(TEXT("own_tier1_tokens == 0 is false after reconnect before stats"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens observed unknown after reconnect before stats"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("own_tier1_tokens != 0 is false after reconnect before stats"), T1Neq0(Context).bTrue);
	TestFalse(TEXT("own_tier2_tokens == 2 is false after reconnect before stats"), T2Eq2(Context).bTrue);
	TestFalse(TEXT("own_tier2_tokens == 0 is false after reconnect before stats"), T2Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens observed unknown after reconnect before stats"), T2Eq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("own_tier2_tokens != 0 is false after reconnect before stats"), T2Neq0(Context).bTrue);

	FEntitySpawn ReconnectSpawn;
	ReconnectSpawn.EntityId = TEXT("own");
	ReconnectSpawn.SessionGeneration = 2;
	ReconnectSpawn.StateTick = 1;
	Combat->ApplySpawn(ReconnectSpawn);

	TestFalse(TEXT("tokens still unknown after reconnect spawn before fresh stats"), Combat->GetOwn().bCpKnown);
	TestFalse(TEXT("own_tier1_tokens is false after reconnect spawn before stats"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens observed unknown after reconnect spawn before stats"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("own_tier2_tokens is false after reconnect spawn before stats"), T2Eq0(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens observed unknown after reconnect spawn before stats"), T2Eq0(Context).Observed, FString(TEXT("unknown")));

	// Fresh session stats with tick 2 (< previous session tick 25) proves stats-fence was reset
	FStatsChanged FreshStats;
	FreshStats.Entity = TEXT("own");
	FreshStats.Tick = 2;
	FreshStats.Hp = 100;
	FreshStats.MaxHp = 100;
	FreshStats.Cp = 20;
	FreshStats.MaxCp = 20;
	FreshStats.TokenTier1Count = 1;
	FreshStats.TokenTier2Count = 1;
	Combat->ApplyStats(FreshStats);

	TestTrue(TEXT("bCpKnown restored on fresh stats with lower tick"), Combat->GetOwn().bCpKnown);
	TestEqual(TEXT("fresh tier1 restored to 1"), Combat->GetOwn().TokenTier1Count, 1u);
	TestEqual(TEXT("fresh tier2 restored to 1"), Combat->GetOwn().TokenTier2Count, 1u);
	TestTrue(TEXT("own_tier1_tokens == 1 holds after reconnect restoration"), T1Eq1(Context).bTrue);
	TestEqual(TEXT("own_tier1_tokens observed 1 after reconnect"), T1Eq1(Context).Observed, FString(TEXT("1")));
	TestTrue(TEXT("own_tier2_tokens == 1 holds after reconnect restoration"), T2Eq1(Context).bTrue);
	TestEqual(TEXT("own_tier2_tokens observed 1 after reconnect"), T2Eq1(Context).Observed, FString(TEXT("1")));
	TestFalse(TEXT("own_tier1_tokens == 0 is now false"), T1Eq0(Context).bTrue);
	TestFalse(TEXT("own_tier2_tokens == 0 is now false"), T2Eq0(Context).bTrue);
	TestTrue(TEXT("own_tier1_tokens != 0 is now true"), T1Neq0(Context).bTrue);
	TestTrue(TEXT("own_tier2_tokens != 0 is now true"), T2Neq0(Context).bTrue);
	TestTrue(TEXT("own_tier1_tokens > 0 is now true"), T1Gt0(Context).bTrue);
	TestTrue(TEXT("own_tier2_tokens > 0 is now true"), T2Gt0(Context).bTrue);

	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FClassCatalogueUiTest, "Nightfall.Class.UI.CatalogueAndLayouts", Flags)
bool FClassCatalogueUiTest::RunTest(const FString& Parameters)
{
	FScopedTestGameInstance I;
	auto* State = I.Get<UClassStateSubsystem>(); State->ApplyCatalogue(Catalogue());
	TestTrue(TEXT("path uses catalogue parent edges"), State->PathTo(88) == TArray<uint32>({ 0, 1, 2, 88 }));
	TestEqual(TEXT("unknown path is empty"), State->PathTo(999).Num(), 0);
	// Corrupt metadata cannot loop a widget forever; the server independently rejects cycles.
	auto Cyclic = Catalogue(); Cyclic.Classes[0].ParentClassId = 2; State->ApplyCatalogue(Cyclic);
	TestEqual(TEXT("cycle bounded"), State->PathTo(2).Num(), 3); State->ApplyCatalogue(Catalogue());
	UNightfallLoginScreen* Login = CreateWidget<UNightfallLoginScreen>(I.GameInstance, UNightfallLoginScreen::StaticClass());
	Login->EnsureLayout(); Login->ApplyCatalogue();
	TestEqual(TEXT("race cards come from server, not a fixed list"), Login->NumRaceCards(), 2);
	Login->SelectRace(5);
	TestEqual(TEXT("dwarf has only catalogue fighter path"), Login->NumClassChoices(), 1);
	UNightfallClassDialog* Dialog = NewObject<UNightfallClassDialog>(I.GameInstance);
	Dialog->EnsureLayout();
	Dialog->BindState(State, false);
	FGrpcNightfallV1TransferOptionsResponse Options;
	Options.CurrentClassId = 0; Options.TokenTier1Count = 1;
	auto& Choice = Options.Options.AddDefaulted_GetRef(); Choice.ClassId = 1; Choice.Eligible = true;
	State->ApplyOptions(Options);
	TestEqual(TEXT("all lineage nodes rendered from catalogue"), Dialog->NumChoices(), 4);
	TestFalse(TEXT("opening options does not consent to irreversible transfer"), Dialog->CanConfirm());
	Dialog->Select(88);
	TestFalse(TEXT("unavailable tier cannot be transferred"), Dialog->CanConfirm());
	TestTrue(TEXT("unavailable class learning remains inspectable without transfer"), Dialog->SkillDetailsText().Contains(TEXT("Class 88")) && Dialog->SkillDetailsText().Contains(TEXT("unavailable")));
	Dialog->Select(1);
	TestTrue(TEXT("eligible class selection offers separate confirmation"), Dialog->CanConfirm());
	TestTrue(TEXT("confirmation identifies target and token"), Dialog->ConfirmationText().Contains(TEXT("Class 1")) && Dialog->ConfirmationText().Contains(TEXT("tier 1")));
	State->ResetWorld(TEXT("connection lost"));
	TestFalse(TEXT("cleared options withdraw stale permanent-transfer confirmation"), Dialog->CanConfirm());
	TestTrue(TEXT("cleared options explain that a fresh server check is required"), Dialog->ConfirmationText().Contains(TEXT("unavailable")));
	State->ApplyOptions(Options); Dialog->Select(1);
	TestTrue(TEXT("fresh options permit a new explicit confirmation"), Dialog->CanConfirm());
	Options.Options[0].Eligible = false; Options.Options[0].Unmet.Add(TEXT("missing token")); State->ApplyOptions(Options);
	TestFalse(TEXT("changed server eligibility withdraws confirmation"), Dialog->CanConfirm());
	TestNotNull(TEXT("master dialog builds without Blueprint assets"), Dialog->WidgetTree->RootWidget.Get());
	return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FClassRequestLifecycleTest, "Nightfall.Class.State.AccountAndRequestLifetimes", Flags)
bool FClassRequestLifecycleTest::RunTest(const FString& Parameters)
{
	FScopedTestGameInstance I;
	auto* State = I.Get<UClassStateSubsystem>(); auto* Net = I.Get<UNetClientSubsystem>();
	auto* Flow = I.Get<ULoginFlowSubsystem>(); State->ApplyCatalogue(Catalogue());
	Net->SetOwnEntityId(TEXT("old-owner"));
	FEntitySpawn Spawn; Spawn.EntityId = TEXT("old-owner"); Spawn.SessionGeneration = 4; Spawn.StateTick = 50; Spawn.ClassId = 2; State->ApplySpawn(Spawn);
	const uint64 OldRequest = State->Begin(TEXT("options"));
	State->LastCreated.Id = TEXT("old-owner"); State->Character.Id = TEXT("old-owner"); State->CreationCount = 1;
	Flow->LeaveWorld();
	TestFalse(TEXT("explicit logout removes prior admission"), State->OwnClass.IsSet());
	TestTrue(TEXT("explicit logout removes prior character RPC snapshot"), State->Character.Id.IsEmpty() && State->LastCreated.Id.IsEmpty());
	TestFalse(TEXT("logout cancels pending operation"), State->IsBusy());
	Net->SetOwnEntityId(TEXT("new-owner"));
	Spawn.EntityId = TEXT("new-owner"); Spawn.SessionGeneration = 4; Spawn.StateTick = 1; Spawn.ClassId = 0; State->ApplySpawn(Spawn);
	TestEqual(TEXT("new account accepts colliding generation and lower tick"), State->OwnClass.GetValue(), 0u);
	const uint64 NewRequest = State->Begin(TEXT("options"));
	FGrpcNightfallV1TransferOptionsResponse OldOptions; OldOptions.CurrentClassId = 2; OldOptions.TokenTier1Count = 1;
	bool bOldCallback = false;
	State->CompleteOptions(OldRequest, FNetResult(), OldOptions, [&bOldCallback](const FNetResult&) { bOldCallback = true; });
	TestFalse(TEXT("late old-account callback cannot restore options"), State->HasOptions());
	TestFalse(TEXT("late completion cannot notify the old widget"), bOldCallback);
	TestTrue(TEXT("late completion does not finish new-account request"), State->IsBusy());
	FGrpcNightfallV1TransferOptionsResponse NewOptions; NewOptions.CurrentClassId = 0;
	State->CompleteOptions(NewRequest, FNetResult(), NewOptions, nullptr);
	TestTrue(TEXT("current request completes normally"), State->HasOptions() && !State->IsBusy());
	TestEqual(TEXT("new account options survive old response"), State->Options.CurrentClassId.Value, 0u);
	UNightfallLoginScreen* Login = CreateWidget<UNightfallLoginScreen>(I.GameInstance, UNightfallLoginScreen::StaticClass());
	Login->EnsureLayout(); Login->ApplyCatalogue();
	const uint64 Transfer = State->Begin(TEXT("transfer"));
	Login->SelectRace(5);
	TestFalse(TEXT("race selection cannot enable creation during an outstanding mutation"), Login->CanCreate());
	State->LoadOptions(); State->Create(NightfallCreation::Request(TEXT("Abc"), 1, 0, 1)); State->LoadCatalogue();
	TestEqual(TEXT("refresh and creation cannot replace in-flight mutation serial"), State->OperationSerial, Transfer);
	State->Complete(Transfer, FNetResult(), nullptr);
	TestFalse(TEXT("original mutation completion remains deliverable"), State->IsBusy());
	State->Character.ClassId = 2; Spawn.ClassId = 2; Spawn.StateTick = 3; State->ApplySpawn(Spawn);
	FGrpcNightfallV1TransferOptionsResponse CurrentOptions; CurrentOptions.CurrentClassId = 2; State->ApplyOptions(CurrentOptions);
	auto* Combat = I.Get<UCombatStateSubsystem>(); Combat->ApplySpawn(Spawn);
	FStatsChanged CurrentStats; CurrentStats.Entity = TEXT("new-owner"); CurrentStats.ClassId = 2; CurrentStats.Cp = 17; CurrentStats.MaxCp = 25; CurrentStats.Tick = 5; Combat->ApplyStats(CurrentStats);
	const uint64 Retry = State->Begin(TEXT("transfer"));
	FGrpcNightfallV1ChangeClassResponse Historical; Historical.Character.ClassId = 1; Historical.TokenTier2Count = 1; Historical.GrantedSkillKeys.Add(TEXT("frozen-original-grant"));
	State->CompleteTransfer(Retry, FNetResult(), Historical, nullptr);
	TestEqual(TEXT("immutable receipt preserves original class1"), State->GetLastTransferResponse().Character.ClassId.Value, 1u);
	TestEqual(TEXT("historical receipt cannot rewind current Character read"), State->Character.ClassId.Value, 2u);
	TestEqual(TEXT("historical receipt cannot rewind live class"), State->OwnClass.GetValue(), 2u);
	TestEqual(TEXT("historical receipt cannot rewind class-tree options"), State->Options.CurrentClassId.Value, 2u);
	TestEqual(TEXT("historical balance cannot restore a consumed token"), Combat->GetOwn().TokenTier2Count, 0u);
	TestEqual(TEXT("receipt cannot replace current CP"), Combat->GetOwn().Cp, 17u);
	TestEqual(TEXT("receipt cannot invent an extra class event"), State->GetOwnClassEventCount(), 0);
	FBotPredicateRegistry Registry; Registry.RegisterBuiltins(); FString PredicateError;
	auto ExactGrants = Registry.Parse({ TEXT("transfer_granted_keys"), TEXT("frozen-original-grant") }, PredicateError);
	FBotContext GrantContext{ I.GameInstance, nullptr };
	TestTrue(TEXT("exact grant predicate reads the historical receipt"), ExactGrants && ExactGrants(GrantContext).bTrue);
	State->LastGrantedSkillKeys = { TEXT("different-same-count-grant") };
	TestFalse(TEXT("exact grant predicate rejects a different key with the same count"), ExactGrants(GrantContext).bTrue);
	State->LastGrantedSkillKeys = { TEXT("frozen-original-grant"), TEXT("frozen-original-grant") };
	TestFalse(TEXT("exact grant predicate rejects duplicate wire keys"), ExactGrants(GrantContext).bTrue);
	State->ResetAccount();
	State->CompleteTransfer(Retry, FNetResult(), Historical, nullptr);
	TestTrue(TEXT("logout invalidates receipt callbacks and clears prior frozen data"), State->GetLastTransferResponse().GrantedSkillKeys.IsEmpty());
	ARemoteEntityActor* Proxy = I.GameInstance->GetWorld()->SpawnActor<ARemoteEntityActor>();
	Proxy->DisplayName = TEXT("Visitor"); Proxy->SetClassPresentation(1, TEXT("Class 1"), true);
	TestEqual(TEXT("observer proxy preserves authoritative class identity"), Proxy->GetClassId(), 1u);
	TestTrue(TEXT("public transfer title includes visible cue"), Proxy->HasTransferCue() && Proxy->GetNameplate().Contains(TEXT("Class advanced")) && Proxy->GetNameplate().Contains(TEXT("Class 1")));
	return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FClassObserverProjectionTest, "Nightfall.Class.State.ObserverReplacement", Flags)
bool FClassObserverProjectionTest::RunTest(const FString& Parameters)
{
	FScopedTestGameInstance I;
	auto* Net = I.Get<UNetClientSubsystem>(); auto* State = I.Get<UClassStateSubsystem>(); State->ApplyCatalogue(Catalogue()); Net->SetOwnEntityId(TEXT("own"));
	UWorld* World = I.GameInstance->GetWorld(); auto* Proxies = World->GetSubsystem<UWorldProxySubsystem>(); Proxies->EntityClass = ARemoteEntityActor::StaticClass(); Proxies->OnWorldBeginPlay(*World);
	FEntitySpawn Spawn; Spawn.EntityId = TEXT("visitor"); Spawn.Name = TEXT("Visitor"); Spawn.Kind = 1; Spawn.SessionGeneration = 2; Spawn.StateTick = 5; Spawn.ClassId = 0;
	FServerMessage M; FWorldEvent E; E.Spawn = Spawn; M.Event = E; Net->DispatchServerMessage(M);
	auto* Actor = Proxies->GetProxies()[TEXT("visitor")].Get();
	TestEqual(TEXT("observer spawn projects source catalogue title"), Actor->GetNameplate(), FString(TEXT("Visitor — Class 0")));
	E = FWorldEvent(); E.ClassChanged = FClassChanged{ TEXT("visitor"), 1, 10, 2 }; M.Event = E; Net->DispatchServerMessage(M);
	TestTrue(TEXT("admitted transfer updates actual proxy and cue"), Actor->GetClassId() == 1 && Actor->HasTransferCue());
	Net->DispatchServerMessage(M); TestEqual(TEXT("duplicate event shows one observed transfer"), State->GetObservedTransferCount(), 1);
	E = FWorldEvent(); Spawn.ClassId = 2; Spawn.SessionGeneration = 3; Spawn.StateTick = 1; E.Spawn = Spawn; M.Event = E; Net->DispatchServerMessage(M);
	TestEqual(TEXT("replacement admission updates existing actor"), Actor->GetClassId(), 2u);
	TestTrue(TEXT("replacement title reflects accepted identity"), Actor->GetNameplate().Contains(TEXT("Class 2")));
	TestFalse(TEXT("admission does not invent a transfer cue"), Actor->HasTransferCue());
	Spawn.ClassId = 0; Spawn.SessionGeneration = 2; Spawn.StateTick = 99; E.Spawn = Spawn; M.Event = E; Net->DispatchServerMessage(M);
	TestEqual(TEXT("old replacement leaves current presentation intact"), Actor->GetClassId(), 2u);
	E = FWorldEvent(); E.ClassChanged = FClassChanged{ TEXT("visitor"), 1, 100, 2 }; M.Event = E; Net->DispatchServerMessage(M);
	TestEqual(TEXT("old-generation transfer cannot reanimate prior title"), Actor->GetClassId(), 2u);
	E = FWorldEvent(); FEntitySpawn Master; Master.EntityId = TEXT("guide"); Master.Name = State->GetCatalogue().ClassMaster.Name; Master.Kind = 2; Master.Position = { 126, 128 }; E.Spawn = Master; M.Event = E; Net->DispatchServerMessage(M);
	TestNotNull(TEXT("authoritative noncombat master gets dedicated interaction proxy"), Cast<AClassMasterActor>(Proxies->GetProxies()[TEXT("guide")].Get()));
	TestEqual(TEXT("one server master creates one proxy"), Proxies->GetProxies().Num(), 2);
	Net->DispatchServerMessage(M); TestEqual(TEXT("repeated master admission cannot duplicate marker"), Proxies->GetProxies().Num(), 2);
	auto* OldMaster = Proxies->GetProxies()[TEXT("guide")].Get();
	Net->OnDisconnected.Broadcast(TEXT("connection lost"));
	TestEqual(TEXT("disconnect removes all prior admission proxies"), Proxies->GetProxies().Num(), 0);
	TestTrue(TEXT("old master actor is destroyed and cannot remain clickable"), OldMaster->IsActorBeingDestroyed());
	TestTrue(TEXT("old observer actor is destroyed"), Actor->IsActorBeingDestroyed());
	Net->OnConnected.Broadcast();
	Master.EntityId = TEXT("new-epoch-guide"); E.Spawn = Master; M.Event = E; Net->DispatchServerMessage(M);
	TestEqual(TEXT("new epoch master admission creates exactly one proxy"), Proxies->GetProxies().Num(), 1);
	TestNotNull(TEXT("new master uses its received epoch ID"), Cast<AClassMasterActor>(Proxies->GetProxies()[Master.EntityId].Get()));
	auto* ReplacedMaster = Proxies->GetProxies()[Master.EntityId].Get();
	Net->OnConnected.Broadcast();
	TestTrue(TEXT("direct new admission also destroys prior proxies"), ReplacedMaster->IsActorBeingDestroyed());
	TestEqual(TEXT("new admission begins with an empty AOI proxy map"), Proxies->GetProxies().Num(), 0);
	State->ApplyCatalogue(FGrpcNightfallV1ListClassesResponse());
	Master.EntityId = TEXT("late-catalogue-guide"); E.Spawn = Master; M.Event = E; Net->DispatchServerMessage(M);
	auto* GenericGuide = Proxies->GetProxies()[Master.EntityId].Get();
	TestNull(TEXT("before catalogue the noncombat NPC is generic"), Cast<AClassMasterActor>(GenericGuide));
	Spawn.EntityId = TEXT("late-catalogue-visitor"); Spawn.Name = TEXT("Late visitor"); Spawn.ClassId = 0; Spawn.SessionGeneration = 4; Spawn.StateTick = 1; E.Spawn = Spawn; M.Event = E; Net->DispatchServerMessage(M);
	auto* LateVisitor = Proxies->GetProxies()[Spawn.EntityId].Get();
	auto LateCatalogue = Catalogue(); LateCatalogue.Classes[0].DisplayName = TEXT("Human Armsbearer"); State->ApplyCatalogue(LateCatalogue);
	TestTrue(TEXT("late catalogue destroys the generic guide proxy"), GenericGuide->IsActorBeingDestroyed());
	TestNotNull(TEXT("late catalogue promotes the same authoritative ID to clickable master"), Cast<AClassMasterActor>(Proxies->GetProxies()[Master.EntityId].Get()));
	TestEqual(TEXT("late catalogue promotion preserves exactly one guide and one visitor"), Proxies->GetProxies().Num(), 2);
	TestEqual(TEXT("late catalogue updates current player class label"), LateVisitor->GetNameplate(), FString(TEXT("Late visitor — Human Armsbearer")));
	return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FClassScenarioContractTest, "Nightfall.Class.Scenarios.NativeContracts", Flags)
bool FClassScenarioContractTest::RunTest(const FString& Parameters)
{
	FBotPredicateRegistry Registry; Registry.RegisterBuiltins();
	FScopedTestGameInstance I; auto* Net = I.Get<UNetClientSubsystem>(); auto* Combat = I.Get<UCombatStateSubsystem>(); Net->SetOwnEntityId(TEXT("own"));
	FEntitySpawn Spawn; Spawn.EntityId = TEXT("own"); Spawn.Kind = 1; Spawn.Race = 1; Spawn.Level = 1; Spawn.SessionGeneration = 1; Combat->ApplySpawn(Spawn);
	FBotObservations Observation; Observation.XpAtTargetSelection = 0; Observation.LastXpGainedAmount = 29; Observation.TrackedXp = 29;
	FString Error; auto KillXp = Registry.Parse({ TEXT("kill_xp_matches_fixture") }, Error);
	FBotContext Context{ I.GameInstance, &Observation };
	TestTrue(TEXT("actual Human validates independent XP29 oracle"), KillXp(Context).bTrue);
	Observation.LastXpGainedAmount = 28; Observation.TrackedXp = 28;
	TestFalse(TEXT("Human XP28 fails, never silently permits old amount"), KillXp(Context).bTrue);
	Spawn.Race = 0; Combat->ApplySpawn(Spawn);
	TestTrue(TEXT("legacy synthetic unknown race retains raw XP28 oracle"), KillXp(Context).bTrue);
	Spawn.Race = 2; Combat->ApplySpawn(Spawn);
	TestTrue(TEXT("Elf retains raw XP28 oracle"), KillXp(Context).bTrue);
	const FString Directory = FPaths::ProjectDir() / TEXT("Scenarios"); TArray<FString> Files;
	IFileManager::Get().FindFiles(Files, *(Directory / TEXT("2-*.nfs")), true, false);
	TestTrue(TEXT("nine fixed scenario files plus explicit negatives available"), Files.Num() >= 11);
	for (const FString& File : Files)
	{
		FString Source; TestTrue(*File, FFileHelper::LoadFileToString(Source, *(Directory / File)));
		FBotScenario Scenario; TArray<FString> Errors;
		const bool bOk = BotScenario::Parse(File, Source, Registry, [](const FString& Name) { // DropSocket is registered lazily only when the scenario runner is enabled.
			return Name == TEXT("nf.DropSocket") || IConsoleManager::Get().FindConsoleObject(*Name) != nullptr; }, Scenario, Errors);
		TestTrue(*FString::Printf(TEXT("%s: %s"), *File, *FString::Join(Errors, TEXT("; "))), bOk);
	}
	return true;
}
#endif
