#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Net/NetClientSubsystem.h"
#include "Net/ProtoCodec.h"
#include "Net/TickEstimator.h"
#include "Skills/SkillStateSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "Bot/BotPredicates.h"
#include "Misc/Base64.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"
#include "Serialization/JsonSerializer.h"

// D132 (T22): deadline-relative bot reads advance the newest received tick by the local 100 ms
// clock between frames, so they do not depend on how often the zone sends a tick-bearing frame.

#if WITH_DEV_AUTOMATION_TESTS
namespace TickEstimatorTests
{
	constexpr EAutomationTestFlags Flags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FTickEstimatorGapsTest, "Nightfall.Net.TickEstimator.FrameGaps", TickEstimatorTests::Flags)
bool FTickEstimatorGapsTest::RunTest(const FString& Parameters)
{
	FTickEstimator E;
	TestEqual(TEXT("no base: 0"), E.Estimate(5.0), uint64(0));

	// Frames on the zone's 100 ms schedule plus a constant 37 ms delivery delay, with gaps of 1, 30
	// and 300 ticks. Sampled every 10 ms between arrivals: never ahead of the next received tick,
	// never behind the newest received one, and exactly next-1 just before the next frame.
	const double Latency = 0.037;
	const uint64 Gaps[] = {1, 30, 300, 1, 300, 30, 1, 1};
	uint64 Tick = 1000;
	double Arrival = Tick * FTickEstimator::TICK_SECONDS + Latency;
	E.Observe(Tick, Arrival);
	int32 Samples = 0;
	for (const uint64 Gap : Gaps)
	{
		const uint64 Next = Tick + Gap;
		const double NextArrival = Next * FTickEstimator::TICK_SECONDS + Latency;
		uint64 Previous = Tick;
		for (double T = Arrival; T < NextArrival - 1e-6; T += 0.01)
		{
			const uint64 V = E.Estimate(T);
			++Samples;
			if (!TestTrue(FString::Printf(TEXT("gap %llu: never ahead of next received tick %llu (got %llu at +%.3f s)"), Gap, Next, V, T - Arrival), V <= Next)) return false;
			if (!TestTrue(FString::Printf(TEXT("gap %llu: never behind newest received tick"), Gap), V >= Tick)) return false;
			if (!TestTrue(TEXT("monotonic between frames"), V >= Previous)) return false;
			Previous = V;
		}
		TestEqual(FString::Printf(TEXT("gap %llu: advanced to next-1 just before the frame"), Gap), E.Estimate(NextArrival - 0.001), Next - 1);
		TestEqual(FString::Printf(TEXT("gap %llu: at 100 ms per tick"), Gap), E.Estimate(Arrival + 0.1 * double(Gap / 2) + 0.001), Tick + Gap / 2);
		E.Observe(Next, NextArrival);
		TestEqual(FString::Printf(TEXT("gap %llu: rebased on the received tick"), Gap), E.Estimate(NextArrival), Next);
		Tick = Next; Arrival = NextArrival;
	}
	TestTrue(TEXT("samples taken"), Samples > 6000);

	// A zone that ran slow (stall): the frame arrives later than the projection. The estimate is
	// clamped down to the received tick at arrival and advances from there.
	E.Reset(); E.Observe(500, 10.0);
	TestEqual(TEXT("stall: projection before the late frame"), E.Estimate(12.05), uint64(520));
	E.Observe(510, 12.05);
	TestEqual(TEXT("stall: clamped to the received tick"), E.Estimate(12.05), uint64(510));
	TestEqual(TEXT("stall: advances from the clamp"), E.Estimate(12.351), uint64(513));
	// The same received tick again (another frame of that tick) does not move the base.
	E.Observe(510, 12.2); TestEqual(TEXT("same tick keeps its arrival base"), E.Estimate(12.351), uint64(513));
	// A frame earlier than the projection (jitter) rebases upward.
	E.Observe(530, 12.4); TestEqual(TEXT("early frame rebases up"), E.Estimate(12.4), uint64(530));
	TestEqual(TEXT("clock going backwards holds the received tick"), E.Estimate(1.0), uint64(530));
	E.Reset(); TestFalse(TEXT("reset forgets the base"), E.HasBase()); TestEqual(TEXT("reset estimate"), E.Estimate(20.0), uint64(0));

	// Wired into the net client: every tick-bearing message rebases it, a new socket resets it.
	FScopedTestGameInstance Scope;
	auto* N = Scope.Get<UNetClientSubsystem>();
	double Now = 100.0;
	N->SetMonotonicClockForTesting([&Now]() { return Now; });
	TestEqual(TEXT("net: nothing received"), N->GetEstimatedTick(), uint64(0));
	FServerMessage Ack; Ack.Ack.Emplace(); Ack.Ack->Seq = 1; Ack.Ack->Tick = 40;
	N->DispatchServerMessage(Ack);
	TestEqual(TEXT("net: received tick"), N->GetEstimatedTick(), uint64(40));
	Now += 2.25;
	TestEqual(TEXT("net: advanced 22 ticks with no frame"), N->GetEstimatedTick(), uint64(62));
	TestEqual(TEXT("net: received tick unchanged"), N->GetLatestReceivedTick(), uint64(40));
	auto* Skills = Scope.Get<USkillStateSubsystem>();
	TestEqual(TEXT("skill state: estimated"), Skills->GetEstimatedTick(), uint64(62));
	TestEqual(TEXT("skill state: received-only tick unchanged"), Skills->GetTick(), uint64(40));
	return true;
}

