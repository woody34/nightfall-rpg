#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "IWebSocket.h"
#include "Net/NetClientSubsystem.h"

#if WITH_DEV_AUTOMATION_TESTS

// The play ticket travels in the upgrade request's Authorization header, never in the URL
// (plan §8 #7), and every reconnect presents a ticket nobody has used yet (§8 #9).

namespace
{
	constexpr EAutomationTestFlags NetTestFlags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;

	/** Records Connect() and Send() and lets the test fire the socket's events. */
	class FFakeWebSocket final : public IWebSocket
	{
	public:
		int32 ConnectCalls = 0;
		TArray<TArray<uint8>> Sent;

		virtual void Connect() override { ++ConnectCalls; }
		virtual void Close(int32 Code, const FString& Reason) override {}
		virtual bool IsConnected() override { return false; }
		virtual void Send(const FString& Data) override {}
		virtual void Send(const void* Data, SIZE_T Size, bool bIsBinary) override { Sent.Emplace(static_cast<const uint8*>(Data), static_cast<int32>(Size)); }
		virtual void SetTextMessageMemoryLimit(uint64 TextMessageMemoryLimit) override {}
		virtual FWebSocketConnectedEvent& OnConnected() override { return Connected; }
		virtual FWebSocketConnectionErrorEvent& OnConnectionError() override { return ConnectionError; }
		virtual FWebSocketClosedEvent& OnClosed() override { return Closed; }
		virtual FWebSocketMessageEvent& OnMessage() override { return Message; }
		virtual FWebSocketBinaryMessageEvent& OnBinaryMessage() override { return BinaryMessage; }
		virtual FWebSocketRawMessageEvent& OnRawMessage() override { return RawMessage; }
		virtual FWebSocketMessageSentEvent& OnMessageSent() override { return MessageSent; }

		FWebSocketConnectedEvent Connected;
		FWebSocketConnectionErrorEvent ConnectionError;
		FWebSocketClosedEvent Closed;
		FWebSocketMessageEvent Message;
		FWebSocketBinaryMessageEvent BinaryMessage;
		FWebSocketRawMessageEvent RawMessage;
		FWebSocketMessageSentEvent MessageSent;
	};

	struct FSocketRecorder
	{
		TArray<FWsUpgradeRequest> Requests;
		TArray<TSharedRef<FFakeWebSocket>> Sockets;
		TArray<TPair<float, TFunction<void()>>> Timers;

		void Install(UNetClientSubsystem* Net)
		{
			Net->SetSocketFactoryForTesting([this](const FWsUpgradeRequest& Request) -> TSharedRef<IWebSocket>
			{
				Requests.Add(Request);
				return Sockets.Add_GetRef(MakeShared<FFakeWebSocket>());
			});
			Net->SetSchedulerForTesting([this](float Delay, TFunction<void()> Fn) { Timers.Add({ Delay, MoveTemp(Fn) }); });
		}

		float RunNextTimer()
		{
			if (Timers.IsEmpty()) return -1.f;
			TPair<float, TFunction<void()>> Timer = MoveTemp(Timers[0]);
			Timers.RemoveAt(0);
			Timer.Value();
			return Timer.Key;
		}
	};

	const TCHAR* const WsUrl = TEXT("ws://localhost:3000/ws");

	/** The bytes of a ClientMessage carrying only `seq`: what a keep-alive must be. */
	TArray<uint8> KeepAliveBytes(uint32 Seq)
	{
		FClientMessage Msg;
		Msg.Seq = Seq;
		TArray<uint8> Bytes;
		NightfallProto::Encode(Msg, Bytes);
		return Bytes;
	}
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FNetClientUpgradeHeaderTest, "Nightfall.Net.NetClient.TicketInUpgradeHeader", NetTestFlags)

bool FNetClientUpgradeHeaderTest::RunTest(const FString& Parameters)
{
	const FString Sentinel = TEXT("SENTINEL_TICKET_b64url_0123456789");
	const FWsUpgradeRequest Pure = UNetClientSubsystem::MakeUpgradeRequest(WsUrl, Sentinel);
	TestEqual(TEXT("URL unchanged"), Pure.Url, FString(WsUrl));
	TestEqual(TEXT("Authorization header"), Pure.Headers.FindRef(TEXT("Authorization")), TEXT("Bearer ") + Sentinel);

	FScopedTestGameInstance Instance;
	UNetClientSubsystem* Net = Instance.Get<UNetClientSubsystem>();
	FSocketRecorder Recorder;
	Recorder.Install(Net);

	Net->Connect(WsUrl, Sentinel);
	if (!TestEqual(TEXT("one upgrade request"), Recorder.Requests.Num(), 1))
	{
		return false;
	}
	const FWsUpgradeRequest& Request = Recorder.Requests[0];
	TestEqual(TEXT("connects to ws_url as given"), Request.Url, FString(WsUrl));
	TestFalse(TEXT("ticket is not in the URL"), Request.Url.Contains(Sentinel) || Request.Url.Contains(TEXT("ticket")));
	TestEqual(TEXT("ticket is the bearer"), Request.Headers.FindRef(TEXT("Authorization")), TEXT("Bearer ") + Sentinel);
	TestEqual(TEXT("only the Authorization header"), Request.Headers.Num(), 1);
	TestEqual(TEXT("socket opened once"), Recorder.Sockets[0]->ConnectCalls, 1);

	// Without a ticket provider a failed attempt is final: the consumed ticket is never retried.
	Recorder.Sockets[0]->ConnectionError.Broadcast(TEXT("HTTP 401"));
	TestEqual(TEXT("no provider: nothing scheduled"), Recorder.Timers.Num(), 0);
	TestEqual(TEXT("no provider: no second request"), Recorder.Requests.Num(), 1);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FNetClientReconnectTicketTest, "Nightfall.Net.NetClient.ReconnectUsesFreshTicket", NetTestFlags)

