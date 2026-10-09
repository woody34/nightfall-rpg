#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "IWebSocket.h"
#include "Nightfall.h"
#include "Auth/AuthSubsystem.h"
#include "Bot/BotLogSentinel.h"
#include "Bot/BotPredicates.h"
#include "Bot/BotScenarioRunner.h"
#include "Bot/BotSteps.h"
#include "Combat/CombatStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "HAL/IConsoleManager.h"
#include "XmlFile.h"

#if WITH_DEV_AUTOMATION_TESTS

// Phase 1a E1.1-E1.3 without a server: the scenario parser, the step executor on a fake clock,
// the JUnit shape, the log/ensure sentinel with synthetic lines, and every built-in predicate
// driven false -> true by typed events fed through UNetClientSubsystem::DispatchServerMessage.

namespace
{
	constexpr EAutomationTestFlags BotTestFlags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;

	const TCHAR* const OwnId = TEXT("0b6e2f6e-0000-4000-8000-000000000001");
	const TCHAR* const Wolf = TEXT("0b6e2f6e-0000-4000-8000-0000000000a1");
	const TCHAR* const Boar = TEXT("0b6e2f6e-0000-4000-8000-0000000000a2");

	/** A registry with switchable test predicates and the built-ins. */
	struct FTestPredicates
	{
		FBotPredicateRegistry Registry;
		bool bReady = false;
		bool bNever = false;
		int32 Count = 0;

		FTestPredicates()
		{
			Registry.RegisterBuiltins();
			Registry.RegisterFlag(TEXT("ready"), TEXT("test"), [this](const FBotContext&) { return bReady; });
			Registry.RegisterFlag(TEXT("never"), TEXT("test"), [this](const FBotContext&) { return bNever; });
			Registry.RegisterNumber(TEXT("count"), TEXT("test"), [this](const FBotContext&) { return TOptional<double>(Count); });
		}
	};

	const BotScenario::FCommandExists KnownCommands = [](const FString& Name)
	{
		return Name.Equals(TEXT("nf.Login"), ESearchCase::IgnoreCase) || Name.Equals(TEXT("nf.EnterWorld"), ESearchCase::IgnoreCase) || Name.Equals(TEXT("nf.Attack"), ESearchCase::IgnoreCase);
	};

	/** An executor on a fake clock that records every command it runs. */
	struct FExecRig
	{
		FTestPredicates Predicates;
		TArray<FString> Ran;
		TArray<FString> Trace;
		TSet<FString> FailingCommands;
		TUniquePtr<FBotScenarioExecutor> Executor;
		double Now = 100.0;

		bool Load(FAutomationTestBase& Test, const FString& Text)
		{
			FBotScenario Scenario;
			TArray<FString> Errors;
			if (!Test.TestTrue(FString::Printf(TEXT("parses: %s"), *FString::Join(Errors, TEXT("; "))), BotScenario::Parse(TEXT("test"), Text, Predicates.Registry, KnownCommands, Scenario, Errors))) return false;
			Executor = MakeUnique<FBotScenarioExecutor>(MoveTemp(Scenario),
				[this](const FString& Line, FString& OutError)
				{
					Ran.Add(Line);
					if (FailingCommands.Contains(Line)) { OutError = TEXT("refused"); return false; }
					return true;
				},
				[](const FBotPredicateFn& Fn) { return Fn(FBotContext()); },
				[this](const FString& Line) { Trace.Add(Line); });
			Executor->Start(Now);
			return true;
		}

		bool Tick(double Advance = 0.1)
		{
			Now += Advance;
			return Executor->Tick(Now);
		}

		/** Ticks until the run ends (at most N ticks); returns the number of ticks. */
		int32 RunToEnd(int32 MaxTicks = 10000, double Step = 0.1)
		{
			int32 N = 0;
			while (N < MaxTicks && Tick(Step)) ++N;
			return N;
		}

		const FBotTestCase* Case(int32 Line) const
		{
			return Executor->GetTestCases().FindByPredicate([Line](const FBotTestCase& C) { return C.Line == Line; });
		}
	};

	/** Records every frame the client sends (as in CombatStateTest). */
	class FRecordingSocket final : public IWebSocket
	{
	public:
		TArray<TArray<uint8>> Sent;
		virtual void Connect() override {}
		virtual void Close(int32 Code, const FString& Reason) override {}
		virtual bool IsConnected() override { return true; }
		virtual void Send(const FString& Data) override {}
		virtual void Send(const void* Data, SIZE_T Size, bool bIsBinary) override { Sent.Emplace_GetRef().Append(static_cast<const uint8*>(Data), static_cast<int32>(Size)); }
		virtual void SetTextMessageMemoryLimit(uint64 Limit) override {}
		virtual FWebSocketConnectedEvent& OnConnected() override { return Connected; }
		virtual FWebSocketConnectionErrorEvent& OnConnectionError() override { return ConnectionError; }
		virtual FWebSocketClosedEvent& OnClosed() override { return Closed; }
		virtual FWebSocketMessageEvent& OnMessage() override { return Message; }
		virtual FWebSocketBinaryMessageEvent& OnBinaryMessage() override { return BinaryMessage; }
		virtual FWebSocketRawMessageEvent& OnRawMessage() override { return RawMessage; }
		virtual FWebSocketMessageSentEvent& OnMessageSent() override { return MessageSent; }
		/** Intent field number of frame I (seq is one byte here): 12 SetTarget, 13 Attack, 14 StopAttack, 15 Respawn. */
		int32 IntentField(int32 I) const { return Sent.IsValidIndex(I) && Sent[I].Num() > 2 ? Sent[I][2] >> 3 : 0; }

		FWebSocketConnectedEvent Connected;
		FWebSocketConnectionErrorEvent ConnectionError;
		FWebSocketClosedEvent Closed;
		FWebSocketMessageEvent Message;
		FWebSocketBinaryMessageEvent BinaryMessage;
		FWebSocketRawMessageEvent RawMessage;
		FWebSocketMessageSentEvent MessageSent;
	};

