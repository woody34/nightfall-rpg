#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Skills/SkillStateSubsystem.h"
#include "Effects/EffectStateSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#if WITH_DEV_AUTOMATION_TESTS
namespace Phase3StateTests
{
	constexpr EAutomationTestFlags Flags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;
	FEntitySpawn Spawn(const FString& Id, uint32 Inc = 1, uint64 Tick = 10)
	{
		FEntitySpawn S; S.EntityId = Id; S.Name = Id; S.Kind = 1; S.bCombatant = S.bAttackable = true; S.Hp = S.MaxHp = 100; S.LifeIncarnation = Inc; S.StateTick = Tick; S.SessionGeneration = 1; return S;
	}
	void Send(UNetClientSubsystem* N, const FWorldEvent& E) { FServerMessage M; M.Event = E; N->DispatchServerMessage(M); }
	void Identities(UNetClientSubsystem* N) { N->SetOwnEntityId(TEXT("owner")); FWorldEvent E; E.Spawn = Spawn(TEXT("owner")); Send(N,E); E.Spawn = Spawn(TEXT("other")); Send(N,E); }
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillStateFencesTest, "Nightfall.Skill.StateFences", Phase3StateTests::Flags)
bool FSkillStateFencesTest::RunTest(const FString& Parameters)
{
	using namespace Phase3StateTests;
	FScopedTestGameInstance Scope; auto* N = Scope.Get<UNetClientSubsystem>(); auto* S = Scope.Get<USkillStateSubsystem>(); auto* C = Scope.Get<UCombatStateSubsystem>(); Identities(N);
	FSkillStateSync Sync; Sync.Entity = TEXT("owner"); Sync.Incarnation = 1; Sync.Tick = Sync.BaselineTick = 10; Sync.bBaseline = true; Sync.Sp = 91; Sync.Learned.Add({3,2}); Sync.Reuse.Add({3,20}); Sync.DeathPenaltyLevel = 2;
	FWorldEvent E; E.SkillStateSync = Sync; Send(N,E);
	TestTrue(TEXT("same-tick bootstrap accepted"), S->HasOwnerState()); TestEqual(TEXT("received SP"), S->GetSp().GetValue(), uint64(91)); TestEqual(TEXT("reuse ticks"), S->ReuseRemaining(3).GetValue(), uint64(10));
	FStatsChanged Stats; Stats.Entity = TEXT("owner"); Stats.Incarnation = 1; Stats.Tick = Stats.BaselineTick = 10; Stats.bBaseline = true; Stats.Hp = 80; Stats.MaxHp = 120; Stats.Mp = 45; Stats.MaxMp = 60; Stats.Sp = 91;
	E = {}; E.StatsChanged = Stats; Send(N,E); TestEqual(TEXT("owner MP hydration"), C->GetOwn().Mp, uint32(45));
	FTargetChanged Selection; Selection.Entity = TEXT("owner"); Selection.Target = TEXT("other"); Selection.TargetIncarnation = 1; Selection.Tick = Selection.BaselineTick = 10; Selection.bBaseline = true;
	E = {}; E.TargetChanged = Selection; Send(N,E); TestEqual(TEXT("same-tick selection hydration"), C->GetOwn().TargetId, FString(TEXT("other")));
	Selection.bBaseline = false; Selection.Target.Reset(); E.TargetChanged = Selection; Send(N,E); TestEqual(TEXT("ordinary same-tick selection suppressed"), C->GetOwn().TargetId, FString(TEXT("other")));
	Selection.Tick = 11; E.TargetChanged = Selection; Send(N,E); TestTrue(TEXT("later clear accepted"), C->GetOwn().TargetId.IsEmpty());
	FSkillReuse Reuse; Reuse.Entity = TEXT("owner"); Reuse.Incarnation = 1; Reuse.SkillId = 3; Reuse.Tick = 10; Reuse.ReadyTick = 99;
	E = {}; E.SkillReuse = Reuse; Send(N,E); TestEqual(TEXT("ordinary same-tick reuse ignored"), S->ReuseRemaining(3).GetValue(), uint64(9));
	FSkillCastStarted Cast; Cast.Caster = TEXT("other"); Cast.CasterIncarnation = 1; Cast.CastId = 8; Cast.SkillId = 3; Cast.Tick = Cast.BaselineTick = 10; Cast.StartTick = 7; Cast.CompleteTick = 16; Cast.bBaseline = true; Cast.Target = TEXT("hidden"); Cast.TargetIncarnation = 5;
	E = {}; E.SkillCastStarted = Cast; Send(N,E); TestTrue(TEXT("cast hydrated"), S->FindCast(TEXT("other"))->Cast.IsSet()); TestFalse(TEXT("hidden endpoint omitted"), S->FindCast(TEXT("other"))->Cast->Target.IsSet()); TestEqual(TEXT("no hydration transition"), S->GetCastTransitions(), 0);
	FSkillCastCancelled Cancel; Cancel.Caster = TEXT("other"); Cancel.CasterIncarnation = 1; Cancel.CastId = 8; Cancel.Tick = 11; Cancel.Reason = 2; E = {}; E.SkillCastCancelled = Cancel; Send(N,E); Send(N,E); TestEqual(TEXT("deduped cast completion"), S->GetCastTransitions(), 1);
	E = {}; E.SkillCastStarted = Cast; Send(N,E); TestFalse(TEXT("stale hydration cannot revive cancelled cast"), S->FindCast(TEXT("other"))->Cast.IsSet());
	FClassChanged Class; Class.Entity = TEXT("owner"); Class.SessionGeneration = 1; Class.ClassId = 2; Class.Tick = 12; E = {}; E.ClassChanged = Class; Send(N,E);
	Stats.bBaseline = false; Stats.Tick = 12; Stats.Mp = 32; E = {}; E.StatsChanged = Stats; Send(N,E); TestEqual(TEXT("class fact does not advance identity baseline"), C->GetOwn().Mp, uint32(32));
	Stats.bBaseline = true; Stats.Tick = 13; Stats.Mp = 999; E.StatsChanged = Stats; Send(N,E); TestEqual(TEXT("late baseline mark rejected"), C->GetOwn().Mp, uint32(32));
	FSpGained Sp; Sp.Entity = TEXT("owner"); Sp.Incarnation = 1; Sp.Tick = 12; Sp.Amount = 7; Sp.Total = 98; E = {}; E.SpGained = Sp; Send(N,E); Send(N,E); TestEqual(TEXT("SP deduped"), S->GetSpGainedTotal(), uint64(7));
	FEntityRespawned Revive; Revive.Entity = TEXT("other"); Revive.Incarnation = 2; Revive.Tick = 13; Revive.Hp = 65; E = {}; E.EntityRespawned = Revive; Send(N,E);
	FVitalsChanged V; V.Entity = TEXT("other"); V.Incarnation = 2; V.Tick = V.BaselineTick = 13; V.bBaseline = true; V.Hp = 72; V.MaxHp = 150; E = {}; E.VitalsChanged = V; Send(N,E);
	TestEqual(TEXT("revival final maxHP accepted at baseline"), C->FindEntity(TEXT("other"))->MaxHp, uint32(150)); TestEqual(TEXT("revival final HP"), C->FindEntity(TEXT("other"))->Hp, uint32(72));
	V.Incarnation = 1; V.Tick = 14; V.bBaseline = false; V.MaxHp = 999; E.VitalsChanged = V; Send(N,E); TestEqual(TEXT("old life vitals ignored"), C->FindEntity(TEXT("other"))->MaxHp, uint32(150));
	auto Replacement = Spawn(TEXT("owner"),1,20); Replacement.SessionGeneration = 2; E = {}; E.Spawn = Replacement; Send(N,E); TestFalse(TEXT("generation replacement invalidates owner baseline"), S->GetSp().IsSet());
	E = {}; E.SkillStateSync = Sync; Send(N,E); TestFalse(TEXT("old baseline rejected after replacement"), S->GetSp().IsSet()); N->OnDisconnected.Broadcast(TEXT("test")); TestTrue(TEXT("disconnect clears public state"), S->FindPvp(TEXT("other")) == nullptr);
	return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEffectHydrationTest, "Nightfall.Effect.Hydration", Phase3StateTests::Flags)
