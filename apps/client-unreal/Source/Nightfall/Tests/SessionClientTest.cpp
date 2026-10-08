#include "Misc/AutomationTest.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"
#include "Engine/Engine.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"
#include "HAL/PlatformProcess.h"
#include "Nightfall.h"
#include "Net/SessionClientSubsystem.h"
#include "TurboLinkGrpcManager.h"

#if WITH_DEV_AUTOMATION_TESTS

// Live round trip against the Rust API. Needs `moon run api:dev` (gRPC on the endpoint in
// [/Script/Nightfall.NetSettings], localhost:50051 by default). Fails, by design, without it.

namespace
{
	/** [workspace.package] version from the repo's Cargo.toml; the API reports it as server_version. */
	FString ReadWorkspaceVersion()
	{
		const FString CargoToml = FPaths::Combine(FPaths::ProjectDir(), TEXT("../../Cargo.toml"));
		TArray<FString> Lines;
		if (!FFileHelper::LoadFileToStringArray(Lines, *CargoToml))
		{
			return FString();
		}
		bool bInWorkspacePackage = false;
		for (const FString& Raw : Lines)
		{
			const FString Line = Raw.TrimStartAndEnd();
			if (Line.StartsWith(TEXT("[")))
			{
				bInWorkspacePackage = Line == TEXT("[workspace.package]");
				continue;
			}
			FString Key, Value;
			if (bInWorkspacePackage && Line.Split(TEXT("="), &Key, &Value) && Key.TrimEnd() == TEXT("version"))
			{
				return Value.TrimStartAndEnd().TrimQuotes();
			}
		}
		return FString();
	}
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSessionClientPingTest, "Nightfall.Net.SessionClient.Ping",
	EAutomationTestFlags::EditorContext | EAutomationTestFlags::EngineFilter)

bool FSessionClientPingTest::RunTest(const FString& Parameters)
{
	// A standalone game instance gives the subsystems (TurboLink's manager, our client) a real
	// owner, exactly as in a packaged game.
	UGameInstance* GameInstance = NewObject<UGameInstance>(GEngine);
	GameInstance->AddToRoot();
	GameInstance->InitializeStandalone();
	ON_SCOPE_EXIT
	{
		UWorld* World = GameInstance->GetWorld();
		GameInstance->Shutdown();
		if (World != nullptr)
		{
			GEngine->DestroyWorldContext(World);
			World->DestroyWorld(false);
		}
		GameInstance->RemoveFromRoot();
	};

	USessionClient* Session = GameInstance->GetSubsystem<USessionClient>();
	if (!TestNotNull(TEXT("SessionClient subsystem"), Session) || !TestNotNull(TEXT("gRPC manager"), Session->GetGrpcManager()))
	{
		return false;
	}

	bool bDone = false;
	FNetResult Result;
	FGrpcNightfallV1PingResponse Response;
	Session->Ping([&](const FNetResult& InResult, const FGrpcNightfallV1PingResponse& InResponse)
	{
		bDone = true;
		Result = InResult;
		Response = InResponse;
	});

	// TurboLink completes calls from its tick. Pump it directly: an editor running tests has no
	// game world ticking this instance. The call itself carries a 5 s deadline.
	const double GiveUpAt = FPlatformTime::Seconds() + 10.0;
	while (!bDone && FPlatformTime::Seconds() < GiveUpAt)
	{
		Session->GetGrpcManager()->Tick(0.01f);
		FPlatformProcess::Sleep(0.01f);
	}

	if (!TestTrue(TEXT("Ping completed"), bDone))
	{
		return false;
	}
	if (!TestEqual(FString::Printf(TEXT("Ping status %s (%s); is the API running?"),
			*UEnum::GetValueAsString(Result.Error), *Result.Message), Result.Error, ENetError::None))
	{
		return false;
	}
	UE_LOG(LogNightfall, Display, TEXT("Ping: %s replied server_version=%s server_time_ms=%lld"),
		*Session->GetEndpoint(), *Response.ServerVersion, Response.ServerTimeMs);

	const FString Expected = ReadWorkspaceVersion();
	if (Expected.IsEmpty())
	{
		AddWarning(TEXT("Cargo.toml not found next to the project; only checking server_version is non-empty"));
		TestFalse(TEXT("server_version non-empty"), Response.ServerVersion.IsEmpty());
	}
	else
	{
		TestEqual(TEXT("server_version matches Cargo workspace version"), Response.ServerVersion, Expected);
	}
	TestTrue(TEXT("server_time_ms is a plausible unix time"), Response.ServerTimeMs > 1'600'000'000'000LL);
	return true;
}

#endif
