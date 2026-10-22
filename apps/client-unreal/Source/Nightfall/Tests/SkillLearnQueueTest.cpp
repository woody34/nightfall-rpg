#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Skills/SkillStateSubsystem.h"
#include "Net/NetClientSubsystem.h"

#if WITH_DEV_AUTOMATION_TESTS

// D160 / T47: a learn issued while a LearnSkill RPC is in flight is queued (FIFO, bounded), not dropped.
// It goes out unchanged after the current RPC completes; a same-key retry then compares against the
// first frozen receipt, a different key starts a new receipt history. Offline: the RPC is a captured
// provider; replies are delivered by the test.

namespace
{
	constexpr EAutomationTestFlags LearnQueueFlags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;
	const TCHAR* const KeyA = TEXT("01970000-0000-7000-8000-00000000004a");
	const TCHAR* const KeyB = TEXT("01970000-0000-7000-8000-00000000004b");

	struct FCapturedLearn
	{
		FGrpcNightfallV1LearnSkillRequest Request;
		USessionClient::FLearnCallback Callback;
	};

	struct FLearnRig
	{
		FScopedTestGameInstance Scope;
		UNetClientSubsystem* Net = Scope.Get<UNetClientSubsystem>();
		USkillStateSubsystem* Skills = Scope.Get<USkillStateSubsystem>();
		TArray<FCapturedLearn> Sent;
		TArray<FNetResult> Completed;

		FLearnRig()
		{
			Net->SetOwnEntityId(TEXT("own"));
			Skills->SetLearnProviderForTesting([this](const FGrpcNightfallV1LearnSkillRequest& Request, USessionClient::FLearnCallback Callback)
			{
				Sent.Add(FCapturedLearn{ Request, MoveTemp(Callback) });
			});
		}

		USkillStateSubsystem::FDone Done() { return [this](const FNetResult& R) { Completed.Add(R); }; }

		void Reply(int32 Index, uint32 SkillId, uint32 Rank, uint64 SpRemaining)
		{
			FGrpcNightfallV1LearnSkillResponse Receipt;
			Receipt.SkillId = SkillId; Receipt.SkillLevel = Rank; Receipt.SpRemaining = SpRemaining; Receipt.Tick = 11; Receipt.Hp = 99; Receipt.Incarnation = 1;
			Sent[Index].Callback(FNetResult(), Receipt);
		}
	};
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillLearnQueueTest, "Nightfall.Skill.LearnQueuedWhileBusy", LearnQueueFlags)

bool FSkillLearnQueueTest::RunTest(const FString& Parameters)
{
	FLearnRig Rig;
	USkillStateSubsystem* S = Rig.Skills;

	// L30: the keyed learn goes out at once.
	S->Learn(1405, 1, Rig.Done(), KeyA);
	TestEqual(TEXT("first learn sent"), Rig.Sent.Num(), 1);
	TestTrue(TEXT("busy while in flight"), S->IsBusy());
	TestEqual(TEXT("nothing queued"), S->QueuedLearns(), 0);

	// L33 fires before the reply lands: queued, not dropped.
	S->Learn(1405, 1, Rig.Done(), KeyA);
	TestEqual(TEXT("same-key retry is not sent while busy"), Rig.Sent.Num(), 1);
	TestEqual(TEXT("same-key retry queued"), S->QueuedLearns(), 1);
	// A different-key learn behind it is kept too, in order.
	S->Learn(1406, 1, Rig.Done(), KeyB);
	TestEqual(TEXT("different-key learn queued behind"), S->QueuedLearns(), 2);
	TestTrue(TEXT("still busy"), S->IsBusy());
	TestFalse(TEXT("no learn result while requests are pending"), S->HasLearnResult());

	// The first reply completes L30 and sends the queued retry unchanged.
	Rig.Reply(0, 1405, 1, 400);
	TestEqual(TEXT("first completion delivered"), Rig.Completed.Num(), 1);
	TestEqual(TEXT("queued retry sent after completion"), Rig.Sent.Num(), 2);
	TestEqual(TEXT("retry keeps its idempotency key"), Rig.Sent[1].Request.IdempotencyKey, FString(KeyA));
	TestEqual(TEXT("retry keeps its skill"), Rig.Sent[1].Request.SkillId.Value, 1405u);
	TestEqual(TEXT("retry keeps its rank"), Rig.Sent[1].Request.SkillLevel.Value, 1u);
	TestEqual(TEXT("one learn still queued"), S->QueuedLearns(), 1);
	TestTrue(TEXT("busy with the retry"), S->IsBusy());
	TestFalse(TEXT("learn result hidden while the retry is in flight"), S->HasLearnResult());
	TestFalse(TEXT("no comparison after a single receipt"), S->ReceiptMatchesFirst().IsSet());

	// The server replays the frozen receipt: learn_receipt_matches_first becomes true.
	Rig.Reply(1, 1405, 1, 400);
	TestEqual(TEXT("retry completion delivered"), Rig.Completed.Num(), 2);
	TestTrue(TEXT("same key: receipt matches the first"), S->ReceiptMatchesFirst().IsSet() && *S->ReceiptMatchesFirst());
	TestEqual(TEXT("different-key learn sent next"), Rig.Sent.Num(), 3);
	TestEqual(TEXT("different key preserved"), Rig.Sent[2].Request.IdempotencyKey, FString(KeyB));
	TestEqual(TEXT("different skill preserved"), Rig.Sent[2].Request.SkillId.Value, 1406u);
	TestEqual(TEXT("queue drained"), S->QueuedLearns(), 0);
	TestTrue(TEXT("busy with the last learn"), S->IsBusy());

	// A different key is a new learn: a first receipt, no comparison, then the subsystem is idle.
	Rig.Reply(2, 1406, 1, 300);
	TestEqual(TEXT("all three completions delivered once each"), Rig.Completed.Num(), 3);
	for (const FNetResult& R : Rig.Completed) TestTrue(TEXT("each completion ok"), R.IsOk());
	TestFalse(TEXT("new key: no comparison yet"), S->ReceiptMatchesFirst().IsSet());
	TestEqual(TEXT("latest receipt is the new learn"), S->GetLearnReceipt().SkillId.Value, 1406u);
	TestFalse(TEXT("idle after the queue drained"), S->IsBusy());
	TestTrue(TEXT("learn result visible when nothing is pending"), S->HasLearnResult());
	TestEqual(TEXT("last key is the new one"), S->GetLastLearnKey(), FString(KeyB));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillLearnQueueBoundsTest, "Nightfall.Skill.LearnQueueBoundedAndResetClears", LearnQueueFlags)