namespace TickEstimatorTests
{
	struct FFixtureFrame { uint64 Tick = 0; int64 ServerTimeMs = 0; FServerMessage Message; };

	struct FWindowResult
	{
		uint64 Deadline = 0;
		TSet<uint64> Estimated;      // predicate values seen while the received flag was still set
		TSet<uint64> ReceivedOnly;   // deadline - newest received tick at the same samples
		uint64 FirstWithinTen = 0;   // estimated tick when "target_flag_remaining <= 10" first held
		uint64 LastSetTickBeforeClear = 0;
		bool bZeroBeforeDeadline = false;
	};

	bool Load(FAutomationTestBase& T, const FString& Name, FString& Observer, FString& Victim, TArray<FFixtureFrame>& Frames)
	{
		const FString Path = FPaths::Combine(FPaths::ProjectDir(), TEXT("Scenarios/fixtures"), Name);
		FString Text; TSharedPtr<FJsonObject> Json;
		if (!T.TestTrue(TEXT("fixture readable: ") + Path, FFileHelper::LoadFileToString(Text, *Path))) return false;
		auto Reader = TJsonReaderFactory<>::Create(Text);
		if (!T.TestTrue(TEXT("fixture JSON"), FJsonSerializer::Deserialize(Reader, Json) && Json.IsValid())) return false;
		if (!T.TestEqual(TEXT("server-produced ServerMessage frames"), Json->GetStringField(TEXT("format")), FString(TEXT("nightfall.v1.ServerMessage/base64")))) return false;
		Observer = Json->GetStringField(TEXT("observer")); Victim = Json->GetStringField(TEXT("victim"));
		for (const auto& V : Json->GetArrayField(TEXT("frames")))
		{
			const auto& O = V->AsObject(); FFixtureFrame F; TArray<uint8> Bytes;
			F.Tick = uint64(O->GetNumberField(TEXT("tick"))); F.ServerTimeMs = int64(O->GetNumberField(TEXT("server_time_ms")));
			if (!T.TestTrue(TEXT("frame base64"), FBase64::Decode(O->GetStringField(TEXT("b64")), Bytes))
				|| !T.TestTrue(TEXT("server frame decodes"), NightfallProto::Decode(Bytes.GetData(), Bytes.Num(), F.Message))) return false;
			Frames.Add(MoveTemp(F));
		}
		return T.TestTrue(TEXT("frames present"), Frames.Num() > 100);
	}