bool FNetClientReconnectTicketTest::RunTest(const FString& Parameters)
{
	FScopedTestGameInstance Instance;
	UNetClientSubsystem* Net = Instance.Get<UNetClientSubsystem>();
	FSocketRecorder Recorder;
	Recorder.Install(Net);

	int32 ProviderCalls = 0;
	bool bProviderFails = false;
	Net->SetTicketProvider([&](UNetClientSubsystem::FTicketCallback OnTicket)
	{
		++ProviderCalls;
		if (bProviderFails)
		{
			OnTicket(false, FPlayTicket());
			return;
		}
		FPlayTicket Ticket;
		Ticket.WsUrl = WsUrl;
		Ticket.Ticket = FString::Printf(TEXT("fresh-%d"), ProviderCalls);
		OnTicket(true, Ticket);
	});

	Net->Connect(WsUrl, TEXT("first"));
	TestEqual(TEXT("connect does not ask the provider"), ProviderCalls, 0);

	Recorder.Sockets.Last()->ConnectionError.Broadcast(TEXT("refused"));
	Recorder.Sockets.Last()->Closed.Broadcast(1006, TEXT("refused"), false);   // both may fire; one reconnect
	TestEqual(TEXT("one reconnect scheduled"), Recorder.Timers.Num(), 1);
	TestEqual(TEXT("first backoff 0.5 s"), Recorder.RunNextTimer(), 0.5f);
	TestEqual(TEXT("provider asked once"), ProviderCalls, 1);
	TestEqual(TEXT("second attempt made"), Recorder.Requests.Num(), 2);

	Recorder.Sockets.Last()->Closed.Broadcast(1006, TEXT("dropped"), false);
	TestEqual(TEXT("second backoff 1 s"), Recorder.RunNextTimer(), 1.f);

	bProviderFails = true;   // e.g. gRPC unavailable: retry after the next backoff, no socket
	Recorder.Sockets.Last()->Closed.Broadcast(1006, TEXT("dropped"), false);
	TestEqual(TEXT("third backoff 2 s"), Recorder.RunNextTimer(), 2.f);
	TestEqual(TEXT("no socket without a ticket"), Recorder.Requests.Num(), 3);
	bProviderFails = false;
	TestEqual(TEXT("fourth backoff 4 s"), Recorder.RunNextTimer(), 4.f);
	TestEqual(TEXT("fourth attempt made"), Recorder.Requests.Num(), 4);

	// A successful connection resets the backoff.
	Recorder.Sockets.Last()->Connected.Broadcast();
	TestTrue(TEXT("connected"), Net->IsConnected());
	Recorder.Sockets.Last()->Closed.Broadcast(1001, TEXT("going away"), true);
	TestEqual(TEXT("the connection's keep-alive check (cancelled)"), Recorder.RunNextTimer(), UNetClientSubsystem::KeepAliveSeconds);
	TestEqual(TEXT("backoff reset"), Recorder.RunNextTimer(), 0.5f);

	TSet<FString> Seen;
	for (const FWsUpgradeRequest& Request : Recorder.Requests)
	{
		const FString Auth = Request.Headers.FindRef(TEXT("Authorization"));
		TestFalse(FString::Printf(TEXT("ticket %s presented once"), *Auth), Seen.Contains(Auth));
		Seen.Add(Auth);
		TestFalse(TEXT("ticket never in URL"), Request.Url.Contains(TEXT("fresh")) || Request.Url.Contains(TEXT("first")));
	}

	// Disconnect cancels a pending reconnect.
	Recorder.Sockets.Last()->Closed.Broadcast(1006, TEXT("dropped"), false);
	const int32 Before = Recorder.Requests.Num();
	Net->Disconnect();
	Recorder.RunNextTimer();
	TestEqual(TEXT("no reconnect after Disconnect"), Recorder.Requests.Num(), Before);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FNetClientKeepAliveTest, "Nightfall.Net.NetClient.KeepAliveWhenIdle", NetTestFlags)

