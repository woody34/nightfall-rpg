#include "BotSteps.h"
#include "Misc/SecureHash.h"
#include "BotPredicates.h"
#include "BotScenarioRunner.h"
#include "Nightfall.h"
#include "Auth/AuthSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Containers/Ticker.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"
#include "HAL/IConsoleManager.h"

// --- Parser ------------------------------------------------------------------------------------

namespace
{
	bool ParseSeconds(const FString& Text, bool bAllowZero, double& Out)
	{
		if (!Text.IsNumeric()) return false;
		Out = FCString::Atod(*Text);
		return bAllowZero ? Out >= 0.0 : Out > 0.0;
	}

	FString FormatSeconds(double Seconds)
	{
		return FString::Printf(TEXT("%.2f s"), Seconds);
	}
}

FString BotScenario::StripComment(const FString& Line)
{
	int32 Hash = INDEX_NONE;
	FString Text = Line.FindChar(TEXT('#'), Hash) ? Line.Left(Hash) : Line;
	Text.TrimStartAndEndInline();
	return Text;
}

bool BotScenario::Parse(const FString& Name, const FString& Text, const FBotPredicateRegistry& Predicates,
	const FCommandExists& CommandExists, FBotScenario& Out, TArray<FString>& OutErrors)
{
	Out = FBotScenario();
	Out.Name = Name;
	TArray<FString> Lines;
	Text.ParseIntoArrayLines(Lines, /*CullEmpty*/ false);

	for (int32 I = 0; I < Lines.Num(); ++I)
	{
		const int32 LineNo = I + 1;
		const FString Source = StripComment(Lines[I]);
		if (Source.IsEmpty()) continue;
		auto Error = [&](const FString& Message) { OutErrors.Add(FString::Printf(TEXT("line %d: %s: %s"), LineNo, *Message, *Source)); };

		TArray<FString> Tokens = FBotPredicateRegistry::Tokenize(Source);
		const FString Head = Tokens[0];
		Tokens.RemoveAt(0);

		FBotStep Step;
		Step.Line = LineNo;
		Step.Source = Source;
		FString PredicateError;

		if (Head.Equals(TEXT("nf.WaitFor"), ESearchCase::IgnoreCase))
		{
			Step.Kind = EBotStepKind::WaitFor;
			if (Tokens.Num() < 2 || !ParseSeconds(Tokens.Last(), false, Step.Seconds))
			{
				Error(TEXT("nf.WaitFor needs a predicate and a timeout in seconds (> 0) as its last argument"));
				continue;
			}
			Tokens.Pop();
			Step.PredicateText = FString::Join(Tokens, TEXT(" "));
			Step.Predicate = Predicates.Parse(Tokens, PredicateError);
		}
		else if (Head.Equals(TEXT("nf.Expect"), ESearchCase::IgnoreCase))
		{
			Step.Kind = EBotStepKind::Expect;
			Step.PredicateText = FString::Join(Tokens, TEXT(" "));
			Step.Predicate = Predicates.Parse(Tokens, PredicateError);
		}
		else if (Head.Equals(TEXT("nf.Sleep"), ESearchCase::IgnoreCase))
		{
			Step.Kind = EBotStepKind::Sleep;
			if (Tokens.Num() != 1 || !ParseSeconds(Tokens[0], true, Step.Seconds)) { Error(TEXT("nf.Sleep needs one duration in seconds")); continue; }
		}
		else if (Head.Equals(TEXT("nf.Within"), ESearchCase::IgnoreCase))
		{
			Step.Kind = EBotStepKind::Within;
			if (Tokens.Num() != 1 || !ParseSeconds(Tokens[0], false, Step.Seconds)) { Error(TEXT("nf.Within needs one budget in seconds (> 0)")); continue; }
			if (Out.BudgetSeconds > 0.0) { Error(FString::Printf(TEXT("a second nf.Within (the first is on line %d)"), Out.BudgetLine)); continue; }
			Out.BudgetSeconds = Step.Seconds;
			Out.BudgetLine = LineNo;
		}
		else
		{
			Step.Kind = EBotStepKind::Command;
			if (Head.StartsWith(TEXT("nf."), ESearchCase::IgnoreCase) && !(CommandExists && CommandExists(Head)))
			{
				Error(FString::Printf(TEXT("unknown command '%s'"), *Head));
				continue;
			}
		}

		if (Step.IsAssertion() && !Step.Predicate)
		{
			Error(PredicateError.IsEmpty() ? FString(TEXT("bad predicate")) : PredicateError);
			continue;
		}
		Out.Steps.Add(MoveTemp(Step));
	}
	if (OutErrors.IsEmpty() && Out.Steps.IsEmpty()) OutErrors.Add(TEXT("the scenario has no steps"));
	return OutErrors.IsEmpty();
}