	/**
	 * Delivers the observer's recorded frames at their recorded server time plus a fixed delay,
	 * samples the bot predicate every 20 ms of the local clock, and collects, per victim flag
	 * window, what `target_flag_remaining` read while the received flag was still set.
	 */
	bool Run(FAutomationTestBase& T, const FString& Name, TArray<FWindowResult>& Windows)
	{
		FString Observer, Victim; TArray<FFixtureFrame> Frames;
		if (!Load(T, Name, Observer, Victim, Frames)) return false;
		FScopedTestGameInstance Scope;
		auto* N = Scope.Get<UNetClientSubsystem>(); auto* S = Scope.Get<USkillStateSubsystem>(); auto* Combat = Scope.Get<UCombatStateSubsystem>();
		N->SetOwnEntityId(Observer);
		double Now = 0.0; N->SetMonotonicClockForTesting([&Now]() { return Now; });
		FBotPredicateRegistry R; R.RegisterBuiltins(); FBotContext C; C.GameInstance = Scope.GameInstance; FString Error;
		const auto Remaining = R.Parse(FBotPredicateRegistry::Tokenize(TEXT("target_flag_remaining <= 10")), Error);
		const auto FlagNone = R.Parse(FBotPredicateRegistry::Tokenize(TEXT("target_flag == none")), Error);
		if (!T.TestTrue(TEXT("predicates parse"), Remaining && FlagNone)) return false;
		const int64 Origin = Frames[0].ServerTimeMs; const double Delay = 0.035;
		auto ArrivalOf = [&](const FFixtureFrame& F) { return double(F.ServerTimeMs - Origin) / 1000.0 + Delay; };
		FWindowResult* Open = nullptr; int32 Samples = 0;
		for (int32 I = 0; I < Frames.Num(); ++I)
		{
			Now = ArrivalOf(Frames[I]);
			N->DispatchServerMessage(Frames[I].Message);
			if (I + 1 < Frames.Num() && Frames[I + 1].Tick == Frames[I].Tick) continue;   // the rest of this tick's frames arrive together
			const uint64 NextTick = I + 1 < Frames.Num() ? Frames[I + 1].Tick : MAX_uint64;
			const double NextArrival = I + 1 < Frames.Num() ? ArrivalOf(Frames[I + 1]) : Now + 0.5;
			for (double At = Now; At < NextArrival - 1e-6; At += 0.02)
			{
				Now = At; ++Samples;
				const uint64 Estimated = N->GetEstimatedTick();
				if (!T.TestTrue(FString::Printf(TEXT("%s: estimate %llu never ahead of the next received tick %llu"), *Name, Estimated, NextTick), Estimated <= NextTick)) return false;
				if (!T.TestTrue(TEXT("estimate never behind the newest received tick"), Estimated >= N->GetLatestReceivedTick())) return false;
				const auto* P = S->FindPvp(Victim);
				const bool Targeted = Combat->GetOwn().TargetId.Equals(Victim, ESearchCase::IgnoreCase);
				const bool Set = P && !P->Karma && P->FlagUntilTick > S->GetTick();   // the received state still shows the flag
				if (!Targeted || !Set) { Open = nullptr; continue; }
				if (!Open || Open->Deadline != P->FlagUntilTick) { Windows.AddDefaulted(); Open = &Windows.Last(); Open->Deadline = P->FlagUntilTick; }
				const FBotPredicateValue V = Remaining(C);
				const uint64 Value = FCString::Strtoui64(*V.Observed, nullptr, 10);
				T.TestEqual(TEXT("predicate reads deadline - estimated tick"), Value, P->FlagUntilTick > S->GetEstimatedTick() ? P->FlagUntilTick - S->GetEstimatedTick() : uint64(0));
				Open->Estimated.Add(Value); Open->ReceivedOnly.Add(P->FlagUntilTick - S->GetTick());
				if (V.bTrue && !Open->FirstWithinTen) Open->FirstWithinTen = S->GetEstimatedTick();
				if (Value == 0 && S->GetEstimatedTick() < P->FlagUntilTick) Open->bZeroBeforeDeadline = true;
				if (FlagNone(C).bTrue != (S->GetEstimatedTick() >= P->FlagUntilTick)) { T.AddError(TEXT("target_flag == none must follow the estimated tick")); return false; }
				Open->LastSetTickBeforeClear = N->GetLatestReceivedTick();
			}
		}
		UE_LOG(LogTemp, Display, TEXT("D132 %s: %d frames, %d samples, %d flag windows on the target"), *Name, Frames.Num(), Samples, Windows.Num());
		return T.TestTrue(TEXT("target flag windows witnessed"), !Windows.IsEmpty());
	}