	/** A game instance with a recording socket, bound observations and an event feed (Story 5.2 fixture shapes). */
	struct FWorldRig
	{
		FScopedTestGameInstance Instance;
		UNetClientSubsystem* Net = Instance.Get<UNetClientSubsystem>();
		UCombatStateSubsystem* Combat = Instance.Get<UCombatStateSubsystem>();
		TSharedPtr<FRecordingSocket> Socket;
		FBotObservations Observations;

		FWorldRig()
		{
			Net->SetSocketFactoryForTesting([this](const FWsUpgradeRequest&) -> TSharedRef<IWebSocket>
			{
				Socket = MakeShared<FRecordingSocket>();
				return Socket.ToSharedRef();
			});
			Net->SetSchedulerForTesting([](float, TFunction<void()>) {});
			Observations.Bind(Instance.GameInstance);
		}
		~FWorldRig() { Observations.Unbind(); }

		void Connect()
		{
			Net->Connect(TEXT("ws://localhost:3000/ws"), TEXT("ticket"));
			Socket->Connected.Broadcast();
			Net->SetOwnEntityId(OwnId);
		}

		FBotPredicateValue Eval(const FString& Text)
		{
			Observations.Observe(Combat);
			FString Error;
			const FBotPredicateFn Fn = FBotPredicateRegistry::Get().Parse(FBotPredicateRegistry::Tokenize(Text), Error);
			if (!Fn) return { false, TEXT("PARSE ERROR: ") + Error };
			return Fn(FBotContext{ Instance.GameInstance, &Observations });
		}

		void Event(const FWorldEvent& E) { FServerMessage M; M.Event = E; Net->DispatchServerMessage(M); }
		void Ack(uint32 Seq) { FServerMessage M; M.Ack = FAck{ Seq, 1 }; Net->DispatchServerMessage(M); }
		void Reject(uint32 Seq, uint32 Reason) { FServerMessage M; M.Rejected = FIntentRejected{ Seq, Reason, TEXT("") }; Net->DispatchServerMessage(M); }
		void Spawn(const TCHAR* Id, const TCHAR* Name, uint32 Kind, uint32 Inc, uint32 Hp, uint32 MaxHp, uint32 Level, bool bAttackable, FNetVec2 Pos = FNetVec2())
		{
			FEntitySpawn S;
			S.EntityId = Id; S.Name = Name; S.Kind = Kind; S.SessionGeneration = 1; S.Position = Pos;
			S.bCombatant = true; S.LifeIncarnation = Inc; S.bAttackable = bAttackable; S.Hp = Hp; S.MaxHp = MaxHp; S.Level = Level;
			FWorldEvent E; E.Spawn = S; Event(E);
		}
		void Move(const TCHAR* Id, FNetVec2 Pos, int64 TimeMs)
		{
			FWorldEvent E; E.Move = FEntityMove{ Id, Pos, Pos, 0.f, TimeMs, 1 }; Event(E);
		}
		void Stats(uint32 Hp, uint32 MaxHp, uint32 Level, uint64 Xp) { FWorldEvent E; E.StatsChanged = FStatsChanged{ OwnId, Hp, MaxHp, 50, 50, Level, Xp }; Event(E); }
		void Xp(uint64 Amount, uint64 Total) { FWorldEvent E; E.XpGained = FXpGained{ OwnId, Amount, Total }; Event(E); }
		void Target(const TCHAR* Target) { FWorldEvent E; E.TargetChanged = FTargetChanged{ OwnId, Target }; Event(E); }
		void Hit(const TCHAR* Attacker, const TCHAR* Target, uint64 Tick, uint32 Damage, uint32 HpAfter, uint32 Inc)
		{
			FWorldEvent E; E.AttackResult = FAttackResult{ Attacker, Target, Tick, ENetAttackOutcome::Hit, Damage, HpAfter, Inc }; Event(E);
		}
		void Died(const TCHAR* Id, uint64 Tick, uint32 Inc) { FWorldEvent E; E.EntityDied = FEntityDied{ Id, Tick, TEXT(""), Inc }; Event(E); }
	};
}

// --- E1.1 parser -------------------------------------------------------------------------------

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotParserTest, "Nightfall.Bot.Parser", BotTestFlags)

