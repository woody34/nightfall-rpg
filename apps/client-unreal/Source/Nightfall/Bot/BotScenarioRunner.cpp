#include "BotScenarioRunner.h"
#include "Combat/CombatStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Engine/Engine.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"
#include "Kismet/GameplayStatics.h"
#include "HAL/FileManager.h"
#include "HAL/IConsoleManager.h"
#include "HAL/PlatformMisc.h"
#include "Misc/CommandLine.h"
#include "Misc/FileHelper.h"
#include "Misc/Guid.h"
#include "Misc/Parse.h"
#include "Misc/Paths.h"
#include "Dom/JsonObject.h"

DEFINE_LOG_CATEGORY(LogNightfallBot);

namespace
{
	constexpr int32 MaxSentinelLinesInReport = 50;
}

FString UBotScenarioRunner::ResolveScenarioPath(const FString& Path)
{
	if (!FPaths::IsRelative(Path)) return Path;
	for (const FString& Base : { FPaths::LaunchDir(), FPaths::ProjectDir() })
	{
		const FString Candidate = FPaths::ConvertRelativePathToFull(Base, Path);
		if (FPaths::FileExists(Candidate)) return Candidate;
	}
	return FPaths::ConvertRelativePathToFull(FPaths::LaunchDir(), Path);
}

void UBotScenarioRunner::Initialize(FSubsystemCollectionBase& Collection)
{
	Collection.InitializeDependency<UNetClientSubsystem>();
	Collection.InitializeDependency<UCombatStateSubsystem>();
	Super::Initialize(Collection);

	FString Path;
	if (!FParse::Value(FCommandLine::Get(), TEXT("BotScenario="), Path) || Path.IsEmpty()) return;   // inert
#if UE_BUILD_SHIPPING
	UE_LOG(LogNightfallBot, Warning, TEXT("-BotScenario is ignored in shipping builds"));
	return;
#else
	// Only the first game instance of the process runs the scenario (PIE / tests make more).
	static bool bClaimed = false;
	if (bClaimed) return;
	bClaimed = true;

	bActive = true;
	Diagnostics = MakeUnique<FBotDiagnostics>();
	Diagnostics->Bind(GetGameInstance()->GetSubsystem<UNetClientSubsystem>());
	LaunchSeconds = FPlatformTime::Seconds();
	ScenarioPath = ResolveScenarioPath(Path);
	ScenarioName = FPaths::GetBaseFilename(ScenarioPath);
	if (FParse::Value(FCommandLine::Get(), TEXT("BotLoop="), LoopSeconds)
		&& (!FMath::IsFinite(LoopSeconds) || LoopSeconds <= 0.0))
	{
		ParseErrors.Add(TEXT("BotLoop must be a positive finite duration in seconds"));
	}
	if (!FParse::Value(FCommandLine::Get(), TEXT("BotOutDir="), OutDir) || OutDir.IsEmpty())
	{
		OutDir = FPaths::Combine(FPaths::ProjectSavedDir(), TEXT("Sim"));
	}
	OutDir = FPaths::ConvertRelativePathToFull(FPaths::LaunchDir(), OutDir);
	IFileManager::Get().MakeDirectory(*OutDir, /*Tree*/ true);

	// The run log mirrors every line from here on; the sentinel counts from here on too.
	RunLog.Reset(IFileManager::Get().CreateFileWriter(*FPaths::Combine(OutDir, ScenarioName + TEXT(".log"))));
	Sentinel.SetMirror([this](const FString& Line)
	{
		if (!RunLog) return;
		FTCHARToUTF8 Utf8(*(Line + TEXT("\n")));
		RunLog->Serialize(const_cast<ANSICHAR*>(Utf8.Get()), Utf8.Length());
	});
	Sentinel.Start();

	const FString AllowPath = FPaths::ChangeExtension(ScenarioPath, TEXT("allow"));
	FString AllowText;
	if (FFileHelper::LoadFileToString(AllowText, *AllowPath))
	{
		TArray<FBotAllowEntry> Entries;
		TArray<FString> Errors;
		FBotLogSentinel::ParseAllowList(AllowText, Entries, Errors);
		for (const FString& Error : Errors) ParseErrors.Add(FString::Printf(TEXT("%s: %s"), *FPaths::GetCleanFilename(AllowPath), *Error));
		Sentinel.SetAllowList(MoveTemp(Entries));
	}

	TestOnlyCommands = BotSteps::RegisterTestOnlyCommands();

	FString Text;
	if (!FFileHelper::LoadFileToString(Text, *ScenarioPath))
	{
		ParseErrors.Add(FString::Printf(TEXT("cannot read scenario file %s"), *ScenarioPath));
	}
	else
	{
		FBotScenario Scenario;
		TArray<FString> Errors;
		BotScenario::Parse(ScenarioName, Text, FBotPredicateRegistry::Get(),
			[](const FString& Name) { return IConsoleManager::Get().FindConsoleObject(*Name) != nullptr; }, Scenario, Errors);
		ParseErrors.Append(Errors);
		if (ParseErrors.IsEmpty())
		{
			Executor = MakeUnique<FBotScenarioExecutor>(MoveTemp(Scenario),
				[this](const FString& Line, FString& OutError) { return ExecLine(Line, OutError); },
				[this](const FBotPredicateFn& Predicate) { return Predicate(MakeContext()); },
				[this](const FString& Line) { Log(Line); });
		}
	}

	// A scenario needs a fresh, isolated account (plan D5): without a dev token on the command
	// line, nf.Login uses a new test:<uuid> account (the API must run with AUTH_DEV_TOKENS=1).
	FString Unused;
	if (!FParse::Value(FCommandLine::Get(), TEXT("DevToken="), Unused) && !FParse::Value(FCommandLine::Get(), TEXT("DevTokenFile="), Unused))
	{
		GeneratedAccount = FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower);
		FCommandLine::Append(*FString::Printf(TEXT(" -DevToken=test:%s"), *GeneratedAccount));
	}

	MakeContext();   // start observing acks / rejections / damage numbers before the first step
	UE_LOG(LogNightfallBot, Display, TEXT("bot: scenario %s, artifacts in %s%s"), *ScenarioPath, *OutDir,
		GeneratedAccount.IsEmpty() ? TEXT("") : *FString::Printf(TEXT(", account test:%s"), *GeneratedAccount));
	TickHandle = FTSTicker::GetCoreTicker().AddTicker(FTickerDelegate::CreateUObject(this, &UBotScenarioRunner::Tick));