// --- Executor ----------------------------------------------------------------------------------

FBotScenarioExecutor::FBotScenarioExecutor(FBotScenario InScenario, FExecFn InExec, FEvalFn InEval, FTraceFn InTrace)
	: Scenario(MoveTemp(InScenario)), Exec(MoveTemp(InExec)), Eval(MoveTemp(InEval)), TraceFn(MoveTemp(InTrace))
{
}

void FBotScenarioExecutor::Trace(double Now, const FString& Text) const
{
	if (TraceFn) TraceFn(FString::Printf(TEXT("[%8.3f] %s"), Now - StartSeconds, *Text));
}

FBotTestCase& FBotScenarioExecutor::CaseFor(int32 StepIndex)
{
	if (const int32* Found = CaseOfStep.Find(StepIndex)) return Cases[*Found];
	const FBotStep& Step = Scenario.Steps[StepIndex];
	FBotTestCase& Case = Cases.AddDefaulted_GetRef();
	Case.Name = FString::Printf(TEXT("L%d %s"), Step.Line, *Step.Source);
	Case.Line = Step.Line;
	Case.Status = FBotTestCase::EStatus::Skipped;
	Case.Message = TEXT("not reached");
	CaseOfStep.Add(StepIndex, Cases.Num() - 1);
	return Case;
}

void FBotScenarioExecutor::Start(double NowSeconds)
{
	StartSeconds = EndSeconds = NowSeconds;
	bStarted = true;
	for (int32 I = 0; I < Scenario.Steps.Num(); ++I)
	{
		if (Scenario.Steps[I].IsAssertion() || Scenario.Steps[I].Kind == EBotStepKind::Within) CaseFor(I);
	}
	Trace(NowSeconds, FString::Printf(TEXT("scenario %s: %d steps%s"), *Scenario.Name, Scenario.Steps.Num(),
		Scenario.BudgetSeconds > 0.0 ? *FString::Printf(TEXT(", budget %s"), *FormatSeconds(Scenario.BudgetSeconds)) : TEXT("")));
}

