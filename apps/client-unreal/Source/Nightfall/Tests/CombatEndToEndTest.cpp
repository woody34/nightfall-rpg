#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "Nightfall.h"
#include "Auth/AuthSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "Containers/Ticker.h"
#include "Game/LoginFlowSubsystem.h"
#include "HAL/PlatformProcess.h"
#include "Misc/CommandLine.h"
#include "Misc/Guid.h"
#include "Misc/Parse.h"
#include "Net/NetClientSubsystem.h"
#include "Net/SessionClientSubsystem.h"
#include "TurboLinkGrpcManager.h"

#if WITH_DEV_AUTOMATION_TESTS

// Live: dev-token login -> character -> play ticket -> WebSocket -> own EntitySpawn, then the
// combat loop against the real server and its fixture zone (keltir_a / keltir_b spawn slots).
// Needs the API running with AUTH_DEV_TOKENS=1 (`moon run api:dev` with the compose services up).
// When the server is not reachable the test is SKIPPED with a warning that says so; pass
// -RequireCombatServer on the command line (CI against a live stack) to make that a failure.
//
// Asserted, in order:
//  - a keltir is in the area of interest (the fixture exists);
//  - the click sends SetTarget then Attack; repeated clicks while Pending send nothing;
//  - TargetChanged names the keltir; the Attack is Acked, not rejected;
//  - a matching AttackResult (own entity -> keltir) arrives within the timeout, the projection
//    holds its HP, and replaying it shows no second damage number;
//  - repeated clicks while Active send nothing;
//  - a ground click sends StopAttack, which the server Acks.
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FCombatEndToEndTest, "Nightfall.Combat.EndToEnd",
	EAutomationTestFlags::EditorContext | EAutomationTestFlags::EngineFilter)

