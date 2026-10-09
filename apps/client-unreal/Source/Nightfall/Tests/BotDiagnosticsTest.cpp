#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "Bot/BotDiagnostics.h"
#include "Bot/BotSteps.h"
#include "Combat/CombatStateSubsystem.h"
#include "Dom/JsonObject.h"
#include "Net/NetClientSubsystem.h"
#include "Serialization/JsonReader.h"
#include "Serialization/JsonSerializer.h"

#if WITH_DEV_AUTOMATION_TESTS
namespace
{
	constexpr EAutomationTestFlags Flags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotFailureBundleTest, "Nightfall.Bot.Diagnostics.FailureBundle", Flags)
bool FBotFailureBundleTest::RunTest(const FString& Parameters)
{
	FScopedTestGameInstance GI;
	UNetClientSubsystem* Net = GI.Get<UNetClientSubsystem>();
	UCombatStateSubsystem* Combat = GI.Get<UCombatStateSubsystem>();
	const FString OwnId = TEXT("0b6e2f6e-0000-4000-8000-000000000001");
	const FString NpcId = TEXT("0b6e2f6e-0000-4000-8000-0000000000a1");
	Net->SetOwnEntityId(OwnId);
	FServerMessage Spawn;
	Spawn.Event.Emplace();
	Spawn.Event->Spawn.Emplace();
	Spawn.Event->Spawn->EntityId = OwnId;
	Spawn.Event->Spawn->Kind = 1;
	Spawn.Event->Spawn->bCombatant = true;
	Spawn.Event->Spawn->Hp = 81;
	Spawn.Event->Spawn->MaxHp = 126;
	Spawn.Event->Spawn->Level = 1;
	Net->DispatchServerMessage(Spawn);
	Spawn.Event->Spawn->EntityId = NpcId;
	Spawn.Event->Spawn->Kind = 2;
	Spawn.Event->Spawn->LifeIncarnation = 7;
	Spawn.Event->Spawn->Hp = 9;
	Net->DispatchServerMessage(Spawn);
	Combat->ApplyStats(FStatsChanged{ OwnId, 81, 126, 0, 48, 1, 41 });
	FBotDiagnostics Diagnostics;
	// ServerMessage.ack { seq:1, tick:42 }; retain only the last50 envelopes.
	const TArray<uint8> Ack = { 0x0a, 0x04, 0x08, 0x01, 0x10, 0x2a };
	for (int32 I = 0; I < 60; ++I) Diagnostics.ObserveFrame(false, Ack, 100.0 + I);
	const FBotContext Context{ GI.GameInstance, nullptr };
	Diagnostics.BeginMove(150.0);
	Diagnostics.ObservePosition(Context, 150.0);
	const FBotFailedStep Failed{ 12, TEXT("nf.WaitFor target_hp == 0 10"), TEXT("target_hp == 0"), TEXT("9"), 10.25, 20.5 };
	const auto Bundle = Diagnostics.FailureBundle(TEXT("seeded-failure"), OwnId, Context, Failed, TEXT("timed out"));
	TSharedPtr<FJsonObject> Parsed;
	TestTrue(TEXT("versioned bundle parses as JSON"), FJsonSerializer::Deserialize(TJsonReaderFactory<>::Create(FBotDiagnostics::Json(Bundle)), Parsed));
	if (!Parsed.IsValid()) return false;
	TestEqual(TEXT("schema version"), Parsed->GetIntegerField(TEXT("schema_version")), FBotDiagnostics::FailureSchemaVersion);
	TestEqual(TEXT("bounded last50 typed events"), Parsed->GetArrayField(TEXT("events")).Num(), 50);
	TestEqual(TEXT("oldest retained arrival"), Parsed->GetArrayField(TEXT("events"))[0]->AsObject()->GetNumberField(TEXT("arrival_monotonic_seconds")), 110.0);
	TestEqual(TEXT("failing tick reproducible"), Parsed->GetStringField(TEXT("tick_last")), FString(TEXT("42")));
	TestEqual(TEXT("session selector"), Parsed->GetStringField(TEXT("replay_session")), OwnId);
	const auto Step = Parsed->GetObjectField(TEXT("failure"));
	TestEqual(TEXT("last evaluated predicate value"), Step->GetStringField(TEXT("last_value")), FString(TEXT("9")));
	TestEqual(TEXT("wait retained"), Step->GetNumberField(TEXT("wait_seconds")), 10.25);
	const auto Own = Parsed->GetObjectField(TEXT("projection"))->GetObjectField(TEXT("own"));
	TestEqual(TEXT("own XP exact"), Own->GetStringField(TEXT("xp")), FString(TEXT("41")));
	TestEqual(TEXT("own HP"), Own->GetIntegerField(TEXT("hp")), 81);
	TestEqual(TEXT("proxy incarnation"), Parsed->GetObjectField(TEXT("projection"))->GetArrayField(TEXT("proxies"))[0]->AsObject()->GetIntegerField(TEXT("incarnation")), 7);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotContractCoverageTest, "Nightfall.Bot.Diagnostics.ContractCoverage", Flags)
