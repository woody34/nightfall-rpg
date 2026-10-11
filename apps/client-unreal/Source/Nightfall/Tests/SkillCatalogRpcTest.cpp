#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "NightfallRpcTestServer.h"
#include "Net/NetSettings.h"
#include "Net/NetClientSubsystem.h"
#include "Skills/SkillStateSubsystem.h"
#include "TurboLinkGrpcConfig.h"
#include "TurboLinkGrpcManager.h"
#include "Misc/OutputDevice.h"
#include "Misc/OutputDeviceRedirector.h"
#include "HAL/PlatformProcess.h"
#include "HAL/FileManager.h"
#include "Misc/Paths.h"
#include "Misc/Guid.h"

#if WITH_DEV_AUTOMATION_TESTS
namespace SkillCatalogRpcTests
{
 class FTransportTest : public FAutomationTestBase
 {
 public:
  FTransportTest(const FString& Name,bool bComplex) : FAutomationTestBase(Name,bComplex) {}
  bool bInspectingRpcError=false;
  bool ShouldCaptureLogCategory(const FName& Category) const override
  {
   // Expected-message filters downgrade severity before GLog sees it. During the
   // negative RPC cases our own device verifies the exact severity and count instead.
   return !bInspectingRpcError || Category!=TEXT("LogTurboLink");
  }
 };
 struct FEndpointOverride
 {
  FString OldEndpoint=GetDefault<UNetSettings>()->GrpcEndpoint;
  TMap<FString,FString> OldServices=GetDefault<UTurboLinkGrpcConfig>()->ServiceEndPoint;
  explicit FEndpointOverride(const FString& Endpoint) {GetMutableDefault<UNetSettings>()->GrpcEndpoint=Endpoint;}
  ~FEndpointOverride()
  {
   GetMutableDefault<UNetSettings>()->GrpcEndpoint=OldEndpoint;
   GetMutableDefault<UTurboLinkGrpcConfig>()->ServiceEndPoint=MoveTemp(OldServices);
  }
 };
 struct FLogCapture final : public FOutputDevice
 {
  int32 Warnings=0,Errors=0;
  FString WarningMessage,ErrorMessage;
  FLogCapture() {GLog->AddOutputDevice(this);}
  ~FLogCapture() override {GLog->RemoveOutputDevice(this);}
  void Serialize(const TCHAR* Message,ELogVerbosity::Type Verbosity,const FName& Category) override
  {
   if(Category!=TEXT("LogTurboLink"))return;
   if(Verbosity==ELogVerbosity::Warning){++Warnings;WarningMessage=Message;}
   if(Verbosity==ELogVerbosity::Error){++Errors;ErrorMessage=Message;}
  }
 };
 bool Pump(USessionClient* Session,const int32& Completed)
 {
  const double Deadline=FPlatformTime::Seconds()+10.0;
  while(!Completed && FPlatformTime::Seconds()<Deadline)
  {
   Session->GetGrpcManager()->Tick(0.01f);FPlatformProcess::Sleep(0.01f);
  }
  // Drain later completion-queue activity so a duplicate callback is observable.
  for(int32 Index=0;Index<10;++Index){Session->GetGrpcManager()->Tick(0.01f);FPlatformProcess::Sleep(0.01f);}
  return Completed!=0;
 }
}

