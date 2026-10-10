#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Bot/SkillAuditSubsystem.h"
#include "Bot/BotPredicates.h"
#include "Skills/SkillStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"
#include "Serialization/JsonSerializer.h"
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillObserverAuditTest,"Nightfall.Skill.ObserverCastIdentity",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillObserverAuditTest::RunTest(const FString& Parameters)
{
 FString Text;const FString Path=FPaths::Combine(FPaths::ProjectDir(),TEXT("Scenarios/fixtures/phase3-observer-mutual.json"));
 if(!TestTrue(TEXT("read authored harness wire fixture"),FFileHelper::LoadFileToString(Text,*Path)))return false;
 TSharedPtr<FJsonObject> Json;auto Reader=TJsonReaderFactory<>::Create(Text);if(!TestTrue(TEXT("fixture JSON"),FJsonSerializer::Deserialize(Reader,Json)))return false;
 TArray<FWorldEvent> Messages;
 for(const auto& Value:Json->GetArrayField(TEXT("messages")))
 {
  const auto Row=Value->AsObject();const auto F=Row->GetObjectField(TEXT("fields"));const FString Type=Row->GetStringField(TEXT("event"));FWorldEvent E;
  if(Type==TEXT("EntitySpawn"))
  {
   FEntitySpawn S;S.EntityId=F->GetStringField(TEXT("entity_id"));S.Kind=1;S.LifeIncarnation=uint32(F->GetIntegerField(TEXT("life_incarnation")));S.SessionGeneration=uint32(F->GetIntegerField(TEXT("session_generation")));S.StateTick=uint64(F->GetIntegerField(TEXT("state_tick")));S.Hp=uint32(F->GetIntegerField(TEXT("hp")));S.MaxHp=uint32(F->GetIntegerField(TEXT("max_hp")));S.bCombatant=F->GetBoolField(TEXT("combatant"));E.Spawn=S;
  }
  else if(Type==TEXT("SkillCastStarted"))
  {
   FSkillCastStarted S;S.Caster=F->GetStringField(TEXT("caster"));S.CasterIncarnation=uint32(F->GetIntegerField(TEXT("caster_incarnation")));S.Target=F->GetStringField(TEXT("target"));S.TargetIncarnation=uint32(F->GetIntegerField(TEXT("target_incarnation")));S.SkillId=uint32(F->GetIntegerField(TEXT("skill_id")));S.SkillLevel=uint32(F->GetIntegerField(TEXT("skill_level")));S.CastId=uint64(F->GetIntegerField(TEXT("cast_id")));S.Tick=uint64(F->GetIntegerField(TEXT("tick")));S.StartTick=uint64(F->GetIntegerField(TEXT("start_tick")));S.LaunchTick=uint64(F->GetIntegerField(TEXT("launch_tick")));S.CompleteTick=uint64(F->GetIntegerField(TEXT("complete_tick")));S.BaselineTick=uint64(F->GetIntegerField(TEXT("baseline_tick")));S.bBaseline=F->GetBoolField(TEXT("baseline"));E.SkillCastStarted=S;
  }
  else if(Type==TEXT("SkillCastResult"))
  {
   FSkillCastResult R;R.Caster=F->GetStringField(TEXT("caster"));R.CasterIncarnation=uint32(F->GetIntegerField(TEXT("caster_incarnation")));R.CastId=uint64(F->GetIntegerField(TEXT("cast_id")));R.Tick=uint64(F->GetIntegerField(TEXT("tick")));R.SkillId=uint32(F->GetIntegerField(TEXT("skill_id")));R.SkillLevel=uint32(F->GetIntegerField(TEXT("skill_level")));if(!TestTrue(TEXT("authored empty result targets"),F->GetArrayField(TEXT("targets")).IsEmpty()))return false;E.SkillCastResult=R;
  }
  else {AddError(TEXT("unsupported fixture event"));return false;}
  Messages.Add(E);
 }
 if(!TestEqual(TEXT("authored message sequence"),Messages.Num(),7))return false;
 const FString A=Messages[0].Spawn->EntityId,B=Messages[1].Spawn->EntityId;
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* Audit=Scope.Get<USkillAuditSubsystem>();auto* Skills=Scope.Get<USkillStateSubsystem>();N->SetOwnEntityId(Json->GetStringField(TEXT("observer")));
 FBotPredicateRegistry Registry;Registry.RegisterBuiltins();FBotContext C;C.GameInstance=Scope.GameInstance;
 auto Holds=[&](const FString& P){FString Error;auto Fn=Registry.Parse(FBotPredicateRegistry::Tokenize(P),Error);TestTrue(TEXT("correlated predicate grammar"),bool(Fn));return Fn && Fn(C).bTrue;};
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};
 auto Baseline=[&](const FString& Id){return Holds(TEXT("observed_cast_baseline_for ")+Id+TEXT(" 1177"));};
 auto Result=[&](const FString& Id){return Holds(TEXT("cast_result_for ")+Id+TEXT(" 1177"));};
 auto Once=[&](const FString& Id){return Holds(TEXT("cast_terminal_once_for ")+Id+TEXT(" 1177"));};
 auto Reset=[&]{Audit->Reset();N->OnDisconnected.Broadcast(TEXT("fixture reset"));};
 TestFalse(TEXT("absence before identity is unknown"),Holds(TEXT("entity_not_in_view ")+A));
 for(int32 I=0;I<4;++I)Send(Messages[I]);
 TestTrue(TEXT("mutual hydration sees entire identity batch"),Holds(TEXT("spawn_before_skill_references")));TestTrue(TEXT("original in-flight A baseline"),Baseline(A));TestTrue(TEXT("original in-flight B baseline"),Baseline(B));TestEqual(TEXT("hydration has no gameplay start counts"),Skills->GetCastTransitions(),0);
 Send(Messages[4]);TestFalse(TEXT("Might result cannot satisfy Wind Strike"),Result(A));
 Send(Messages[5]);Send(Messages[6]);Send(Messages[5]);TestTrue(TEXT("designated A result"),Result(A));TestTrue(TEXT("designated B result"),Result(B));TestTrue(TEXT("duplicate envelope still one designated terminal"),Once(A));TestTrue(TEXT("B one designated terminal"),Once(B));
 FWorldEvent Depart;Depart.Despawn=FEntityDespawn{A};Send(Depart);TestTrue(TEXT("witnessed departure"),Holds(TEXT("entity_not_in_view ")+A));TestFalse(TEXT("departed cast cannot satisfy current identity"),Result(A));
 Reset();TestFalse(TEXT("transport reset forgets departure"),Holds(TEXT("entity_not_in_view ")+A));
 TestEqual(TEXT("authored negative cases"),Json->GetArrayField(TEXT("negative_mutations")).Num(),6);
 // Execute every authored mutation independently; these are client dispatch fixtures, not a server recording.
 for(int32 Case=0;Case<6;++Case)
 {
  Reset();auto Mutated=Messages;
  if(Case==0)Swap(Mutated[1],Mutated[2]);
  if(Case==1)Mutated[2].SkillCastStarted->TargetIncarnation=999;
  if(Case==2)Mutated[5].SkillCastResult->CastId=999;
  if(Case==3)Mutated[5].SkillCastResult->SkillId=1068;
  if(Case==4)Mutated[2].SkillCastStarted->bBaseline=false;
  if(Case==5)Mutated[2].SkillCastStarted->CompleteTick=Mutated[2].SkillCastStarted->BaselineTick;
  for(const auto& E:Mutated)Send(E);
  if(Case==0 || Case==1)TestFalse(TEXT("interleaved/stale reference cannot pass"),Holds(TEXT("spawn_before_skill_references")));
  if(Case==2 || Case==3)TestFalse(TEXT("wrong cast/skill cannot satisfy designated result"),Result(A));
  if(Case==4 || Case==5)TestFalse(TEXT("ordinary/completed start cannot pass pending hydration"),Baseline(A));
 }
 return true;
}
#endif