bool FBotScenarioExecutor::Tick(double Now)
{
	if (!bStarted || bFinished) return false;

	if (Scenario.BudgetSeconds > 0.0 && Now - StartSeconds > Scenario.BudgetSeconds)
	{
		const FString Message = FString::Printf(TEXT("nf.Within %s exceeded"), *FormatSeconds(Scenario.BudgetSeconds));
		if (Current < Scenario.Steps.Num())
		{
			FailStep(Scenario.Steps[Current], Now, FString::Printf(TEXT("interrupted: %s"), *Message), FString());
		}
		else
		{
			bFailed = true;
			Failure = Message;
			Finish(Now);
		}
		return false;
	}

	while (Current < Scenario.Steps.Num())
	{
		const FBotStep& Step = Scenario.Steps[Current];
		if (StepStartSeconds < 0.0)
		{
			StepStartSeconds = Now;
			Trace(Now, FString::Printf(TEXT("L%d > %s"), Step.Line, *Step.Source));
		}
		const double Waited = Now - StepStartSeconds;
		auto Advance = [this] { ++Current; StepStartSeconds = -1.0; };

		switch (Step.Kind)
		{
		case EBotStepKind::Command:
		{
			FString Error;
			if (!Exec(Step.Source, Error))
			{
				FailStep(Step, Now, FString::Printf(TEXT("command failed: %s"), *Error), FString());
				return false;
			}
			Advance();
			break;
		}
		case EBotStepKind::Expect:
		{
			const FBotPredicateValue V = Eval(Step.Predicate);
			LastObserved = V.Observed;
			if (!V.bTrue)
			{
				FailStep(Step, Now, FString::Printf(TEXT("expected '%s', observed %s"), *Step.PredicateText, *V.Observed),
					FString::Printf(TEXT("predicate: %s\nobserved: %s"), *Step.PredicateText, *V.Observed));
				return false;
			}
			FBotTestCase& Case = CaseFor(Current);
			Case.Status = FBotTestCase::EStatus::Passed;
			Case.Message.Reset();
			Trace(Now, FString::Printf(TEXT("L%d   ok (observed %s)"), Step.Line, *V.Observed));
			Advance();
			break;
		}
		case EBotStepKind::WaitFor:
		{
			const FBotPredicateValue V = Eval(Step.Predicate);
			LastObserved = V.Observed;
			if (V.bTrue)
			{
				FBotTestCase& Case = CaseFor(Current);
				Case.Status = FBotTestCase::EStatus::Passed;
				Case.Message.Reset();
				Case.Seconds = Waited;
				Trace(Now, FString::Printf(TEXT("L%d   ok after %s (observed %s)"), Step.Line, *FormatSeconds(Waited), *V.Observed));
				Advance();
				return true;   // the next step runs on the next tick
			}
			if (Waited >= Step.Seconds)
			{
				FailStep(Step, Now, FString::Printf(TEXT("timed out after %s waiting for '%s'; last observed %s"), *FormatSeconds(Waited), *Step.PredicateText, *V.Observed),
					FString::Printf(TEXT("predicate: %s\ntimeout: %s\nlast observed: %s"), *Step.PredicateText, *FormatSeconds(Step.Seconds), *V.Observed));
				return false;
			}
			return true;
		}
		case EBotStepKind::Sleep:
			if (Waited < Step.Seconds) return true;
			Advance();
			break;
		case EBotStepKind::Within:
			Advance();   // the budget is enforced on every tick and reported at exit
			break;
		}
	}
	Finish(Now);
	return false;
}

void FBotScenarioExecutor::FailStep(const FBotStep& Step, double Now, const FString& Message, const FString& Detail)
{
	if (!FailedStep.IsSet())
	{
		FailedStep = FBotFailedStep{ Step.Line, Step.Source, Step.PredicateText,
			Step.IsAssertion() ? LastObserved : FString(), StepStartSeconds >= 0.0 ? Now - StepStartSeconds : 0.0,
			Now - StartSeconds };
	}
	const int32 StepIndex = Scenario.Steps.IndexOfByPredicate([&](const FBotStep& S) { return S.Line == Step.Line; });
	FBotTestCase& Case = CaseFor(StepIndex);
	Case.Status = FBotTestCase::EStatus::Failed;
	if (Step.Kind == EBotStepKind::Expect) Case.FailureType = TEXT("bot_assertion");
	else if (Step.Kind == EBotStepKind::WaitFor) Case.FailureType = TEXT("bot_expectation");
	Case.Seconds = StepStartSeconds >= 0.0 ? Now - StepStartSeconds : 0.0;
	Case.Message = Message;
	Case.Detail = Detail;
	bFailed = true;
	Failure = FString::Printf(TEXT("L%d %s: %s"), Step.Line, *Step.Source, *Message);
	Trace(Now, FString::Printf(TEXT("L%d   FAILED: %s"), Step.Line, *Message));
	for (FBotTestCase& Other : Cases)
	{
		if (Other.Status == FBotTestCase::EStatus::Skipped) Other.Message = FString::Printf(TEXT("not reached: line %d failed"), Step.Line);
	}
	Finish(Now);
}

void FBotScenarioExecutor::Abort(double NowSeconds, const FString& Reason)
{
	if (!bStarted || bFinished) return;
	if (Current < Scenario.Steps.Num())
	{
		FailStep(Scenario.Steps[Current], NowSeconds, FString::Printf(TEXT("aborted: %s"), *Reason), FString());
	}
	else
	{
		bFailed = true;
		Failure = Reason;
		Finish(NowSeconds);
	}
}

