#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Bot/SkillAuditSubsystem.h"
#include "Bot/BotPredicates.h"
#include "Net/NetClientSubsystem.h"
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillAuditTest,"Nightfall.Skill.ReceivedAudit",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillAuditTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* Audit=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 FBotPredicateRegistry R;R.RegisterBuiltins();FBotContext C;C.GameInstance=Scope.GameInstance;
 auto Holds=[&](const TCHAR* T){FString Error;auto P=R.Parse(FBotPredicateRegistry::Tokenize(T),Error);TestTrue(TEXT("registered"),bool(P));return P && P(C).bTrue;};
 for(const TCHAR* T:{TEXT("cast_dispositions_once"),TEXT("public_vitals_coherent"),TEXT("spawn_before_skill_references"),TEXT("cp_absorption_matches_received"),TEXT("resurrect_offer_state != accepted"),TEXT("resurrection_refunds == 0")})TestFalse(TEXT("no vacuous evidence"),Holds(T));
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};
 FEntitySpawn S;S.EntityId=TEXT("owner");S.Kind=1;S.LifeIncarnation=1;S.SessionGeneration=1;S.StateTick=10;S.Hp=S.MaxHp=100;S.bCombatant=true;FWorldEvent E;E.Spawn=S;Send(E);
 S.EntityId=TEXT("peer");E.Spawn=S;Send(E);
 FStatsChanged Stats;Stats.Entity=TEXT("owner");Stats.Incarnation=1;Stats.Tick=Stats.BaselineTick=10;Stats.bBaseline=true;Stats.Hp=Stats.MaxHp=100;Stats.Cp=20;Stats.Mp=40;Stats.DeathPenaltyLevel=2;E={};E.StatsChanged=Stats;Send(E);
 FSkillStateSync Sync;Sync.Entity=TEXT("owner");Sync.Incarnation=1;Sync.Tick=Sync.BaselineTick=10;Sync.bBaseline=true;Sync.DeathPenaltyLevel=2;E={};E.SkillStateSync=Sync;Send(E);
 TestTrue(TEXT("owner hydration accepted at identity tick"),Holds(TEXT("owner_skill_baseline_coherent")));TestTrue(TEXT("private/public baseline agrees"),Holds(TEXT("pvp_private_stats_coherent")));
 FSkillCastStarted Start;Start.Caster=TEXT("peer");Start.CasterIncarnation=1;Start.Target=TEXT("owner");Start.TargetIncarnation=1;Start.Tick=11;Start.CastId=3;Start.SkillId=1177;E={};E.SkillCastStarted=Start;Send(E);
 TestTrue(TEXT("known references"),Holds(TEXT("spawn_before_skill_references")));TestFalse(TEXT("unfinished cast fails disposition"),Holds(TEXT("cast_dispositions_once")));
 FSkillCastResult Result;Result.Caster=Start.Caster;Result.CasterIncarnation=1;Result.CastId=3;Result.Tick=12;Result.SkillId=Start.SkillId;Result.SkillLevel=Start.SkillLevel;FSkillTargetResult Target;Target.Target=TEXT("owner");Target.TargetIncarnation=1;Target.Tick=12;Target.CpAbsorbed=15;Target.TargetHpAfter=100;Result.Targets.Add(Target);Stats.Tick=12;Stats.bBaseline=false;Stats.Cp=5;E={};E.StatsChanged=Stats;Send(E);
 E={};E.SkillCastResult=Result;Send(E);Send(E);
 TestTrue(TEXT("duplicate result counted once"),Holds(TEXT("own_cp_damage_total == 15")));TestTrue(TEXT("one terminal disposition"),Holds(TEXT("cast_dispositions_once")));
 TestTrue(TEXT("received absorption equals CP delta"),Holds(TEXT("cp_absorption_matches_received")));
 FVitalsChanged V;V.Entity=TEXT("owner");V.Incarnation=1;V.Tick=12;V.Hp=100;V.MaxHp=150;E={};E.VitalsChanged=V;Send(E);TestTrue(TEXT("absolute final max-only vitals"),Holds(TEXT("public_vitals_coherent")));
 FSkillCastCancelled Cancel;Cancel.Caster=TEXT("peer");Cancel.CasterIncarnation=1;Cancel.CastId=3;Cancel.Tick=13;Cancel.Reason=1;E={};E.SkillCastCancelled=Cancel;Send(E);TestFalse(TEXT("contradictory terminal cannot pass"),Holds(TEXT("cast_dispositions_once")));
 FEffectApplied Effect;Effect.Target=TEXT("owner");Effect.TargetIncarnation=1;Effect.Caster=TEXT("owner");Effect.CasterIncarnation=1;Effect.EffectId=1;Effect.SkillId=1068;Effect.SkillLevel=1;Effect.Tick=Effect.AppliedTick=13;Effect.EndTick=12013;E={};E.EffectApplied=Effect;Send(E);
 N->OnDisconnected.Broadcast(TEXT("reconnect"));TestFalse(TEXT("transport evidence clears"),Holds(TEXT("own_cp_damage_total == 15")));
 S.EntityId=TEXT("owner");S.SessionGeneration=2;S.StateTick=20;E={};E.Spawn=S;Send(E);Stats.Tick=Stats.BaselineTick=20;Stats.bBaseline=true;E={};E.StatsChanged=Stats;Send(E);TestTrue(TEXT("penalty snapshot survives matching owner reconnect"),Holds(TEXT("death_penalty_matches_before_disconnect")));
 Sync.Tick=Sync.BaselineTick=20;E={};E.SkillStateSync=Sync;Send(E);
 Effect.EffectId=2;Effect.Tick=Effect.AppliedTick=Effect.BaselineTick=20;Effect.EndTick=12020;Effect.bBaseline=true;E={};E.EffectApplied=Effect;Send(E);
 TestTrue(TEXT("restore correlates new effect identity with retained skill/source"),Holds(TEXT("retained_effect_identity_coherent")));
 Stats.Tick=21;Stats.bBaseline=false;Stats.DeathPenaltyLevel=3;E.StatsChanged=Stats;Send(E);TestFalse(TEXT("penalty mismatch detected"),Holds(TEXT("death_penalty_matches_before_disconnect")));
 FEntityDied D;D.Entity=TEXT("owner");D.Incarnation=1;D.Tick=22;E={};E.EntityDied=D;Send(E);
 FResurrectOffer O;O.Target=TEXT("owner");O.TargetIncarnation=1;O.Tick=23;O.OfferId=8;O.ExpiryTick=100;O.State=1;E={};E.ResurrectOffer=O;Send(E);TestTrue(TEXT("offer bound to received death"),Holds(TEXT("resurrect_offer_matches_death")));
 O.Tick=24;O.State=2;E.ResurrectOffer=O;Send(E);TestTrue(TEXT("terminal offer preserved"),Holds(TEXT("resurrect_offer_state == declined")));TestTrue(TEXT("decline has no received refund"),Holds(TEXT("resurrection_refunds == 0")));
 O.State=1;O.Tick=25;E.ResurrectOffer=O;Send(E);TestTrue(TEXT("offered duplicate cannot undo terminal"),Holds(TEXT("resurrect_offer_state == declined")));
 return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillCpSampleLimitTest,"Nightfall.Skill.CpSampleLimit",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillCpSampleLimitTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* Audit=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};
 FEntitySpawn Spawn;Spawn.EntityId=TEXT("owner");Spawn.Kind=1;Spawn.LifeIncarnation=1;Spawn.SessionGeneration=1;Spawn.StateTick=10;Spawn.Hp=Spawn.MaxHp=100;Spawn.bCombatant=true;
 FWorldEvent E;E.Spawn=Spawn;Send(E);Spawn.EntityId=TEXT("peer");E.Spawn=Spawn;Send(E);
 FStatsChanged Stats;Stats.Entity=TEXT("owner");Stats.Incarnation=1;Stats.Tick=Stats.BaselineTick=10;Stats.bBaseline=true;Stats.Hp=Stats.MaxHp=100;Stats.Cp=20;E={};E.StatsChanged=Stats;Send(E);
 FAttackResult Damage;Damage.Attacker=TEXT("peer");Damage.Target=TEXT("owner");Damage.TargetIncarnation=1;Damage.Tick=11;Damage.TargetHpAfter=100;Damage.CpDamage=15;
 Stats.Tick=11;Stats.bBaseline=false;Stats.Cp=5;E.StatsChanged=Stats;Send(E);E={};E.AttackResult=Damage;Send(E);
 auto CpEvidence=[&](){return Audit->Check(TEXT("cp_absorption_matches_received"));};
 auto Evidence=CpEvidence();TestTrue(TEXT("valid received damage/resource pair witnesses CP absorption"),Evidence.IsSet() && *Evidence);
 // Two retained samples above, then synthetic unchanged resources up to the exact audit cap.
 for(uint64 Tick=12;Tick<1034;++Tick){Stats.Tick=Tick;E={};E.StatsChanged=Stats;Send(E);}
 Evidence=CpEvidence();TestTrue(TEXT("all 1024 retained samples remain complete"),Evidence.IsSet() && *Evidence);
 Damage.Tick=1034;Damage.CpDamage=1;Stats.Tick=1034; // Deliberately inconsistent: CP did not fall.
 E={};E.StatsChanged=Stats;Send(E);E={};E.AttackResult=Damage;Send(E);
 TestFalse(TEXT("discarded contradictory resource interval makes CP evidence UNKNOWN"),CpEvidence().IsSet());
 TestEqual(TEXT("later damage was received and audited"),Audit->Number(TEXT("own_cp_damage_total")).Get(0),uint64(16));
 FBotPredicateRegistry Registry;Registry.RegisterBuiltins();FBotContext Context;Context.GameInstance=Scope.GameInstance;
 FString Error;auto Predicate=Registry.Parse(FBotPredicateRegistry::Tokenize(TEXT("cp_absorption_matches_received")),Error);
 if(!TestTrue(TEXT("CP predicate remains registered"),bool(Predicate)))return false;
 const auto Result=Predicate(Context);TestFalse(TEXT("incomplete evidence cannot pass registered predicate"),Result.bTrue);TestTrue(TEXT("registered predicate reports UNKNOWN"),Result.Observed.Contains(TEXT("UNKNOWN")));
 N->OnDisconnected.Broadcast(TEXT("sample limit reset"));
 TestFalse(TEXT("disconnect cannot retain a prior passing witness"),CpEvidence().IsSet());
 return true;
}
// Captured r9 3-pvp-flag-a: public tick123/deadline1323 followed by
// StopAttack StatsChanged tick124 with the SAME deadline. These are current copies.
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillFlagCopyTicksTest,"Nightfall.Skill.FlagCopyIndependentTicks",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillFlagCopyTicksTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* A=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};
 auto Check=[&](){return A->Check(TEXT("own_flag_deadline_matches_received"));};
 TestFalse(TEXT("no received state is unknown"),Check().IsSet());
 FEntitySpawn S;S.EntityId=TEXT("owner");S.Kind=1;S.LifeIncarnation=1;S.SessionGeneration=1;S.StateTick=103;FWorldEvent E;E.Spawn=S;Send(E);
 FStatsChanged Stats;Stats.Entity=TEXT("owner");Stats.Incarnation=1;Stats.Tick=Stats.BaselineTick=103;Stats.bBaseline=true;E={};E.StatsChanged=Stats;Send(E);
 TestTrue(TEXT("baseline copies agree"),Check().Get(false));
 Stats.bBaseline=false;Stats.Tick=123;Stats.FlagUntilTick=1323;E.StatsChanged=Stats;Send(E);
 TestTrue(TEXT("different copies are a known mismatch"),Check().IsSet());TestFalse(TEXT("no invented public refresh"),Check().Get(true));
 FPvpStateChanged P;P.Entity=TEXT("owner");P.Incarnation=1;P.Tick=123;P.FlagUntilTick=1323;E={};E.PvpStateChanged=P;Send(E);
 TestTrue(TEXT("matching refresh received"),Check().Get(false));
 Stats.Tick=124;Stats.Mp=17;E={};E.StatsChanged=Stats;Send(E);
 TestTrue(TEXT("later unrelated stats retain exact deadline proof"),Check().Get(false));
 P.Tick=122;P.FlagUntilTick=1316;E={};E.PvpStateChanged=P;Send(E);
 TestTrue(TEXT("stale public copy cannot overwrite"),Check().Get(false));
 Stats.Tick=125;Stats.FlagUntilTick=1400;E={};E.StatsChanged=Stats;Send(E);
 TestFalse(TEXT("new differing private deadline still fails"),Check().Get(true));
 P.Tick=126;P.FlagUntilTick=1400;E={};E.PvpStateChanged=P;Send(E);
 TestTrue(TEXT("public may be newer than private too"),Check().Get(false));
 P.Tick=127;P.FlagUntilTick=0;E.PvpStateChanged=P;Send(E);
 TestFalse(TEXT("expiry must be received on both copies"),Check().Get(true));
 Stats.Tick=128;Stats.FlagUntilTick=0;E={};E.StatsChanged=Stats;Send(E);
 TestTrue(TEXT("received expiry copies agree"),Check().Get(false));
 FEntityRespawned R;R.Entity=TEXT("owner");R.Incarnation=2;R.Tick=130;R.Hp=100;E={};E.EntityRespawned=R;Send(E);
 TestFalse(TEXT("old-life agreement is not evidence"),Check().IsSet());
 Stats.Incarnation=2;Stats.Tick=Stats.BaselineTick=130;Stats.bBaseline=true;E={};E.StatsChanged=Stats;Send(E);
 TestFalse(TEXT("new private copy cannot pair with old public life"),Check().IsSet());
 P.Incarnation=2;P.Tick=P.BaselineTick=130;P.bBaseline=true;E={};E.PvpStateChanged=P;Send(E);
 TestTrue(TEXT("new-life baseline copies agree"),Check().Get(false));
 A->Disconnected(TEXT("test"));TestFalse(TEXT("disconnect clears evidence"),Check().IsSet());
 return true;
}

