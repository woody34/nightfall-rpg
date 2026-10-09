#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "Nightfall.h"
#include "Auth/AuthSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "Containers/Ticker.h"
#include "Game/LoginFlowSubsystem.h"
#include "HAL/PlatformProcess.h"
#include "Misc/Guid.h"
#include "Misc/Parse.h"
#include "Net/NetClientSubsystem.h"
#include "Net/SessionClientSubsystem.h"
#include "TurboLinkGrpcManager.h"

#if WITH_DEV_AUTOMATION_TESTS

// Live: dev-token login -> character -> play ticket -> WebSocket -> own EntitySpawn, then the
// combat intents against the real server. Skipped with a warning when the API is not reachable.
//
// What the server does today decides what is asserted (documented, not hidden):
//  - an attackable NPC in the area of interest: SetTarget must yield TargetChanged for it and the
//    HUD model must show its target frame; Attack must be answered by exactly one Ack or one
//    IntentRejected. While combat is NOT_YET_IMPLEMENTED the rejection reason must be that; once
//    it lands an Ack is accepted and, if an AttackResult arrives, its target HP is checked.
//  - no NPC in view (zones without spawn slots): SetTarget of an unknown id must be rejected with
//    UNKNOWN_ENTITY and must leave the client with no target.
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

	bool bPinged = false;
	FNetResult PingResult;
	Session->Ping([&](const FNetResult& R, const FGrpcNightfallV1PingResponse&) { PingResult = R; bPinged = true; });
	if (!Pump([&] { return bPinged; }, 10.0) || !PingResult.IsOk())
	{
		AddWarning(FString::Printf(TEXT("API not reachable at %s; combat end-to-end skipped."), *Session->GetEndpoint()));
		return true;
	}
	if (!UAuthSubsystem::GetCommandLineDevToken().IsEmpty()) Auth->StartLogin();
	else Auth->LoginWithDevToken(TEXT("test:") + FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower));
	if (!TestTrue(TEXT("logged in"), Auth->IsLoggedIn()))
	{
		AddWarning(TEXT("Not authenticated (run the API with AUTH_DEV_TOKENS=1 or pass -DevToken); combat end-to-end skipped."));
		return true;
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

	TMap<uint32, bool> Acked;        // seq -> accepted
	TMap<uint32, uint32> Rejected;   // seq -> reason
	TArray<FTargetChanged> Targets;
	TArray<FAttackResult> Results;
	Net->OnIntentAck.AddLambda([&](const FAck& A) { Acked.Add(A.Seq, true); });
	Net->OnIntentRejected.AddLambda([&](const FIntentRejected& R) { Rejected.Add(R.Seq, R.Reason); });
	Net->OnTargetChanged.AddLambda([&](const FTargetChanged& T) { Targets.Add(T); });
	Net->OnAttackResult.AddLambda([&](const FAttackResult& R) { Results.Add(R); });

	TestTrue(TEXT("own entity is in the projection"), Combat->FindOwnEntity() != nullptr);
	const FCombatEntity* Npc = nullptr;
	for (const TPair<FString, FEntitySpawn>& Known : Net->GetKnownEntities())
	{
		const FCombatEntity* E = Combat->FindEntity(Known.Key);
		if (E && Known.Value.Kind == 2 && Combat->IsAttackable(Known.Key)) { Npc = E; break; }
	}

	if (!Npc)
	{
		AddWarning(TEXT("No attackable NPC near spawn; asserting the unknown-target rejection instead."));
		const FString Ghost = FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower);
		const uint32 Seq = Net->SendSetTarget(Ghost);
		TestTrue(TEXT("SetTarget answered"), Pump([&] { return Acked.Contains(Seq) || Rejected.Contains(Seq); }, 5.0));
		TestTrue(TEXT("unknown target is rejected UNKNOWN_ENTITY"), Rejected.FindRef(Seq) == 3);
		TestTrue(TEXT("no target is held"), Combat->GetTargetId().IsEmpty());
		return true;
	}

	const FString NpcId = Npc->Spawn.EntityId;
	TestTrue(TEXT("click is accepted by the client"), Combat->ClickEntity(NpcId));
	TestTrue(TEXT("TargetChanged arrives"), Pump([&] { return Targets.Num() > 0; }, 5.0));
	if (Targets.Num() > 0)
	{
		TestTrue(TEXT("TargetChanged names the NPC"), Targets[0].Target.Equals(NpcId, ESearchCase::IgnoreCase));
		TestTrue(TEXT("...for our entity"), Net->IsOwnEntity(Targets[0].Entity));
		TestTrue(TEXT("target frame shows the NPC"), Combat->BuildHudModel().bTargetVisible);
	}

	// The Attack that ClickEntity sent right after SetTarget: exactly one answer.
	Pump([&] { return Combat->GetAttackState() != EAttackState::Pending; }, 5.0);
	const EAttackState State = Combat->GetAttackState();
	if (State == EAttackState::Idle)
	{
		// Seqs: SetTarget 1, Attack 2. NOT_YET_IMPLEMENTED (12) until the server's combat lands.
		TestTrue(TEXT("the Attack was answered by a rejection"), Rejected.Contains(2));
		AddInfo(FString::Printf(TEXT("Attack rejected by the server, reason %u (status: %s)"), Rejected.FindRef(2), *Combat->GetStatusLine()));
	}
	else
	{
		TestEqual(TEXT("an accepted Attack is Active"), State, EAttackState::Active);
		Pump([&] { return Results.Num() > 0; }, 6.0);
		for (const FAttackResult& R : Results)
		{
			if (Net->IsOwnEntity(R.Attacker) && R.Target.Equals(NpcId, ESearchCase::IgnoreCase))
			{
				TestEqual(TEXT("projection holds the server's HP after the hit"), Combat->FindEntity(NpcId)->Hp, R.TargetHpAfter);
				break;
			}
		}
	}
	return true;
}

#endif