void FBotScenarioExecutor::Finish(double Now)
{
	if (bFinished) return;
	bFinished = true;
	EndSeconds = Now;
	const double Elapsed = Now - StartSeconds;
	if (Scenario.BudgetSeconds > 0.0)
	{
		const int32 WithinIndex = Scenario.Steps.IndexOfByPredicate([](const FBotStep& S) { return S.Kind == EBotStepKind::Within; });
		FBotTestCase& Case = CaseFor(WithinIndex);
		Case.Seconds = Elapsed;
		if (Elapsed <= Scenario.BudgetSeconds)
		{
			Case.Status = FBotTestCase::EStatus::Passed;
			Case.Message.Reset();
		}
		else
		{
			Case.Status = FBotTestCase::EStatus::Failed;
			Case.FailureType = TEXT("bot_scenario");
			Case.Message = FString::Printf(TEXT("scenario took %s, budget %s"), *FormatSeconds(Elapsed), *FormatSeconds(Scenario.BudgetSeconds));
			bFailed = true;
			if (Failure.IsEmpty()) Failure = Case.Message;
			if (!FailedStep.IsSet()) FailedStep = FBotFailedStep{ Scenario.BudgetLine, TEXT("nf.Within"),
				TEXT("scenario wall-clock budget"), FString::SanitizeFloat(Elapsed), Elapsed, Elapsed };
		}
	}
	Cases.StableSort([](const FBotTestCase& A, const FBotTestCase& B) { return A.Line < B.Line; });
	CaseOfStep.Reset();
	Trace(Now, FString::Printf(TEXT("scenario %s %s in %s"), *Scenario.Name, bFailed ? TEXT("FAILED") : TEXT("passed"), *FormatSeconds(Elapsed)));
}

// --- JUnit -------------------------------------------------------------------------------------

FString BotJUnit::Escape(const FString& Text)
{
	FString Out;
	Out.Reserve(Text.Len());
	for (const TCHAR C : Text)
	{
		switch (C)
		{
		case TEXT('&'): Out += TEXT("&amp;"); break;
		case TEXT('<'): Out += TEXT("&lt;"); break;
		case TEXT('>'): Out += TEXT("&gt;"); break;
		case TEXT('"'): Out += TEXT("&quot;"); break;
		case TEXT('\''): Out += TEXT("&apos;"); break;
		default:
			// XML 1.0 forbids most control characters; keep tab, newline, carriage return.
			if (C >= 0x20 || C == TEXT('\t') || C == TEXT('\n') || C == TEXT('\r')) Out.AppendChar(C);
			break;
		}
	}
	return Out;
}

