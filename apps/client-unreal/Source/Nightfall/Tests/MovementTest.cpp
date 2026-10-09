#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "Nightfall.h"
#include "IWebSocket.h"
#include "Game/NightfallCharacter.h"
#include "Game/OwnEntityComponent.h"
#include "NightfallPlayerController.h"
#include "Net/NetClientSubsystem.h"
#include "Engine/World.h"
#include "Auth/AuthSubsystem.h"
#include "Containers/Ticker.h"
#include "Game/LoginFlowSubsystem.h"
#include "HAL/PlatformProcess.h"
#include "Misc/Guid.h"
#include "Misc/Parse.h"
#include "Net/SessionClientSubsystem.h"
#include "TurboLinkGrpcManager.h"
#include "GameFramework/CharacterMovementComponent.h"

#if WITH_DEV_AUTOMATION_TESTS

// Click -> MoveTo (tiles) -> EntityMove for the own entity -> pawn reconciled. The click path is
// driven with a synthetic hit location; the server's answers are fed through the component's seams.

namespace
{
	class FRecordingSocket final : public IWebSocket
	{
	public:
		TArray<TArray<uint8>> Sent;

		virtual void Connect() override {}
		virtual void Close(int32 Code, const FString& Reason) override {}
		virtual bool IsConnected() override { return true; }
		virtual void Send(const FString& Data) override {}
		virtual void Send(const void* Data, SIZE_T Size, bool bIsBinary) override
		{
			Sent.Emplace_GetRef().Append(static_cast<const uint8*>(Data), static_cast<int32>(Size));
		}
		virtual void SetTextMessageMemoryLimit(uint64 Limit) override {}
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
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FClickToMoveTest, "Nightfall.Movement.ClickToMove",
	EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)

bool FClickToMoveTest::RunTest(const FString& Parameters)
{
	FScopedTestGameInstance Instance;
	UNetClientSubsystem* Net = Instance.Get<UNetClientSubsystem>();
	TSharedPtr<FRecordingSocket> Socket;
	Net->SetSocketFactoryForTesting([&](const FWsUpgradeRequest&) -> TSharedRef<IWebSocket>
	{
		Socket = MakeShared<FRecordingSocket>();
		return Socket.ToSharedRef();
	});
	Net->SetSchedulerForTesting([](float, TFunction<void()>) {});
	Net->Connect(TEXT("ws://localhost:3000/ws"), TEXT("ticket"));
	Socket->Connected.Broadcast();
	Net->SetOwnEntityId(TEXT("0b6e2f6e-0000-4000-8000-000000000001"));
	TestTrue(TEXT("own entity recognised, case-insensitively"), Net->IsOwnEntity(TEXT("0B6E2F6E-0000-4000-8000-000000000001")));
	TestFalse(TEXT("another entity is not ours"), Net->IsOwnEntity(TEXT("someone-else")));

	UWorld* World = Instance.GameInstance->GetWorld();
	if (!TestNotNull(TEXT("test world"), World)) return false;
	ANightfallPlayerController* Controller = World->SpawnActor<ANightfallPlayerController>();
	ANightfallCharacter* Pawn = World->SpawnActor<ANightfallCharacter>(FVector(100.0, 100.0, 96.0), FRotator::ZeroRotator);
	if (!TestNotNull(TEXT("controller"), Controller) || !TestNotNull(TEXT("pawn"), Pawn)) return false;
	Controller->Possess(Pawn);
	UOwnEntityComponent* Own = Pawn->OwnEntity;
	if (!TestNotNull(TEXT("pawn has an OwnEntityComponent"), Own)) return false;

	// Click at (5000, 7000) cm = tile (50, 70).
	const uint32 Seq = Controller->MoveToWorldLocation(FVector(5000.0, 7000.0, 0.0));
	TestEqual(TEXT("first intent seq"), Seq, 1u);
	if (!TestEqual(TEXT("one frame sent"), Socket->Sent.Num(), 1)) return false;

	// ClientMessage{seq=1, move_to{destination{x=50, y=70}}}: 08 01 | 52 len | 0A len | 0D f32 | 15 f32
	const TArray<uint8>& Frame = Socket->Sent[0];
	TestEqual(TEXT("seq field"), Frame.Num() >= 2 ? Frame[0] : 0, 0x08);
	const int32 MoveToAt = Frame.IndexOfByKey(0x52);
	if (!TestTrue(TEXT("move_to (field 10) present"), MoveToAt >= 0)) return false;
	float X = 0.f, Y = 0.f;
	const int32 XAt = Frame.IndexOfByKey(0x0D), YAt = Frame.IndexOfByKey(0x15);
	if (!TestTrue(TEXT("destination x and y present"), XAt > MoveToAt && YAt > XAt)) return false;
	FMemory::Memcpy(&X, &Frame[XAt + 1], 4);
	FMemory::Memcpy(&Y, &Frame[YAt + 1], 4);
	TestEqual(TEXT("MoveTo x is in tiles"), X, 50.f);
	TestEqual(TEXT("MoveTo y is in tiles"), Y, 70.f);

	// The server's EntityMove for our entity (tile units) moves the pawn there.
	FEntityMove Move;
	Move.EntityId = Net->GetOwnEntityId();
	Move.Position = FNetVec2{ 12.f, 20.f };
	Move.Destination = FNetVec2{ 50.f, 70.f };
	Move.Speed = 4.f;
	Own->ApplyMove(Move);
	const FVector Moved = Pawn->GetActorLocation();
	TestTrue(TEXT("pawn snapped to the server position (1200, 2000) cm"), FMath::IsNearlyEqual(Moved.X, 1200.0, 1.0) && FMath::IsNearlyEqual(Moved.Y, 2000.0, 1.0));
	TestEqual(TEXT("server speed (tiles/s) becomes walk speed (cm/s)"), Pawn->GetCharacterMovement()->MaxWalkSpeed, 400.f);

	// Someone else's move does not touch the pawn.
	Move.EntityId = TEXT("someone-else");
	Move.Position = FNetVec2{ 200.f, 200.f };
	Own->ApplyMove(Move);
	TestTrue(TEXT("other entity ignored"), FMath::IsNearlyEqual(Pawn->GetActorLocation().X, 1200.0, 1.0));

	// A rejection for our seq cancels the preview and keeps the pawn at the server position.
	const uint32 Second = Controller->MoveToWorldLocation(FVector(-100000.0, 0.0, 0.0));
	Own->ApplyRejected(FIntentRejected{ Second, 1, TEXT("out of bounds") });
	TestTrue(TEXT("pawn still at the server position after rejection"), FMath::IsNearlyEqual(Pawn->GetActorLocation().X, 1200.0, 1.0));
	TestEqual(TEXT("reason text"), UOwnEntityComponent::RejectReasonText(1), FString(TEXT("destination is outside the zone")));
	return true;
}


// Live: dev-token login -> character -> play ticket -> WebSocket -> own EntitySpawn -> click ->
// Ack -> EntityMove for the own entity -> pawn follows; then an out-of-bounds click is rejected.
// Skipped with a warning when the API is not reachable.
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FMovementEndToEndTest, "Nightfall.Movement.EndToEnd",
	EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)

