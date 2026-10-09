#include "Misc/AutomationTest.h"
#include "Net/ProtoCodec.h"

#if WITH_DEV_AUTOMATION_TESTS

// Byte-level checks of the WebSocket codec against hand-assembled protobuf, so a regression in
// the generated code, the bridge, or the FNet* mapping shows up without a server.

namespace
{
	constexpr EAutomationTestFlags CodecTestFlags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FProtoCodecEncodeTest, "Nightfall.Net.ProtoCodec.Encode", CodecTestFlags)

bool FProtoCodecEncodeTest::RunTest(const FString& Parameters)
{
	TArray<uint8> Bytes;

	FClientMessage Move;
	Move.Seq = 1;
	Move.MoveTo = FMoveToIntent{ FNetVec2{ 1.f, 2.f } };
	NightfallProto::Encode(Move, Bytes);
	// seq=1; move_to(10) { destination(1) { x(1)=1.0f, y(2)=2.0f } }
	const TArray<uint8> ExpectedMove = { 0x08, 0x01, 0x52, 0x0c, 0x0a, 0x0a, 0x0d, 0x00, 0x00, 0x80, 0x3f, 0x15, 0x00, 0x00, 0x00, 0x40 };
	TestEqual(TEXT("MoveTo bytes"), Bytes, ExpectedMove);

	FClientMessage Bare;
	Bare.Seq = 7;
	NightfallProto::Encode(Bare, Bytes);
	const TArray<uint8> ExpectedBare = { 0x08, 0x07 };
	TestEqual(TEXT("intent-less message encodes seq only"), Bytes, ExpectedBare);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FProtoCodecDecodeTest, "Nightfall.Net.ProtoCodec.Decode", CodecTestFlags)

bool FProtoCodecDecodeTest::RunTest(const FString& Parameters)
{
	FServerMessage Msg;

	// ack(1) { seq=5, tick=9 }
	const uint8 Ack[] = { 0x0a, 0x04, 0x08, 0x05, 0x10, 0x09 };
	if (TestTrue(TEXT("ack decodes"), NightfallProto::Decode(Ack, sizeof(Ack), Msg)) && TestTrue(TEXT("ack set"), Msg.Ack.IsSet()))
	{
		TestEqual(TEXT("ack seq"), Msg.Ack->Seq, 5u);
		TestEqual(TEXT("ack tick"), Msg.Ack->Tick, uint64(9));
		TestFalse(TEXT("no event"), Msg.Event.IsSet());
	}

	// rejected(3) { seq=3, reason=RATE_LIMITED(5), detail="x" }
	const uint8 Rejected[] = { 0x1a, 0x07, 0x08, 0x03, 0x10, 0x05, 0x1a, 0x01, 'x' };
	if (TestTrue(TEXT("rejected decodes"), NightfallProto::Decode(Rejected, sizeof(Rejected), Msg)) && TestTrue(TEXT("rejected set"), Msg.Rejected.IsSet()))
	{
		TestEqual(TEXT("rejected seq"), Msg.Rejected->Seq, 3u);
		TestEqual(TEXT("rejected reason"), Msg.Rejected->Reason, 5u);
		TestEqual(TEXT("rejected detail"), Msg.Rejected->Detail, FString(TEXT("x")));
		TestFalse(TEXT("decode resets previous ack"), Msg.Ack.IsSet());
	}

	// event(2) { spawn(1) { entity_id="e", name="n", kind=PLAYER(1), session_generation=2 } }
	const uint8 Spawn[] = { 0x12, 0x0c, 0x0a, 0x0a, 0x0a, 0x01, 'e', 0x12, 0x01, 'n', 0x20, 0x01, 0x28, 0x02 };
	if (TestTrue(TEXT("spawn decodes"), NightfallProto::Decode(Spawn, sizeof(Spawn), Msg))
		&& TestTrue(TEXT("event set"), Msg.Event.IsSet()) && TestTrue(TEXT("spawn set"), Msg.Event->Spawn.IsSet()))
	{
		TestEqual(TEXT("spawn id"), Msg.Event->Spawn->EntityId, FString(TEXT("e")));
		TestEqual(TEXT("spawn name"), Msg.Event->Spawn->Name, FString(TEXT("n")));
		TestEqual(TEXT("spawn kind"), Msg.Event->Spawn->Kind, 1u);
		TestEqual(TEXT("spawn generation"), Msg.Event->Spawn->SessionGeneration, 2u);
		TestFalse(TEXT("no move"), Msg.Event->Move.IsSet());
	}

	const uint8 Empty[] = { 0 };
	TestTrue(TEXT("empty frame is a valid, empty ServerMessage"), NightfallProto::Decode(Empty, 0, Msg));
	TestFalse(TEXT("empty frame has no payload"), Msg.Ack.IsSet() || Msg.Event.IsSet() || Msg.Rejected.IsSet());

	const uint8 Truncated[] = { 0x0a, 0x04, 0x08 };
	TestFalse(TEXT("truncated frame is rejected"), NightfallProto::Decode(Truncated, sizeof(Truncated), Msg));
	return true;
}

#endif
