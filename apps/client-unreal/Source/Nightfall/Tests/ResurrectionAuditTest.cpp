#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Net/NetClientSubsystem.h"
#include "Bot/SkillAuditSubsystem.h"

#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FResurrectionOrderTest,"Nightfall.Skill.ResurrectionTerminalOrder",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FResurrectionOrderTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* Audit=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};
 auto Stats=[&](uint32 Inc,uint64 Tick,uint32 Hp,uint64 Xp,bool Baseline=false){FStatsChanged S;S.Entity=TEXT("owner");S.Incarnation=Inc;S.Tick=Tick;S.Hp=Hp;S.MaxHp=100;S.Mp=25;S.MaxMp=50;S.Cp=40;S.MaxCp=80;S.Xp=Xp;S.bBaseline=Baseline;S.BaselineTick=Baseline?Tick:0;FWorldEvent E;E.StatsChanged=S;Send(E);};
 FResurrectOffer O;O.Target=TEXT("owner");O.TargetIncarnation=1;O.OfferId=8;O.Tick=12;O.ExpiryTick=100;O.State=1;
 auto Offer=[&](const FResurrectOffer& V){FWorldEvent E;E.ResurrectOffer=V;Send(E);};
 uint32 Generation=0;
 auto Setup=[&]{Audit->Reset();FEntitySpawn S;S.EntityId=TEXT("owner");S.Kind=1;S.LifeIncarnation=1;S.SessionGeneration=++Generation;S.StateTick=10;S.Hp=S.MaxHp=100;FWorldEvent E;E.Spawn=S;Send(E);Stats(1,10,100,1000,true);FEntityDied D;D.Entity=TEXT("owner");D.Incarnation=1;D.Tick=11;E={};E.EntityDied=D;Send(E);Stats(1,11,0,900);Offer(O);};
 auto Revive=[&]{FEntityRespawned R;R.Entity=TEXT("owner");R.Incarnation=2;R.Tick=20;R.Hp=65;FWorldEvent E;E.EntityRespawned=R;Send(E);};
 auto Refunds=[&]{return Audit->Number(TEXT("resurrection_refunds")).Get(MAX_uint64);};
 auto Accepted=O;Accepted.State=4;Accepted.Tick=20;
 // Actual producer order: new identity + owner resource baseline, then the old-corpse terminal.
 Setup();Revive();Stats(2,20,65,950,true);TestEqual(TEXT("stats alone do not prove a skill refund"),Refunds(),uint64(0));
 Offer(Accepted);TestEqual(TEXT("old-corpse accepted terminal completes refund"),Refunds(),uint64(1));TestEqual(TEXT("terminal retained for predicates"),Audit->OfferState().Get(0),uint32(4));
 TestTrue(TEXT("revival vitals are classified after late terminal"),Audit->Check(TEXT("resurrection_vitals_match_fixture")).Get(false));Offer(Accepted);TestEqual(TEXT("terminal retransmission counts once"),Refunds(),uint64(1));
 Stats(2,21,70,960);Offer(Accepted);TestEqual(TEXT("later XP does not add refunds"),Refunds(),uint64(1));
 Setup();Revive();Offer(Accepted);TestEqual(TEXT("terminal waits for resources"),Refunds(),uint64(0));Stats(2,20,65,950,true);TestEqual(TEXT("either order completes same offer"),Refunds(),uint64(1));
 for(int32 Case=0;Case<4;++Case)
 {
  Setup();Revive();Stats(2,20,65,950,true);auto Bad=Accepted;
  if(Case==0)++Bad.OfferId;if(Case==1)Bad.TargetIncarnation=0;if(Case==2)--Bad.Tick;if(Case==3)Bad.Target=TEXT("other");
  Offer(Bad);TestEqual(TEXT("unrelated old-life terminal cannot correlate"),Refunds(),uint64(0));
 }
 Setup();auto Declined=O;Declined.State=2;Declined.Tick=13;Offer(Declined);Revive();Stats(2,20,65,950,true);Offer(Accepted);TestEqual(TEXT("decline cannot become a refund"),Refunds(),uint64(0));
 Setup();Revive();Stats(2,20,65,900,true);Offer(Accepted);TestEqual(TEXT("accepted no-XP revival has no witnessed refund"),Refunds(),uint64(0));TestTrue(TEXT("no-XP accepted revival still classifies vitals"),Audit->Check(TEXT("resurrection_vitals_match_fixture")).Get(false));
 Setup();Revive();Stats(2,20,65,950,true);FEntityDied D;D.Entity=TEXT("owner");D.Incarnation=2;D.Tick=21;FWorldEvent E;E.EntityDied=D;Send(E);Stats(2,21,0,850);Offer(Accepted);TestEqual(TEXT("later death cannot reuse old accepted terminal"),Refunds(),uint64(0));
 return true;
}
#endif
