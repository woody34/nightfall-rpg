#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Auth/AuthSubsystem.h"
#include "Game/LoginFlowSubsystem.h"
#include "Net/SessionClientSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "Skills/SkillStateSubsystem.h"
#include "Effects/EffectStateSubsystem.h"
#include "TurboLinkGrpcManager.h"
#include "Containers/Ticker.h"
#include "HAL/PlatformProcess.h"
#include "Misc/Guid.h"
#if WITH_DEV_AUTOMATION_TESTS
namespace Phase3LiveTests
{
 constexpr EAutomationTestFlags Flags=EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter;
 bool Admission(FAutomationTestBase& Test,const FString& Domain)
 {
  NightfallTest::AllowApiUnavailableLogs(Test);FScopedTestGameInstance Scope;
  auto* Session=Scope.Get<USessionClient>();auto* Auth=Scope.Get<UAuthSubsystem>();auto* Flow=Scope.Get<ULoginFlowSubsystem>();auto* Net=Scope.Get<UNetClientSubsystem>();auto* Skills=Scope.Get<USkillStateSubsystem>();auto* Combat=Scope.Get<UCombatStateSubsystem>();
  Flow->bTravelOnConnect=false;
  auto Pump=[&](TFunction<bool()> Done,double Seconds=10.0)
  {
   const double End=FPlatformTime::Seconds()+Seconds;
   while(!Done() && FPlatformTime::Seconds()<End){Session->GetGrpcManager()->Tick(0.01f);FTSTicker::GetCoreTicker().Tick(0.01f);FPlatformProcess::Sleep(0.01f);}
   return Done();
  };
  bool Done=false;FNetResult Result;
  Session->Ping([&](const FNetResult& R,const auto&){Result=R;Done=true;});
  if(!Pump([&]{return Done;})){Test.AddError(TEXT("Ping callback timed out; not an offline skip"));return false;}
  if(Result.Error==ENetError::Unavailable)return NightfallTest::SkipLive(Test,TEXT("SKIPPED: no API listening; no live Phase 3 coverage"));
  if(!Result.IsOk()){Test.AddError(TEXT("Reachable API Ping failed: ")+Result.Message);return false;}
  if(!UAuthSubsystem::GetCommandLineDevToken().IsEmpty())Auth->StartLogin();else Auth->LoginWithDevToken(TEXT("test:")+FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower));
  if(!Auth->IsLoggedIn()){Test.AddError(TEXT("Reachable API requires valid test authentication"));return false;}
  if(Domain==TEXT("Skill"))
  {
   Done=false;Skills->LoadCatalog([&](const FNetResult& R){Result=R;Done=true;});
   if(!Pump([&]{return Done;}) || !Result.IsOk() || !Skills->HasCatalog() || Skills->GetCatalog().Skills.IsEmpty()){Test.AddError(TEXT("Reachable API must publish an executable Phase 3 catalogue: ")+Result.Message);return false;}
  }
  FString Character;FParse::Value(FCommandLine::Get(),TEXT("Phase3LiveCharacter="),Character);
  if(Character.IsEmpty())
  {
   const FString Hex=FGuid::NewGuid().ToString(EGuidFormats::Digits);FString Name=TEXT("Skill");for(int32 I=0;I<10;++I)Name.AppendChar(TEXT('a')+FParse::HexDigit(Hex[I]));
   Done=false;Flow->CreateCharacter(Name,EGrpcNightfallV1Race::RACE_HUMAN,[&](const FNetResult& R,const auto& C){Result=R;Character=C.Id;Done=true;});
   if(!Pump([&]{return Done;}) || !Result.IsOk() || Character.IsEmpty()){Test.AddError(TEXT("Live test character creation failed: ")+Result.Message);return false;}
  }
  Done=false;Flow->EnterWorld(Character,[&](const FNetResult& R){Result=R;Done=true;});
  if(!Pump([&]{return Done;}) || !Result.IsOk()){Test.AddError(TEXT("Reachable API world admission refused: ")+Result.Message);return false;}
  if(!Pump([&]{return Skills->HasOwnerState() && Combat->FindOwnEntity() && Combat->GetOwn().bMpKnown;},15.0)){Test.AddError(TEXT("World admission did not hydrate private Phase 3 baseline"));return false;}
  if(Domain==TEXT("Pvp"))Test.TestNotNull(TEXT("received owner PvP bootstrap"),Skills->FindPvp(Character));
  if(Domain==TEXT("Effect"))
  {
   auto* Effects=Scope.Get<UEffectStateSubsystem>();Test.TestNotNull(TEXT("effect projection"),Effects);
   for(const auto& E:Effects->Active(Character))Test.TestTrue(TEXT("hydrated effect target incarnation"),E.TargetIncarnation==Combat->FindOwnEntity()->Incarnation);
  }
  Test.TestTrue(TEXT("received SP known"),Skills->GetSp().IsSet());Flow->LeaveWorld();return !Test.HasAnyErrors();
 }
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillLiveTest,"Nightfall.Skill.LiveAdmission",Phase3LiveTests::Flags)
bool FSkillLiveTest::RunTest(const FString& Parameters){return Phase3LiveTests::Admission(*this,TEXT("Skill"));}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FPvpLiveTest,"Nightfall.Pvp.LiveAdmission",Phase3LiveTests::Flags)
bool FPvpLiveTest::RunTest(const FString& Parameters){return Phase3LiveTests::Admission(*this,TEXT("Pvp"));}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEffectLiveTest,"Nightfall.Effect.LiveAdmission",Phase3LiveTests::Flags)
bool FEffectLiveTest::RunTest(const FString& Parameters){return Phase3LiveTests::Admission(*this,TEXT("Effect"));}
#endif
