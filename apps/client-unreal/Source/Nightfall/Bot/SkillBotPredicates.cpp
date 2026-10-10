#include "SkillBotPredicates.h"
#include "BotPredicates.h"
#include "BotScenarioRunner.h"
#include "Skills/SkillStateSubsystem.h"
#include "Effects/EffectStateSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"
#include "HAL/IConsoleManager.h"
#include "Misc/Guid.h"

namespace SkillBotDetail
{
	USkillStateSubsystem* State(const FBotContext& C) { return C.GameInstance ? C.GameInstance->GetSubsystem<USkillStateSubsystem>() : nullptr; }
	UEffectStateSubsystem* Effects(const FBotContext& C) { return C.GameInstance ? C.GameInstance->GetSubsystem<UEffectStateSubsystem>() : nullptr; }
	FString Entity(const FBotContext& C, const FString& Arg)
	{
		if (Arg.Equals(TEXT("self"), ESearchCase::IgnoreCase) || Arg.Equals(TEXT("own"), ESearchCase::IgnoreCase)) return C.Net() ? C.Net()->GetOwnEntityId() : FString();
		if (Arg.Equals(TEXT("target"), ESearchCase::IgnoreCase)) return C.Combat() ? C.Combat()->GetOwn().TargetId : FString();
		return Arg;
	}
	bool EntityArg(const FString& A) { FGuid Guid; return A == TEXT("own") || A == TEXT("self") || A == TEXT("target") || FGuid::Parse(A,Guid); }
	bool Id(const FString& A, uint32& Out)
	{
		uint64 Value; if (!SkillBotPredicates::ParseUnsigned(A,Value) || !Value || Value > MAX_uint32) return false; Out = uint32(Value); return true;
	}
	bool Operator(const FString& S) { return S == TEXT("==") || S == TEXT("!=") || S == TEXT("<") || S == TEXT("<=") || S == TEXT(">") || S == TEXT(">="); }
	bool Compare(uint64 V, const FString& Op, uint64 N)
	{
		if (Op == TEXT("==")) return V == N; if (Op == TEXT("!=")) return V != N; if (Op == TEXT("<")) return V < N; if (Op == TEXT("<=")) return V <= N; if (Op == TEXT(">")) return V > N; return V >= N;
	}
	using FRead = TFunction<TOptional<uint64>(const FBotContext&, const TArray<FString>&)>;
	void Number(FBotPredicateRegistry& R, const FString& Name, int32 Prefix, FRead Read, TFunction<bool(const TArray<FString>&)> Validate = nullptr)
	{
		R.Register({Name, Name + TEXT(" [entity|id] <op> <unsigned n>"), TEXT("Received authoritative state, exact unsigned comparison"), [Prefix,Read,Validate](const TArray<FString>& A, FString& Error)->FBotPredicateFn
		{
			uint64 N;
			if (A.Num() != Prefix+2 || !Operator(A[Prefix]) || !SkillBotPredicates::ParseUnsigned(A[Prefix+1],N) || (Validate && !Validate(A))) { Error = TEXT("invalid integer comparison arguments"); return nullptr; }
			return [A,Read,N,Prefix](const FBotContext& C)->FBotPredicateValue { const auto V = Read(C,A); return {V.IsSet() && Compare(V.GetValue(), A[Prefix], N), V.IsSet() ? FString::Printf(TEXT("%llu"),V.GetValue()) : FString(TEXT("unknown"))}; };
		}});
	}
	FString ErrorName(ENetError E)
	{
		static const TCHAR* Names[] = {TEXT("none"),TEXT("cancelled"),TEXT("unknown"),TEXT("invalid_argument"),TEXT("deadline_exceeded"),TEXT("not_found"),TEXT("already_exists"),TEXT("permission_denied"),TEXT("resource_exhausted"),TEXT("failed_precondition"),TEXT("aborted"),TEXT("out_of_range"),TEXT("unimplemented"),TEXT("internal"),TEXT("unavailable"),TEXT("data_loss"),TEXT("unauthenticated")};
		return Names[static_cast<uint8>(E)];
	}
	bool ErrorMatches(const FString& A, ENetError E) { return A.Equals(ErrorName(E),ESearchCase::IgnoreCase) || A.Equals(ErrorName(E).Replace(TEXT("_"),TEXT("")),ESearchCase::IgnoreCase) || A == FString::FromInt(static_cast<uint8>(E)); }
}
bool SkillBotPredicates::ParseUnsigned(const FString& Text, uint64& Out)
{
	if (Text.IsEmpty()) return false; Out=0;
	for (TCHAR C : Text) { if (C < TEXT('0') || C > TEXT('9')) return false; const uint64 D = C-TEXT('0'); if (Out > (MAX_uint64-D)/10) return false; Out=Out*10+D; } return true;
}
void SkillBotPredicates::Register(FBotPredicateRegistry& R)
{
	using namespace SkillBotDetail;
 for (const FString& Name : {FString(TEXT("initial_hp")),FString(TEXT("initial_mp")),FString(TEXT("initial_cp"))})
  Number(R,Name,0,[Name](const FBotContext& C,const auto&)->TOptional<uint64>{const auto* S=State(C);if(!S)return {};const auto V=Name==TEXT("initial_hp")?S->GetInitialHp():Name==TEXT("initial_mp")?S->GetInitialMp():S->GetInitialCp();return V.IsSet()?TOptional<uint64>(*V):TOptional<uint64>();});
 Number(R,TEXT("target_flag_remaining"),0,[](const FBotContext& C,const auto&)->TOptional<uint64>{const auto* S=State(C);const auto* P=S?S->FindPvp(Entity(C,TEXT("target"))):nullptr;return P?TOptional<uint64>(P->FlagUntilTick>S->GetTick()?P->FlagUntilTick-S->GetTick():0):TOptional<uint64>();});
 for (const FString& Name : {FString(TEXT("entity_alive")),FString(TEXT("entity_dead")),FString(TEXT("entity_in_view"))})
  R.Register({Name,Name+TEXT(" <own|target|uuid>"),TEXT("Known authoritative entity life"),[Name](const TArray<FString>& A,FString& Error)->FBotPredicateFn
  {
   if(A.Num()!=1 || !EntityArg(A[0])){Error=TEXT("needs an entity");return nullptr;}
   return [Name,Arg=A[0]](const FBotContext& C)->FBotPredicateValue{const auto* E=C.Combat()?C.Combat()->FindEntity(Entity(C,Arg)):nullptr;return {E && (Name==TEXT("entity_in_view") || (Name==TEXT("entity_dead")?E->bDead:!E->bDead && E->Hp>0)),!E?FString(TEXT("unknown")):E->bDead?FString(TEXT("dead")):FString(TEXT("alive"))};};
  }});
 R.Register({TEXT("entity_at"),TEXT("entity_at <entity> <x> <y> <tol>"),TEXT("Newest received position sample"),[](const TArray<FString>& A,FString& Error)->FBotPredicateFn
 {
  double X=0,Y=0,Tol=0;if(A.Num()!=4 || !EntityArg(A[0]) || !LexTryParseString(X,*A[1]) || !LexTryParseString(Y,*A[2]) || !LexTryParseString(Tol,*A[3]) || !FMath::IsFinite(X) || !FMath::IsFinite(Y) || !FMath::IsFinite(Tol) || Tol<0){Error=TEXT("needs entity, finite x/y and nonnegative tolerance");return nullptr;}
  return [Arg=A[0],X,Y,Tol](const FBotContext& C)->FBotPredicateValue{FVector2D Pos;const FString Id=Entity(C,Arg);const bool Known=C.Combat() && C.Combat()->FindEntity(Id) && BotPredicates::EntityPosition(C,Id,Pos);return {Known && FVector2D::Distance(Pos,FVector2D(X,Y))<=Tol,Known?FString::Printf(TEXT("(%.2f, %.2f)"),Pos.X,Pos.Y):FString(TEXT("unknown"))};};
 }});
 R.Register({TEXT("entity_casting"),TEXT("entity_casting <entity> <skill_id>"),TEXT("Current received cast including admission hydration"),[](const TArray<FString>& A,FString& Error)->FBotPredicateFn
 {
  uint32 Skill;if(A.Num()!=2 || !EntityArg(A[0]) || !Id(A[1],Skill)){Error=TEXT("needs entity and positive skill id");return nullptr;}
  return [Arg=A[0],Skill](const FBotContext& C)->FBotPredicateValue{const auto* S=State(C);const FString Id=Entity(C,Arg);const auto* V=S?S->FindCast(Id):nullptr;const bool Known=C.Combat() && C.Combat()->FindEntity(Id);return {Known && V && V->Cast.IsSet() && V->Cast->SkillId==Skill,!Known?FString(TEXT("unknown")):V && V->Cast.IsSet()?FString::FromInt(V->Cast->SkillId):FString(TEXT("idle"))};};
 }});
 for (const FString& Name : {FString(TEXT("cast_cancel_reason")),FString(TEXT("observed_cast_cancel_reason"))})
  R.RegisterEquality(Name,Name+TEXT(" == <reason>"),TEXT("Most recent accepted cast cancellation"),[](const FString& V,FString& Error){for(const TCHAR* Reason:{TEXT("stopped"),TEXT("moved"),TEXT("damage"),TEXT("incapacitated"),TEXT("target_lost"),TEXT("out_of_range"),TEXT("resources"),TEXT("caster_died"),TEXT("peace_zone"),TEXT("transfer")})if(V==Reason)return true;Error=TEXT("invalid cast cancellation reason");return false;},[Name](const FBotContext& C,const FString& V,FString& O)
  {
   static const TCHAR* Reasons[]={TEXT("unknown"),TEXT("stopped"),TEXT("moved"),TEXT("damage"),TEXT("incapacitated"),TEXT("target_lost"),TEXT("out_of_range"),TEXT("resources"),TEXT("caster_died"),TEXT("peace_zone"),TEXT("transfer")};
   const auto* S=State(C);uint32 Reason=0;if(S){if(Name==TEXT("observed_cast_cancel_reason") && S->GetObservedCancel().IsSet())Reason=S->GetObservedCancel()->Reason;else if(Name==TEXT("cast_cancel_reason") && C.Net()){if(const auto* Cast=S->FindCast(C.Net()->GetOwnEntityId()))Reason=Cast->CancelReason;}}
   O=Reason<UE_ARRAY_COUNT(Reasons)?Reasons[Reason]:TEXT("unknown");return Reason>0 && O==V;
  });
	auto ValidId = [](const TArray<FString>& A) { uint32 I; return Id(A[0],I); };
	Number(R,TEXT("skill_reuse_remaining"),1,[](const FBotContext& C,const TArray<FString>& A)->TOptional<uint64> { const auto* S=State(C); return S ? S->ReuseRemaining(uint32(FCString::Strtoui64(*A[0],nullptr,10))) : TOptional<uint64>(); },ValidId);
	Number(R,TEXT("learned_rank"),1,[](const FBotContext& C,const TArray<FString>& A)->TOptional<uint64> { const auto* S=State(C); return S && S->HasSkillBaseline() ? TOptional<uint64>(S->GetLearned().FindRef(uint32(FCString::Strtoui64(*A[0],nullptr,10)))) : TOptional<uint64>(); },ValidId);
	for (const FString& Name : {FString(TEXT("own_karma")),FString(TEXT("own_pk")),FString(TEXT("own_pvp"))})
		Number(R,Name,0,[Name](const FBotContext& C,const auto&)->TOptional<uint64> { const auto* S=State(C); const auto* P=S && C.Net() ? S->FindPvp(C.Net()->GetOwnEntityId()) : nullptr; return P ? TOptional<uint64>(Name==TEXT("own_karma") ? P->Karma : Name==TEXT("own_pk") ? P->PkCount : P->PvpCount) : TOptional<uint64>(); });
	Number(R,TEXT("own_sp"),0,[](const FBotContext& C,const auto&)->TOptional<uint64>{ const auto* S=State(C); return S ? S->GetSp() : TOptional<uint64>(); });
	Number(R,TEXT("sp_gained_total"),0,[](const FBotContext& C,const auto&)->TOptional<uint64>{ const auto* S=State(C); return S && S->HasOwnerState() ? TOptional<uint64>(S->GetSpGainedTotal()) : TOptional<uint64>(); });
	Number(R,TEXT("own_death_penalty"),0,[](const FBotContext& C,const auto&)->TOptional<uint64>{ const auto* S=State(C); return S && S->HasOwnerState() ? TOptional<uint64>(S->GetDeathPenalty()) : TOptional<uint64>(); });
 Number(R,TEXT("buff_replacements"),0,[](const FBotContext& C,const auto&)->TOptional<uint64>{const auto* S=Effects(C);return S && C.Net()?S->Replacements(C.Net()->GetOwnEntityId()):TOptional<uint64>();});
 R.RegisterFlag(TEXT("replacement_remove_before_apply"),TEXT("Witnessed replacement removal followed by a new effect ID for same skill/pool"),[](const FBotContext& C){const auto* S=Effects(C);return S && S->ReplacementOrdered().IsSet() && *S->ReplacementOrdered();});
 R.RegisterEquality(TEXT("effect_remove_reason"),TEXT("effect_remove_reason == <reason>"),TEXT("Accepted removal of a known effect ID"),[](const FString& V,FString& Error){for(const TCHAR* Reason:{TEXT("expired"),TEXT("replaced"),TEXT("overflow"),TEXT("dispelled"),TEXT("damage"),TEXT("death"),TEXT("transfer")})if(V==Reason)return true;Error=TEXT("invalid effect removal reason");return false;},[](const FBotContext& C,const FString& V,FString& O){static const TCHAR* Names[]={TEXT("unknown"),TEXT("expired"),TEXT("replaced"),TEXT("overflow"),TEXT("dispelled"),TEXT("damage"),TEXT("death"),TEXT("transfer")};const auto* S=Effects(C);const auto Reason=S?S->GetRemoveReason():TOptional<uint32>();O=Reason.IsSet() && *Reason<UE_ARRAY_COUNT(Names)?Names[*Reason]:TEXT("unknown");return Reason.IsSet() && *Reason>0 && O==V;});
	Number(R,TEXT("effect_count"),0,[](const FBotContext& C,const auto&)->TOptional<uint64>{ const auto* S=Effects(C); const auto* N=C.Net(); return S && N && S->IsKnown(N->GetOwnEntityId()) ? TOptional<uint64>(S->Active(N->GetOwnEntityId()).Num()) : TOptional<uint64>(); });
	Number(R,TEXT("vitals_max_hp"),1,[](const FBotContext& C,const TArray<FString>& A)->TOptional<uint64>{ const auto* E=C.Combat() ? C.Combat()->FindEntity(Entity(C,A[0])) : nullptr; return E ? TOptional<uint64>(E->MaxHp) : TOptional<uint64>(); },[](const auto& A){return EntityArg(A[0]);});
	for (const FString& Name : {FString(TEXT("skill_ready")),FString(TEXT("buff_active")),FString(TEXT("buff_inactive")),FString(TEXT("debuff_active")),FString(TEXT("own_mp_cost_matches"))})
		R.Register({Name,Name+TEXT(" <skill_id>"),TEXT("Received skill/effect state"),[Name](const TArray<FString>& A,FString& Error)->FBotPredicateFn
		{
			uint32 Skill; if(A.Num()!=1 || !Id(A[0],Skill)){Error=TEXT("needs a positive uint32 skill id");return nullptr;}
			return [Name,Skill](const FBotContext& C)->FBotPredicateValue
			{
				if(Name==TEXT("own_mp_cost_matches")) return {false,TEXT("unknown: catalogue/SkillStateSync contract has no per-rank MP costs")};
				if(Name==TEXT("skill_ready")){ const auto* S=State(C); const auto V=S ? S->ReuseRemaining(Skill) : TOptional<uint64>(); return {V.IsSet() && *V==0,V.IsSet()?FString::Printf(TEXT("remaining %llu ticks"),*V):FString(TEXT("unknown"))}; }
				const auto* S=Effects(C); const auto* N=C.Net(); const bool Known=S && N && S->IsKnown(N->GetOwnEntityId()); const auto Rows=S && N ? S->Active(N->GetOwnEntityId()) : TArray<FEffectApplied>(); bool Active=false;
				for(const auto& E:Rows) if(E.SkillId==Skill && (Name==TEXT("debuff_active") ? E.Pool==3 : E.Pool==1 || E.Pool==2)) Active=true;
				return {Known && (Name==TEXT("buff_inactive")?!Active:Active),Known?(Active?FString(TEXT("active")):FString(TEXT("absent"))):FString(TEXT("unknown"))};
			};
		}});
	for (const FString& Name : {FString(TEXT("entity_effect_active")),FString(TEXT("entity_effect_inactive"))})
		R.Register({Name,Name+TEXT(" <own|target|uuid> <skill_id>"),TEXT("Visible entity effects"),[Name](const TArray<FString>& A,FString& Error)->FBotPredicateFn
		{
			uint32 Skill; if(A.Num()!=2 || !EntityArg(A[0]) || !Id(A[1],Skill)){Error=TEXT("needs entity and positive skill id");return nullptr;}
			return [Arg=A[0],Skill,Name](const FBotContext& C)->FBotPredicateValue { const auto* S=Effects(C); const FString Id=Entity(C,Arg); const bool Known=S && S->IsKnown(Id) && C.Combat() && C.Combat()->FindEntity(Id); const bool Active=S && S->Has(Id,Skill); return {Known && (Name==TEXT("entity_effect_active") ? Active : !Active), !Known?FString(TEXT("unknown")):Active?FString(TEXT("active")):FString(TEXT("absent"))}; };
		}});
	R.Register({TEXT("learnable_contains"),TEXT("learnable_contains <id> <rank>"),TEXT("Actual ListLearnableSkills row"),[](const TArray<FString>& A,FString& Error)->FBotPredicateFn
	{
		uint32 IdValue,Rank; if(A.Num()!=2 || !Id(A[0],IdValue) || !Id(A[1],Rank)){Error=TEXT("needs positive skill and rank");return nullptr;}
		return [IdValue,Rank](const FBotContext& C)->FBotPredicateValue { const auto* S=State(C); if(!S || !S->HasLearnable()) return {false,TEXT("unknown")}; for(const auto& Row:S->GetLearnable().Skills) if(Row.Skill.SkillId.Value==IdValue && Row.Skill.SkillLevel.Value==Rank) return {true,TEXT("present")}; return {false,TEXT("absent")}; };
	}});
	R.Register({TEXT("catalogue_skill_status"),TEXT("catalogue_skill_status <id> == executable|deferred"),TEXT("Server catalogue capability"),[](const TArray<FString>& A,FString& Error)->FBotPredicateFn
	{
		uint32 IdValue; if(A.Num()!=3 || !Id(A[0],IdValue) || A[1]!=TEXT("==") || (A[2]!=TEXT("executable") && A[2]!=TEXT("deferred"))){Error=TEXT("invalid catalogue status arguments");return nullptr;}
		return [IdValue,V=A[2]](const FBotContext& C)->FBotPredicateValue { const auto* S=State(C); const auto* Row=S ? S->FindSkill(IdValue) : nullptr; const FString Status=Row?(Row->EffectImplemented?TEXT("executable"):TEXT("deferred")):TEXT("unknown"); return {Row && V==Status,Status}; };
	}});
	for (const FString& Name : {FString(TEXT("own_flag")),FString(TEXT("target_flag"))})
		R.RegisterEquality(Name,Name+TEXT(" == none|pvp|chaotic"),TEXT("Received karma and flag deadline"),[](const FString& V,FString& E){ if(V==TEXT("none") || V==TEXT("pvp") || V==TEXT("chaotic")) return true; E=TEXT("invalid flag"); return false; },[Name](const FBotContext& C,const FString& V,FString& O){const auto* S=State(C); O=S ? S->Flag(Entity(C,Name==TEXT("own_flag")?TEXT("own"):TEXT("target"))) : TEXT("unknown"); return O!=TEXT("unknown") && O==V;});
	R.RegisterEquality(TEXT("cast_state"),TEXT("cast_state == idle|casting"),TEXT("Received owner cast lifecycle"),[](const FString& V,FString& E){if(V==TEXT("idle") || V==TEXT("casting")) return true; E=TEXT("invalid cast state");return false;},[](const FBotContext& C,const FString& V,FString& O){const auto* S=State(C); const auto* N=C.Net(); const bool Known=S && N && C.Combat() && C.Combat()->FindOwnEntity(); const auto* Cast=Known?S->FindCast(N->GetOwnEntityId()):nullptr; O=!Known?TEXT("unknown"):Cast && Cast->Cast.IsSet()?TEXT("casting"):TEXT("idle");return Known && O==V;});
	R.RegisterEquality(TEXT("learn_rpc_error"),TEXT("learn_rpc_error == <canonical code|0-16>"),TEXT("Completed LearnSkill canonical status"),[](const FString& V,FString& E){for(uint8 I=0;I<=16;++I)if(ErrorMatches(V,static_cast<ENetError>(I)))return true;E=TEXT("invalid gRPC code");return false;},[](const FBotContext& C,const FString& V,FString& O){const auto* S=State(C);O=S && S->HasLearnResult()?ErrorName(S->GetLearnResult().Error):TEXT("pending");return S && S->HasLearnResult() && ErrorMatches(V,S->GetLearnResult().Error);});
	R.RegisterFlag(TEXT("resurrect_offer_pending"),TEXT("Received unexpired offered resurrection"),[](const FBotContext& C){const auto* S=State(C);return S && S->OfferPending();});
 for(const FString& Name:{FString(TEXT("cast_starts")),FString(TEXT("cast_results")),FString(TEXT("cast_cancellations")),FString(TEXT("observed_cast_results")),FString(TEXT("cast_damage_results")),FString(TEXT("skill_learned_events")),FString(TEXT("sp_gained_events"))})
  Number(R,Name,0,[Name](const FBotContext& C,const auto&)->TOptional<uint64>{const auto* S=State(C);return S?S->History(Name):TOptional<uint64>();});
 R.RegisterFlag(TEXT("learn_receipt_matches_first"),TEXT("Identical frozen successful replies for the same retry key"),[](const FBotContext& C){const auto* S=State(C);return S && S->ReceiptMatchesFirst().IsSet() && *S->ReceiptMatchesFirst();});
	R.RegisterFlag(TEXT("skill_rpc_done"),TEXT("Newest skill RPC completed"),[](const FBotContext& C){const auto* S=State(C);return S && S->RpcDone();});
}