bool FBotParserTest::RunTest(const FString& Parameters)
{
	FTestPredicates P;
	FBotScenario S;
	TArray<FString> Errors;
	const FString Text =
		TEXT("# header comment\n")
		TEXT("\n")
		TEXT("nf.Login            # trailing comment\n")
		TEXT("   nf.WaitFor connected 15\n")
		TEXT("nf.WaitFor proxies >= 1 10\n")
		TEXT("nf.Expect target != none\n")
		TEXT("nf.Sleep 0.5\n")
		TEXT("Log LogNightfall Verbose\n")
		TEXT("nf.Within 90 # budget\n");
	TestTrue(TEXT("valid scenario parses"), BotScenario::Parse(TEXT("demo"), Text, P.Registry, KnownCommands, S, Errors));
	TestEqual(TEXT("no errors"), Errors.Num(), 0);
	TestEqual(TEXT("name"), S.Name, FString(TEXT("demo")));
	if (TestEqual(TEXT("comments and blank lines dropped: 7 steps"), S.Steps.Num(), 7))
	{
		TestEqual(TEXT("line numbers are the file's"), S.Steps[0].Line, 3);
		TestEqual(TEXT("comment stripped"), S.Steps[0].Source, FString(TEXT("nf.Login")));
		TestTrue(TEXT("command"), S.Steps[0].Kind == EBotStepKind::Command);
		TestTrue(TEXT("waitfor"), S.Steps[1].Kind == EBotStepKind::WaitFor);
		TestEqual(TEXT("waitfor timeout is the last token"), S.Steps[1].Seconds, 15.0);
		TestEqual(TEXT("predicate text without the timeout"), S.Steps[2].PredicateText, FString(TEXT("proxies >= 1")));
		TestTrue(TEXT("expect"), S.Steps[3].Kind == EBotStepKind::Expect);
		TestTrue(TEXT("sleep"), S.Steps[4].Kind == EBotStepKind::Sleep && S.Steps[4].Seconds == 0.5);
		TestTrue(TEXT("non-nf lines pass through as commands"), S.Steps[5].Kind == EBotStepKind::Command);
		TestTrue(TEXT("within"), S.Steps[6].Kind == EBotStepKind::Within);
	}
	TestEqual(TEXT("budget"), S.BudgetSeconds, 90.0);
	TestEqual(TEXT("budget line"), S.BudgetLine, 9);

	// Every error is found at parse time, with its line.
	struct FBad { const TCHAR* Line; const TCHAR* Needle; };
	const FBad Bad[] = {
		{ TEXT("nf.WaitFor no_such_thing 5"), TEXT("unknown predicate 'no_such_thing'") },
		{ TEXT("nf.Expect no_such_thing"), TEXT("unknown predicate") },
		{ TEXT("nf.WaitFor connected"), TEXT("timeout") },
		{ TEXT("nf.WaitFor connected -1"), TEXT("timeout") },
		{ TEXT("nf.WaitFor proxies => 1 5"), TEXT("bad operator") },
		{ TEXT("nf.WaitFor proxies >= many 5"), TEXT("not a number") },
		{ TEXT("nf.Expect connected yes"), TEXT("takes no arguments") },
		{ TEXT("nf.Expect attack_state == running"), TEXT("not idle, pending or active") },
		{ TEXT("nf.Expect rejected == SOMETHING_ELSE"), TEXT("unknown reject reason") },
		{ TEXT("nf.Expect own_at 1 2"), TEXT("three numbers") },
		{ TEXT("nf.Teleport 1 2"), TEXT("unknown command 'nf.Teleport'") },
		{ TEXT("nf.Sleep soon"), TEXT("nf.Sleep") },
		{ TEXT("nf.Within 0"), TEXT("nf.Within") },
	};
	for (const FBad& B : Bad)
	{
		Errors.Reset();
		const bool bOk = BotScenario::Parse(TEXT("bad"), FString::Printf(TEXT("nf.Login\n%s\n"), B.Line), P.Registry, KnownCommands, S, Errors);
		TestFalse(FString::Printf(TEXT("rejected: %s"), B.Line), bOk);
		const FString All = FString::Join(Errors, TEXT("; "));
		TestTrue(FString::Printf(TEXT("'%s' names the problem (%s)"), B.Line, *All), All.Contains(B.Needle) && All.Contains(TEXT("line 2")));
	}
	Errors.Reset();
	TestFalse(TEXT("two nf.Within"), BotScenario::Parse(TEXT("bad"), TEXT("nf.Within 5\nnf.Within 6\n"), P.Registry, KnownCommands, S, Errors));
	TestTrue(TEXT("second nf.Within named"), Errors.Num() == 1 && Errors[0].Contains(TEXT("first is on line 1")));
	Errors.Reset();
	TestFalse(TEXT("empty scenario"), BotScenario::Parse(TEXT("bad"), TEXT("# nothing\n\n"), P.Registry, KnownCommands, S, Errors));
	Errors.Reset();
	TestFalse(TEXT("all errors collected, not just the first"), BotScenario::Parse(TEXT("bad"), TEXT("nf.Expect nope\nnf.Expect nada\n"), P.Registry, KnownCommands, S, Errors));
	TestEqual(TEXT("two errors"), Errors.Num(), 2);
	TestEqual(TEXT("StripComment"), BotScenario::StripComment(TEXT("  nf.Attack   # go ")), FString(TEXT("nf.Attack")));
	return true;
}

// --- E1.1 executor -----------------------------------------------------------------------------

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotExecutorOrderTest, "Nightfall.Bot.Executor.StepOrder", BotTestFlags)

bool FBotExecutorOrderTest::RunTest(const FString& Parameters)
{
	FExecRig Rig;
	if (!Rig.Load(*this, TEXT("nf.Login\nnf.WaitFor ready 5\nnf.EnterWorld\nnf.Expect count == 0\nnf.Sleep 1\nnf.Attack\nnf.WaitFor count >= 2 5\nnf.Expect ready\n"))) return false;

	TestTrue(TEXT("tick 1: still running"), Rig.Tick());
	TestEqual(TEXT("tick 1 runs the first command, then waits"), Rig.Ran, TArray<FString>({ TEXT("nf.Login") }));
	TestTrue(TEXT("tick 2: still waiting"), Rig.Tick());
	TestEqual(TEXT("nothing more while the wait is false"), Rig.Ran.Num(), 1);

	Rig.Predicates.bReady = true;
	TestTrue(TEXT("tick 3: wait passes"), Rig.Tick());
	TestEqual(TEXT("the step after a passed WaitFor runs on the next tick"), Rig.Ran.Num(), 1);
	TestTrue(TEXT("tick 4"), Rig.Tick());
	TestEqual(TEXT("EnterWorld ran, then the Expect, then the sleep holds"), Rig.Ran, TArray<FString>({ TEXT("nf.Login"), TEXT("nf.EnterWorld") }));
	Rig.Tick(0.5);
	TestEqual(TEXT("sleep 1 s not over after 0.5 s"), Rig.Ran.Num(), 2);
	Rig.Tick(0.6);
	TestEqual(TEXT("after the sleep: nf.Attack"), Rig.Ran.Last(), FString(TEXT("nf.Attack")));
	Rig.Predicates.Count = 2;
	Rig.RunToEnd();
	TestTrue(TEXT("finished"), Rig.Executor->IsFinished());
	TestTrue(TEXT("passed"), Rig.Executor->HasPassed());
	TestEqual(TEXT("exit code 0"), UBotScenarioRunner::ExitCodeFor(Rig.Executor->HasPassed(), 0), 0);
	TestEqual(TEXT("each command once, in order"), Rig.Ran, TArray<FString>({ TEXT("nf.Login"), TEXT("nf.EnterWorld"), TEXT("nf.Attack") }));

	const TArray<FBotTestCase>& Cases = Rig.Executor->GetTestCases();
	if (TestEqual(TEXT("one testcase per WaitFor / Expect"), Cases.Num(), 4))
	{
		for (const FBotTestCase& C : Cases) TestTrue(FString::Printf(TEXT("%s passed"), *C.Name), C.Status == FBotTestCase::EStatus::Passed);
		TestEqual(TEXT("testcase named after its line"), Cases[0].Name, FString(TEXT("L2 nf.WaitFor ready 5")));
		TestTrue(TEXT("WaitFor time is the wait"), FMath::IsNearlyEqual(Cases[0].Seconds, 0.2, 1e-6));
	}
	TestTrue(TEXT("trace names the steps"), Rig.Trace.ContainsByPredicate([](const FString& L) { return L.Contains(TEXT("L3 > nf.EnterWorld")); }));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotExecutorTimeoutTest, "Nightfall.Bot.Executor.TimeoutStopsTheRun", BotTestFlags)