	/** The scenario's window (3-pvp-flag-a L77): the last full flag window on the victim. */
	const FWindowResult* LongWindow(const TArray<FWindowResult>& Windows)
	{
		const FWindowResult* Found = nullptr;
		for (const auto& W : Windows) if (W.Estimated.Contains(11)) Found = &W;
		return Found;
	}

	void CheckWindow(FAutomationTestBase& T, const FString& Name, const FWindowResult& W)
	{
		UE_LOG(LogTemp, Display, TEXT("D132 %s: deadline %llu, newest received tick before the clear %llu, first <=10 at estimated tick %llu"), *Name, W.Deadline, W.LastSetTickBeforeClear, W.FirstWithinTen);
		for (uint64 V = 10; V >= 1; --V) T.TestTrue(FString::Printf(TEXT("%s: target_flag_remaining read %llu before the received clear"), *Name, V), W.Estimated.Contains(V));
		T.TestEqual(FString::Printf(TEXT("%s: <= 10 first holds 10 ticks before the deadline"), *Name), W.FirstWithinTen, W.Deadline - 10);
		T.TestFalse(FString::Printf(TEXT("%s: never 0 before the deadline"), *Name), W.bZeroBeforeDeadline);
	}
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FTickEstimatorPvpFlagV6Test, "Nightfall.Bot.TargetFlagRemaining.V6Recording", TickEstimatorTests::Flags)
bool FTickEstimatorPvpFlagV6Test::RunTest(const FString& Parameters)
{
	using namespace TickEstimatorTests;
	TArray<FWindowResult> Windows;
	if (!Run(*this, TEXT("t22-pvp-flag-v6-observer.json"), Windows)) return false;
	const FWindowResult* W = LongWindow(Windows);
	if (!TestNotNull(TEXT("V6: the L77 flag window"), W)) return false;
	// The recording reproduces T22: the newest received tick stays more than 10 ticks before the
	// deadline until the clear, so the received-only read jumps over the 1..10 window.
	TestTrue(TEXT("V6: newest received tick before the clear is > 10 ticks before the deadline"), W->LastSetTickBeforeClear + 10 < W->Deadline);
	bool bReceivedOnlyInWindow = false;
	for (uint64 V = 1; V <= 10; ++V) bReceivedOnlyInWindow |= W->ReceivedOnly.Contains(V);
	TestFalse(TEXT("V6: received-only read never saw 1..10 (the T22 symptom)"), bReceivedOnlyInWindow);
	CheckWindow(*this, TEXT("V6"), *W);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FTickEstimatorPvpFlagV5Test, "Nightfall.Bot.TargetFlagRemaining.V5Recording", TickEstimatorTests::Flags)
bool FTickEstimatorPvpFlagV5Test::RunTest(const FString& Parameters)
{
	using namespace TickEstimatorTests;
	TArray<FWindowResult> Windows;
	if (!Run(*this, TEXT("t22-pvp-flag-v5-observer.json"), Windows)) return false;
	const FWindowResult* W = LongWindow(Windows);
	if (!TestNotNull(TEXT("V5: the L77 flag window"), W)) return false;
	CheckWindow(*this, TEXT("V5"), *W);
	return true;
}
#endif