bool FNetClientKeepAliveTest::RunTest(const FString& Parameters)
{
	// The server closes a session with no inbound frame for 60 s (4408). An idle client sends a
	// no-intent ClientMessage after 20-40 s of silence; any intent sent in between postpones it.
	FScopedTestGameInstance Instance;
	UNetClientSubsystem* Net = Instance.Get<UNetClientSubsystem>();
	FSocketRecorder Recorder;
	Recorder.Install(Net);
	Net->SetTicketProvider([](UNetClientSubsystem::FTicketCallback OnTicket)
	{
		FPlayTicket Ticket;
		Ticket.WsUrl = WsUrl;
		Ticket.Ticket = TEXT("fresh");
		OnTicket(true, Ticket);
	});

	Net->Connect(WsUrl, TEXT("first"));
	TestEqual(TEXT("nothing armed before the socket is open"), Recorder.Timers.Num(), 0);
	TSharedRef<FFakeWebSocket> Socket = Recorder.Sockets.Last();
	Socket->Connected.Broadcast();
	if (!TestEqual(TEXT("one keep-alive timer armed on connect"), Recorder.Timers.Num(), 1)) return false;
	TestEqual(TEXT("checks every 20 s"), Recorder.Timers[0].Key, UNetClientSubsystem::KeepAliveSeconds);
	TestTrue(TEXT("well inside the server's 60 s"), 2.f * UNetClientSubsystem::KeepAliveSeconds < 60.f);

	// Idle: the first check sends one keep-alive and re-arms.
	Recorder.RunNextTimer();
	if (!TestEqual(TEXT("idle: one keep-alive sent"), Socket->Sent.Num(), 1)) return false;
	const uint32 KeepAliveSeq = Net->GetLastSentSeq();
	TestTrue(TEXT("keep-alive takes a fresh seq"), KeepAliveSeq > 0);
	TestTrue(TEXT("keep-alive is a ClientMessage with no intent"), Socket->Sent[0] == KeepAliveBytes(KeepAliveSeq));
	TestEqual(TEXT("re-armed"), Recorder.Timers.Num(), 1);

	// The server's IntentRejected{INVALID} for it reaches no gameplay listener.
	int32 Rejections = 0;
	Net->OnIntentRejected.AddLambda([&](const FIntentRejected&) { ++Rejections; });
	FServerMessage Answer;
	Answer.Rejected = FIntentRejected{ KeepAliveSeq, 6, TEXT("intent is required") };
	Net->DispatchServerMessage(Answer);
	TestEqual(TEXT("keep-alive rejection swallowed"), Rejections, 0);
	Net->DispatchServerMessage(Answer);
	TestEqual(TEXT("a second rejection with that seq is not a keep-alive's"), Rejections, 1);

	// Outbound traffic since the last check postpones the keep-alive by one interval.
	const uint32 MoveSeq = Net->SendMoveTo(FNetVec2{ 3.f, 4.f });
	TestEqual(TEXT("move sent"), Socket->Sent.Num(), 2);
	Recorder.RunNextTimer();
	TestEqual(TEXT("busy: no keep-alive"), Socket->Sent.Num(), 2);
	TestEqual(TEXT("busy: still armed"), Recorder.Timers.Num(), 1);
	Recorder.RunNextTimer();
	TestEqual(TEXT("idle again: keep-alive"), Socket->Sent.Num(), 3);
	TestTrue(TEXT("seq keeps increasing"), Net->GetLastSentSeq() > MoveSeq);
	TestTrue(TEXT("second keep-alive bytes"), Socket->Sent.Last() == KeepAliveBytes(Net->GetLastSentSeq()));

	// A server close cancels it; the reconnect's socket gets its own, single timer.
	Socket->Closed.Broadcast(4408, TEXT("idle timeout"), true);
	TestEqual(TEXT("keep-alive + reconnect timers"), Recorder.Timers.Num(), 2);
	Recorder.RunNextTimer();   // the cancelled keep-alive check
	TestEqual(TEXT("cancelled check sends nothing"), Socket->Sent.Num(), 3);
	TestEqual(TEXT("cancelled check does not re-arm"), Recorder.Timers.Num(), 1);
	TestEqual(TEXT("reconnect backoff"), Recorder.RunNextTimer(), 0.5f);
	TSharedRef<FFakeWebSocket> Second = Recorder.Sockets.Last();
	TestTrue(TEXT("new socket"), Second != Socket);
	TestEqual(TEXT("not armed until open"), Recorder.Timers.Num(), 0);
	Second->Connected.Broadcast();
	TestEqual(TEXT("armed for the new socket"), Recorder.Timers.Num(), 1);

	// Disconnect cancels it.
	Net->Disconnect();
	Recorder.RunNextTimer();
	TestEqual(TEXT("nothing sent after Disconnect"), Second->Sent.Num(), 0);
	TestEqual(TEXT("no re-arm after Disconnect"), Recorder.Timers.Num(), 0);
	return true;
}

#endif