bool FBotExecutorTimeoutTest::RunTest(const FString& Parameters)
{
	FExecRig Rig;
	if (!Rig.Load(*this, TEXT("nf.Login\nnf.WaitFor never 2\nnf.EnterWorld\nnf.Expect ready\n"))) return false;
	Rig.Tick(0.1);
	int32 Ticks = 0;
	while (Rig.Tick(0.25)) ++Ticks;
	TestTrue(TEXT("fails at the timeout, not before (2 s / 0.25 s ticks)"), Ticks >= 7 && Ticks <= 8);
	TestTrue(TEXT("finished"), Rig.Executor->IsFinished());
	TestFalse(TEXT("failed"), Rig.Executor->HasPassed());
	TestEqual(TEXT("exit code 1"), UBotScenarioRunner::ExitCodeFor(Rig.Executor->HasPassed(), 0), 1);
	TestEqual(TEXT("first failure stops: nf.EnterWorld never runs"), Rig.Ran, TArray<FString>({ TEXT("nf.Login") }));
	const FBotTestCase* Wait = Rig.Case(2);
	const FBotTestCase* Later = Rig.Case(4);
	if (TestNotNull(TEXT("wait case"), Wait) && TestNotNull(TEXT("later case"), Later))
	{
		TestTrue(TEXT("the wait failed"), Wait->Status == FBotTestCase::EStatus::Failed);
		TestTrue(TEXT("message: timed out, with the last observation"), Wait->Message.Contains(TEXT("timed out after")) && Wait->Message.Contains(TEXT("last observed false")));
		TestTrue(TEXT("its time is the timeout"), Wait->Seconds >= 2.0 && Wait->Seconds < 2.3);
		TestTrue(TEXT("the later Expect is skipped"), Later->Status == FBotTestCase::EStatus::Skipped);
		TestTrue(TEXT("...naming the failed line"), Later->Message.Contains(TEXT("line 2 failed")));
	}
	TestTrue(TEXT("failure summary names the line"), Rig.Executor->GetFailure().StartsWith(TEXT("L2 nf.WaitFor never 2")));
	TestFalse(TEXT("no more ticks after the end"), Rig.Tick());

	// A failing Expect and a refused command stop the run the same way.
	FExecRig Expect;
	if (!Expect.Load(*this, TEXT("nf.Expect ready\nnf.Login\n"))) return false;
	Expect.RunToEnd();
	TestTrue(TEXT("failed Expect: Login never ran"), Expect.Ran.IsEmpty());
	TestTrue(TEXT("failed Expect names the observation"), Expect.Case(1) && Expect.Case(1)->Message.Contains(TEXT("observed false")));

	FExecRig Command;
	Command.FailingCommands.Add(TEXT("nf.Login"));
	if (!Command.Load(*this, TEXT("nf.Login\nnf.Expect ready\n"))) return false;
	Command.RunToEnd();
	TestFalse(TEXT("refused command fails the run"), Command.Executor->HasPassed());
	TestTrue(TEXT("...as a failed testcase for the command line"), Command.Case(1) && Command.Case(1)->Status == FBotTestCase::EStatus::Failed && Command.Case(1)->Message.Contains(TEXT("refused")));
	TestTrue(TEXT("...and the Expect is skipped"), Command.Case(2) && Command.Case(2)->Status == FBotTestCase::EStatus::Skipped);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotExecutorBudgetTest, "Nightfall.Bot.Executor.WithinBudget", BotTestFlags)