IMPLEMENT_CUSTOM_SIMPLE_AUTOMATION_TEST(FSkillCatalogRpcTest,SkillCatalogRpcTests::FTransportTest,"Nightfall.Skill.CatalogueRpcFailures",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillCatalogRpcTest::RunTest(const FString& Parameters)
{
 using namespace SkillCatalogRpcTests;
 {
  auto Server=MakeNightfallRpcTestServer(true);
  if(!TestFalse(TEXT("owned loopback status server started on an ephemeral port"),Server->Endpoint().IsEmpty()))return false;
  FEndpointOverride Endpoint(Server->Endpoint());FScopedTestGameInstance Scope;
  auto* Session=Scope.Get<USessionClient>();auto* Skills=Scope.Get<USkillStateSubsystem>();auto* Net=Scope.Get<UNetClientSubsystem>();
  const FString HotbarFile=FPaths::Combine(FPaths::ProjectSavedDir(),TEXT("Tests"),FGuid::NewGuid().ToString()+TEXT("-rpc-hotbar.ini"));
  ON_SCOPE_EXIT {IFileManager::Get().Delete(*HotbarFile);};
  Skills->SetHotbarConfigForTesting(HotbarFile);Net->SetOwnEntityId(TEXT("catalogue-rpc-owner"));
  FEntitySpawn Spawn;Spawn.EntityId=Net->GetOwnEntityId();Spawn.LifeIncarnation=1;Spawn.StateTick=10;Skills->ApplySpawn(Spawn);
  FSkillStateSync Sync;Sync.Entity=Spawn.EntityId;Sync.Incarnation=1;Sync.Tick=Sync.BaselineTick=10;Sync.bBaseline=true;Sync.Learned.Add({3,1});Skills->ApplySync(Sync);
  FGrpcNightfallV1GetSkillCatalogResponse Catalog;auto& Row=Catalog.Skills.AddDefaulted_GetRef();Row.SkillId=3;Row.EffectImplemented=true;Row.OperateType=EGrpcNightfallV1SkillOperateType::SKILL_OPERATE_TYPE_ACTIVE_INSTANT;Skills->ApplyCatalog(Catalog);
  TestTrue(TEXT("successful catalogue initially authorizes learned active skill"),Skills->IsCastable(3));
  TestEqual(TEXT("hotbar initially contains the learned skill"),Skills->Hotbar()[0],3u);
  int32 Completed=0;FNetResult Failure;FLogCapture Logs;
  Skills->LoadCatalog([&](const FNetResult& Result){++Completed;Failure=Result;});
  TestFalse(TEXT("catalogue refresh immediately closes casting"),Skills->IsCastable(3));
  if(!TestTrue(TEXT("UNAVAILABLE RPC completes through generated transport"),Pump(Session,Completed)))return false;
  TestEqual(TEXT("catalogue completes exactly once"),Completed,1);
  TestEqual(TEXT("catalogue failure status preserved"),Failure.Error,ENetError::Unavailable);
  TestEqual(TEXT("catalogue failure message preserved"),Failure.Message,FString(TEXT("catalogue fixture unavailable")));
  TestEqual(TEXT("subsystem receives unchanged status"),Skills->GetLastResult().Error,Failure.Error);
  TestEqual(TEXT("subsystem receives unchanged message"),Skills->GetLastResult().Message,Failure.Message);
  TestEqual(TEXT("UNAVAILABLE logs one Warning"),Logs.Warnings,1);
  TestEqual(TEXT("UNAVAILABLE logs no Error"),Logs.Errors,0);
  TestTrue(TEXT("warning identifies failed catalogue"),Logs.WarningMessage.Contains(TEXT("GetSkillCatalog unavailable:")) && Logs.WarningMessage.Contains(Failure.Message));
  TestFalse(TEXT("failed catalogue unavailable"),Skills->HasCatalog());TestFalse(TEXT("failed catalogue no longer busy"),Skills->IsBusy());
  TestFalse(TEXT("old active metadata cannot authorize casting"),Skills->IsCastable(3));
  TestEqual(TEXT("hotbar removes castable entry after failure"),Skills->Hotbar()[0],0u);
  TestEqual(TEXT("number-key hotbar action cannot cast"),Skills->CastSlot(0),0u);

  TGuardValue<bool> InspectRpcError(bInspectingRpcError,true);
  int32 LearnCompleted=0;FNetResult LearnFailure;FGrpcNightfallV1LearnSkillRequest Request;
  Session->LearnSkill(Request,[&](const FNetResult& Result,const auto& Response)
  {++LearnCompleted;LearnFailure=Result;TestEqual(TEXT("failed learning response empty"),Response.SkillId.Value,0u);});
  TestTrue(TEXT("another RPC completes its UNAVAILABLE failure"),Pump(Session,LearnCompleted));
  TestEqual(TEXT("learning callback once"),LearnCompleted,1);TestEqual(TEXT("other RPC unchanged status"),LearnFailure.Error,ENetError::Unavailable);
  TestEqual(TEXT("other RPC unchanged message"),LearnFailure.Message,FString(TEXT("learning fixture unavailable")));
  TestEqual(TEXT("another RPC still logs Error for UNAVAILABLE"),Logs.Errors,1);
  TestTrue(TEXT("other RPC keeps CallRpcError prefix"),Logs.ErrorMessage.Contains(TEXT("CallRpcError:")) && Logs.ErrorMessage.Contains(LearnFailure.Message));
  TestEqual(TEXT("warning policy applies only to catalogue"),Logs.Warnings,1);
 }
 {
  auto Server=MakeNightfallRpcTestServer(false);
  if(!TestFalse(TEXT("second status server started"),Server->Endpoint().IsEmpty()))return false;
  FEndpointOverride Endpoint(Server->Endpoint());FScopedTestGameInstance Scope;
  auto* Session=Scope.Get<USessionClient>();int32 Completed=0;FNetResult Failure;FLogCapture Logs;
  TGuardValue<bool> InspectRpcError(bInspectingRpcError,true);
  Session->GetSkillCatalog({},[&](const FNetResult& Result,const auto& Response)
  {++Completed;Failure=Result;TestTrue(TEXT("failed catalogue response empty"),Response.Skills.IsEmpty());});
  TestTrue(TEXT("other catalogue status completes"),Pump(Session,Completed));
  TestEqual(TEXT("other catalogue status callback once"),Completed,1);
  TestEqual(TEXT("other catalogue status unchanged"),Failure.Error,ENetError::FailedPrecondition);
  TestEqual(TEXT("other catalogue failure message unchanged"),Failure.Message,FString(TEXT("catalogue fixture precondition")));
  TestEqual(TEXT("other catalogue status retains Error logging"),Logs.Errors,1);
  TestEqual(TEXT("other catalogue status is not downgraded"),Logs.Warnings,0);
 }
 return true;
}
#endif
