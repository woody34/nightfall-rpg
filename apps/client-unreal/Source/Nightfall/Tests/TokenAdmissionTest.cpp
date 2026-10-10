#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "IWebSocket.h"
#include "Bot/BotPredicates.h"
#include "Bot/ClassBotPredicates.h"
#include "Combat/CombatStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Net/ProtoCodec.h"

#if WITH_DEV_AUTOMATION_TESTS

namespace
{
	constexpr EAutomationTestFlags Flags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;

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

	struct FPb
	{
		TArray<uint8> B;
		void Varint(uint64 V) { while (V >= 0x80) { B.Add(static_cast<uint8>(V | 0x80)); V >>= 7; } B.Add(static_cast<uint8>(V)); }
		FPb& U(int32 Field, uint64 V) { Varint(static_cast<uint64>(Field) << 3); Varint(V); return *this; }
		FPb& S(int32 Field, const char* Text)
		{
			const int32 Len = FCStringAnsi::Strlen(Text);
			Varint((static_cast<uint64>(Field) << 3) | 2); Varint(Len);
			B.Append(reinterpret_cast<const uint8*>(Text), Len);
			return *this;
		}
		FPb& M(int32 Field, const FPb& Inner)
		{
			Varint((static_cast<uint64>(Field) << 3) | 2); Varint(Inner.B.Num()); B.Append(Inner.B);
			return *this;
		}
	};