bool FBotExecutorBudgetTest::RunTest(const FString& Parameters)
{
	// Within the budget: a passed nf.Within testcase with the elapsed time.
	FExecRig Fast;
	if (!Fast.Load(*this, TEXT("nf.Sleep 1\nnf.Within 3\n"))) return false;
	Fast.RunToEnd();
	TestTrue(TEXT("fast run passes"), Fast.Executor->HasPassed());
	const FBotTestCase* Within = Fast.Case(2);
	TestTrue(TEXT("nf.Within is reported, passed"), Within && Within->Status == FBotTestCase::EStatus::Passed && Within->Seconds >= 1.0);

	// Over budget: the run is cut off at the budget even though the wait has time left.
	FExecRig Slow;
	if (!Slow.Load(*this, TEXT("nf.WaitFor never 60\nnf.Within 2\n"))) return false;
	const int32 Ticks = Slow.RunToEnd(10000, 0.5);
	TestTrue(TEXT("cut off near the budget, not at the 60 s timeout"), Ticks <= 5);
	TestFalse(TEXT("over budget fails"), Slow.Executor->HasPassed());
	TestTrue(TEXT("the wait is reported as interrupted"), Slow.Case(1) && Slow.Case(1)->Message.Contains(TEXT("nf.Within")));
	TestTrue(TEXT("nf.Within failed with the elapsed time"), Slow.Case(2) && Slow.Case(2)->Status == FBotTestCase::EStatus::Failed && Slow.Case(2)->Message.Contains(TEXT("budget")));

	// Aborted from outside (engine shutdown): failed, reported.
	FExecRig Aborted;
	if (!Aborted.Load(*this, TEXT("nf.WaitFor never 60\n"))) return false;
	Aborted.Tick();
	Aborted.Executor->Abort(Aborted.Now, TEXT("engine shut down"));
	TestTrue(TEXT("abort finishes the run as failed"), Aborted.Executor->IsFinished() && !Aborted.Executor->HasPassed());
	TestTrue(TEXT("abort reason on the running step"), Aborted.Case(1) && Aborted.Case(1)->Message.Contains(TEXT("engine shut down")));
	return true;
}

// --- E1.1 JUnit --------------------------------------------------------------------------------

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotJUnitTest, "Nightfall.Bot.JUnit.Shape", BotTestFlags)

bool FBotJUnitTest::RunTest(const FString& Parameters)
{
	FExecRig Rig;
	if (!Rig.Load(*this, TEXT("nf.WaitFor ready 5\nnf.Expect count >= 1 # <&>\nnf.Expect ready\nnf.Within 30\n"))) return false;
	Rig.Predicates.bReady = true;
	Rig.RunToEnd();

	FBotLogSentinel Sentinel;
	Sentinel.Serialize(TEXT("synthetic \"quoted\" <error>"), ELogVerbosity::Error, TEXT("LogNightfall"));
	const TArray<FBotTestCase> Cases = UBotScenarioRunner::ReportCases(*Rig.Executor, Sentinel);
	const FString Xml = BotJUnit::Write(TEXT("1-demo"), Cases, 12.5, { { TEXT("scenario_file"), TEXT("/x/1-demo.nfs") }, { TEXT("exit_code"), TEXT("1") } });

	FXmlFile File(Xml, EConstructMethod::ConstructFromBuffer);
	if (!TestTrue(FString::Printf(TEXT("well-formed XML: %s"), *File.GetLastError()), File.IsValid())) return false;
	const FXmlNode* Root = File.GetRootNode();
	TestEqual(TEXT("root"), Root->GetTag(), FString(TEXT("testsuites")));
	const FXmlNode* Suite = Root->FindChildNode(TEXT("testsuite"));
	if (!TestNotNull(TEXT("testsuite"), Suite)) return false;
	TestEqual(TEXT("suite name is the scenario"), Suite->GetAttribute(TEXT("name")), FString(TEXT("1-demo")));
	TestEqual(TEXT("tests = WaitFor + 2 Expects + Within + sentinel"), Suite->GetAttribute(TEXT("tests")), FString(TEXT("5")));
	TestEqual(TEXT("failures: the Expect on count and the sentinel"), Suite->GetAttribute(TEXT("failures")), FString(TEXT("2")));
	TestEqual(TEXT("time"), Suite->GetAttribute(TEXT("time")), FString(TEXT("12.500")));
	const FXmlNode* Properties = Suite->FindChildNode(TEXT("properties"));
	TestTrue(TEXT("properties in order"), Properties && Properties->GetChildrenNodes().Num() == 2 && Properties->GetChildrenNodes()[0]->GetAttribute(TEXT("name")) == TEXT("scenario_file"));

	TArray<const FXmlNode*> TestCases;
	for (const FXmlNode* Child : Suite->GetChildrenNodes()) if (Child->GetTag() == TEXT("testcase")) TestCases.Add(Child);
	if (!TestEqual(TEXT("testcase elements"), TestCases.Num(), 5)) return false;
	TestEqual(TEXT("classname"), TestCases[0]->GetAttribute(TEXT("classname")), FString(TEXT("sim.1-demo")));
	TestEqual(TEXT("first testcase: the WaitFor"), TestCases[0]->GetAttribute(TEXT("name")), FString(TEXT("L1 nf.WaitFor ready 5")));
	TestNull(TEXT("passed testcase has no children"), TestCases[0]->FindChildNode(TEXT("failure")));
	const FXmlNode* Failure = TestCases[1]->FindChildNode(TEXT("failure"));
	TestTrue(TEXT("failed Expect carries <failure message>"), Failure && Failure->GetAttribute(TEXT("message")).Contains(TEXT("observed 0")));
	TestNotNull(TEXT("not reached Expect is <skipped>"), TestCases[2]->FindChildNode(TEXT("skipped")));
	TestTrue(TEXT("nf.Within reported"), TestCases[3]->GetAttribute(TEXT("name")).Contains(TEXT("nf.Within 30")));
	const FXmlNode* Sentinel4 = TestCases[4]->FindChildNode(TEXT("failure"));
	TestEqual(TEXT("last testcase is the sentinel"), TestCases[4]->GetAttribute(TEXT("name")), FString(TEXT("log and ensure sentinel")));
	TestTrue(TEXT("sentinel failure names the line"), Sentinel4 && Sentinel4->GetContent().Contains(TEXT("synthetic")));
	TestTrue(TEXT("...escaped"), Xml.Contains(TEXT("synthetic &quot;quoted&quot; &lt;error&gt;")));
	TestEqual(TEXT("Escape"), BotJUnit::Escape(TEXT("a<b>&\"'\x01")), FString(TEXT("a&lt;b&gt;&amp;&quot;&apos;")));
	return true;
}

// --- E1.3 sentinel -----------------------------------------------------------------------------

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotSentinelTest, "Nightfall.Bot.Sentinel.Counting", BotTestFlags)