#endif
}

void UBotScenarioRunner::Deinitialize()
{
	if (bActive && !bCompleted)
	{
		// The engine is going away mid-run (killed, crashed map load, ...): still leave a report.
		if (Executor && bBegun) Executor->Abort(FPlatformTime::Seconds(), TEXT("the engine shut down before the scenario finished"));
		Complete();
	}
	FTSTicker::GetCoreTicker().RemoveTicker(TickHandle);
	for (IConsoleObject* Command : TestOnlyCommands) IConsoleManager::Get().UnregisterConsoleObject(Command);
	TestOnlyCommands.Reset();
	if (bObserving) Observations.Unbind();
	if (Diagnostics) Diagnostics->Unbind();
	Sentinel.Stop();
	RunLog.Reset();
	Super::Deinitialize();
}

FBotContext UBotScenarioRunner::MakeContext()
{
	if (!bObserving)
	{
		Observations.Bind(GetGameInstance());
		bObserving = true;
	}
	return FBotContext{ GetGameInstance(), &Observations };
}

void UBotScenarioRunner::Log(const FString& Line)
{
	UE_LOG(LogNightfallBot, Display, TEXT("bot: %s"), *Line);
}

bool UBotScenarioRunner::ExecLine(const FString& Line, FString& OutError)
{
	if (Diagnostics && Line.StartsWith(TEXT("nf.ClickMove "), ESearchCase::IgnoreCase))
	{
		Diagnostics->BeginMove(FPlatformTime::Seconds());
		Diagnostics->ObservePosition(MakeContext(), FPlatformTime::Seconds());
	}
	UWorld* World = GetGameInstance()->GetWorld();
	if (!World)
	{
		OutError = TEXT("no world to run the command in");
		return false;
	}
	if (IConsoleManager::Get().ProcessUserConsoleInput(*Line, *GLog, World)) return true;
	if (GEngine && GEngine->Exec(World, *Line)) return true;
	OutError = TEXT("no console command or exec handler accepted it");
	return false;
}