bool FEffectHydrationTest::RunTest(const FString& Parameters)
{
	using namespace Phase3StateTests;
	FScopedTestGameInstance Scope; auto* N = Scope.Get<UNetClientSubsystem>(); auto* S = Scope.Get<UEffectStateSubsystem>(); Identities(N);
	FEffectApplied A; A.Target = TEXT("owner"); A.TargetIncarnation = 1; A.EffectId = 1; A.SkillId = 44; A.SkillLevel = 2; A.Pool = 3; A.EndTick = 20; A.Tick = A.BaselineTick = 10; A.bBaseline = true; A.AppliedTick = 3;
	FWorldEvent E; E.EffectApplied = A; Send(N,E); Send(N,E); TestTrue(TEXT("same-tick debuff hydration"), S->Has(TEXT("owner"),44,3)); TestEqual(TEXT("hydration no transitions"), S->GetTransitionCount(),0); TestEqual(TEXT("original application preserved"), S->Active(TEXT("owner"))[0].AppliedTick,uint64(3));
	A.EffectId = 2; A.bBaseline = false; E.EffectApplied = A; Send(N,E); TestEqual(TEXT("same-tick ordinary effect suppressed"), S->Active(TEXT("owner")).Num(),1);
	A.Tick = 11; A.Pool = 1; A.SkillId = 45; E.EffectApplied = A; Send(N,E); TestEqual(TEXT("received buff pool"), S->Active(TEXT("owner")).Num(),2);
	FEffectRemoved R; R.Target = TEXT("owner"); R.TargetIncarnation = 1; R.EffectId = 2; R.Tick = 11; E = {}; E.EffectRemoved = R; Send(N,E); Send(N,E); TestFalse(TEXT("same-tick removal accepted"), S->Has(TEXT("owner"),45));
	E = {}; E.EffectApplied = A; Send(N,E); TestFalse(TEXT("tombstone prevents stale reapply"), S->Has(TEXT("owner"),45));
	FServerMessage Clock; Clock.Ack = FAck{1,20}; N->DispatchServerMessage(Clock); TestFalse(TEXT("expiry uses received authoritative tick"), S->Has(TEXT("owner"),44));
	N->OnDisconnected.Broadcast(TEXT("test")); TestEqual(TEXT("scope reset"), S->Active(TEXT("owner")).Num(),0); return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FPvpStateTest, "Nightfall.Pvp.Projection", Phase3StateTests::Flags)