bool FBotSentinelTest::RunTest(const FString& Parameters)
{
	TestTrue(TEXT("LogNightfall watched"), FBotLogSentinel::IsWatchedCategory(TEXT("LogNightfall")));
	TestTrue(TEXT("LogTurboLink watched"), FBotLogSentinel::IsWatchedCategory(TEXT("LogTurboLink")));
	TestTrue(TEXT("LogNet* watched"), FBotLogSentinel::IsWatchedCategory(TEXT("LogNetTraffic")) && FBotLogSentinel::IsWatchedCategory(TEXT("LogNet")));
	TestFalse(TEXT("LogTemp not watched"), FBotLogSentinel::IsWatchedCategory(TEXT("LogTemp")));
	TestFalse(TEXT("the runner's own category not watched"), FBotLogSentinel::IsWatchedCategory(TEXT("LogNightfallBot")));

	TArray<FBotAllowEntry> Allow;
	TArray<FString> Errors;
	TestTrue(TEXT("allow-list parses"), FBotLogSentinel::ParseAllowList(
		TEXT("# category | substring | reason\n")
		TEXT("LogNet | known noisy | the API restarts between scenarios\n")
		TEXT("ensure | NavMesh | no nav mesh headless\n"), Allow, Errors));
	TestEqual(TEXT("two entries"), Allow.Num(), 2);
	TestEqual(TEXT("reason kept"), Allow.Num() > 0 ? Allow[0].Reason : FString(), FString(TEXT("the API restarts between scenarios")));
	TArray<FBotAllowEntry> Bad;
	Errors.Reset();
	TestFalse(TEXT("an entry without a reason is rejected"), FBotLogSentinel::ParseAllowList(TEXT("LogNet | text\nLogNet | text |  \n"), Bad, Errors));
	TestEqual(TEXT("both bad lines named"), Errors.Num(), 2);

	FBotLogSentinel Sentinel;
	TArray<FString> Mirrored;
	Sentinel.SetMirror([&](const FString& Line) { Mirrored.Add(Line); });
	Sentinel.SetAllowList(Allow);
	Sentinel.Serialize(TEXT("synthetic error line"), ELogVerbosity::Error, TEXT("LogNightfall"));
	Sentinel.Serialize(TEXT("a warning"), ELogVerbosity::Warning, TEXT("LogNightfall"));
	Sentinel.Serialize(TEXT("someone else's error"), ELogVerbosity::Error, TEXT("LogTemp"));
	Sentinel.Serialize(TEXT("a known noisy line"), ELogVerbosity::Error, TEXT("LogNet"));
	Sentinel.Serialize(TEXT("fatal in grpc"), ELogVerbosity::Fatal, TEXT("LogTurboLink"));
	Sentinel.NoteEnsure(TEXT("ensure(NavMesh) at x.cpp:1"));
	Sentinel.NoteEnsure(TEXT("ensure(Ptr) at y.cpp:2"));
	TestEqual(TEXT("unallowed: the LogNightfall error, the LogTurboLink fatal, one ensure"), Sentinel.NumUnallowed(), 3);
	TestEqual(TEXT("allowed: the noisy LogNet line and the NavMesh ensure"), Sentinel.NumAllowed(), 2);
	const TArray<FString> Lines = Sentinel.GetUnallowed();
	TestTrue(TEXT("the line is named with its category"), Lines.Num() == 3 && Lines[0] == TEXT("LogNightfall Error: synthetic error line"));
	TestEqual(TEXT("every line mirrored to the run log"), Mirrored.Num(), 7);
	TestEqual(TEXT("allow-list hits counted"), Sentinel.GetAllowList()[0].Hits, 1);

	// D6: every step passed, one error logged -> exit 1, and JUnit names the line.
	FExecRig Rig;
	if (!Rig.Load(*this, TEXT("nf.Login\n"))) return false;
	Rig.RunToEnd();
	TestTrue(TEXT("steps passed"), Rig.Executor->HasPassed());
	FBotLogSentinel One;
	One.Serialize(TEXT("synthetic error line"), ELogVerbosity::Error, TEXT("LogNightfall"));
	TestEqual(TEXT("one unallowed error -> exit 1"), UBotScenarioRunner::ExitCodeFor(Rig.Executor->HasPassed(), One.NumUnallowed()), 1);
	const TArray<FBotTestCase> Cases = UBotScenarioRunner::ReportCases(*Rig.Executor, One);
	TestTrue(TEXT("sentinel testcase failed and names the line"), Cases.Num() == 1 && Cases[0].Status == FBotTestCase::EStatus::Failed && Cases[0].Detail.Contains(TEXT("synthetic error line")));
	FBotLogSentinel Clean;
	Clean.SetAllowList(Allow);
	Clean.Serialize(TEXT("a known noisy line"), ELogVerbosity::Error, TEXT("LogNet"));
	TestEqual(TEXT("allow-listed line passes -> exit 0"), UBotScenarioRunner::ExitCodeFor(Rig.Executor->HasPassed(), Clean.NumUnallowed()), 0);

	// The real path: Start() hooks GLog. (An expected UE_LOG Error is swallowed by the automation
	// framework before output devices see it, so the hookup is checked with a Display line; the
	// counting above is the same Serialize call.)
	FBotLogSentinel Live;
	TArray<FString> LiveLines;
	Live.SetMirror([&](const FString& Line) { LiveLines.Add(Line); });
	Live.Start();
	UE_LOG(LogNightfall, Display, TEXT("bot sentinel probe line"));
	GLog->Flush();
	Live.Stop();
	UE_LOG(LogNightfall, Display, TEXT("bot sentinel probe after stop"));
	GLog->Flush();
	TestTrue(TEXT("GLog lines reach the sentinel"), LiveLines.ContainsByPredicate([](const FString& L) { return L.Contains(TEXT("LogNightfall: Display: bot sentinel probe line")); }));
	TestFalse(TEXT("nothing after Stop"), LiveLines.ContainsByPredicate([](const FString& L) { return L.Contains(TEXT("probe after stop")); }));
	return true;
}

