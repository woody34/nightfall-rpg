#include "Misc/AutomationTest.h"
#include "Bot/BotPredicates.h"
#include "Bot/BotScenarioRunner.h"
#include "Bot/BotSteps.h"
#include "HAL/FileManager.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"

#if WITH_DEV_AUTOMATION_TESTS

// D155 / T49: a -BotLoop run re-runs the -BotScenario script per iteration; with -BotLoopScenario
// iteration 1 runs the first script and every later iteration runs the loop script. Offline: the
// executor on a fake clock, FBotLoopPlan for the script selection and testcase accounting, and the
// real scenario loader for the fail-closed parse errors. No server, no world.

namespace
{
	constexpr EAutomationTestFlags LoopTestFlags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;

	const BotScenario::FCommandExists LoopCommands = [](const FString& Name)
	{
		return Name.Equals(TEXT("nf.Login"), ESearchCase::IgnoreCase) || Name.Equals(TEXT("nf.EnterWorld"), ESearchCase::IgnoreCase)
			|| Name.Equals(TEXT("nf.Enchant"), ESearchCase::IgnoreCase) || Name.Equals(TEXT("nf.Equip"), ESearchCase::IgnoreCase);
	};

	// The first script spends the scroll (L3) and asserts on it (L4); the loop script never names it.
	const TCHAR* const FirstScript = TEXT("nf.Login\nnf.EnterWorld\nnf.Enchant 951 template:71 01970000-0000-7000-8000-000000000042\nnf.Expect ready\n");
	const TCHAR* const LoopScript = TEXT("nf.Login\nnf.EnterWorld\nnf.Equip 1\nnf.Expect ready\nnf.Expect ready\n");

	/** Parses, runs one iteration to the end on a fake clock and records the commands it sent. */
	struct FLoopRig
	{
		FBotPredicateRegistry Registry;
		TArray<FString> Ran;
		double Now = 100.0;

		FLoopRig()
		{
			Registry.RegisterBuiltins();
			Registry.RegisterFlag(TEXT("ready"), TEXT("test"), [](const FBotContext&) { return true; });
		}

		bool Parse(FAutomationTestBase& Test, const TCHAR* Name, const TCHAR* Text, FBotScenario& Out)
		{
			TArray<FString> Errors;
			const bool bParsed = BotScenario::Parse(Name, Text, Registry, LoopCommands, Out, Errors);
			return Test.TestTrue(FString::Printf(TEXT("%s parses: %s"), Name, *FString::Join(Errors, TEXT("; "))), bParsed);
		}

		/** One iteration exactly as the runner builds it: a fresh executor over Plan.Next(). */
		TUniquePtr<FBotScenarioExecutor> Run(const FBotLoopPlan& Plan)
		{
			FBotScenario Scenario = Plan.Next();
			auto Executor = MakeUnique<FBotScenarioExecutor>(MoveTemp(Scenario),
				[this](const FString& Line, FString&) { Ran.Add(Line); return true; },
				[](const FBotPredicateFn& Fn) { return Fn(FBotContext()); },
				[](const FString&) {});
			Executor->Start(Now);
			for (int32 N = 0; N < 1000 && Executor->Tick(Now += 0.1); ++N) {}
			return Executor;
		}
	};

	int32 CountNamed(const TArray<FBotTestCase>& Cases, const TCHAR* Prefix)
	{
		int32 N = 0;
		for (const FBotTestCase& Case : Cases) if (Case.Name.StartsWith(Prefix)) ++N;
		return N;
	}

	int32 CountRan(const TArray<FString>& Ran, const TCHAR* Line)
	{
		int32 N = 0;
		for (const FString& L : Ran) if (L == Line) ++N;
		return N;
	}
}

// --- flag absent: every iteration re-runs the first script -----------------------------------

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotLoopSameScriptTest, "Nightfall.Bot.Loop.SameScriptWithoutLoopScenario", LoopTestFlags)