bool FBotContractCoverageTest::RunTest(const FString& Parameters)
{
	FBotDiagnostics Diagnostics;
	FClientMessage Respawn;
	Respawn.Seq = 1;
	Respawn.bRespawn = true;
	TArray<uint8> Bytes;
	NightfallProto::Encode(Respawn, Bytes);
	Diagnostics.ObserveFrame(true, Bytes, 10.0);
	// ServerMessage.event.attack_started {}, which has no consumer in FWorldEvent. Wire coverage
	// must still include it; projection callbacks are intentionally not the counting seam.
	Diagnostics.ObserveFrame(false, TArray<uint8>{ 0x12, 0x02, 0x5a, 0x00 }, 10.1);
	Diagnostics.ObserveFrame(false, TArray<uint8>{ 0x1a, 0x04, 0x08, 0x01, 0x10, 0x06 }, 10.2);
	Diagnostics.ObserveClose(4409);
	const auto Coverage = Diagnostics.Coverage()->GetObjectField(TEXT("counts"));
	TestEqual(TEXT("Respawn sent once"), Coverage->GetObjectField(TEXT("intents"))->GetStringField(TEXT("respawn")), FString(TEXT("1")));
	TestEqual(TEXT("unseen StopMove is in descriptor catalogue"), Coverage->GetObjectField(TEXT("intents"))->GetStringField(TEXT("stop_move")), FString(TEXT("0")));
	TestEqual(TEXT("unprojected AttackStarted counted"), Coverage->GetObjectField(TEXT("events"))->GetStringField(TEXT("attack_started")), FString(TEXT("1")));
	TestEqual(TEXT("INVALID rejection counted before keepalive filtering"), Coverage->GetObjectField(TEXT("reasons"))->GetStringField(TEXT("REJECT_REASON_INVALID")), FString(TEXT("1")));
	TestEqual(TEXT("replacement close counted"), Coverage->GetObjectField(TEXT("close_codes"))->GetStringField(TEXT("4409")), FString(TEXT("1")));
	Diagnostics.ResetIteration();
	TestEqual(TEXT("process coverage survives loop reset"), Diagnostics.Coverage()->GetObjectField(TEXT("counts"))->GetObjectField(TEXT("intents"))->GetStringField(TEXT("respawn")), FString(TEXT("1")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotFailureValueTest, "Nightfall.Bot.Diagnostics.LastEvaluation", Flags)
bool FBotFailureValueTest::RunTest(const FString& Parameters)
{
	FBotScenario Scenario;
	Scenario.Name = TEXT("failure");
	FBotStep Step;
	Step.Line = 7;
	Step.Source = TEXT("nf.WaitFor own_hp == 0 1");
	Step.PredicateText = TEXT("own_hp == 0");
	Step.Kind = EBotStepKind::WaitFor;
	Step.Seconds = 1.0;
	Step.Predicate = [](const FBotContext&) { return FBotPredicateValue{ false, TEXT("81") }; };
	Scenario.Steps.Add(Step);
	int32 Evaluations = 0;
	FBotScenarioExecutor Executor(MoveTemp(Scenario), [](const FString&, FString&) { return true; },
		[&](const FBotPredicateFn& Fn) { ++Evaluations; return Fn(FBotContext()); });
	Executor.Start(100.0);
	Executor.Tick(100.0);
	Executor.Tick(101.1);
	TestTrue(TEXT("timeout retains first failure context"), Executor.GetFailedStep().IsSet());
	TestEqual(TEXT("two live evaluations only"), Evaluations, 2);
	TestTrue(TEXT("predicate timeout classification is explicit"), BotJUnit::Write(TEXT("test"), Executor.GetTestCases(), 1.1, {}).Contains(TEXT("type=\"bot_expectation\"")));
	if (Executor.GetFailedStep().IsSet())
	{
		TestEqual(TEXT("last value preserved"), Executor.GetFailedStep()->Observed, FString(TEXT("81")));
		TestTrue(TEXT("timeout wait preserved"), FMath::IsNearlyEqual(Executor.GetFailedStep()->WaitSeconds, 1.1));
	}
	return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotUnevaluatedFailureTest, "Nightfall.Bot.Diagnostics.UnevaluatedFailure", Flags)
bool FBotUnevaluatedFailureTest::RunTest(const FString& Parameters)
{
	FBotScenario Scenario;
	Scenario.Name = TEXT("unevaluated");
	FBotStep First;
	First.Line = 1;
	First.Kind = EBotStepKind::WaitFor;
	First.Source = TEXT("nf.WaitFor connected 1");
	First.PredicateText = TEXT("connected");
	First.Predicate = [](const FBotContext&) { return FBotPredicateValue{ true, TEXT("connected") }; };
	Scenario.Steps.Add(First);
	FBotStep Second = First;
	Second.Line = 2;
	Second.Source = TEXT("nf.WaitFor own_hp == 0 1");
	Second.PredicateText = TEXT("own_hp == 0");
	Second.Predicate = [](const FBotContext&) { return FBotPredicateValue{ false, TEXT("81") }; };
	Scenario.Steps.Add(Second);
	int32 Evaluations = 0;
	FBotScenarioExecutor Executor(MoveTemp(Scenario), [](const FString&, FString&) { return true; },
		[&](const FBotPredicateFn& Fn) { ++Evaluations; return Fn(FBotContext()); });
	Executor.Start(100.0);
	Executor.Tick(100.0); // connected passes; next predicate has not evaluated
	Executor.Abort(100.1, TEXT("engine shutdown"));
	TestEqual(TEXT("shutdown does not evaluate next predicate"), Evaluations, 1);
	TestTrue(TEXT("shutdown retains failure context"), Executor.GetFailedStep().IsSet());
	if (Executor.GetFailedStep().IsSet())
	{
		TestEqual(TEXT("current assertion cannot inherit preceding value"), Executor.GetFailedStep()->Observed, FString(TEXT("not evaluated")));
		TestEqual(TEXT("current assertion context"), Executor.GetFailedStep()->Predicate, FString(TEXT("own_hp == 0")));
	}
	const FString Xml = BotJUnit::Write(TEXT("test"), Executor.GetTestCases(), 0.1, {});
	TestTrue(TEXT("shutdown is infrastructure, not a quarantinable assertion"), Xml.Contains(TEXT("type=\"failure\"")));
	return true;
}
#endif