// --- E1.2 predicates ---------------------------------------------------------------------------

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotPredicatesTest, "Nightfall.Bot.Predicates.Transitions", BotTestFlags)

bool FBotPredicatesTest::RunTest(const FString& Parameters)
{
	FWorldRig Rig;
	auto Is = [&](const TCHAR* Text, bool bExpected)
	{
		const FBotPredicateValue V = Rig.Eval(Text);
		TestEqual(FString::Printf(TEXT("%s is %s (observed %s)"), Text, bExpected ? TEXT("true") : TEXT("false"), *V.Observed), V.bTrue, bExpected);
	};

	// Login and connection.
	Is(TEXT("connected"), false);
	Rig.Instance.Get<UAuthSubsystem>()->LoginWithDevToken(TEXT("test:00000000-0000-4000-8000-000000000001"));
	Is(TEXT("connected"), true);
	Is(TEXT("ws_connected"), false);
	Is(TEXT("proxies >= 0"), true);
	Rig.Connect();
	Is(TEXT("ws_connected"), true);
	Is(TEXT("in_world"), false);   // no own spawn yet (and no world map in a test)

	// Spawns: own entity, a wolf and a boar.
	Is(TEXT("own_hp < 400"), false);   // unknown before the spawn
	Is(TEXT("own_at 0 0 1"), false);
	Rig.Spawn(OwnId, TEXT("Hero"), 1, 0, 300, 300, 3, false, { 10.f, 10.f });
	Is(TEXT("own_at 10 10 0.5"), true);
	Is(TEXT("in_world"), false);   // still no world map: in_world needs ANightfallGameMode
	Is(TEXT("proxies >= 1"), false);
	Rig.Spawn(Wolf, TEXT("Wolf"), 2, 1, 100, 100, 2, true, { 14.f, 10.f });
	Rig.Spawn(Boar, TEXT("Boar"), 2, 1, 80, 80, 2, true, { 40.f, 40.f });
	Is(TEXT("proxies >= 2"), true);
	Is(TEXT("proxies == 2"), true);
	const int64 NowMs = FDateTime::UtcNow().ToUnixTimestamp() * 1000 + FDateTime::UtcNow().GetMillisecond();
	Rig.Move(OwnId, { 20.f, 21.f }, NowMs + 1000);   // server time after the spawn's sample
	Is(TEXT("own_at 20 21 0.1"), true);
	Is(TEXT("own_at 10 10 1"), false);
	Is(TEXT("own_hp == 300"), true);
	Is(TEXT("own_level == 3"), true);
	Is(TEXT("own_level >= 4"), false);
	Is(TEXT("xp_known"), false);
	Rig.Stats(300, 300, 3, 1000);
	Is(TEXT("xp_known"), true);

	// nearest_attackable: the wolf (14,10) is closer to (20,21) than the boar (40,40).
	TestEqual(TEXT("nearest attackable"), BotPredicates::NearestAttackable(FBotContext{ Rig.Instance.GameInstance, &Rig.Observations }), FString(Wolf));

	// nf.Target through the console, as a scenario runs it.
	Is(TEXT("target == none"), true);
	Is(TEXT("target != none"), false);
	TestTrue(TEXT("nf.Target is a console command"), IConsoleManager::Get().ProcessUserConsoleInput(TEXT("nf.Target nearest_attackable"), *GLog, Rig.Instance.GameInstance->GetWorld()));
	const int32 SetTargetFrame = Rig.Socket->Sent.Num() - 1;
	TestEqual(TEXT("nf.Target sent SetTarget"), Rig.Socket->IntentField(SetTargetFrame), 12);
	Is(TEXT("target != none"), false);   // not until the server confirms
	Rig.Ack(1);
	Rig.Target(Wolf);
	Is(TEXT("target == Wolf"), true);
	Is(TEXT(" target == 0b6e2f6e-0000-4000-8000-0000000000a1"), true);
	Is(TEXT("target != none"), true);
	Is(TEXT("last_ack_seq == 1"), true);

	// nf.Attack: pending until Acked, then active.
	Is(TEXT("attack_state == idle"), true);
	TestTrue(TEXT("nf.Attack is a console command"), IConsoleManager::Get().ProcessUserConsoleInput(TEXT("nf.Attack"), *GLog, Rig.Instance.GameInstance->GetWorld()));
	TestEqual(TEXT("nf.Attack sent Attack"), Rig.Socket->IntentField(Rig.Socket->Sent.Num() - 1), 13);
	Is(TEXT("attack_state == pending"), true);
	IConsoleManager::Get().ProcessUserConsoleInput(TEXT("nf.Attack"), *GLog, Rig.Instance.GameInstance->GetWorld());
	TestEqual(TEXT("a second nf.Attack sends nothing"), Rig.Socket->Sent.Num(), SetTargetFrame + 2);
	Rig.Ack(2);
	Is(TEXT("attack_state == active"), true);
	Is(TEXT("last_ack_seq == 2"), true);

	// Hits: damage numbers, target HP, own HP.
	Is(TEXT("damage_numbers >= 1"), false);
	Rig.Hit(OwnId, Wolf, 10, 40, 60, 1);
	Is(TEXT("damage_numbers >= 1"), true);
	Is(TEXT("target_hp < 100"), true);
	Is(TEXT("target_hp == 60"), true);
	Rig.Hit(OwnId, Wolf, 10, 40, 60, 1);   // replayed: no second number
	Is(TEXT("damage_numbers == 1"), true);
	Rig.Hit(Wolf, OwnId, 11, 25, 275, 0);
	Is(TEXT("own_hp < 300"), true);
	Is(TEXT("damage_numbers == 2"), true);

	// The kill: HP 0, target clears, target_hp still reads the last selection.
	Is(TEXT("target_hp == 0"), false);
	Rig.Hit(OwnId, Wolf, 12, 60, 0, 1);
	Rig.Died(Wolf, 12, 1);
	Is(TEXT("target == none"), true);
	Is(TEXT("target_hp == 0"), true);
	Is(TEXT("attack_state == idle"), true);
	{
		FWorldEvent E; E.Despawn = FEntityDespawn{ Wolf }; Rig.Event(E);   // the corpse leaves the projection
	}
	Is(TEXT("target_hp == 0"), true);   // still: kept from the events
	Rig.Xp(50, 1050);
	Is(TEXT("own_xp == 1050"), true);

	// nf.StopAttack after a new Attack.
	IConsoleManager::Get().ProcessUserConsoleInput(TEXT("nf.Target 0b6e2f6e-0000-4000-8000-0000000000a2"), *GLog, Rig.Instance.GameInstance->GetWorld());
	IConsoleManager::Get().ProcessUserConsoleInput(TEXT("nf.Attack"), *GLog, Rig.Instance.GameInstance->GetWorld());
	Is(TEXT("attack_state == pending"), true);
	IConsoleManager::Get().ProcessUserConsoleInput(TEXT("nf.StopAttack"), *GLog, Rig.Instance.GameInstance->GetWorld());
	TestEqual(TEXT("nf.StopAttack sent StopAttack"), Rig.Socket->IntentField(Rig.Socket->Sent.Num() - 1), 14);
	Is(TEXT("attack_state == idle"), true);

	// Rejections, by name or number.
	Is(TEXT("rejected == none"), true);
	IConsoleManager::Get().ProcessUserConsoleInput(TEXT("nf.Target no-such-entity"), *GLog, Rig.Instance.GameInstance->GetWorld());
	Rig.Reject(Rig.Net->GetLastSentSeq(), 3);
	Is(TEXT("rejected == UNKNOWN_ENTITY"), true);
	Is(TEXT("rejected == REJECT_REASON_UNKNOWN_ENTITY"), true);
	Is(TEXT("rejected == 3"), true);
	Is(TEXT("rejected != TOO_FAR"), true);
	Is(TEXT("rejected == none"), false);

	// Own death and nf.Respawn.
	Is(TEXT("own_dead"), false);
	Rig.Hit(Boar, OwnId, 20, 275, 0, 0);
	Rig.Died(OwnId, 20, 0);
	Is(TEXT("own_dead"), true);
	TestTrue(TEXT("nf.Respawn is a console command"), IConsoleManager::Get().ProcessUserConsoleInput(TEXT("nf.Respawn"), *GLog, Rig.Instance.GameInstance->GetWorld()));
	TestEqual(TEXT("nf.Respawn sent Respawn"), Rig.Socket->IntentField(Rig.Socket->Sent.Num() - 1), 15);
	{
		FWorldEvent E; E.EntityRespawned = FEntityRespawned{ OwnId, 30, FNetVec2{ 126.f, 126.f }, 195, 0 }; Rig.Event(E);
	}
	Is(TEXT("own_dead"), false);
	Is(TEXT("own_hp == 195"), true);
	Is(TEXT("own_at 126 126 0.5"), true);

	// Every built-in has a usage and a description (the README lists them).
	for (const FBotPredicateDef& Def : FBotPredicateRegistry::Get().All())
	{
		TestFalse(FString::Printf(TEXT("%s has a usage"), *Def.Name), Def.Usage.IsEmpty());
		TestFalse(FString::Printf(TEXT("%s has a description"), *Def.Name), Def.Description.IsEmpty());
	}
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotSelectTargetTest, "Nightfall.Bot.Commands.SelectAfterClear", BotTestFlags)