FString BotJUnit::Write(const FString& Scenario, const TArray<FBotTestCase>& Cases, double TotalSeconds,
	const TArray<TPair<FString, FString>>& Properties)
{
	int32 Failures = 0, Skipped = 0;
	for (const FBotTestCase& Case : Cases)
	{
		Failures += Case.Status == FBotTestCase::EStatus::Failed;
		Skipped += Case.Status == FBotTestCase::EStatus::Skipped;
	}
	const FString Suite = Escape(Scenario);
	FString Xml = TEXT("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
	Xml += FString::Printf(TEXT("<testsuites name=\"nightfall-sim\" tests=\"%d\" failures=\"%d\" errors=\"0\" skipped=\"%d\" time=\"%.3f\">\n"),
		Cases.Num(), Failures, Skipped, TotalSeconds);
	Xml += FString::Printf(TEXT("  <testsuite name=\"%s\" tests=\"%d\" failures=\"%d\" errors=\"0\" skipped=\"%d\" time=\"%.3f\" timestamp=\"%s\">\n"),
		*Suite, Cases.Num(), Failures, Skipped, TotalSeconds, *FDateTime::UtcNow().ToIso8601());
	Xml += TEXT("    <properties>\n");
	for (const TPair<FString, FString>& P : Properties)
	{
		Xml += FString::Printf(TEXT("      <property name=\"%s\" value=\"%s\"/>\n"), *Escape(P.Key), *Escape(P.Value));
	}
	Xml += TEXT("    </properties>\n");
	for (const FBotTestCase& Case : Cases)
	{
		const FString Head = FString::Printf(TEXT("    <testcase classname=\"sim.%s\" name=\"%s\" time=\"%.3f\""), *Suite, *Escape(Case.Name), Case.Seconds);
		switch (Case.Status)
		{
		case FBotTestCase::EStatus::Passed:
			Xml += Head + TEXT("/>\n");
			break;
		case FBotTestCase::EStatus::Skipped:
			Xml += Head + FString::Printf(TEXT(">\n      <skipped message=\"%s\"/>\n    </testcase>\n"), *Escape(Case.Message));
			break;
		case FBotTestCase::EStatus::Failed:
			Xml += Head + FString::Printf(TEXT(">\n      <failure message=\"%s\" type=\"%s\">%s</failure>\n    </testcase>\n"),
				*Escape(Case.Message), *Escape(Case.FailureType), *Escape(Case.Detail.IsEmpty() ? Case.Message : Case.Detail));
			break;
		}
	}
	Xml += TEXT("  </testsuite>\n</testsuites>\n");
	return Xml;
}

// --- Console commands (E1.2): also usable by hand in a running client -------------------------

#if !UE_BUILD_SHIPPING
namespace
{
	UGameInstance* GameInstanceOf(UWorld* World) { return World ? World->GetGameInstance() : nullptr; }

	UCombatStateSubsystem* CombatOf(UWorld* World)
	{
		UGameInstance* GI = GameInstanceOf(World);
		return GI ? GI->GetSubsystem<UCombatStateSubsystem>() : nullptr;
	}

	FAutoConsoleCommandWithWorldAndArgs TargetCommand(TEXT("nf.Target"), TEXT("nf.Target <entity id|nearest_attackable|attacker|last|none>: SetTarget without attacking"),
		FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& Args, UWorld* World)
		{
			UCombatStateSubsystem* Combat = CombatOf(World);
			if (!Combat || Args.Num() != 1)
			{
				UE_LOG(LogNightfallBot, Warning, TEXT("nf.Target: usage nf.Target <entity id|nearest_attackable|attacker|last|none>"));
				return;
			}
			FString Id = Args[0];
			if (Id.Equals(TEXT("none"), ESearchCase::IgnoreCase)) Id.Reset();
			else if (Id.Equals(TEXT("attacker"), ESearchCase::IgnoreCase))
			{
				// The nearest living NPC that swung at us: also one whose spawn said "not attackable"
				// (it was walking home) and that the server made attackable again without an event.
				UGameInstance* GI = GameInstanceOf(World);
				UBotScenarioRunner* Runner = GI ? GI->GetSubsystem<UBotScenarioRunner>() : nullptr;
				Id = Runner ? BotPredicates::NearestAttacker(Runner->MakeContext()) : FString();
				if (Id.IsEmpty())
				{
					UE_LOG(LogNightfallBot, Warning, TEXT("nf.Target attacker: no living NPC that attacked us is in view"));
					return;
				}
			}
			else if (Id.Equals(TEXT("last"), ESearchCase::IgnoreCase))
			{
				// The newest selection the bot saw, also after it cleared (e.g. an NPC that walked home).
				UGameInstance* GI = GameInstanceOf(World);
				UBotScenarioRunner* Runner = GI ? GI->GetSubsystem<UBotScenarioRunner>() : nullptr;
				Id = Runner ? BotPredicates::CurrentOrLastTarget(Runner->MakeContext()) : FString();
				if (Id.IsEmpty())
				{
					UE_LOG(LogNightfallBot, Warning, TEXT("nf.Target last: nothing was selected yet"));
					return;
				}
			}
			else if (Id.Equals(TEXT("nearest_attackable"), ESearchCase::IgnoreCase))
			{
				const FBotContext Context{ GameInstanceOf(World), nullptr };
				Id = BotPredicates::NearestAttackable(Context);
				if (Id.IsEmpty())
				{
					UE_LOG(LogNightfallBot, Warning, TEXT("nf.Target: no attackable entity in view"));
					return;
				}
			}
			const uint32 Seq = Combat->SelectTarget(Id);
			UE_LOG(LogNightfallBot, Display, TEXT("nf.Target %s: %s"), Id.IsEmpty() ? TEXT("none") : *Id,
				Seq ? *FString::Printf(TEXT("SetTarget seq %u"), Seq) : TEXT("nothing sent (already selected, or not connected)"));
		}));

	FAutoConsoleCommandWithWorld AttackCommand(TEXT("nf.Attack"), TEXT("Attack the selection (same rules as clicking it: one Attack until it ends)"),
		FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* World)
		{
			if (UCombatStateSubsystem* Combat = CombatOf(World))
			{
				const uint32 Seq = Combat->AttackSelection();
				UE_LOG(LogNightfallBot, Display, TEXT("nf.Attack: %s"), Seq ? *FString::Printf(TEXT("Attack seq %u"), Seq) : TEXT("nothing sent (no selection, already attacking, dead, or not connected)"));
			}
		}));

	FAutoConsoleCommandWithWorld StopAttackCommand(TEXT("nf.StopAttack"), TEXT("StopAttack while attacking (what a ground click sends)"),
		FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* World)
		{
			if (UCombatStateSubsystem* Combat = CombatOf(World))
			{
				const uint32 Seq = Combat->NoteGroundClick();
				UE_LOG(LogNightfallBot, Display, TEXT("nf.StopAttack: %s"), Seq ? *FString::Printf(TEXT("StopAttack seq %u"), Seq) : TEXT("nothing sent (not attacking)"));
			}
		}));

	FAutoConsoleCommandWithWorld MarkCommand(TEXT("nf.Mark"), TEXT("Remembers the landed-hit counts per target, the baseline of target_hits_since_mark and target_hit_from_full"),
		FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* World)
		{
			UGameInstance* GI = GameInstanceOf(World);
			if (UBotScenarioRunner* Runner = GI ? GI->GetSubsystem<UBotScenarioRunner>() : nullptr)
			{
				Runner->MarkHits();
				UE_LOG(LogNightfallBot, Display, TEXT("nf.Mark: hit counts remembered"));
			}
		}));

	// nf.Respawn (the dead overlay's button) is defined with the playtest commands in Game/PlaytestCommands.cpp.

	/** Interactive nf.Expect / nf.WaitFor: evaluates against the runner's observations and logs the outcome. */
	FBotPredicateFn ParseInteractive(const TArray<FString>& Tokens, const TCHAR* Command)
	{
		FString Error;
		FBotPredicateFn Fn = FBotPredicateRegistry::Get().Parse(Tokens, Error);
		if (!Fn) UE_LOG(LogNightfallBot, Warning, TEXT("%s: %s"), Command, *Error);
		return Fn;
	}

	FAutoConsoleCommandWithWorldAndArgs ExpectCommand(TEXT("nf.Expect"), TEXT("nf.Expect <predicate>: logs whether it holds now (in a scenario: fails the run when it does not)"),
		FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& Args, UWorld* World)
		{
			UGameInstance* GI = GameInstanceOf(World);
			UBotScenarioRunner* Runner = GI ? GI->GetSubsystem<UBotScenarioRunner>() : nullptr;
			const FBotPredicateFn Fn = ParseInteractive(Args, TEXT("nf.Expect"));
			if (!Runner || !Fn) return;
			const FBotPredicateValue V = Fn(Runner->MakeContext());
			UE_LOG(LogNightfallBot, Display, TEXT("nf.Expect %s: %s (observed %s)"), *FString::Join(Args, TEXT(" ")), V.bTrue ? TEXT("true") : TEXT("FALSE"), *V.Observed);
		}));

	FAutoConsoleCommandWithWorldAndArgs WaitForCommand(TEXT("nf.WaitFor"), TEXT("nf.WaitFor <predicate> <timeoutSeconds>: logs when it holds or times out"),
		FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& Args, UWorld* World)
		{
			UGameInstance* GI = GameInstanceOf(World);
			UBotScenarioRunner* Runner = GI ? GI->GetSubsystem<UBotScenarioRunner>() : nullptr;
			double Timeout = 0.0;
			if (!Runner || Args.Num() < 2 || !ParseSeconds(Args.Last(), false, Timeout))
			{
				UE_LOG(LogNightfallBot, Warning, TEXT("nf.WaitFor: usage nf.WaitFor <predicate> <timeoutSeconds>"));
				return;
			}
			TArray<FString> Tokens(Args);
			Tokens.Pop();
			const FBotPredicateFn Fn = ParseInteractive(Tokens, TEXT("nf.WaitFor"));
			if (!Fn) return;
			const FString Text = FString::Join(Tokens, TEXT(" "));
			const double Start = FPlatformTime::Seconds();
			FTSTicker::GetCoreTicker().AddTicker(FTickerDelegate::CreateLambda([Runner = TWeakObjectPtr<UBotScenarioRunner>(Runner), Fn, Text, Start, Timeout](float)
			{
				if (!Runner.IsValid()) return false;
				const FBotPredicateValue V = Fn(Runner->MakeContext());
				const double Waited = FPlatformTime::Seconds() - Start;
				if (V.bTrue || Waited >= Timeout)
				{
					UE_LOG(LogNightfallBot, Display, TEXT("nf.WaitFor %s: %s after %.2f s (observed %s)"), *Text, V.bTrue ? TEXT("true") : TEXT("TIMED OUT"), Waited, *V.Observed);
					return false;
				}
				return true;
			}));
		}));

	FAutoConsoleCommandWithWorldAndArgs SleepCommand(TEXT("nf.Sleep"), TEXT("nf.Sleep <seconds>: a pause between scenario steps (no effect typed by hand)"),
		FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>&, UWorld*) {}));

	FAutoConsoleCommandWithWorldAndArgs WithinCommand(TEXT("nf.Within"), TEXT("nf.Within <seconds>: a scenario's wall-clock budget (no effect typed by hand)"),
		FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>&, UWorld*) {}));
}