bool UBotScenarioRunner::Tick(float DeltaSeconds)
{
	if (bCompleted) return false;
	if (!Executor)
	{
		Complete();   // parse errors: report and exit
		return false;
	}
	if (bResettingWorld)
	{
		const UWorld* World = GetGameInstance()->GetWorld();
		if (!World || !World->HasBegunPlay() || World->GetOutermost()->GetName() != TEXT("/Game/Maps/L_Login")) return true;
		bResettingWorld = false;
	}
	if (!bBegun)
	{
		// Commands need a world (the login map) to run in.
		const UWorld* World = GetGameInstance()->GetWorld();
		if (!World || !World->HasBegunPlay()) return true;
		Begin();
	}
	Observations.Observe(GetGameInstance()->GetSubsystem<UCombatStateSubsystem>());
	if (Diagnostics) Diagnostics->ObservePosition(MakeContext(), FPlatformTime::Seconds());
	if (Executor->Tick(FPlatformTime::Seconds())) return true;
	if (AdvanceLoop()) return true;
	Complete();
	return false;
}

void UBotScenarioRunner::Begin()
{
	bBegun = true;
	if (LoopStartSeconds == 0.0) LoopStartSeconds = FPlatformTime::Seconds();
	Executor->Start(FPlatformTime::Seconds());
}

bool UBotScenarioRunner::AdvanceLoop()
{
	if (LoopSeconds <= 0.0 || !Executor->HasPassed() || Sentinel.NumUnallowed() != 0) return false;
	++LoopIterations;
	for (FBotTestCase Case : Executor->GetTestCases())
	{
		Case.Name = FString::Printf(TEXT("iteration %d: %s"), LoopIterations, *Case.Name);
		LoopCases.Add(MoveTemp(Case));
	}
	// Finish whole scenarios, never turn an unfinished assertion into a passing timeout.
	if (FPlatformTime::Seconds() - LoopStartSeconds >= LoopSeconds) return false;
	Log(FString::Printf(TEXT("loop iteration %d passed; resetting login and projections"), LoopIterations));
	FString Error;
	if (!ExecLine(TEXT("nf.Logout"), Error))
	{
		ParseErrors.Add(TEXT("loop logout failed: ") + Error);
		Executor.Reset();
		return false;
	}
	Observations.Reset();
	if (Diagnostics) Diagnostics->ResetIteration();
	// The fixture begins at the new-character spawn. Avoid carrying position/HP/XP across loops.
	// Explicit user-supplied tokens are retained; generated disposable identities rotate.
	if (!GeneratedAccount.IsEmpty())
	{
		const FString Previous = GeneratedAccount;
		GeneratedAccount = FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower);
		const FString CommandLine = FString(FCommandLine::Get()).Replace(*Previous, *GeneratedAccount);
		FCommandLine::Set(*CommandLine);
	}
	FBotScenario Scenario = Executor->GetScenario();
	Executor = MakeUnique<FBotScenarioExecutor>(MoveTemp(Scenario),
		[this](const FString& Line, FString& OutError) { return ExecLine(Line, OutError); },
		[this](const FBotPredicateFn& Predicate) { return Predicate(MakeContext()); },
		[this](const FString& Line) { Log(Line); });
	bBegun = false;
	// nf.Logout disconnects but does not destroy actors. Reload the login world to remove
	// old proxies and pawn state, and wait for its BeginPlay before the scenario's nf.Login.
	bResettingWorld = true;
	UGameplayStatics::OpenLevel(GetGameInstance()->GetWorld(), FName(TEXT("/Game/Maps/L_Login")));
	return true;
}

TArray<FBotTestCase> UBotScenarioRunner::ReportCases(const FBotScenarioExecutor& Executor, const FBotLogSentinel& Sentinel)
{
	TArray<FBotTestCase> Cases = Executor.GetTestCases();
	FBotTestCase& Case = Cases.AddDefaulted_GetRef();
	Case.Name = TEXT("log and ensure sentinel");
	Case.Line = TNumericLimits<int32>::Max();
	const TArray<FString> Unallowed = Sentinel.GetUnallowed();
	if (Unallowed.IsEmpty())
	{
		Case.Status = FBotTestCase::EStatus::Passed;
		return Cases;
	}
	Case.Status = FBotTestCase::EStatus::Failed;
	Case.Message = FString::Printf(TEXT("%d unallowed error line(s) or ensure(s); first: %s"), Unallowed.Num(), *Unallowed[0]);
	TArray<FString> Shown(Unallowed);
	if (Shown.Num() > MaxSentinelLinesInReport)
	{
		Shown.SetNum(MaxSentinelLinesInReport);
		Shown.Add(FString::Printf(TEXT("... and %d more (see the run log)"), Unallowed.Num() - MaxSentinelLinesInReport));
	}
	Case.Detail = FString::Join(Shown, TEXT("\n"));
	return Cases;
}

