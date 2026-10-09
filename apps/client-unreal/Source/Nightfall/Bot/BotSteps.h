#pragma once

#include "CoreMinimal.h"
#include "BotPredicates.h"

class FBotPredicateRegistry;

/**
 * Scenario files (`Scenarios/<name>.nfs`, plan D3): one step per line, `#` starts a comment, blank
 * lines are ignored. The runner handles nf.WaitFor / nf.Expect / nf.Sleep / nf.Within itself;
 * every other line is a console command (nf.Login, nf.EnterWorld, nf.ClickMove, nf.Target, ...).
 */
enum class EBotStepKind : uint8
{
	Command,   // any console command; nf.* names must exist when the scenario is parsed
	WaitFor,   // nf.WaitFor <predicate> <timeoutSeconds>: polled every tick until true or timed out
	Expect,    // nf.Expect <predicate>: must hold when the step runs
	Sleep,     // nf.Sleep <seconds>
	Within,    // nf.Within <seconds>: wall-clock budget of the whole scenario, checked at exit
};

struct FBotStep
{
	int32 Line = 0;            // 1-based line in the file
	FString Source;            // the line without its comment, trimmed
	EBotStepKind Kind = EBotStepKind::Command;
	FString PredicateText;     // WaitFor / Expect
	FBotPredicateFn Predicate;
	double Seconds = 0.0;      // WaitFor timeout, Sleep duration, Within budget

	/** WaitFor and Expect are JUnit testcases. */
	bool IsAssertion() const { return Kind == EBotStepKind::WaitFor || Kind == EBotStepKind::Expect; }
};

struct FBotScenario
{
	FString Name;              // file name without extension; names the JUnit suite and artifacts
	TArray<FBotStep> Steps;
	double BudgetSeconds = 0.0;   // nf.Within; 0 = no budget
	int32 BudgetLine = 0;
};

namespace BotScenario
{
	/** True when a console command of that name exists; injected so tests need no console. */
	using FCommandExists = TFunction<bool(const FString& CommandName)>;

	/**
	 * Parses a scenario. Every error is collected (line: message); the scenario is usable only
	 * when OutErrors is empty. Bad predicate names and arguments, missing or bad timeouts, unknown
	 * nf.* commands and a second nf.Within are errors here, never at run time.
	 */
	NIGHTFALL_API bool Parse(const FString& Name, const FString& Text, const FBotPredicateRegistry& Predicates,
		const FCommandExists& CommandExists, FBotScenario& Out, TArray<FString>& OutErrors);

	/** Removes a trailing `# comment` and surrounding whitespace. */
	NIGHTFALL_API FString StripComment(const FString& Line);
}

/** One JUnit testcase. */
struct FBotTestCase
{
	enum class EStatus : uint8 { Passed, Failed, Skipped };

	FString Name;              // "L12 nf.WaitFor target_hp == 0 60"
	int32 Line = 0;
	EStatus Status = EStatus::Skipped;
	double Seconds = 0.0;
	FString Message;           // failure / skip reason (one line)
	FString Detail;            // failure body
	FString FailureType = TEXT("failure"); // explicit predicate failures may be quarantined; infrastructure stays generic
};

/** Structured first failure; never re-evaluate a predicate while writing diagnostic artifacts. */
struct FBotFailedStep
{
	int32 Line = 0;
	FString Source;
	FString Predicate;
	FString Observed;
	double WaitSeconds = 0.0;
	double ElapsedSeconds = 0.0;
};

/**
 * Runs a parsed scenario one tick at a time. Steps run in order; commands, Expects and finished
 * sleeps run back to back in one tick, a WaitFor polls once per tick and, when it holds, the
 * next step runs on the following tick (so events that arrive in the same frame burst are all
 * applied before an Expect reads them). The first failing step ends the run; every assertion
 * after it is reported as skipped. The engine is reached only through the two functions passed
 * in, so tests drive it with a fake clock.
 */
class NIGHTFALL_API FBotScenarioExecutor
{
public:
	/** Runs a console command line; false (with a reason) fails the step. */
	using FExecFn = TFunction<bool(const FString& CommandLine, FString& OutError)>;
	/** Evaluates a bound predicate against the live projections. */
	using FEvalFn = TFunction<FBotPredicateValue(const FBotPredicateFn& Predicate)>;
	/** Receives one run-log line (step start, result, timing). */
	using FTraceFn = TFunction<void(const FString& Line)>;

	FBotScenarioExecutor(FBotScenario InScenario, FExecFn InExec, FEvalFn InEval, FTraceFn InTrace = nullptr);

	void Start(double NowSeconds);

	/** Advances; returns true while the scenario is still running. */
	bool Tick(double NowSeconds);

	/** Ends the run as failed from outside (e.g. the engine is shutting down mid-scenario). */
	void Abort(double NowSeconds, const FString& Reason);

	bool IsFinished() const { return bFinished; }
	bool HasPassed() const { return bFinished && !bFailed; }
	double ElapsedSeconds() const { return EndSeconds - StartSeconds; }
	const FBotScenario& GetScenario() const { return Scenario; }
	int32 GetCurrentStep() const { return Current; }

	/** One per WaitFor / Expect (passed, failed or skipped), plus nf.Within when present and a failed command step. */
	const TArray<FBotTestCase>& GetTestCases() const { return Cases; }
	/** Step that failed the run, its message; empty when passed. */
	const FString& GetFailure() const { return Failure; }
	const TOptional<FBotFailedStep>& GetFailedStep() const { return FailedStep; }

private:
	void FailStep(const FBotStep& Step, double Now, const FString& Message, const FString& Detail);
	void Finish(double Now);
	void Trace(double Now, const FString& Text) const;
	FBotTestCase& CaseFor(int32 StepIndex);

	FBotScenario Scenario;
	FExecFn Exec;
	FEvalFn Eval;
	FTraceFn TraceFn;

	TArray<FBotTestCase> Cases;
	TMap<int32, int32> CaseOfStep;   // step index -> Cases index
	int32 Current = 0;
	double StepStartSeconds = -1.0;  // < 0: current step not started
	double StartSeconds = 0.0;
	double EndSeconds = 0.0;
	bool bStarted = false;
	bool bFinished = false;
	bool bFailed = false;
	FString Failure;
	TOptional<FBotFailedStep> FailedStep;
	FString LastObserved;
};

namespace BotJUnit
{
	/**
	 * JUnit XML: <testsuites><testsuite name=Scenario ...><properties/><testcase classname="sim.<scenario>" .../></testsuite></testsuites>.
	 * Properties are emitted in order.
	 */
	NIGHTFALL_API FString Write(const FString& Scenario, const TArray<FBotTestCase>& Cases, double TotalSeconds,
		const TArray<TPair<FString, FString>>& Properties);

	NIGHTFALL_API FString Escape(const FString& Text);
}

#if !UE_BUILD_SHIPPING
class IConsoleObject;

namespace BotSteps
{
	/**
	 * Registers the test-only commands (nf.DropSocket). The runner calls it only when
	 * -BotScenario is on the command line and unregisters them on shutdown (plan R5).
	 */
	TArray<IConsoleObject*> RegisterTestOnlyCommands();
}
#endif