bool FSkillLearnQueueBoundsTest::RunTest(const FString& Parameters)
{
	{
		FLearnRig Rig;
		USkillStateSubsystem* S = Rig.Skills;
		S->Learn(1405, 1, Rig.Done(), KeyA);
		for (int32 I = 0; I < USkillStateSubsystem::MaxQueuedLearns; ++I) S->Learn(1405, 1, Rig.Done(), KeyA);
		TestEqual(TEXT("queue holds the bound"), S->QueuedLearns(), USkillStateSubsystem::MaxQueuedLearns);
		TestEqual(TEXT("nothing refused yet"), Rig.Completed.Num(), 0);
		S->Learn(1405, 1, Rig.Done(), KeyA);
		TestEqual(TEXT("queue does not grow past the bound"), S->QueuedLearns(), USkillStateSubsystem::MaxQueuedLearns);
		if (TestEqual(TEXT("the overflow is refused at once"), Rig.Completed.Num(), 1))
		{
			TestTrue(TEXT("refused as resource exhausted"), Rig.Completed[0].Error == ENetError::ResourceExhausted);
		}
		TestEqual(TEXT("only the in-flight learn was sent"), Rig.Sent.Num(), 1);

		// Every queued request goes out, one per completion, in order.
		for (int32 I = 0; I < USkillStateSubsystem::MaxQueuedLearns; ++I)
		{
			Rig.Reply(I, 1405, 1, 400);
			TestEqual(TEXT("next queued learn sent"), Rig.Sent.Num(), I + 2);
		}
		Rig.Reply(USkillStateSubsystem::MaxQueuedLearns, 1405, 1, 400);
		TestEqual(TEXT("all learns completed plus the one refusal"), Rig.Completed.Num(), USkillStateSubsystem::MaxQueuedLearns + 2);
		TestFalse(TEXT("idle"), S->IsBusy());
	}
	{
		// Logout / reset discards queued learns and the stale completion sends nothing.
		FLearnRig Rig;
		USkillStateSubsystem* S = Rig.Skills;
		S->Learn(1405, 1, Rig.Done(), KeyA);
		S->Learn(1406, 1, Rig.Done(), KeyB);
		TestEqual(TEXT("one queued before reset"), S->QueuedLearns(), 1);
		S->Reset();
		TestEqual(TEXT("reset clears the queue"), S->QueuedLearns(), 0);
		TestFalse(TEXT("reset clears busy"), S->IsBusy());
		Rig.Reply(0, 1405, 1, 400);
		TestEqual(TEXT("stale completion sends no queued learn"), Rig.Sent.Num(), 1);
		TestEqual(TEXT("stale completion is discarded"), Rig.Completed.Num(), 0);
		TestFalse(TEXT("no learn result after reset"), S->HasLearnResult());
	}
	return true;
}

#endif