bool FCombatEndToEndTest::RunTest(const FString& Parameters)
{
	FScopedTestGameInstance Instance;
	USessionClient* Session = Instance.Get<USessionClient>();
	UAuthSubsystem* Auth = Instance.Get<UAuthSubsystem>();
	ULoginFlowSubsystem* Flow = Instance.Get<ULoginFlowSubsystem>();
	UNetClientSubsystem* Net = Instance.Get<UNetClientSubsystem>();
	UCombatStateSubsystem* Combat = Instance.Get<UCombatStateSubsystem>();
	Flow->bTravelOnConnect = false;

	auto Pump = [&](TFunctionRef<bool()> Done, double TimeoutSeconds)
	{
		const double GiveUpAt = FPlatformTime::Seconds() + TimeoutSeconds;
		while (!Done() && FPlatformTime::Seconds() < GiveUpAt)
		{
			Session->GetGrpcManager()->Tick(0.01f);
			FTSTicker::GetCoreTicker().Tick(0.01f);
			FPlatformProcess::Sleep(0.01f);
		}
		return Done();
	};

	const bool bRequireServer = FParse::Param(FCommandLine::Get(), TEXT("RequireCombatServer"));
	auto Skip = [&](const FString& Why)
	{
		const FString Text = FString::Printf(TEXT("SKIPPED (no combat coverage): %s"), *Why);
		if (bRequireServer) AddError(Text + TEXT(" [-RequireCombatServer]")); else AddWarning(Text);
		return !bRequireServer;
	};

	bool bPinged = false;
	FNetResult PingResult;
	Session->Ping([&](const FNetResult& R, const FGrpcNightfallV1PingResponse&) { PingResult = R; bPinged = true; });
	if (!Pump([&] { return bPinged; }, 10.0) || !PingResult.IsOk())
	{
		return Skip(FString::Printf(TEXT("API not reachable at %s; start it with `moon run api:dev`"), *Session->GetEndpoint()));
	}
	if (!UAuthSubsystem::GetCommandLineDevToken().IsEmpty()) Auth->StartLogin();
	else Auth->LoginWithDevToken(TEXT("test:") + FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower));
	if (!Auth->IsLoggedIn())
	{
		return Skip(TEXT("not authenticated (run the API with AUTH_DEV_TOKENS=1 or pass -DevToken)"));
	}

	const FString Hex = FGuid::NewGuid().ToString(EGuidFormats::Digits);
	FString Name = TEXT("Fight");
	for (int32 I = 0; I < 10; ++I) Name.AppendChar(TEXT('a') + static_cast<TCHAR>(FParse::HexDigit(Hex[Hex.Len() - 1 - I])));
	bool bCreated = false;
	FNetResult CreateResult;
	FGrpcNightfallV1Character Character;
	Flow->CreateCharacter(Name, EGrpcNightfallV1Race::RACE_HUMAN, [&](const FNetResult& R, const FGrpcNightfallV1Character& C) { CreateResult = R; Character = C; bCreated = true; });
	if (!TestTrue(TEXT("character created"), Pump([&] { return bCreated; }, 10.0)) || !TestTrue(*CreateResult.Message, CreateResult.IsOk())) return false;

	bool bTicketed = false;
	Flow->EnterWorld(Character.Id, [&](const FNetResult&) { bTicketed = true; });
	if (!TestTrue(TEXT("ticket issued"), Pump([&] { return bTicketed; }, 10.0))) return false;
	if (!TestTrue(TEXT("own EntitySpawn received"), Pump([&] { return Net->GetKnownEntities().Contains(Character.Id); }, 10.0))) return false;
	Pump([] { return false; }, 1.0);   // let the rest of the area of interest arrive

	TSet<uint32> Acked;
	TMap<uint32, uint32> Rejected;   // seq -> reason
	TArray<FTargetChanged> Targets;
	TArray<FAttackResult> Results;
	Net->OnIntentAck.AddLambda([&](const FAck& A) { Acked.Add(A.Seq); });
	Net->OnIntentRejected.AddLambda([&](const FIntentRejected& R) { Rejected.Add(R.Seq, R.Reason); });
	Net->OnTargetChanged.AddLambda([&](const FTargetChanged& T) { Targets.Add(T); });
	Net->OnAttackResult.AddLambda([&](const FAttackResult& R) { Results.Add(R); });
	TArray<FDamageNumber> Numbers;
	Combat->OnDamageNumber.AddLambda([&](const FDamageNumber& N) { Numbers.Add(N); });

	if (!TestTrue(TEXT("own entity is in the projection"), Combat->FindOwnEntity() != nullptr)) return false;
	// New characters start far from the fixture zone's keltir slots (test_zone.toml: slot a at
	// (100, 100)), outside the area of interest: walk there first, like a player would.
	FNetVec2 OwnPosition;
	Net->OnEntityMove.AddLambda([&](const FEntityMove& M) { if (Net->IsOwnEntity(M.EntityId)) OwnPosition = M.Position; });
	const FNetVec2 Waypoints[] = { { 40.f, 40.f }, { 80.f, 85.f }, { 97.f, 100.f } };   // one MoveTo covers at most 64 tiles
	auto Reached = [&](const FNetVec2& W) { return FVector2D::Distance(FVector2D(OwnPosition.X, OwnPosition.Y), FVector2D(W.X, W.Y)) < 1.0; };
	for (const FNetVec2& W : Waypoints)
	{
		Net->SendMoveTo(W);
		if (!TestTrue(*FString::Printf(TEXT("walked to (%.0f, %.0f)"), W.X, W.Y), Pump([&] { return Reached(W); }, 60.0))) return false;
	}

	FString NpcId;
	Pump([&]
	{
		for (const TPair<FString, FEntitySpawn>& Known : Net->GetKnownEntities())
		{
			if (Known.Value.Kind == 2 && Known.Value.TemplateId == TEXT("keltir") && Combat->IsAttackable(Known.Key)) { NpcId = Known.Key; return true; }
		}
		return false;
	}, 5.0);
	if (NpcId.IsEmpty())
	{
		for (const TPair<FString, FEntitySpawn>& Known : Net->GetKnownEntities())
		{
			AddInfo(FString::Printf(TEXT("in view: %s kind=%u template='%s' attackable=%d at (%.1f, %.1f)"), *Known.Key, Known.Value.Kind, *Known.Value.TemplateId, Known.Value.bAttackable, Known.Value.Position.X, Known.Value.Position.Y));
		}
		AddError(TEXT("the fixture keltir (packages/data/zones/test_zone.toml spawn slot) is not in view and attackable"));
		return false;
	}

	// Click: SetTarget then Attack, two seqs; repeated clicks while Pending send nothing.
	const uint32 Base = Net->GetLastSentSeq();
	TestTrue(TEXT("click is accepted by the client"), Combat->ClickEntity(NpcId));
	TestEqual(TEXT("click sent SetTarget + Attack"), Net->GetLastSentSeq(), Base + 2);
	const uint32 AttackSeq = Base + 2;
	TestEqual(TEXT("attack is pending"), Combat->GetAttackState(), EAttackState::Pending);
	Combat->ClickEntity(NpcId);
	Combat->ClickEntity(NpcId);
	TestEqual(TEXT("repeated clicks while pending send nothing"), Net->GetLastSentSeq(), AttackSeq);

	TestTrue(TEXT("TargetChanged arrives"), Pump([&] { return Targets.Num() > 0; }, 5.0));
	if (Targets.Num() > 0)
	{
		TestTrue(TEXT("TargetChanged names the keltir"), Targets[0].Target.Equals(NpcId, ESearchCase::IgnoreCase));
		TestTrue(TEXT("...for our entity"), Net->IsOwnEntity(Targets[0].Entity));
		TestTrue(TEXT("target frame shows the keltir"), Combat->BuildHudModel().bTargetVisible);
	}

	// The Attack must be accepted: a rejection of any reason fails the test.
	Pump([&] { return Acked.Contains(AttackSeq) || Rejected.Contains(AttackSeq); }, 5.0);
	if (!TestFalse(*FString::Printf(TEXT("the Attack was not rejected (reason %u, status: %s)"), Rejected.FindRef(AttackSeq), *Combat->GetStatusLine()), Rejected.Contains(AttackSeq))) return false;
	if (!TestTrue(TEXT("the Attack was Acked"), Acked.Contains(AttackSeq))) return false;
	TestEqual(TEXT("an accepted Attack is Active"), Combat->GetAttackState(), EAttackState::Active);
	Combat->ClickEntity(NpcId);
	Combat->ClickEntity(NpcId);
	TestEqual(TEXT("repeated clicks while active send nothing"), Net->GetLastSentSeq(), AttackSeq);

	// A matching AttackResult must arrive within the timeout.
	auto IsOurSwing = [&](const FAttackResult& R) { return Net->IsOwnEntity(R.Attacker) && R.Target.Equals(NpcId, ESearchCase::IgnoreCase); };
	auto HasOurSwing = [&] { return Results.ContainsByPredicate(IsOurSwing); };
	if (!TestTrue(TEXT("an AttackResult from us on the keltir arrives within 10s"), Pump(HasOurSwing, 10.0))) return false;
	const FAttackResult Swing = *Results.FindByPredicate(IsOurSwing);
	TestTrue(TEXT("the swing has a real outcome"), Swing.Outcome != ENetAttackOutcome::Unspecified);
	if (const FCombatEntity* Keltir = Combat->FindEntity(NpcId))
	{
		TestTrue(TEXT("projection holds the server's HP (or a later swing's)"), Keltir->Hp <= Swing.TargetHpAfter);
	}

	// Damage-number dedupe: one number per distinct (tick, attacker, target); a replay adds none.
	TSet<FString> Distinct;
	for (const FAttackResult& R : Results) Distinct.Add(FString::Printf(TEXT("%llu|%s|%s"), R.Tick, *R.Attacker.ToLower(), *R.Target.ToLower()));
	TestEqual(TEXT("one damage number per distinct swing"), Numbers.Num(), Distinct.Num());
	const int32 NumbersBefore = Numbers.Num();
	Combat->ApplyAttackResult(Swing);
	Combat->ApplyAttackResult(Swing);
	TestEqual(TEXT("replayed AttackResult shows no second number"), Numbers.Num(), NumbersBefore);

	// Ground click: StopAttack is sent (and acknowledged), and our swings stop.
	if (!TestEqual(TEXT("still attacking before the ground click (the keltir survived)"), Combat->GetAttackState(), EAttackState::Active)) return false;
	const uint32 StopSeq = Combat->NoteGroundClick();
	TestTrue(TEXT("ground click sent StopAttack"), StopSeq != 0 && StopSeq == Net->GetLastSentSeq());
	TestEqual(TEXT("attack ended locally"), Combat->GetAttackState(), EAttackState::Idle);
	Pump([&] { return Acked.Contains(StopSeq) || Rejected.Contains(StopSeq); }, 5.0);
	TestTrue(TEXT("the server Acked StopAttack"), Acked.Contains(StopSeq));
	TestEqual(TEXT("a second ground click sends nothing"), Combat->NoteGroundClick(), 0u);
	Pump([] { return false; }, 1.5);   // anything already in flight lands
	const int32 Settled = Results.FilterByPredicate(IsOurSwing).Num();
	Pump([] { return false; }, 3.0);
	TestEqual(TEXT("no further swings after StopAttack"), Results.FilterByPredicate(IsOurSwing).Num(), Settled);
	return true;
}

#endif
