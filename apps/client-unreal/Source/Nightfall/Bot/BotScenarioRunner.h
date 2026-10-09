#pragma once

#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "Containers/Ticker.h"
#include "BotPredicates.h"
#include "BotSteps.h"
#include "BotLogSentinel.h"
#include "BotDiagnostics.h"
#include "BotScenarioRunner.generated.h"

DECLARE_LOG_CATEGORY_EXTERN(LogNightfallBot, Log, All);

class IConsoleObject;

/**
 * Headless simulation driver (Phase 1a E1.1, plan D2). With `-BotScenario=<path.nfs>` on the
 * command line it parses the scenario, runs it one step per game-thread tick, writes
 * `<out>/<scenario>.xml` (JUnit) and `<out>/<scenario>.log` (the run log: every log line plus the
 * step trace), and exits the process with 0 (passed) or 1 (anything failed). Without the flag it
 * does nothing at all, so the shipped game is unchanged; shipping builds ignore the flag.
 *
 * Command line (the contract scripts rely on; README "Simulation"):
 *   -BotScenario=<path>   scenario file; absolute, or relative to the launch dir, then the project dir
 *   -BotOutDir=<dir>      artifact directory (default <Project>/Saved/Sim)
 *   -DevToken / -DevTokenFile as for a normal run; when neither is given, the runner logs in as a
 *                         fresh `test:<uuid>` account (the API must run with AUTH_DEV_TOKENS=1)
 *
 * Pass criteria (D6): every step passed, nf.Within held, and the log sentinel saw no Error / Fatal
 * line in LogNightfall, LogTurboLink, LogNet* and no ensure that `<scenario>.allow` does not cover.
 */
UCLASS()
class NIGHTFALL_API UBotScenarioRunner : public UGameInstanceSubsystem
{
	GENERATED_BODY()

public:
	virtual void Initialize(FSubsystemCollectionBase& Collection) override;
	virtual void Deinitialize() override;

	/** True when this process runs a scenario. */
	bool IsActive() const { return bActive; }

	/** The predicate context over this game instance; binds the observations on first use. */
	FBotContext MakeContext();
	/** nf.Mark: hit-count baseline for target_hits_since_mark / target_hit_from_full. */
	void MarkHits() { MakeContext(); Observations.MarkHits(); }

	const FBotObservations& GetObservations() const { return Observations; }

	/** Exit code for a finished run: 0 only when the steps passed and the sentinel is clean. */
	static int32 ExitCodeFor(bool bStepsPassed, int32 UnallowedLogLines) { return bStepsPassed && UnallowedLogLines == 0 ? 0 : 1; }

	/**
	 * The scenario's testcases plus the sentinel testcase ("log and ensure sentinel": failed when
	 * any unallowed line or ensure was seen, each named in the failure body).
	 */
	static TArray<FBotTestCase> ReportCases(const FBotScenarioExecutor& Executor, const FBotLogSentinel& Sentinel);

	/** Finds the scenario file: absolute, else under the launch dir, else under the project dir. */
	static FString ResolveScenarioPath(const FString& Path);

private:
	bool Tick(float DeltaSeconds);
	void Begin();
	void Complete();
	bool AdvanceLoop();
	void WriteArtifacts(const TArray<FBotTestCase>& Cases, double TotalSeconds, int32 ExitCode);
	bool ExecLine(const FString& Line, FString& OutError);
	void Log(const FString& Line);

	bool bActive = false;
	bool bBegun = false;
	bool bCompleted = false;
	bool bResettingWorld = false;
	FString ScenarioPath;
	FString ScenarioName;
	FString OutDir;
	FString GeneratedAccount;   // the test:<uuid> account this run created, if any
	TArray<FString> ParseErrors;

	TUniquePtr<FBotScenarioExecutor> Executor;
	FBotLogSentinel Sentinel;
	FBotObservations Observations;
	TUniquePtr<FBotDiagnostics> Diagnostics;
	bool bObserving = false;
	TUniquePtr<FArchive> RunLog;
	FTSTicker::FDelegateHandle TickHandle;
	TArray<IConsoleObject*> TestOnlyCommands;
	double LaunchSeconds = 0.0;
	double LoopSeconds = 0.0;
	double LoopStartSeconds = 0.0;
	int32 LoopIterations = 0;
	TArray<FBotTestCase> LoopCases;
};