namespace BotSteps
{
	TArray<IConsoleObject*> RegisterTestOnlyCommands()
	{
		TArray<IConsoleObject*> Registered;
		// R5: test-only commands exist only in non-shipping builds and only in a -BotScenario run.
		Registered.Add(IConsoleManager::Get().RegisterConsoleCommand(TEXT("nf.DropSocket"),
			TEXT("Test only (-BotScenario): drops the WebSocket as a network failure would; the client reconnects with a fresh ticket"),
			FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* World)
			{
				UGameInstance* GI = GameInstanceOf(World);
				if (UNetClientSubsystem* Net = GI ? GI->GetSubsystem<UNetClientSubsystem>() : nullptr)
				{
					UE_LOG(LogNightfallBot, Display, TEXT("nf.DropSocket: dropping the WebSocket"));
					Net->DropSocketForTesting();
				}
			}), ECVF_Default));
		// Two processes of one -SimGroup that must act as the same account (the replacement
		// scenarios) log in with a token derived from the group id.
		Registered.Add(IConsoleManager::Get().RegisterConsoleCommand(TEXT("nf.LoginGroup"),
			TEXT("Test only (-BotScenario): logs in as the test:<uuid> account derived from -SimGroup, shared by every process of the group"),
			FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* World)
			{
				FString Group;
				UGameInstance* GI = GameInstanceOf(World);
				UAuthSubsystem* Auth = GI ? GI->GetSubsystem<UAuthSubsystem>() : nullptr;
				if (!Auth || !FParse::Value(FCommandLine::Get(), TEXT("SimGroup="), Group) || Group.IsEmpty())
				{
					UE_LOG(LogNightfallBot, Error, TEXT("nf.LoginGroup: needs -SimGroup=<id> (run it with run-sim-multi.sh)"));
					return;
				}
				const FString Digest = FMD5::HashAnsiString(*Group);   // 32 hex digits
				const FString Uuid = FString::Printf(TEXT("%s-%s-%s-%s-%s"), *Digest.Left(8), *Digest.Mid(8, 4), *Digest.Mid(12, 4), *Digest.Mid(16, 4), *Digest.Mid(20, 12));
				UE_LOG(LogNightfallBot, Display, TEXT("nf.LoginGroup: account test:%s"), *Uuid);
				Auth->LoginWithDevToken(FString::Printf(TEXT("test:%s"), *Uuid));
			}), ECVF_Default));
		return Registered;
	}
}
#endif