bool FMovementEndToEndTest::RunTest(const FString& Parameters)
{
	NightfallTest::AllowApiUnavailableLogs(*this);
	FScopedTestGameInstance Instance;
	USessionClient* Session = Instance.Get<USessionClient>();
	UAuthSubsystem* Auth = Instance.Get<UAuthSubsystem>();
	ULoginFlowSubsystem* Flow = Instance.Get<ULoginFlowSubsystem>();
	UNetClientSubsystem* Net = Instance.Get<UNetClientSubsystem>();
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
		return NightfallTest::SkipLive(*this, FString::Printf(TEXT("API not reachable at %s; movement end-to-end skipped."), *Session->GetEndpoint()));
	}
	if (!UAuthSubsystem::GetCommandLineDevToken().IsEmpty()) Auth->StartLogin();
	else Auth->LoginWithDevToken(TEXT("test:") + FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower));
	if (!TestTrue(TEXT("logged in"), Auth->IsLoggedIn())) return false;

	const FString Hex = FGuid::NewGuid().ToString(EGuidFormats::Digits);
	FString Name = TEXT("Walk");
	for (int32 I = 0; I < 10; ++I) Name.AppendChar(TEXT('a') + static_cast<TCHAR>(FParse::HexDigit(Hex[Hex.Len() - 1 - I])));
	bool bCreated = false;
	FNetResult CreateResult;
	FGrpcNightfallV1Character Character;
	Flow->CreateCharacter(Name, EGrpcNightfallV1Race::RACE_HUMAN, [&](const FNetResult& R, const FGrpcNightfallV1Character& C) { CreateResult = R; Character = C; bCreated = true; });
	if (!TestTrue(TEXT("character created"), Pump([&] { return bCreated; }, 10.0)) || !TestTrue(*CreateResult.Message, CreateResult.IsOk())) return false;

	bool bTicketed = false;
	Flow->EnterWorld(Character.Id, [&](const FNetResult&) { bTicketed = true; });
	if (!TestTrue(TEXT("ticket issued"), Pump([&] { return bTicketed; }, 10.0))) return false;
	TestEqual(TEXT("own entity id is the character id"), Net->GetOwnEntityId(), Character.Id);

	// The server sends EntitySpawn for every entity in AOI, ours included.
	if (!TestTrue(TEXT("connected and own EntitySpawn received"), Pump([&] { return Net->GetKnownEntities().Contains(Character.Id); }, 10.0))) return false;
	const FEntitySpawn Spawn = Net->GetKnownEntities()[Character.Id];

	int32 Acks = 0, MovesForUs = 0;
	uint32 RejectedSeq = 0, RejectedReason = 0;
	float LastSpeed = 0.f;
	FNetVec2 LastServerPos = Spawn.Position;
	Net->OnIntentAck.AddLambda([&](const FAck&) { ++Acks; });
	Net->OnIntentRejected.AddLambda([&](const FIntentRejected& R) { RejectedSeq = R.Seq; RejectedReason = R.Reason; });
	Net->OnEntityMove.AddLambda([&](const FEntityMove& M)
	{
		if (M.EntityId == Character.Id) { ++MovesForUs; LastSpeed = M.Speed; LastServerPos = M.Position; }
	});

	UWorld* World = Instance.GameInstance->GetWorld();
	ANightfallPlayerController* Controller = World->SpawnActor<ANightfallPlayerController>();
	ANightfallCharacter* Pawn = World->SpawnActor<ANightfallCharacter>(
		FVector(Spawn.Position.X * 100.0, Spawn.Position.Y * 100.0, 96.0), FRotator::ZeroRotator);
	Controller->Possess(Pawn);
	Pawn->OwnEntity->BeginPlay();   // binds the net delegates; the test world is not begun
	const FVector Start = Pawn->GetActorLocation();

	// Walk 12 tiles east (inside the 256-tile zone).
	const FVector Goal(Start.X + 1200.0, Start.Y, 0.0);
	const uint32 Seq = Controller->MoveToWorldLocation(Goal);
	TestTrue(TEXT("MoveTo acked"), Pump([&] { return Acks >= 1; }, 5.0));
	TestEqual(TEXT("not rejected"), RejectedSeq, 0u);
	TestTrue(TEXT("EntityMove for the own entity arrived"), Pump([&] { return MovesForUs >= 1; }, 5.0));
	TestTrue(TEXT("server speed reported"), LastSpeed > 0.f);
	// No nav mesh in this world, so the pawn only moves by server corrections: wait until the
	// server has walked far enough to force one.
	Pump([&] { return FVector::Dist2D(FVector(LastServerPos.X * 100.0, LastServerPos.Y * 100.0, 0.0), Start) > 500.0; }, 8.0);
	TestTrue(TEXT("pawn followed the server east"), Pawn->GetActorLocation().X > Start.X + 300.0);
	UE_LOG(LogNightfall, Display, TEXT("Movement.EndToEnd: seq %u start %s now %s server tile (%.2f, %.2f), %d moves, speed %.2f"),
		Seq, *Start.ToString(), *Pawn->GetActorLocation().ToString(), LastServerPos.X, LastServerPos.Y, MovesForUs, LastSpeed);

	// Out of bounds is rejected and reported by seq.
	const uint32 Bad = Controller->MoveToWorldLocation(FVector(-5000000.0, 0.0, 0.0));
	TestTrue(TEXT("out-of-bounds MoveTo rejected"), Pump([&] { return RejectedSeq == Bad; }, 5.0));
	TestEqual(TEXT("reason OUT_OF_BOUNDS"), RejectedReason, 1u);
	return true;
}

#endif