bool FBotLoopSameScriptTest::RunTest(const FString& Parameters)
{
	FLoopRig Rig;
	FBotLoopPlan Plan;
	if (!Rig.Parse(*this, TEXT("first"), FirstScript, Plan.First)) return false;
	TestFalse(TEXT("no loop script"), Plan.Loop.IsSet());

	for (int32 Iteration = 1; Iteration <= 3; ++Iteration)
	{
		TestEqual(TEXT("Next() is the first script"), Plan.Next().Name, FString(TEXT("first")));
		TestFalse(TEXT("never switches scripts"), Plan.SwitchesScript());
		const TUniquePtr<FBotScenarioExecutor> Executor = Rig.Run(Plan);
		TestTrue(TEXT("iteration passed"), Executor->HasPassed());
		TestEqual(TEXT("Accumulate returns the iteration number"), Plan.Accumulate(Executor->GetTestCases()), Iteration);
	}
	TestEqual(TEXT("the keyed enchant was sent on every iteration"), CountRan(Rig.Ran, TEXT("nf.Enchant 951 template:71 01970000-0000-7000-8000-000000000042")), 3);
	TestEqual(TEXT("three iterations accumulated"), Plan.Iterations, 3);
	TestEqual(TEXT("one assertion per iteration"), Plan.Cases.Num(), 3);
	TestEqual(TEXT("iteration 1 case name"), Plan.Cases[0].Name, FString(TEXT("iteration 1: L4 nf.Expect ready")));
	TestEqual(TEXT("iteration 2 case name"), Plan.Cases[1].Name, FString(TEXT("iteration 2: L4 nf.Expect ready")));
	TestEqual(TEXT("iteration 3 case name"), Plan.Cases[2].Name, FString(TEXT("iteration 3: L4 nf.Expect ready")));
	return true;
}

// --- flag present: iteration 1 runs the first script, iterations >= 2 the loop script ----------

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotLoopScenarioFromSecondIterationTest, "Nightfall.Bot.Loop.LoopScenarioFromSecondIteration", LoopTestFlags)

bool FBotLoopScenarioFromSecondIterationTest::RunTest(const FString& Parameters)
{
	FLoopRig Rig;
	FBotLoopPlan Plan;
	FBotScenario Loop;
	if (!Rig.Parse(*this, TEXT("first"), FirstScript, Plan.First) || !Rig.Parse(*this, TEXT("loop"), LoopScript, Loop)) return false;
	Plan.Loop = MoveTemp(Loop);

	// Iteration 1: the first script, scroll spent once.
	TestEqual(TEXT("iteration 1 runs the first script"), Plan.Next().Name, FString(TEXT("first")));
	TestFalse(TEXT("no switch before iteration 1"), Plan.SwitchesScript());
	TUniquePtr<FBotScenarioExecutor> Executor = Rig.Run(Plan);
	TestTrue(TEXT("iteration 1 passed"), Executor->HasPassed());
	TestEqual(TEXT("iteration 1 accumulated"), Plan.Accumulate(Executor->GetTestCases()), 1);
	TestEqual(TEXT("enchant sent once"), CountRan(Rig.Ran, TEXT("nf.Enchant 951 template:71 01970000-0000-7000-8000-000000000042")), 1);

	// Advancing after iteration 1 switches, exactly once, to the loop script.
	TestTrue(TEXT("switches scripts after iteration 1"), Plan.SwitchesScript());
	TestEqual(TEXT("iteration 2 runs the loop script"), Plan.Next().Name, FString(TEXT("loop")));
	Executor = Rig.Run(Plan);
	TestTrue(TEXT("iteration 2 passed"), Executor->HasPassed());
	TestEqual(TEXT("iteration 2 accumulated"), Plan.Accumulate(Executor->GetTestCases()), 2);
	TestEqual(TEXT("enchant still sent once"), CountRan(Rig.Ran, TEXT("nf.Enchant 951 template:71 01970000-0000-7000-8000-000000000042")), 1);
	TestEqual(TEXT("the loop script's equip ran"), CountRan(Rig.Ran, TEXT("nf.Equip 1")), 1);

	TestFalse(TEXT("no second switch log"), Plan.SwitchesScript());
	TestEqual(TEXT("iteration 3 stays on the loop script"), Plan.Next().Name, FString(TEXT("loop")));
	Executor = Rig.Run(Plan);
	TestTrue(TEXT("iteration 3 passed"), Executor->HasPassed());
	Plan.Accumulate(Executor->GetTestCases());
	TestEqual(TEXT("enchant never re-sent"), CountRan(Rig.Ran, TEXT("nf.Enchant 951 template:71 01970000-0000-7000-8000-000000000042")), 1);
	TestEqual(TEXT("equip per loop iteration"), CountRan(Rig.Ran, TEXT("nf.Equip 1")), 2);

	// Testcase accounting: the first script's one assertion, then the loop script's two per iteration.
	TestEqual(TEXT("1 + 2 + 2 cases"), Plan.Cases.Num(), 5);
	TestEqual(TEXT("iteration 1 names"), CountNamed(Plan.Cases, TEXT("iteration 1: ")), 1);
	TestEqual(TEXT("iteration 2 names"), CountNamed(Plan.Cases, TEXT("iteration 2: ")), 2);
	TestEqual(TEXT("iteration 3 names"), CountNamed(Plan.Cases, TEXT("iteration 3: ")), 2);
	TestEqual(TEXT("iteration 1 case is the first script's L4"), Plan.Cases[0].Name, FString(TEXT("iteration 1: L4 nf.Expect ready")));
	TestEqual(TEXT("iteration 2 first case is the loop script's L4"), Plan.Cases[1].Name, FString(TEXT("iteration 2: L4 nf.Expect ready")));
	TestEqual(TEXT("iteration 2 second case is the loop script's L5"), Plan.Cases[2].Name, FString(TEXT("iteration 2: L5 nf.Expect ready")));
	for (const FBotTestCase& Case : Plan.Cases) TestTrue(FString::Printf(TEXT("%s passed"), *Case.Name), Case.Status == FBotTestCase::EStatus::Passed);
	return true;
}