bool FPvpStateTest::RunTest(const FString& Parameters)
{
	using namespace Phase3StateTests;
	FScopedTestGameInstance Scope; auto* N = Scope.Get<UNetClientSubsystem>(); auto* S = Scope.Get<USkillStateSubsystem>(); Identities(N);
	FPvpStateChanged P; P.Entity = TEXT("other"); P.Incarnation = 1; P.Tick = P.BaselineTick = 10; P.bBaseline = true; P.FlagUntilTick = 30; P.PkCount = 4; P.PvpCount = 7;
	FWorldEvent E; E.PvpStateChanged = P; Send(N,E); TestEqual(TEXT("public purple bootstrap"), S->Flag(TEXT("other")),FString(TEXT("pvp"))); TestEqual(TEXT("public counters"), S->FindPvp(TEXT("other"))->PvpCount,uint32(7));
	P.bBaseline = false; P.Tick = 11; P.Karma = 99; E.PvpStateChanged = P; Send(N,E); TestEqual(TEXT("karma takes red priority"), S->Flag(TEXT("other")),FString(TEXT("chaotic")));
	P.Incarnation = 0; P.Tick = 12; P.Karma = 0; E.PvpStateChanged = P; Send(N,E); TestEqual(TEXT("stale PvP incarnation rejected"), S->FindPvp(TEXT("other"))->Karma,uint32(99));
	FEntityDied D; D.Entity = TEXT("owner"); D.Incarnation = 1; D.Tick = 11; E = {}; E.EntityDied = D; Send(N,E);
	FResurrectOffer O; O.Target = TEXT("owner"); O.TargetIncarnation = 1; O.OfferId = 4; O.State = 1; O.ExpiryTick = 50; O.Tick = 12; E = {}; E.ResurrectOffer = O; Send(N,E); TestTrue(TEXT("received offer pending"), S->OfferPending()); O.State = 2; O.Tick = 13; E.ResurrectOffer = O; Send(N,E); TestFalse(TEXT("received decline clears offer"), S->OfferPending()); O.State = 1; E.ResurrectOffer = O; Send(N,E); TestFalse(TEXT("equal-tick replay cannot undo terminal offer"), S->OfferPending());
	return true;
}
#endif
