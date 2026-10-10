#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Skills/SkillStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "UI/NightfallSkillDialog.h"
#include "Components/ComboBoxString.h"
#include "Components/Button.h"
#include "Misc/Paths.h"
#include "Misc/Guid.h"
#include "HAL/FileManager.h"
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillHotbarTest,"Nightfall.Skill.StableConfigurableHotbar",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillHotbarTest::RunTest(const FString& Parameters)
{
 const FString File=FPaths::Combine(FPaths::ProjectSavedDir(),TEXT("Tests"),FGuid::NewGuid().ToString()+TEXT("-hotbar.ini"));
 IFileManager::Get().MakeDirectory(*FPaths::GetPath(File),true);
 {
  FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<USkillStateSubsystem>();N->SetOwnEntityId(TEXT("hotbar-owner"));S->SetHotbarConfigForTesting(File);
  FEntitySpawn Spawn;Spawn.EntityId=TEXT("hotbar-owner");Spawn.LifeIncarnation=1;Spawn.StateTick=10;S->ApplySpawn(Spawn);
  FSkillStateSync Sync;Sync.Entity=Spawn.EntityId;Sync.Incarnation=1;Sync.Tick=Sync.BaselineTick=10;Sync.bBaseline=true;
  FGrpcNightfallV1GetSkillCatalogResponse Catalog;TMap<uint32,FString> Types;
  for(uint32 Id=100;Id<112;++Id){Sync.Learned.Add({Id,1});auto& Row=Catalog.Skills.AddDefaulted_GetRef();Row.SkillId=Id;Row.DisplayName=FString::Printf(TEXT("Skill %u"),Id);Row.EffectImplemented=true;Types.Add(Id,TEXT("A1"));}
  for(uint32 Id=1;Id<=3;++Id){Sync.Learned.Add({Id,1});auto& Row=Catalog.Skills.AddDefaulted_GetRef();Row.SkillId=Id;Row.EffectImplemented=true;Types.Add(Id,Id==1?TEXT("P"):Id==2?TEXT("T"):TEXT("UNKNOWN"));}
  S->ApplySync(Sync);S->ApplyCatalog(Catalog);
  TestEqual(TEXT("legacy catalogue without operation metadata fails closed"),S->AvailableSkills().Num(),0);TestFalse(TEXT("implemented alone does not permit assignment"),S->AssignSlot(0,100));TestEqual(TEXT("empty slot sends nothing"),S->CastSlot(0),0u);
  // Typed fixture models the required server catalogue addition; current proto lacks that field.
  S->ApplyCatalog(Catalog,Types);TestEqual(TEXT("all twelve active learned skills available"),S->AvailableSkills().Num(),12);
  TestFalse(TEXT("passive excluded"),S->AvailableSkills().Contains(1));TestFalse(TEXT("toggle is not an active CastSkill slot"),S->AvailableSkills().Contains(2));TestFalse(TEXT("unknown type excluded"),S->AvailableSkills().Contains(3));
  const auto Initial=S->Hotbar();TestEqual(TEXT("nine stable numbered slots"),Initial.Num(),9);TestEqual(TEXT("initial deterministic mapping"),Initial[0],100u);
  FSkillLearned Learned;Learned.Entity=Spawn.EntityId;Learned.Incarnation=1;Learned.Tick=11;Learned.SkillId=99;Learned.SkillLevel=1;S->ApplyLearned(Learned);
  auto& Lower=Catalog.Skills.AddDefaulted_GetRef();Lower.SkillId=99;Lower.DisplayName=TEXT("Lower id");Lower.EffectImplemented=true;Types.Add(99,TEXT("A2"));S->ApplyCatalog(Catalog,Types);
  TestTrue(TEXT("learning a lower id never shifts existing bindings"),S->Hotbar()==Initial);
  auto* Dialog=NewObject<UNightfallSkillDialog>(Scope.GameInstance);Dialog->EnsureLayout();Dialog->BindState(S,false);
  TestEqual(TEXT("picker includes all thirteen active choices"),Dialog->HotbarSkillControl()->GetOptionCount(),13);
  Dialog->HotbarSkillControl()->SetSelectedOption(TEXT("111: Skill 111"));Dialog->HotbarSlotControl()->SetSelectedOption(TEXT("1"));Dialog->AssignControl()->OnClicked.Broadcast();
  TestEqual(TEXT("actual UI assigns skill beyond the first nine"),S->Hotbar()[0],111u);TestEqual(TEXT("other numbered binding stable"),S->Hotbar()[1],101u);
  TestFalse(TEXT("passive cannot be explicitly assigned"),S->AssignSlot(1,1));TestFalse(TEXT("unknown skill cannot be assigned"),S->AssignSlot(1,500));TestFalse(TEXT("invalid numbered slot rejected"),S->AssignSlot(9,100));
  TestTrue(TEXT("move skill to another binding"),S->AssignSlot(8,101));TestEqual(TEXT("one binding per skill"),S->Hotbar()[1],0u);
  TestTrue(TEXT("clear is configurable"),S->AssignSlot(2,0));TestEqual(TEXT("cleared slot retained"),S->Hotbar()[2],0u);
  Learned.Tick=12;Learned.SkillId=111;Learned.SkillLevel=2;S->ApplyLearned(Learned);TestEqual(TEXT("rank upgrade keeps binding"),S->Hotbar()[0],111u);
  TestTrue(TEXT("slot map was written to disk"),IFileManager::Get().FileExists(*File));
  S->Reset();Spawn.LifeIncarnation=2;Spawn.StateTick=20;S->ApplySpawn(Spawn);Sync.Incarnation=2;Sync.Tick=Sync.BaselineTick=20;S->ApplySync(Sync);S->ApplyCatalog(Catalog,Types);TestEqual(TEXT("reconnect retains map"),S->Hotbar()[0],111u);
 }
 {
  FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<USkillStateSubsystem>();N->SetOwnEntityId(TEXT("hotbar-owner"));S->SetHotbarConfigForTesting(File);
  FEntitySpawn Spawn;Spawn.EntityId=TEXT("hotbar-owner");Spawn.LifeIncarnation=2;Spawn.StateTick=20;S->ApplySpawn(Spawn);
  FSkillStateSync Sync;Sync.Entity=Spawn.EntityId;Sync.Incarnation=2;Sync.Tick=Sync.BaselineTick=20;Sync.bBaseline=true;Sync.Learned.Add({111,1});Sync.Learned.Add({101,1});S->ApplySync(Sync);
  FGrpcNightfallV1GetSkillCatalogResponse Catalog;for(uint32 Id:{111u,101u}){auto& Row=Catalog.Skills.AddDefaulted_GetRef();Row.SkillId=Id;Row.EffectImplemented=true;}S->ApplyCatalog(Catalog,{{111,TEXT("A1")},{101,TEXT("A1")}});
  TestEqual(TEXT("new game instance reloads saved slot1"),S->Hotbar()[0],111u);TestEqual(TEXT("saved moved skill remains slot9"),S->Hotbar()[8],101u);TestEqual(TEXT("saved empty slot stays empty"),S->Hotbar()[2],0u);
  N->SetOwnEntityId(TEXT("other-owner"));S->Reset();Spawn.EntityId=TEXT("other-owner");S->ApplySpawn(Spawn);Sync.Entity=Spawn.EntityId;S->ApplySync(Sync);S->ApplyCatalog(Catalog,{{111,TEXT("A1")},{101,TEXT("A1")}});TestEqual(TEXT("another character has its own map"),S->Hotbar()[0],101u);
 }
 IFileManager::Get().Delete(*File);return true;
}
#endif