bool SkillBotPredicates::ValidateCommand(const TArray<FString>& T, FString& Error)
{
	using namespace SkillBotDetail;
	if(T.IsEmpty())return true; const FString Name=T[0].ToLower(); uint32 Skill=0,Rank=0; uint64 OfferId=0; FGuid Guid;
	bool Valid=true;
	if(Name==TEXT("nf.cast")) Valid=(T.Num()==2 || T.Num()==3) && Id(T[1],Skill) && (T.Num()==2 || T[2]==TEXT("self") || T[2]==TEXT("target") || FGuid::Parse(T[2],Guid));
	else if(Name==TEXT("nf.cancelcast") || Name==TEXT("nf.learnableskills") || Name==TEXT("nf.skillcatalog")) Valid=T.Num()==1;
	else if(Name==TEXT("nf.learn")) Valid=(T.Num()==3 || T.Num()==4) && Id(T[1],Skill) && Id(T[2],Rank) && (T.Num()==3 || FGuid::Parse(T[3],Guid));
	else if(Name==TEXT("nf.acceptresurrect")) Valid=T.Num()<=3 && (T.Num()<2 || T[1]==TEXT("latest") || (ParseUnsigned(T[1],OfferId) && OfferId)) && (T.Num()<3 || T[2]==TEXT("accept") || T[2]==TEXT("decline"));
	if(!Valid) Error=TEXT("invalid Phase 3 command arguments"); return Valid;
}
#if !UE_BUILD_SHIPPING
namespace SkillBotCommands
{
	USkillStateSubsystem* State(UWorld* W) { return W && W->GetGameInstance() ? W->GetGameInstance()->GetSubsystem<USkillStateSubsystem>() : nullptr; }
	UNetClientSubsystem* Net(UWorld* W) { return W && W->GetGameInstance() ? W->GetGameInstance()->GetSubsystem<UNetClientSubsystem>() : nullptr; }
	void RpcLog(const FNetResult& R){UE_LOG(LogNightfallBot,Display,TEXT("skill RPC: %s (%s)"),*SkillBotDetail::ErrorName(R.Error),*R.Message);}
	bool Args(const TCHAR* Name,const TArray<FString>& A){TArray<FString> Tokens;Tokens.Add(Name);Tokens.Append(A);FString Error;if(SkillBotPredicates::ValidateCommand(Tokens,Error))return true;UE_LOG(LogNightfallBot,Warning,TEXT("%s: %s"),Name,*Error);return false;}
	FAutoConsoleCommandWithWorldAndArgs Cast(TEXT("nf.Cast"),TEXT("nf.Cast <skill_id> [target|self|uuid]"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W)
	{
		if(!Args(TEXT("nf.Cast"),A))return;auto* N=Net(W);if(!N)return;TOptional<FString> Target;
		if(A.Num()==2){if(A[1]==TEXT("self"))Target=N->GetOwnEntityId();else if(A[1]!=TEXT("target"))Target=A[1];}
		UE_LOG(LogNightfallBot,Display,TEXT("nf.Cast seq %u"),N->SendCastSkill(uint32(FCString::Strtoui64(*A[0],nullptr,10)),Target));
	}));
	FAutoConsoleCommandWithWorldAndArgs Cancel(TEXT("nf.CancelCast"),TEXT("Cancel current cast"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!Args(TEXT("nf.CancelCast"),A))return;if(auto* N=Net(W))N->SendCancelCast();}));
	FAutoConsoleCommandWithWorldAndArgs Learn(TEXT("nf.Learn"),TEXT("nf.Learn <id> <rank> [idempotency_uuid]"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!Args(TEXT("nf.Learn"),A))return;if(auto* S=State(W))S->Learn(uint32(FCString::Strtoui64(*A[0],nullptr,10)),uint32(FCString::Strtoui64(*A[1],nullptr,10)),RpcLog,A.Num()==3?A[2]:FString());}));
	FAutoConsoleCommandWithWorldAndArgs Learnable(TEXT("nf.LearnableSkills"),TEXT("List received learnable rows"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!Args(TEXT("nf.LearnableSkills"),A))return;if(auto* S=State(W))S->LoadLearnable([Weak=TWeakObjectPtr<USkillStateSubsystem>(S)](const FNetResult& R)
 {RpcLog(R);if(R.IsOk() && Weak.IsValid())for(const auto& Row:Weak->GetLearnable().Skills)UE_LOG(LogNightfallBot,Display,TEXT("skill %u rank %u SP %llu eligible=%d trainer=%d %s"),Row.Skill.SkillId.Value,Row.Skill.SkillLevel.Value,Row.Skill.SpCost.Value,Row.Eligible,Row.TrainerRequired,*FString::Join(Row.Unmet,TEXT(", ")));});}));
	FAutoConsoleCommandWithWorldAndArgs Catalog(TEXT("nf.SkillCatalog"),TEXT("Load skill catalogue"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!Args(TEXT("nf.SkillCatalog"),A))return;if(auto* S=State(W))S->LoadCatalog([Weak=TWeakObjectPtr<USkillStateSubsystem>(S)](const FNetResult& R)
 {RpcLog(R);if(R.IsOk() && Weak.IsValid())for(const auto& Row:Weak->GetCatalog().Skills)UE_LOG(LogNightfallBot,Display,TEXT("skill %u %s %s %s"),Row.SkillId.Value,*Row.DisplayName,Row.EffectImplemented?TEXT("executable"):TEXT("deferred"),*FString::Join(Row.DeferredReasons,TEXT(", ")));});}));
	FAutoConsoleCommandWithWorldAndArgs Resurrect(TEXT("nf.AcceptResurrect"),TEXT("nf.AcceptResurrect [offer_id|latest] [accept|decline]"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W)
	{
		if(!Args(TEXT("nf.AcceptResurrect"),A))return;auto* S=State(W);auto* N=Net(W);if(!S || !N)return;uint64 Offer=0;
		if(A.Num()>0 && A[0]!=TEXT("latest"))SkillBotPredicates::ParseUnsigned(A[0],Offer);else Offer=S->GetLastOfferId();
		if(!Offer){UE_LOG(LogNightfallBot,Warning,TEXT("nf.AcceptResurrect: no received offer"));return;}N->SendAcceptResurrect(Offer,A.Num()<2 || A[1]==TEXT("accept"));
	}));
}
#endif