// --- missing or unparseable loop script: a parse error that fails the run ---------------------

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotLoopScenarioParseErrorTest, "Nightfall.Bot.Loop.MissingOrBrokenLoopScenarioFails", LoopTestFlags)

bool FBotLoopScenarioParseErrorTest::RunTest(const FString& Parameters)
{
	const FString Dir = FPaths::Combine(FPaths::AutomationTransientDir(), TEXT("BotLoopScenarioTest"));
	IFileManager::Get().MakeDirectory(*Dir, /*Tree*/ true);
	const FString Missing = FPaths::Combine(Dir, TEXT("missing-loop.nfs"));
	const FString Broken = FPaths::Combine(Dir, TEXT("broken-loop.nfs"));
	const FString Valid = FPaths::Combine(Dir, TEXT("valid-loop.nfs"));
	IFileManager::Get().Delete(*Missing);
	TestTrue(TEXT("write the broken script"), FFileHelper::SaveStringToFile(TEXT("nf.Login\nnf.WaitFor no_such_predicate_d155 5\nnf.NoSuchCommandD155\n"), *Broken));
	TestTrue(TEXT("write the valid script"), FFileHelper::SaveStringToFile(TEXT("# loop script\nnf.Login\nnf.Sleep 0.1\n"), *Valid));

	FBotScenario Scenario;
	TArray<FString> Errors;
	TestFalse(TEXT("a missing loop script does not load"), UBotScenarioRunner::LoadScenarioFile(Missing, Scenario, Errors));
	if (TestEqual(TEXT("one error for the missing file"), Errors.Num(), 1))
	{
		TestTrue(TEXT("the error names the file"), Errors[0].Contains(TEXT("cannot read scenario file")) && Errors[0].Contains(Missing));
	}

	Errors.Reset();
	TestFalse(TEXT("an unparseable loop script does not load"), UBotScenarioRunner::LoadScenarioFile(Broken, Scenario, Errors));
	TestTrue(TEXT("the bad predicate and the unknown command are both reported"), Errors.Num() >= 2);

	// Errors accumulate across both scripts; the runner appends the loop script's to the first's.
	TArray<FString> Both;
	Both.Add(TEXT("earlier error"));
	TestFalse(TEXT("appends to existing errors"), UBotScenarioRunner::LoadScenarioFile(Missing, Scenario, Both));
	TestEqual(TEXT("earlier errors are kept"), Both.Num(), 2);

	Errors.Reset();
	TestTrue(TEXT("a valid loop script loads"), UBotScenarioRunner::LoadScenarioFile(Valid, Scenario, Errors));
	TestEqual(TEXT("no errors for the valid script"), Errors.Num(), 0);
	TestEqual(TEXT("the scenario is named after the file"), Scenario.Name, FString(TEXT("valid-loop")));
	TestEqual(TEXT("two steps"), Scenario.Steps.Num(), 2);

	// Without an executor the run reports one failed "parse scenario" case and exits 1 (fail closed).
	TArray<FString> ParseErrors;
	ParseErrors.Add(TEXT("loop scenario missing-loop.nfs: cannot read scenario file ") + Missing);
	const FBotTestCase Case = UBotScenarioRunner::StartFailureCase(ParseErrors);
	TestEqual(TEXT("parse scenario case"), Case.Name, FString(TEXT("parse scenario")));
	TestTrue(TEXT("failed"), Case.Status == FBotTestCase::EStatus::Failed);
	TestEqual(TEXT("the first error is the message"), Case.Message, ParseErrors[0]);
	TestEqual(TEXT("exit code 1 when the steps never ran"), UBotScenarioRunner::ExitCodeFor(false, 0), 1);

	IFileManager::Get().Delete(*Broken);
	IFileManager::Get().Delete(*Valid);
	return true;
}

#endif