void UBotScenarioRunner::Complete()
{
	if (bCompleted) return;
	bCompleted = true;

	TArray<FBotTestCase> Cases;
	double Seconds = FPlatformTime::Seconds() - LaunchSeconds;
	bool bStepsPassed = false;
	if (Executor && bBegun)
	{
		Cases = ReportCases(*Executor, Sentinel);
		if (LoopSeconds > 0.0)
		{
			// Successful final iteration is already accumulated; retain its sentinel only.
			if (Executor->HasPassed() && Sentinel.NumUnallowed() == 0 && LoopIterations > 0)
			{
				FBotTestCase SentinelCase = Cases.Last();
				Cases = LoopCases;
				Cases.Add(MoveTemp(SentinelCase));
			}
			else Cases.Insert(LoopCases, 0);
		}
		Seconds = Executor->ElapsedSeconds();
		bStepsPassed = Executor->HasPassed();
	}
	else
	{
		Cases = LoopCases;
		FBotTestCase& Case = Cases.AddDefaulted_GetRef();
		Case.Name = ParseErrors.IsEmpty() ? TEXT("start scenario") : TEXT("parse scenario");
		Case.Status = FBotTestCase::EStatus::Failed;
		Case.Message = ParseErrors.IsEmpty() ? FString(TEXT("the engine exited before the world loaded")) : ParseErrors[0];
		Case.Detail = FString::Join(ParseErrors, TEXT("\n"));
		for (const FString& Error : ParseErrors) UE_LOG(LogNightfallBot, Display, TEXT("bot: scenario error: %s"), *Error);
	}
	const int32 Unallowed = Sentinel.NumUnallowed();
	const int32 ExitCode = ExitCodeFor(bStepsPassed, Unallowed);
	for (const FString& Line : Sentinel.GetUnallowed()) UE_LOG(LogNightfallBot, Display, TEXT("bot: sentinel: %s"), *Line);
	UE_LOG(LogNightfallBot, Display, TEXT("bot: %s %s (steps %s, %d unallowed log line(s)/ensure(s)); exit code %d"),
		*ScenarioName, ExitCode == 0 ? TEXT("PASSED") : TEXT("FAILED"),
		bStepsPassed ? TEXT("passed") : Executor ? *Executor->GetFailure() : TEXT("not run"), Unallowed, ExitCode);
	WriteArtifacts(Cases, LoopSeconds > 0.0 && LoopStartSeconds > 0.0 ? FPlatformTime::Seconds() - LoopStartSeconds : Seconds, ExitCode);

	// Always: on Linux this is what sets the process return code, also when the engine is already
	// exiting (killed or quit mid-run) and a second exit request changes nothing else.
	FPlatformMisc::RequestExitWithStatus(false, static_cast<uint8>(ExitCode), TEXT("UBotScenarioRunner"));
}