bool FBotSelectTargetTest::RunTest(const FString& Parameters)
{
	FWorldRig Rig;
	Rig.Connect();
	Rig.Spawn(OwnId, TEXT("Hero"), 1, 0, 300, 300, 3, false);
	Rig.Spawn(Wolf, TEXT("Wolf"), 2, 1, 100, 100, 2, true);
	TestTrue(TEXT("select the wolf"), Rig.Combat->SelectTarget(Wolf) != 0);
	TestEqual(TEXT("selecting it again sends nothing"), Rig.Combat->SelectTarget(Wolf), 0u);
	Rig.Target(Wolf);
	TestTrue(TEXT("clear"), Rig.Combat->SelectTarget(FString()) != 0);
	TestEqual(TEXT("clearing again sends nothing"), Rig.Combat->SelectTarget(FString()), 0u);
	TestEqual(TEXT("no attack on a pending clear"), Rig.Combat->AttackSelection(), 0u);
	TestTrue(TEXT("re-selecting the wolf before TargetChanged(none) is sent"), Rig.Combat->SelectTarget(Wolf) != 0);
	return true;
}

// --- E1.1 inert without the flag; R5 -----------------------------------------------------------

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotRunnerInertTest, "Nightfall.Bot.Runner.InertWithoutFlag", BotTestFlags)

bool FBotRunnerInertTest::RunTest(const FString& Parameters)
{
	if (FString(FCommandLine::Get()).Contains(TEXT("-BotScenario=")))
	{
		AddInfo(TEXT("running inside a -BotScenario process; inertness is not observable here"));
		return true;
	}
	FScopedTestGameInstance Instance;
	UBotScenarioRunner* Runner = Instance.Get<UBotScenarioRunner>();
	if (!TestNotNull(TEXT("the runner subsystem exists"), Runner)) return false;
	TestFalse(TEXT("inert without -BotScenario"), Runner->IsActive());
	TestNull(TEXT("test-only nf.DropSocket is not registered without -BotScenario (R5)"), IConsoleManager::Get().FindConsoleObject(TEXT("nf.DropSocket")));
	TestNotNull(TEXT("nf.Target is available interactively"), IConsoleManager::Get().FindConsoleObject(TEXT("nf.Target")));
	return true;
}

#endif