	/**
	 * Encodes a golden ServerMessage containing WorldEvent.stats_changed matching packages/proto/nightfall/v1/world.proto:
	 * ServerMessage.event = 2
	 * WorldEvent.stats_changed = 7
	 * StatsChanged fields:
	 * 1 entity, 2 hp, 3 max_hp, 4 mp, 5 max_mp, 6 level, 7 xp, 8 cp, 9 max_cp, 10 class_id, 11 sp,
	 * 12 token_tier_1_count, 13 token_tier_2_count, 14 tick.
	 */
	TArray<uint8> EncodeStatsFrame(const char* EntityId, uint32 Tier1, uint32 Tier2, uint64 Tick)
	{
		FPb Stats;
		Stats.S(1, EntityId)
			 .U(2, 100) // hp
			 .U(3, 100) // max_hp
			 .U(4, 50)  // mp
			 .U(5, 50)  // max_mp
			 .U(6, 1)   // level
			 .U(7, 0)   // xp
			 .U(8, 20)  // cp
			 .U(9, 20)  // max_cp
			 .U(10, 1)  // class_id
			 .U(11, 0)  // sp
			 .U(12, Tier1) // token_tier_1_count
			 .U(13, Tier2) // token_tier_2_count
			 .U(14, Tick); // tick
		FPb Event;
		Event.M(7, Stats); // WorldEvent.stats_changed = 7
		FPb Msg;
		Msg.M(2, Event);   // ServerMessage.event = 2
		return Msg.B;
	}
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FTokenAdmissionTest, "Nightfall.Class.State.TokenAdmission", Flags)
bool FTokenAdmissionTest::RunTest(const FString& Parameters)
{
	FScopedTestGameInstance Rig;
	UNetClientSubsystem* Net = Rig.Get<UNetClientSubsystem>();
	UCombatStateSubsystem* Combat = Rig.Get<UCombatStateSubsystem>();
	TestNotNull(TEXT("Net subsystem exists"), Net);
	TestNotNull(TEXT("Combat subsystem exists"), Combat);
	if (!Net || !Combat)
	{
		return false;
	}

	TArray<TSharedRef<FFakeWebSocket>> Sockets;
	Net->SetSocketFactoryForTesting([&Sockets](const FWsUpgradeRequest&) -> TSharedRef<IWebSocket>
	{
		return Sockets.Add_GetRef(MakeShared<FFakeWebSocket>());
	});
	Net->SetSchedulerForTesting([](float, TFunction<void()>) {});

	const FString OwnId = TEXT("hero-own-entity");
	const FString OtherId = TEXT("other-entity");
	Net->SetOwnEntityId(OwnId);

	// First transport connection
	Net->Connect(TEXT("ws://localhost:3000/ws"), TEXT("ticket-1"));
	TestEqual(TEXT("first socket created"), Sockets.Num(), 1);
	Sockets[0]->Connected.Broadcast();

	// Prior handler registration to verify it is not detached by Bind/Reset/Unbind
	int32 PriorHandlerCalls = 0;
	FDelegateHandle PriorHandle = Net->OnStatsChanged.AddLambda([&PriorHandlerCalls](const FStatsChanged&)
	{
		++PriorHandlerCalls;
	});

	FBotPredicateRegistry Registry;
	Registry.RegisterBuiltins();

	FBotObservations Observations;
	Observations.Bind(Rig.GameInstance);
	FBotContext Context{ Rig.GameInstance, &Observations };

	FString Error;
	auto T1Eq0 = Registry.Parse({ TEXT("initial_tier1_tokens"), TEXT("=="), TEXT("0") }, Error);
	auto T1Eq1 = Registry.Parse({ TEXT("initial_tier1_tokens"), TEXT("=="), TEXT("1") }, Error);
	auto T2Eq0 = Registry.Parse({ TEXT("initial_tier2_tokens"), TEXT("=="), TEXT("0") }, Error);
	auto T2Eq1 = Registry.Parse({ TEXT("initial_tier2_tokens"), TEXT("=="), TEXT("1") }, Error);
	TestTrue(TEXT("predicates parsed successfully"), T1Eq0.IsSet() && T1Eq1.IsSet() && T2Eq0.IsSet() && T2Eq1.IsSet());
	if (!T1Eq0.IsSet() || !T1Eq1.IsSet() || !T2Eq0.IsSet() || !T2Eq1.IsSet())
	{
		Net->OnStatsChanged.Remove(PriorHandle);
		return false;
	}

	// 1. Initial state: unknown before first own StatsChanged in current transport scope
	TestFalse(TEXT("initial tier1 observation unset before stats"), Observations.GetInitialTier1Tokens(Net).IsSet());
	TestFalse(TEXT("initial tier2 observation unset before stats"), Observations.GetInitialTier2Tokens(Net).IsSet());
	TestFalse(TEXT("initial_tier1_tokens == 0 is false when unknown"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("initial_tier1_tokens observed unknown before stats"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("initial_tier2_tokens == 0 is false when unknown"), T2Eq0(Context).bTrue);
	TestEqual(TEXT("initial_tier2_tokens observed unknown before stats"), T2Eq0(Context).Observed, FString(TEXT("unknown")));

	// 2. Other entity ignored: stats for another entity must not set observations
	const TArray<uint8> OtherStatsFrame = EncodeStatsFrame(TCHAR_TO_ANSI(*OtherId), 5, 8, 10);
	Sockets[0]->RawMessage.Broadcast(OtherStatsFrame.GetData(), static_cast<SIZE_T>(OtherStatsFrame.Num()), 0);

	TestFalse(TEXT("other entity stats ignored for tier1"), Observations.GetInitialTier1Tokens(Net).IsSet());
	TestFalse(TEXT("other entity stats ignored for tier2"), Observations.GetInitialTier2Tokens(Net).IsSet());
	TestEqual(TEXT("tier1 remains unknown after other entity stats"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	TestEqual(TEXT("prior handler called for other entity event"), PriorHandlerCalls, 1);

	// 3. First current owner stats0 captured: first typed owner StatsChanged in transport scope captures initial 0 tokens
	const TArray<uint8> FirstOwnStats0Frame = EncodeStatsFrame(TCHAR_TO_ANSI(*OwnId), 0, 0, 20);
	Sockets[0]->RawMessage.Broadcast(FirstOwnStats0Frame.GetData(), static_cast<SIZE_T>(FirstOwnStats0Frame.Num()), 0);

	TestTrue(TEXT("first own tier1 captured"), Observations.GetInitialTier1Tokens(Net).IsSet());
	TestTrue(TEXT("first own tier2 captured"), Observations.GetInitialTier2Tokens(Net).IsSet());
	if (!Observations.GetInitialTier1Tokens(Net).IsSet() || !Observations.GetInitialTier2Tokens(Net).IsSet())
	{
		Observations.Unbind();
		Net->OnStatsChanged.Remove(PriorHandle);
		return false;
	}
	TestEqual(TEXT("first own tier1 count is 0"), Observations.GetInitialTier1Tokens(Net).GetValue(), 0u);
	TestEqual(TEXT("first own tier2 count is 0"), Observations.GetInitialTier2Tokens(Net).GetValue(), 0u);
	TestTrue(TEXT("initial_tier1_tokens == 0 holds"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("initial_tier1_tokens observed 0"), T1Eq0(Context).Observed, FString(TEXT("0")));
	TestTrue(TEXT("initial_tier2_tokens == 0 holds"), T2Eq0(Context).bTrue);
	TestEqual(TEXT("initial_tier2_tokens observed 0"), T2Eq0(Context).Observed, FString(TEXT("0")));
	TestEqual(TEXT("prior handler called for first own stats"), PriorHandlerCalls, 2);

	// 4. Later live updates in same scope do not overwrite initial zero
	const TArray<uint8> LaterStatsFrame = EncodeStatsFrame(TCHAR_TO_ANSI(*OwnId), 3, 5, 30);
	Sockets[0]->RawMessage.Broadcast(LaterStatsFrame.GetData(), static_cast<SIZE_T>(LaterStatsFrame.Num()), 0);

	TestEqual(TEXT("later live update does not replace initial tier1 zero"), Observations.GetInitialTier1Tokens(Net).GetValue(), 0u);
	TestEqual(TEXT("later live update does not replace initial tier2 zero"), Observations.GetInitialTier2Tokens(Net).GetValue(), 0u);
	TestTrue(TEXT("initial_tier1_tokens == 0 still holds after live update"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("prior handler called for later stats"), PriorHandlerCalls, 3);

	// Same-scope stale stats (tick 25 < tick 30) rejected by projection and bot observations
	const TArray<uint8> StaleStatsFrame = EncodeStatsFrame(TCHAR_TO_ANSI(*OwnId), 9, 9, 25);
	Sockets[0]->RawMessage.Broadcast(StaleStatsFrame.GetData(), static_cast<SIZE_T>(StaleStatsFrame.Num()), 0);
	TestEqual(TEXT("stale stats rejected: combat LastStatsTick unchanged"), Combat->GetOwn().LastStatsTick, 30ULL);
	TestEqual(TEXT("initial tier1 unchanged after stale stats"), Observations.GetInitialTier1Tokens(Net).GetValue(), 0u);

	// 5. Closed leaves the generation intact until replacement: reads and capture must still be unknown.
	const auto RetainedOldRaw = Sockets[0]->RawMessage;
	const uint64 ClosedGeneration = Net->GetTransportGeneration();
	const TArray<uint8> ValidStaleStatsFrame = EncodeStatsFrame(TCHAR_TO_ANSI(*OwnId), 99, 99, 35);
	FServerMessage DecodedCheckMsg;
	TestTrue(TEXT("encoded frame decodes as valid ServerMessage before stale callback test"),
		NightfallProto::Decode(ValidStaleStatsFrame.GetData(), ValidStaleStatsFrame.Num(), DecodedCheckMsg));
	TestTrue(TEXT("decoded ServerMessage contains valid StatsChanged event"),
		DecodedCheckMsg.Event.IsSet() && DecodedCheckMsg.Event->StatsChanged.IsSet());

	Sockets[0]->Closed.Broadcast(1000, TEXT("test transport closed"), true);
	TestFalse(TEXT("Net disconnected after actual socket Closed"), Net->IsConnected());
	TestEqual(TEXT("Closed does not advance transport generation"), Net->GetTransportGeneration(), ClosedGeneration);
	TestFalse(TEXT("initial tier1 getter unset immediately after Closed"), Observations.GetInitialTier1Tokens(Net).IsSet());
	TestFalse(TEXT("initial tier2 getter unset immediately after Closed"), Observations.GetInitialTier2Tokens(Net).IsSet());
	TestFalse(TEXT("initial tier1 predicate false immediately after Closed"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("initial tier1 observed unknown immediately after Closed"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	TestEqual(TEXT("initial tier2 observed unknown immediately after Closed"), T2Eq0(Context).Observed, FString(TEXT("unknown")));

	// A newly bound observer has no frozen value to mask an erroneous late-frame capture.
	// Keep the main observer's captured scope to also exercise replacement-generation fencing below.
	FBotObservations ClosedCapture;
	ClosedCapture.Bind(Rig.GameInstance);
	const int32 PriorCallsBeforeClosedFrame = PriorHandlerCalls;
	RetainedOldRaw.Broadcast(ValidStaleStatsFrame.GetData(), static_cast<SIZE_T>(ValidStaleStatsFrame.Num()), 0);
	TestEqual(TEXT("retained callback actually dispatches the valid frame before replacement"), PriorHandlerCalls, PriorCallsBeforeClosedFrame + 1);
	TestFalse(TEXT("closed socket frame cannot capture tier1"), ClosedCapture.InitialTier1Tokens.IsSet());
	TestFalse(TEXT("closed socket frame cannot capture tier2"), ClosedCapture.InitialTier2Tokens.IsSet());
	TestFalse(TEXT("closed socket frame cannot capture scope"), ClosedCapture.InitialTokenScopeGeneration.IsSet());
	TestEqual(TEXT("tier1 remains unknown after closed socket frame"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	ClosedCapture.Unbind();

	// Replacement actual Net.Connect + new socket Connected resets Combat and advances the scope.
	Net->Connect(TEXT("ws://localhost:3000/ws"), TEXT("ticket-2"));
	TestEqual(TEXT("second socket created on reconnect"), Sockets.Num(), 2);
	Sockets[1]->Connected.Broadcast();

	// Scope changed: generation bumped. Before fresh frame arrives, initial predicates are unknown
	TestFalse(TEXT("initial tier1 unknown after reconnect before fresh frame"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("initial tier1 observed unknown after reconnect before fresh frame"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("initial tier2 unknown after reconnect before fresh frame"), T2Eq0(Context).bTrue);
	TestEqual(TEXT("initial tier2 observed unknown after reconnect before fresh frame"), T2Eq0(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("initial tier1 getter unset before fresh frame"), Observations.GetInitialTier1Tokens(Net).IsSet());

	// 6. Retained old RawMessage delegate given a VALID encoded owner StatsChanged frame cannot set observations
	// Broadcast valid frame on old retained delegate
	RetainedOldRaw.Broadcast(ValidStaleStatsFrame.GetData(), static_cast<SIZE_T>(ValidStaleStatsFrame.Num()), 0);

	TestFalse(TEXT("retained old raw callback cannot set observations"), Observations.GetInitialTier1Tokens(Net).IsSet());
	TestEqual(TEXT("initial tier1 remains unknown after old socket callback"), T1Eq0(Context).Observed, FString(TEXT("unknown")));

	// 7. Current new socket same valid owner 1/1 frame captures first1/1
	// Lower tick boundary (tick 10 < tick 30 of prior session) is meaningful and accepted in new session generation
	const TArray<uint8> FreshOwnerStatsFrame = EncodeStatsFrame(TCHAR_TO_ANSI(*OwnId), 1, 1, 10);
	Sockets[1]->RawMessage.Broadcast(FreshOwnerStatsFrame.GetData(), static_cast<SIZE_T>(FreshOwnerStatsFrame.Num()), 0);

	TestTrue(TEXT("current new socket captures fresh initial tier1"), Observations.GetInitialTier1Tokens(Net).IsSet());
	TestTrue(TEXT("current new socket captures fresh initial tier2"), Observations.GetInitialTier2Tokens(Net).IsSet());
	if (!Observations.GetInitialTier1Tokens(Net).IsSet() || !Observations.GetInitialTier2Tokens(Net).IsSet())
	{
		Observations.Unbind();
		Net->OnStatsChanged.Remove(PriorHandle);
		return false;
	}
	TestEqual(TEXT("fresh lower-tick stats admitted after reconnect"), Combat->GetOwn().LastStatsTick, 10ULL);
	TestEqual(TEXT("current new socket initial tier1 is 1"), Observations.GetInitialTier1Tokens(Net).GetValue(), 1u);
	TestEqual(TEXT("current new socket initial tier2 is 1"), Observations.GetInitialTier2Tokens(Net).GetValue(), 1u);
	TestTrue(TEXT("initial_tier1_tokens == 1 holds for fresh admission"), T1Eq1(Context).bTrue);
	TestEqual(TEXT("initial_tier1_tokens observed 1"), T1Eq1(Context).Observed, FString(TEXT("1")));
	TestTrue(TEXT("initial_tier2_tokens == 1 holds for fresh admission"), T2Eq1(Context).bTrue);
	TestEqual(TEXT("initial_tier2_tokens observed 1"), T2Eq1(Context).Observed, FString(TEXT("1")));

	// Later update in new session does not overwrite 1/1
	const TArray<uint8> FreshLaterFrame = EncodeStatsFrame(TCHAR_TO_ANSI(*OwnId), 2, 2, 15);
	Sockets[1]->RawMessage.Broadcast(FreshLaterFrame.GetData(), static_cast<SIZE_T>(FreshLaterFrame.Num()), 0);
	TestEqual(TEXT("later update in new session does not overwrite initial 1/1"), Observations.GetInitialTier1Tokens(Net).GetValue(), 1u);
	TestTrue(TEXT("initial_tier1_tokens == 1 still holds"), T1Eq1(Context).bTrue);

	// 8. New owner/account admission uses an actual replacement transport, resetting Combat's tick fence.
	const FString NewHeroId = TEXT("hero-new-account");
	Net->SetOwnEntityId(NewHeroId);

	TestFalse(TEXT("initial tier1 unknown immediately after owner switch"), T1Eq1(Context).bTrue);
	TestEqual(TEXT("tier1 observed unknown after owner switch"), T1Eq1(Context).Observed, FString(TEXT("unknown")));
	TestFalse(TEXT("getter returns unset after owner switch"), Observations.GetInitialTier1Tokens(Net).IsSet());
	Net->Connect(TEXT("ws://localhost:3000/ws"), TEXT("ticket-3"));
	TestEqual(TEXT("new account creates a third socket"), Sockets.Num(), 3);
	Sockets[2]->Connected.Broadcast();
	TestTrue(TEXT("new account transport connected"), Net->IsConnected());
	TestEqual(TEXT("new account remains the Net owner"), Net->GetOwnEntityId(), NewHeroId);
	TestEqual(TEXT("new account Connected resets Combat stats tick fence"), Combat->GetOwn().LastStatsTick, 0ULL);
	TestEqual(TEXT("new account tier1 unknown until fresh stats"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	TestEqual(TEXT("new account tier2 unknown until fresh stats"), T2Eq0(Context).Observed, FString(TEXT("unknown")));

	// Fresh stats for new hero captured
	const TArray<uint8> NewHeroStatsFrame = EncodeStatsFrame(TCHAR_TO_ANSI(*NewHeroId), 0, 0, 5);
	Sockets[2]->RawMessage.Broadcast(NewHeroStatsFrame.GetData(), static_cast<SIZE_T>(NewHeroStatsFrame.Num()), 0);

	TestTrue(TEXT("new hero initial tier1 captured"), Observations.GetInitialTier1Tokens(Net).IsSet());
	TestTrue(TEXT("new hero initial tier2 captured"), Observations.GetInitialTier2Tokens(Net).IsSet());
	if (!Observations.GetInitialTier1Tokens(Net).IsSet() || !Observations.GetInitialTier2Tokens(Net).IsSet())
	{
		Observations.Unbind();
		Net->OnStatsChanged.Remove(PriorHandle);
		return false;
	}
	TestEqual(TEXT("new account lower tick 5 admitted"), Combat->GetOwn().LastStatsTick, 5ULL);
	TestEqual(TEXT("new hero initial tier1 is 0"), Observations.GetInitialTier1Tokens(Net).GetValue(), 0u);
	TestEqual(TEXT("new hero initial tier2 is 0"), Observations.GetInitialTier2Tokens(Net).GetValue(), 0u);
	TestTrue(TEXT("initial_tier1_tokens == 0 holds for new hero"), T1Eq0(Context).bTrue);
	TestTrue(TEXT("initial_tier2_tokens == 0 holds for new hero"), T2Eq0(Context).bTrue);

	// 9. Unbind clears captured values directly and preserves unrelated observations and handlers.
	Observations.DamageNumbers = 7;
	Observations.Unbind();
	TestFalse(TEXT("Unbind clears captured tier1 before Reset"), Observations.InitialTier1Tokens.IsSet());
	TestFalse(TEXT("Unbind clears captured tier2 before Reset"), Observations.InitialTier2Tokens.IsSet());
	TestFalse(TEXT("Unbind clears captured owner scope"), Observations.InitialTokenScopeEntity.IsSet());
	TestFalse(TEXT("Unbind clears captured generation scope"), Observations.InitialTokenScopeGeneration.IsSet());
	TestFalse(TEXT("Unbind makes tier1 getter unknown"), Observations.GetInitialTier1Tokens(Net).IsSet());
	TestFalse(TEXT("Unbind makes tier2 getter unknown"), Observations.GetInitialTier2Tokens(Net).IsSet());
	TestEqual(TEXT("Unbind makes tier1 predicate unknown"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	TestEqual(TEXT("Unbind preserves unrelated damage observations"), Observations.DamageNumbers, 7);
	const int32 PriorCallsBeforeUnbound = PriorHandlerCalls;

	const TArray<uint8> UnboundStatsFrame = EncodeStatsFrame(TCHAR_TO_ANSI(*NewHeroId), 7, 7, 20);
	Sockets[2]->RawMessage.Broadcast(UnboundStatsFrame.GetData(), static_cast<SIZE_T>(UnboundStatsFrame.Num()), 0);

	TestFalse(TEXT("unbound observations do not capture late stats"), Observations.GetInitialTier1Tokens(Net).IsSet());
	TestEqual(TEXT("prior handler still received the broadcast after unbind"), PriorHandlerCalls, PriorCallsBeforeUnbound + 1);

	// 10. Rebind captures current stats; scenario Reset independently clears that captured value.
	Observations.Bind(Rig.GameInstance);
	Sockets[2]->RawMessage.Broadcast(UnboundStatsFrame.GetData(), static_cast<SIZE_T>(UnboundStatsFrame.Num()), 0);
	TestTrue(TEXT("rebound observations capture tier1 before Reset"), Observations.GetInitialTier1Tokens(Net).IsSet());
	TestTrue(TEXT("rebound observations capture tier2 before Reset"), Observations.GetInitialTier2Tokens(Net).IsSet());
	Observations.Reset();
	TestFalse(TEXT("Reset clears initial tier1"), Observations.GetInitialTier1Tokens(Net).IsSet());
	TestFalse(TEXT("Reset clears initial tier2"), Observations.GetInitialTier2Tokens(Net).IsSet());
	TestFalse(TEXT("initial_tier1_tokens == 0 is false after Reset"), T1Eq0(Context).bTrue);
	TestEqual(TEXT("initial_tier1_tokens observed unknown after Reset"), T1Eq0(Context).Observed, FString(TEXT("unknown")));
	Observations.Unbind();

	Net->OnStatsChanged.Remove(PriorHandle);
	return true;
}

#endif // WITH_DEV_AUTOMATION_TESTS
