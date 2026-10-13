#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "Game/NightfallCharacter.h"
#include "Game/NightfallGameMode.h"
#include "Items/EquipmentPresentationComponent.h"
#include "World/RemoteEntityActor.h"
#include "Components/StaticMeshComponent.h"
#include "Engine/World.h"
#include "Misc/OutputDevice.h"

#if WITH_DEV_AUTOMATION_TESTS

// Integration defect T2: the equipment placeholders were nested default subobjects of the equipment
// component, so instancing a Blueprint subclass (BP_RemoteEntity_C) attached them to the CDO's template
// component and tripped SceneComponent's "Template Mismatch during attachment" ensure. Spawn every
// shipped proxy Blueprint and the player pawn in a real game world, then check the attachment
// structurally (an ensure fires once per process, so a log check alone can be pre-empted) and that no
// ensure or error line was logged while they spawned.
namespace EquipmentSpawnTest
{
 struct FSentinel final : public FOutputDevice
 {
  TArray<FString> Lines;
  FSentinel() { GLog->AddOutputDevice(this); }
  ~FSentinel() override { GLog->RemoveOutputDevice(this); }
  void Serialize(const TCHAR* Message,ELogVerbosity::Type Verbosity,const FName& Category) override
  {
   const FString Line(Message);
   if((Verbosity&ELogVerbosity::VerbosityMask)<=ELogVerbosity::Error || Line.Contains(TEXT("Ensure condition failed")) || Line.Contains(TEXT("Template Mismatch")))
    Lines.Add(Category.ToString()+TEXT(": ")+Line);
  }
 };
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEquipmentSpawnedProxiesTest,"Nightfall.Items.EquipmentPresentation.SpawnedProxies",
 EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FEquipmentSpawnedProxiesTest::RunTest(const FString& Parameters)
{
 UClass* GmClass=LoadClass<ANightfallGameMode>(nullptr,TEXT("/Game/Blueprints/BP_NightfallGameMode.BP_NightfallGameMode_C"));
 TArray<UClass*> Classes;
 for(const TCHAR* Path:{TEXT("/Game/Blueprints/BP_RemoteEntity.BP_RemoteEntity_C"),TEXT("/Game/Blueprints/BP_RemotePlayer.BP_RemotePlayer_C"),TEXT("/Game/Blueprints/BP_Keltir.BP_Keltir_C")})
 {
  UClass* C=LoadClass<ARemoteEntityActor>(nullptr,Path);if(!TestNotNull(Path,C))return false;Classes.Add(C);
 }
 if(!TestNotNull(TEXT("BP_NightfallGameMode"),GmClass))return false;
 UClass* PawnClass=GetDefault<ANightfallGameMode>(GmClass)->DefaultPawnClass.Get();
 if(!PawnClass || !PawnClass->IsChildOf(ANightfallCharacter::StaticClass()))PawnClass=ANightfallCharacter::StaticClass();
 Classes.Add(PawnClass);

 FScopedTestGameInstance Scope;UWorld* World=Scope.GameInstance->GetWorld();
 if(!TestNotNull(TEXT("test world"),World))return false;
 EquipmentSpawnTest::FSentinel Sentinel;
 for(UClass* C:Classes)
 {
  const FString Name=C->GetName();
  FActorSpawnParameters P;P.SpawnCollisionHandlingOverride=ESpawnActorCollisionHandlingMethod::AlwaysSpawn;
  AActor* A=World->SpawnActor<AActor>(C,FVector(100,100,96),FRotator::ZeroRotator,P);
  if(!TestNotNull(*FString::Printf(TEXT("%s spawned"),*Name),A))continue;
  UEquipmentPresentationComponent* E=A->FindComponentByClass<UEquipmentPresentationComponent>();
  if(!TestNotNull(*FString::Printf(TEXT("%s has equipment presentation"),*Name),E))continue;
  TestFalse(*FString::Printf(TEXT("%s equipment is an instance"),*Name),E->IsTemplate());
  TestTrue(*FString::Printf(TEXT("%s equipment registered"),*Name),E->IsRegistered());
  const TPair<const TCHAR*,UStaticMeshComponent*> Placeholders[]={{TEXT("weapon"),E->WeaponControl()},{TEXT("offhand"),E->OffhandControl()},{TEXT("armour"),E->ArmourControl()}};
  for(const auto& Pair:Placeholders)
  {
   const FString What=FString::Printf(TEXT("%s %s placeholder"),*Name,Pair.Key);UStaticMeshComponent* M=Pair.Value;
   if(!TestNotNull(*What,M))continue;
   TestFalse(*(What+TEXT(" is an instance")),M->IsTemplate());
   TestTrue(*(What+TEXT(" registered")),M->IsRegistered());
   TestEqual(*(What+TEXT(" owner")),M->GetOwner(),A);
   TestEqual(*(What+TEXT(" attached to the instanced equipment")),M->GetAttachParent(),static_cast<USceneComponent*>(E));
   TestTrue(*(What+TEXT(" listed under equipment")),E->GetAttachChildren().Contains(M));
   TestFalse(*(What+TEXT(" hidden until equipped")),M->IsVisible());
   TestEqual(*(What+TEXT(" has no collision")),M->GetCollisionEnabled(),ECollisionEnabled::NoCollision);
  }
  // The placeholders follow the equipment component (and so the actor).
  if(E->WeaponControl())
  {
   const FVector Before=E->WeaponControl()->GetComponentLocation();A->AddActorWorldOffset(FVector(250,0,0));
   TestTrue(*FString::Printf(TEXT("%s weapon placeholder follows the actor"),*Name),E->WeaponControl()->GetComponentLocation().Equals(Before+FVector(250,0,0),0.01));
  }
  A->Destroy();
 }
 for(const FString& Line:Sentinel.Lines)AddError(FString::Printf(TEXT("unallowed log line while spawning proxies: %s"),*Line));
 TestEqual(TEXT("no ensure or error logged while spawning proxies"),Sentinel.Lines.Num(),0);
 return true;
}

#endif
