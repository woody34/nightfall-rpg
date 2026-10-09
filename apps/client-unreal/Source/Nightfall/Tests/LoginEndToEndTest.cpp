#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "Nightfall.h"
#include "Auth/AuthSubsystem.h"
#include "Game/LoginFlowSubsystem.h"
#include "Net/SessionClientSubsystem.h"
#include "HAL/PlatformProcess.h"
#include "Misc/Guid.h"
#include "Misc/Parse.h"
#include "TurboLinkGrpcManager.h"

#if WITH_DEV_AUTOMATION_TESTS

// Live: login (developer-token bypass) -> ListMyCharacters -> CreateCharacter -> IssuePlayTicket
// against the running API. Skipped with a warning when the API does not answer Ping.
//
// Token: `-DevToken=<token>` or `-DevTokenFile=<path>` on the editor command line (a real Keycloak
// access token, see Scripts/run-tests.sh), otherwise a fresh `test:<uuid>` account, which the API accepts only
// when started with AUTH_DEV_TOKENS=1.

namespace
{
	/** Pumps TurboLink until bDone or the timeout; editor tests have no game world ticking it. */
	bool PumpUntil(USessionClient* Session, const bool& bDone, double TimeoutSeconds = 10.0)
	{
		const double GiveUpAt = FPlatformTime::Seconds() + TimeoutSeconds;
		while (!bDone && FPlatformTime::Seconds() < GiveUpAt)
		{
			Session->GetGrpcManager()->Tick(0.01f);
			FPlatformProcess::Sleep(0.01f);
		}
		return bDone;
	}

	/** 3-16 ASCII letters, as the API requires; random so reruns do not collide. */
	FString RandomCharacterName()
	{
		const FString Hex = FGuid::NewGuid().ToString(EGuidFormats::Digits);
		FString Name = TEXT("Test");
		for (int32 I = 0; I < 10; ++I)
		{
			const TCHAR C = Hex[Hex.Len() - 1 - I];
			Name.AppendChar(TEXT('a') + static_cast<TCHAR>(FParse::HexDigit(C)));
		}
		return Name;
	}
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FLoginEndToEndTest, "Nightfall.Login.EndToEnd",
	EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)

bool FLoginEndToEndTest::RunTest(const FString& Parameters)
{
	NightfallTest::AllowApiUnavailableLogs(*this);
	FScopedTestGameInstance Instance;
	USessionClient* Session = Instance.Get<USessionClient>();
	UAuthSubsystem* Auth = Instance.Get<UAuthSubsystem>();
	ULoginFlowSubsystem* Flow = Instance.Get<ULoginFlowSubsystem>();
	if (!TestNotNull(TEXT("subsystems"), Session) || !TestNotNull(TEXT("auth"), Auth) || !TestNotNull(TEXT("flow"), Flow))
	{
		return false;
	}
	Flow->bTravelOnConnect = false;

	// Gate: is the API up?
	bool bPinged = false;
	FNetResult PingResult;
	Session->Ping([&](const FNetResult& R, const FGrpcNightfallV1PingResponse&) { PingResult = R; bPinged = true; });
	if (!PumpUntil(Session, bPinged) || !PingResult.IsOk())
	{
		return NightfallTest::SkipLive(*this, FString::Printf(TEXT("API not reachable at %s (%s); end-to-end login skipped. Start it with `moon run api:dev`."),
			*Session->GetEndpoint(), *PingResult.Message));
	}

	// Developer bypass: a token from the command line goes through StartLogin like a player would.
	if (!UAuthSubsystem::GetCommandLineDevToken().IsEmpty())
	{
		Auth->StartLogin();
	}
	else
	{
		Auth->LoginWithDevToken(TEXT("test:") + FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower));
	}
	if (!TestTrue(TEXT("logged in via developer token"), Auth->IsLoggedIn()))
	{
		return false;
	}

	bool bListed = false;
	FNetResult ListResult;
	TArray<FGrpcNightfallV1Character> Before;
	Flow->ListCharacters([&](const FNetResult& R, const TArray<FGrpcNightfallV1Character>& C) { ListResult = R; Before = C; bListed = true; });
	if (!TestTrue(TEXT("ListMyCharacters completed"), PumpUntil(Session, bListed))
		|| !TestEqual(FString::Printf(TEXT("ListMyCharacters status (%s). Unauthenticated: run the API with AUTH_DEV_TOKENS=1 or pass -DevToken=<keycloak access token>"),
			*ListResult.Message), ListResult.Error, ENetError::None))
	{
		return false;
	}

	const FString Name = RandomCharacterName();
	bool bCreated = false;
	FNetResult CreateResult;
	FGrpcNightfallV1Character Created;
	Flow->CreateCharacter(Name, EGrpcNightfallV1Race::RACE_ELF, [&](const FNetResult& R, const FGrpcNightfallV1Character& C) { CreateResult = R; Created = C; bCreated = true; });
	if (!TestTrue(TEXT("CreateCharacter completed"), PumpUntil(Session, bCreated))
		|| !TestEqual(FString::Printf(TEXT("CreateCharacter status (%s)"), *CreateResult.Message), CreateResult.Error, ENetError::None))
	{
		return false;
	}
	TestEqual(TEXT("created name"), Created.Name, Name);
	TestEqual(TEXT("created race"), Created.Race, EGrpcNightfallV1Race::RACE_ELF);
	TestFalse(TEXT("created id"), Created.Id.IsEmpty());

	bListed = false;
	TArray<FGrpcNightfallV1Character> After;
	Flow->ListCharacters([&](const FNetResult& R, const TArray<FGrpcNightfallV1Character>& C) { ListResult = R; After = C; bListed = true; });
	PumpUntil(Session, bListed);
	TestEqual(TEXT("one more character"), After.Num(), Before.Num() + 1);
	TestTrue(TEXT("new character listed"), After.ContainsByPredicate([&](const FGrpcNightfallV1Character& C) { return C.Id == Created.Id; }));

	// Two tickets, two fresh idempotency keys: two distinct single-use tickets.
	TArray<FPlayTicket> Tickets;
	for (int32 Attempt = 0; Attempt < 2; ++Attempt)
	{
		bool bIssued = false;
		FNetResult TicketResult;
		FPlayTicket Ticket;
		Flow->RequestTicket(Created.Id, [&](const FNetResult& R, const FPlayTicket& T) { TicketResult = R; Ticket = T; bIssued = true; });
		if (!TestTrue(TEXT("IssuePlayTicket completed"), PumpUntil(Session, bIssued))
			|| !TestEqual(FString::Printf(TEXT("IssuePlayTicket status (%s)"), *TicketResult.Message), TicketResult.Error, ENetError::None))
		{
			return false;
		}
		TestTrue(TEXT("ticket issued"), Ticket.Ticket.Len() >= 32);
		TestTrue(TEXT("ws_url is a WebSocket URL"), Ticket.WsUrl.StartsWith(TEXT("ws://")) || Ticket.WsUrl.StartsWith(TEXT("wss://")));
		TestFalse(TEXT("ws_url carries no ticket"), Ticket.WsUrl.Contains(Ticket.Ticket));
		Tickets.Add(Ticket);
	}
	TestNotEqual(TEXT("each attempt gets a new ticket"), Tickets[0].Ticket, Tickets[1].Ticket);
	UE_LOG(LogNightfall, Display, TEXT("EndToEnd: created %s (%s), %d characters, ticket for %s"),
		*Created.Name, *Created.Id, After.Num(), *Tickets[0].WsUrl);
	return true;
}

#endif
