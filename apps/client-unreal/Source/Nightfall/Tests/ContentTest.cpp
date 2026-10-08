#include "Misc/AutomationTest.h"
#include "NightfallPlayerController.h"
#include "Game/NightfallGameMode.h"
#include "Net/NetSettings.h"
#include "UI/NightfallLoginScreen.h"
#include "World/RemoteEntityActor.h"
#include "Components/StaticMeshComponent.h"
#include "Engine/Level.h"
#include "Engine/StaticMesh.h"
#include "Engine/StaticMeshActor.h"
#include "Engine/World.h"
#include "GameFramework/PlayerStart.h"
#include "GameFramework/WorldSettings.h"
#include "GameMapsSettings.h"
#include "InputAction.h"
#include "InputMappingContext.h"
#include "NavMesh/NavMeshBoundsVolume.h"

#if WITH_DEV_AUTOMATION_TESTS

// The Story 6.2 content (Scripts/create_content.py) is wired together the way the code expects.

namespace
{
	template <typename T>
	T* GetObjectProperty(UClass* Class, const UObject* Container, const TCHAR* Name)
	{
		const FObjectPropertyBase* Property = FindFProperty<FObjectPropertyBase>(Class, Name);
		return Property != nullptr ? Cast<T>(Property->GetObjectPropertyValue_InContainer(Container)) : nullptr;
	}
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FContentWiringTest, "Nightfall.Content.Wiring",
	EAutomationTestFlags::EditorContext | EAutomationTestFlags::EngineFilter)

bool FContentWiringTest::RunTest(const FString& Parameters)
{
	UClass* RemoteClass = LoadClass<ARemoteEntityActor>(nullptr, TEXT("/Game/Blueprints/BP_RemoteEntity.BP_RemoteEntity_C"));
	UClass* PcClass = LoadClass<ANightfallPlayerController>(nullptr, TEXT("/Game/Blueprints/BP_NightfallPC.BP_NightfallPC_C"));
	UClass* GmClass = LoadClass<ANightfallGameMode>(nullptr, TEXT("/Game/Blueprints/BP_NightfallGameMode.BP_NightfallGameMode_C"));
	UClass* LoginClass = LoadClass<UNightfallLoginScreen>(nullptr, TEXT("/Game/UI/WBP_Login.WBP_Login_C"));
	UInputAction* Click = LoadObject<UInputAction>(nullptr, TEXT("/Game/Input/IA_ClickMove.IA_ClickMove"));
	UInputMappingContext* Imc = LoadObject<UInputMappingContext>(nullptr, TEXT("/Game/Input/IMC_Default.IMC_Default"));
	if (!TestNotNull(TEXT("BP_RemoteEntity"), RemoteClass) || !TestNotNull(TEXT("BP_NightfallPC"), PcClass)
		|| !TestNotNull(TEXT("BP_NightfallGameMode"), GmClass) || !TestNotNull(TEXT("WBP_Login"), LoginClass)
		|| !TestNotNull(TEXT("IA_ClickMove"), Click) || !TestNotNull(TEXT("IMC_Default"), Imc))
	{
		return false;
	}

	const ARemoteEntityActor* Remote = GetDefault<ARemoteEntityActor>(RemoteClass);
	TestTrue(TEXT("BP_RemoteEntity has a placeholder mesh"), Remote->Body != nullptr && Remote->Body->GetStaticMesh() != nullptr);

	const UObject* Pc = PcClass->GetDefaultObject();
	TestEqual(TEXT("BP_NightfallPC mapping context"), GetObjectProperty<UInputMappingContext>(PcClass, Pc, TEXT("DefaultMappingContext")), Imc);
	TestEqual(TEXT("BP_NightfallPC click action"), GetObjectProperty<UInputAction>(PcClass, Pc, TEXT("ClickMoveAction")), Click);
	TestTrue(TEXT("IMC_Default maps left mouse to IA_ClickMove"), Imc->GetMappings().ContainsByPredicate([&](const FEnhancedActionKeyMapping& M)
	{
		return M.Action == Click && M.Key == EKeys::LeftMouseButton;
	}));

	const ANightfallGameMode* Gm = GetDefault<ANightfallGameMode>(GmClass);
	TestEqual(TEXT("game mode uses BP_NightfallPC"), Gm->PlayerControllerClass.Get(), PcClass);
	TestEqual(TEXT("game mode EntityClass is BP_RemoteEntity"), Gm->EntityClass.Get(), RemoteClass);

	UWorld* TestZone = LoadObject<UWorld>(nullptr, TEXT("/Game/Maps/L_TestZone.L_TestZone"));
	if (TestNotNull(TEXT("L_TestZone"), TestZone) && TestNotNull(TEXT("L_TestZone level"), TestZone->PersistentLevel.Get()))
	{
		TestEqual(TEXT("L_TestZone game mode"), TestZone->PersistentLevel->GetWorldSettings()->DefaultGameMode.Get(), GmClass);
		bool bStart = false, bNav = false, bGround = false;
		for (const AActor* Actor : TestZone->PersistentLevel->Actors)
		{
			bStart |= Actor != nullptr && Actor->IsA<APlayerStart>();
			bNav |= Actor != nullptr && Actor->IsA<ANavMeshBoundsVolume>();
			if (const AStaticMeshActor* Mesh = Cast<AStaticMeshActor>(Actor))
			{
				// 256 x 256 tiles of 100 cm, origin at tile (0, 0).
				const FBox Bounds = Mesh->GetComponentsBoundingBox();
				bGround |= FMath::IsNearlyEqual(Bounds.GetSize().X, 25600.0, 1.0) && FMath::IsNearlyEqual(Bounds.Min.Y, 0.0, 1.0);
			}
		}
		TestTrue(TEXT("L_TestZone has a PlayerStart"), bStart);
		TestTrue(TEXT("L_TestZone has nav bounds"), bNav);
		TestTrue(TEXT("L_TestZone has 25600 cm ground from (0, 0)"), bGround);
	}

	UWorld* Login = LoadObject<UWorld>(nullptr, TEXT("/Game/Maps/L_Login.L_Login"));
	if (TestNotNull(TEXT("L_Login"), Login) && TestNotNull(TEXT("L_Login level"), Login->PersistentLevel.Get()))
	{
		TestEqual(TEXT("L_Login game mode"), Login->PersistentLevel->GetWorldSettings()->DefaultGameMode.Get(),
			ANightfallLoginGameMode::StaticClass());
	}

	TestEqual(TEXT("L_Login is the game's default map"), FSoftObjectPath(UGameMapsSettings::GetGameDefaultMap()).GetLongPackageName(), FString(TEXT("/Game/Maps/L_Login")));
	// Ini values containing "//" must be quoted or the parser truncates them.
	TestEqual(TEXT("OidcIssuer read intact from DefaultGame.ini"), GetDefault<UNetSettings>()->OidcIssuer,
		FString(TEXT("http://localhost:8080/realms/nightfall")));
	TestEqual(TEXT("world map setting"), GetDefault<UNetSettings>()->WorldMap.ToString(), FString(TEXT("/Game/Maps/L_TestZone.L_TestZone")));
	return true;
}

#endif