// r11 received wire: Start1452 HP234/CP125; regen1462 237/127;
// PeaceZone cancellation1488; regen1492 240/130. Values are captured, not formulas.
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FPeaceCancelRegenTest,"Nightfall.Skill.PeaceCancellationWithRegen",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FPeaceCancelRegenTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* A=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};
 auto Check=[&](){return A->Check(TEXT("peace_impact_has_no_damage"));};
 auto Begin=[&]()
 {
  A->Reset();FEntitySpawn S;S.EntityId=TEXT("owner");S.Kind=1;S.LifeIncarnation=1;S.SessionGeneration=1;S.StateTick=1432;S.Hp=234;S.MaxHp=309;FWorldEvent E;E.Spawn=S;Send(E);S.EntityId=TEXT("peer");E.Spawn=S;Send(E);
  FStatsChanged Stats;Stats.Entity=TEXT("owner");Stats.Incarnation=1;Stats.Tick=Stats.BaselineTick=1432;Stats.bBaseline=true;Stats.Hp=234;Stats.MaxHp=309;Stats.Cp=125;E={};E.StatsChanged=Stats;Send(E);
  FSkillCastStarted Start;Start.Caster=TEXT("peer");Start.CasterIncarnation=1;Start.Target=TEXT("owner");Start.TargetIncarnation=1;Start.Tick=Start.StartTick=1452;Start.CastId=2;Start.SkillId=1177;Start.SkillLevel=5;Start.LaunchTick=1488;Start.CompleteTick=1492;E={};E.SkillCastStarted=Start;Send(E);
 };
 auto Resources=[&](uint64 Tick,uint32 Hp,uint32 Cp)
 {
  FWorldEvent E;FStatsChanged S;S.Entity=TEXT("owner");S.Incarnation=1;S.Tick=Tick;S.Hp=Hp;S.MaxHp=309;S.Cp=Cp;E.StatsChanged=S;Send(E);
  FVitalsChanged V;V.Entity=S.Entity;V.Incarnation=1;V.Tick=Tick;V.Hp=Hp;V.MaxHp=309;E={};E.VitalsChanged=V;Send(E);
 };
 auto Cancel=[&]()
 {
  FWorldEvent E;FSkillCastCancelled C;C.Caster=TEXT("peer");C.CasterIncarnation=1;C.CastId=2;C.Tick=1488;C.Reason=9;E.SkillCastCancelled=C;Send(E);
 };
 // Synthetic removal rows mutate the captured sequence, not gameplay formulas.
 auto Damage=[&](bool Skill,uint64 Tick,uint32 Hp,uint32 Cp,const TCHAR* Target=TEXT("owner"),uint32 Inc=1)
 {
  FWorldEvent E;
  if(Skill)
  {
   FSkillCastResult R;R.Caster=TEXT("peer");R.CasterIncarnation=1;R.CastId=99;R.Tick=Tick;
   FSkillTargetResult T;T.Target=Target;T.TargetIncarnation=Inc;T.Tick=Tick;T.HpRemoved=Hp;T.CpAbsorbed=Cp;T.Rolled=1;T.TargetHpAfter=236;R.Targets.Add(T);E.SkillCastResult=R;
  }
  else {FAttackResult R;R.Attacker=TEXT("peer");R.Target=Target;R.TargetIncarnation=Inc;R.Tick=Tick;R.Damage=Hp;R.CpDamage=Cp;R.TargetHpAfter=236;E.AttackResult=R;}
  Send(E);
 };
 Begin();TestFalse(TEXT("start alone is unknown"),Check().IsSet());Resources(1462,237,127);Cancel();TestFalse(TEXT("requires resource witness after cancellation"),Check().IsSet());Resources(1492,240,130);
 TestTrue(TEXT("captured regen is retained exactly as received"),Check().Get(false));Cancel();TestTrue(TEXT("duplicate cancellation does not count twice"),Check().Get(false));
 Resources(1500,200,100);TestTrue(TEXT("later unrelated combat cannot rewrite the witnessed interval"),Check().Get(false));
 Begin();Resources(1462,233,127);Cancel();Resources(1492,240,130);TestFalse(TEXT("HP loss cannot be masked by later regen"),Check().Get(true));
 Begin();Resources(1462,237,124);Cancel();Resources(1492,240,130);TestFalse(TEXT("CP loss cannot be masked by later regen"),Check().Get(true));
 for(bool Skill:{false,true})for(bool Cp:{false,true})
 {
  Begin();Resources(1462,Cp?237:236,Cp?126:127);Damage(Skill,1462,Cp?0:1,Cp?1:0);Cancel();Resources(1492,240,130);
  const auto Result=Check();TestTrue(TEXT("positive HP/CP removal fails even when same-tick regen masks it"),Result.IsSet() && !*Result);
  Damage(Skill,1462,Cp?0:1,Cp?1:0);TestFalse(TEXT("duplicate damage cannot clear failure"),Check().Get(true));
  Begin();Damage(Skill,1462,Cp?0:1,Cp?1:0);Resources(1462,Cp?237:236,Cp?126:127);Cancel();Resources(1492,240,130);
  TestFalse(TEXT("damage before resource samples also fails"),Check().Get(true));
  for(uint64 Tick:{uint64(1452),uint64(1488),uint64(1492)})
  {
   Begin();Resources(1462,237,127);Cancel();Resources(1492,240,130);Damage(Skill,Tick,Cp?0:1,Cp?1:0);
   TestFalse(TEXT("start, cancellation and first post-cancel sample are inside the damage interval"),Check().Get(true));
  }
  Begin();Damage(Skill,1451,Cp?0:1,Cp?1:0);Resources(1462,237,127);Cancel();Resources(1492,240,130);Damage(Skill,1493,Cp?0:1,Cp?1:0);
  TestTrue(TEXT("damage outside the witnessed interval is unrelated"),Check().Get(false));
  Begin();Damage(Skill,1462,Cp?0:1,Cp?1:0,TEXT("peer"));Damage(Skill,1462,Cp?0:1,Cp?1:0,TEXT("owner"),0);Damage(Skill,1462,0,0);Resources(1462,237,127);Cancel();Resources(1492,240,130);
  TestTrue(TEXT("other target, stale life and zero removal do not invalidate peace evidence"),Check().Get(false));
 }
 Begin();Resources(1462,237,127);Cancel();Resources(1492,240,130);FWorldEvent E;FSkillCastResult R;R.Caster=TEXT("peer");R.CasterIncarnation=1;R.CastId=2;R.Tick=1492;R.SkillId=1177;R.SkillLevel=5;E.SkillCastResult=R;Send(E);
 const auto Contradiction=Check();TestTrue(TEXT("contradictory result is explicit false, not UNKNOWN"),Contradiction.IsSet() && !*Contradiction);
 A->Disconnected(TEXT("test"));TestFalse(TEXT("transport loss clears evidence"),Check().IsSet());
 return true;
}

#endif