void UBotScenarioRunner::WriteArtifacts(const TArray<FBotTestCase>& Cases, double TotalSeconds, int32 ExitCode)
{
	TArray<FBotTestCase> ReportCasesWithDiagnostics = Cases;
	const UNetClientSubsystem* Net = GetGameInstance()->GetSubsystem<UNetClientSubsystem>();
	TArray<TPair<FString, FString>> Properties;
	Properties.Emplace(TEXT("scenario_file"), ScenarioPath);
	Properties.Emplace(TEXT("exit_code"), FString::FromInt(ExitCode));
	if (LoopSeconds > 0.0)
	{
		Properties.Emplace(TEXT("loop_seconds"), FString::SanitizeFloat(LoopSeconds));
		Properties.Emplace(TEXT("loop_iterations"), FString::FromInt(LoopIterations));
	}
	if (Executor) Properties.Emplace(TEXT("budget_seconds"), FString::SanitizeFloat(Executor->GetScenario().BudgetSeconds));
	Properties.Emplace(TEXT("sentinel_unallowed"), FString::FromInt(Sentinel.NumUnallowed()));
	Properties.Emplace(TEXT("sentinel_allowed"), FString::FromInt(Sentinel.NumAllowed()));
	for (const FBotAllowEntry& Entry : Sentinel.GetAllowList())
	{
		Properties.Emplace(FString::Printf(TEXT("allow.%d"), Entry.Line), FString::Printf(TEXT("%s | %s | %s | hits=%d"), *Entry.Category, *Entry.Substring, *Entry.Reason, Entry.Hits));
	}
	if (!GeneratedAccount.IsEmpty()) Properties.Emplace(TEXT("account"), TEXT("test:") + GeneratedAccount);
	// After nf.Logout the net cache has forgotten the entity; the observation kept its first id.
	const FString OwnEntity = Net && !Net->GetOwnEntityId().IsEmpty() ? Net->GetOwnEntityId() : Observations.FirstOwnEntityId;
	if (!OwnEntity.IsEmpty()) Properties.Emplace(TEXT("own_entity_id"), OwnEntity);
	if (Diagnostics)
	{
		Diagnostics->AddJUnitProperties(Properties);
		const FString CoverageName = ScenarioName + TEXT(".coverage.contract.json");
		FFileHelper::SaveStringToFile(FBotDiagnostics::Json(Diagnostics->Coverage()), *FPaths::Combine(OutDir, CoverageName),
			FFileHelper::EEncodingOptions::ForceUTF8WithoutBOM);
		Properties.Emplace(TEXT("contract_coverage"), CoverageName);
		if (ExitCode != 0)
		{
			const FBotTestCase* FailedCase = Cases.FindByPredicate([](const FBotTestCase& Case) { return Case.Status == FBotTestCase::EStatus::Failed; });
			FBotFailedStep Step;
			if (Executor && Executor->GetFailedStep().IsSet()) Step = Executor->GetFailedStep().GetValue();
			else if (FailedCase) { Step.Line = FailedCase->Line; Step.Source = FailedCase->Name; Step.Observed = FailedCase->Message; Step.WaitSeconds = FailedCase->Seconds; }
			const FString FailureName = ScenarioName + TEXT(".failure.json");
			const FString Message = FailedCase ? FailedCase->Message : TEXT("scenario failed");
			const auto Bundle = Diagnostics->FailureBundle(ScenarioName, OwnEntity, MakeContext(), Step, Message);
			FFileHelper::SaveStringToFile(FBotDiagnostics::Json(Bundle), *FPaths::Combine(OutDir, FailureName),
				FFileHelper::EEncodingOptions::ForceUTF8WithoutBOM);
			Properties.Emplace(TEXT("failure_bundle"), FailureName);
			Properties.Emplace(TEXT("failure_tick"), Bundle->GetStringField(TEXT("tick_last")));
			const FString Summary = FString::Printf(TEXT("Failure bundle: %s; entity=%s; tick=%s; predicate=%s; last_value=%s; wait=%.3fs\nTrace: %s.trace.html"),
				*FailureName, *OwnEntity, *Bundle->GetStringField(TEXT("tick_last")), *Step.Predicate, *Step.Observed, Step.WaitSeconds, *ScenarioName);
			for (FBotTestCase& Case : ReportCasesWithDiagnostics)
				if (Case.Status == FBotTestCase::EStatus::Failed) Case.Detail += TEXT("\n") + Summary;
		}
	}

	const FString JUnitPath = FPaths::Combine(OutDir, ScenarioName + TEXT(".xml"));
	if (!FFileHelper::SaveStringToFile(BotJUnit::Write(ScenarioName, ReportCasesWithDiagnostics, TotalSeconds, Properties), *JUnitPath, FFileHelper::EEncodingOptions::ForceUTF8WithoutBOM))
	{
		UE_LOG(LogNightfallBot, Display, TEXT("bot: could not write %s"), *JUnitPath);
	}
	UE_LOG(LogNightfallBot, Display, TEXT("bot: wrote %s"), *JUnitPath);
	if (RunLog) RunLog->Flush();
}
