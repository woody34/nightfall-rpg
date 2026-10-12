#include "ItemBotPredicates.h"
#include "BotPredicates.h"
#include "SkillBotPredicates.h"
#include "Items/InventoryStateSubsystem.h"
#include "Items/ItemClientMetadata.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"
#include "HAL/IConsoleManager.h"
#include "Algo/Reverse.h"
namespace ItemBotDetail
{
 bool Num(const FString& S,uint64 Max=MAX_uint64,bool Zero=false) { uint64 N=0; return SkillBotPredicates::ParseUnsigned(S,N) && (N || Zero) && N<=Max; }
 bool Guid(const FString& S) { FGuid G; return FGuid::ParseExact(S,EGuidFormats::DigitsWithHyphens,G) && G.IsValid(); }
 bool Item(const FString& S) { return S.StartsWith(TEXT("instance:"))?Num(S.Mid(9),MAX_int64):Num(S.StartsWith(TEXT("template:"))?S.Mid(9):S,MAX_uint32); }
 bool Target(const FString& S) { return S==TEXT("self") || S==TEXT("target") || (S.StartsWith(TEXT("entity:")) && Guid(S.Mid(7))); }
 bool Slot(const FString& S) { return uint8(NightfallItems::Slot(S))!=0; }
 bool Op(const FString& S) { return S==TEXT("==") || S==TEXT("!=") || S==TEXT("<") || S==TEXT("<=") || S==TEXT(">") || S==TEXT(">="); }
 template<typename T> bool Compare(T V,const FString& O,T N) { if (O==TEXT("=="))return V==N; if(O==TEXT("!="))return V!=N; if(O==TEXT("<"))return V<N; if(O==TEXT("<="))return V<=N; if(O==TEXT(">"))return V>N; return V>=N; }
 UInventoryStateSubsystem* State(const FBotContext& C) { return C.GameInstance?C.GameInstance->GetSubsystem<UInventoryStateSubsystem>():nullptr; }
 bool Digits(const FString& S)
 {
  if(S.IsEmpty())return false;for(TCHAR C:S)if(C<TEXT('0') || C>TEXT('9'))return false;return true;
 }
 FString Normalise(const FString& S){int32 I=0;while(I<S.Len()-1 && S[I]==TEXT('0'))++I;return S.Mid(I);}
 bool Ratio(const FString& S,FString& N,FString& D)
 {
  FString L,R;D=TEXT("1");
  if(S.Split(TEXT("/"),&L,&R)){if(!Digits(L) || !Digits(R) || R[0]==TEXT('0'))return false;N=Normalise(L);D=R;return true;}
  if(S.Split(TEXT("."),&L,&R)){if(!Digits(L) || !Digits(R))return false;N=Normalise(L+R);D+=FString::ChrN(R.Len(),TEXT('0'));return true;}
  if(!Digits(S))return false;N=Normalise(S);return true;
 }
 FString Multiply(const FString& S,uint64 Factor)
 {
  if(!Factor || S==TEXT("0"))return TEXT("0");TArray<TCHAR> Digits;Digits.Reserve(S.Len()+24);__uint128_t Carry=0;
  for(int32 I=S.Len()-1;I>=0;--I){const __uint128_t V=__uint128_t(S[I]-TEXT('0'))*Factor+Carry;Digits.Add(TCHAR(TEXT('0')+V%10));Carry=V/10;}
  while(Carry){Digits.Add(TCHAR(TEXT('0')+Carry%10));Carry/=10;}Algo::Reverse(Digits);return FString(Digits.Num(),Digits.GetData());
 }
 bool SignedDigits(const FString& S){return Digits(S.StartsWith(TEXT("-"))?S.Mid(1):S);}
 int32 DecimalOrder(const FString& A,const FString& B){if(A.Len()!=B.Len())return A.Len()<B.Len()?-1:1;const int32 C=A.Compare(B,ESearchCase::CaseSensitive);return C<0?-1:C>0?1:0;}
 int32 SignedOrder(const FString& A,const FString& B)
 {
  const FString Left=Normalise(A.StartsWith(TEXT("-"))?A.Mid(1):A),Right=Normalise(B.StartsWith(TEXT("-"))?B.Mid(1):B);
  const bool NegativeLeft=A.StartsWith(TEXT("-")) && Left!=TEXT("0"),NegativeRight=B.StartsWith(TEXT("-")) && Right!=TEXT("0");
  if(NegativeLeft!=NegativeRight)return NegativeLeft?-1:1;
  const int32 Order=DecimalOrder(Left,Right);return NegativeLeft?-Order:Order;
 }
 const TCHAR* Operations[]={TEXT("unknown"),TEXT("equip"),TEXT("unequip"),TEXT("use"),TEXT("destroy"),TEXT("enchant"),TEXT("toggle_auto_shot"),TEXT("refine"),TEXT("cancel_refine"),TEXT("exchange_sa"),TEXT("crystallize"),TEXT("grant")};
 const TCHAR* Outcomes[]={TEXT("unknown"),TEXT("success"),TEXT("safe_failed"),TEXT("blessed_reset"),TEXT("destroyed"),TEXT("success_no_increment")};
 const TCHAR* Errors[]={TEXT("none"),TEXT("FAILED_PRECONDITION"),TEXT("UNAVAILABLE"),TEXT("PERMISSION_DENIED"),TEXT("INVALID_ARGUMENT")};
 const TCHAR* ErrorName(ENetError E) { if(E==ENetError::None)return Errors[0];if(E==ENetError::FailedPrecondition)return Errors[1];if(E==ENetError::Unavailable)return Errors[2];if(E==ENetError::PermissionDenied)return Errors[3];if(E==ENetError::InvalidArgument)return Errors[4];return TEXT("unknown"); }
}
bool ItemBotPredicates::ValidateCommand(const TArray<FString>& T,FString& Error)
{
 using namespace ItemBotDetail; if (T.IsEmpty())return true; FString Name=T[0].ToLower(); TArray<FString>A=T;A.RemoveAt(0);
 const TSet<FString> Names={TEXT("nf.equip"),TEXT("nf.unequip"),TEXT("nf.useitem"),TEXT("nf.destroy"),TEXT("nf.crystallize"),TEXT("nf.enchant"),TEXT("nf.autoshot"),TEXT("nf.refine"),TEXT("nf.cancelrefine"),TEXT("nf.exchangesa"),TEXT("nf.grantitem"),TEXT("nf.itemcatalog"),TEXT("nf.markitemstate")};
 if(!Names.Contains(Name))return true; bool V=false;
 if(Name==TEXT("nf.itemcatalog") || Name==TEXT("nf.markitemstate")) V=A.IsEmpty();
 else {
  if (!A.IsEmpty() && Guid(A.Last()))A.Pop();
  if(Name==TEXT("nf.equip"))V=(A.Num()==1 || A.Num()==2) && Item(A[0]) && (A.Num()==1 || Slot(A[1]));
  else if(Name==TEXT("nf.unequip"))V=A.Num()==1 && Slot(A[0]);
  else if(Name==TEXT("nf.useitem"))V=(A.Num()==1 || A.Num()==2) && Item(A[0]) && (A.Num()==1 || Target(A[1]));
  else if(Name==TEXT("nf.destroy"))V=A.Num()==2 && Item(A[0]) && Num(A[1],MAX_int64);
  else if(Name==TEXT("nf.crystallize"))V=A.Num()==1 && Item(A[0]);
  else if(Name==TEXT("nf.enchant"))V=(A.Num()==2 || A.Num()==3) && Item(A[0]) && Item(A[1]) && (A.Num()==2 || (A[2].StartsWith(TEXT("support:")) && Item(A[2].Mid(8))));
  else if(Name==TEXT("nf.autoshot"))V=A.Num()==2 && Num(A[0],MAX_uint32) && (A[1]==TEXT("on") || A[1]==TEXT("off"));
  else if(Name==TEXT("nf.refine") || Name==TEXT("nf.exchangesa"))V=(A.Num()==2 || A.Num()==3) && Item(A[0]) && (Name==TEXT("nf.refine")?Item(A[1]):Num(A[1])) && (A.Num()==2 || (Target(A[2]) && A[2]!=TEXT("self")));
  else if(Name==TEXT("nf.cancelrefine"))V=(A.Num()==1 || A.Num()==2) && Item(A[0]) && (A.Num()==1 || (Target(A[1]) && A[1]!=TEXT("self")));
  else if(Name==TEXT("nf.grantitem"))V=A.Num()==2 && Num(A[0],MAX_uint32) && Num(A[1],MAX_int64);
 }
 if(!V)Error=TEXT("invalid Phase 4 command arguments");return V;
}
void ItemBotPredicates::Register(FBotPredicateRegistry& R)
{
 using namespace ItemBotDetail;
 // Register explicitly to keep the harness's native-registration audit readable.
 for (const FString& Name : {FString(TEXT("inventory_has")), FString(TEXT("equipped")), FString(TEXT("enchant_level")), FString(TEXT("weight_pct")), FString(TEXT("overload_level")), FString(TEXT("grade_penalty")), FString(TEXT("autoshot_on")), FString(TEXT("arrows_remaining")), FString(TEXT("inventory_revision")), FString(TEXT("item_baseline")), FString(TEXT("item_catalog_ready")), FString(TEXT("item_operation")), FString(TEXT("enchant_outcome")), FString(TEXT("refine_outcome")), FString(TEXT("enchant_result_consistent")), FString(TEXT("refined")), FString(TEXT("item_rpc_error")), FString(TEXT("item_receipt_matches_first")), FString(TEXT("blacksmith_known")), FString(TEXT("refine_result_consistent")), FString(TEXT("inventory_revision_since_mark")), FString(TEXT("owner_stat_delta")), FString(TEXT("owner_stats_match_mark"))}) R.Register({Name,Name+TEXT(" (E6 phase4-grammar.py)"),TEXT("Received item facts only"),[Name](const TArray<FString>& A,FString& Error)->FBotPredicateFn
 {
  uint64 N=0;FString Numerator,Denominator;bool Valid=false; const bool Flag=Name==TEXT("item_baseline") || Name==TEXT("item_catalog_ready") || Name==TEXT("enchant_result_consistent") || Name==TEXT("item_receipt_matches_first") || Name==TEXT("blacksmith_known") || Name==TEXT("refine_result_consistent") || Name==TEXT("owner_stats_match_mark");
  if(Flag)Valid=A.IsEmpty();
  else if(Name==TEXT("owner_stat_delta"))Valid=A.Num()==3 && (A[0]==TEXT("max_hp") || A[0]==TEXT("p_def")) && Op(A[1]) && SignedDigits(A[2]);
  else if(Name==TEXT("autoshot_on") || Name==TEXT("refined"))Valid=A.Num()==1 && (Name==TEXT("refined")?Item(A[0]):Num(A[0],MAX_uint32));
  else if(Name==TEXT("item_operation") || Name==TEXT("enchant_outcome") || Name==TEXT("refine_outcome") || Name==TEXT("item_rpc_error")) {
   if(A.Num()==2 && A[0]==TEXT("==")) { if(Name==TEXT("item_operation")){for(int32 I=1;I<UE_ARRAY_COUNT(Operations);++I)if(I!=7 && I!=8)Valid|=A[1]==Operations[I];}else if(Name==TEXT("enchant_outcome")){for(int32 I=1;I<UE_ARRAY_COUNT(Outcomes);++I)Valid|=A[1]==Outcomes[I];}else if(Name==TEXT("refine_outcome"))Valid=A[1]==TEXT("applied") || A[1]==TEXT("removed");else for(const auto* E:Errors)Valid|=A[1]==E; }
  } else if(Name==TEXT("inventory_has") || Name==TEXT("equipped") || Name==TEXT("enchant_level") || Name==TEXT("grade_penalty"))Valid=A.Num()==3 && (Name==TEXT("inventory_has")?Num(A[0],MAX_uint32):Name==TEXT("grade_penalty")?(A[0]==TEXT("weapon") || A[0]==TEXT("armor")):Slot(A[0])) && (Name==TEXT("equipped")?(A[1]==TEXT("==") || A[1]==TEXT("!=")):Op(A[1])) && SkillBotPredicates::ParseUnsigned(A[2],N);
  else Valid=A.Num()==2 && Op(A[0]) && (Name==TEXT("weight_pct")?Ratio(A[1],Numerator,Denominator):SkillBotPredicates::ParseUnsigned(A[1],N));
  if(!Valid){Error=TEXT("invalid Phase 4 predicate arguments");return nullptr;}
  return [Name,A,N,Numerator,Denominator](const FBotContext& C)->FBotPredicateValue {
   const auto* S=State(C);if(!S)return {false,TEXT("unknown")};
   if(Name==TEXT("item_baseline"))return {S->HasBaseline(),S->HasBaseline()?TEXT("received baseline"):TEXT("unknown")};
   if(Name==TEXT("item_catalog_ready"))return {S->HasCatalog(),S->HasCatalog()?TEXT("catalogue ready"):TEXT("unknown")};
   if(Name==TEXT("blacksmith_known"))return {!S->Blacksmith().IsEmpty(),S->Blacksmith().IsEmpty()?TEXT("unknown"):S->Blacksmith()};
   if(Name==TEXT("item_rpc_error")) { const FString E=S->RpcDone()?ErrorName(S->RpcResult().Error):TEXT("unknown");return {S->RpcDone() && E==A[1],E}; }
   if(Name==TEXT("item_receipt_matches_first")) { const auto V=S->ReceiptMatchesFirst();return {V.IsSet() && *V,V.IsSet()?(*V?TEXT("matching receipt"):TEXT("receipt mismatch")):TEXT("unknown")}; }
   if(!S->HasBaseline())return {false,TEXT("unknown")};
   if(Name==TEXT("refine_result_consistent")){const auto V=S->RefineConsistent();return {V.IsSet() && *V,V.IsSet()?TEXT("received refine/inventory coherence"):TEXT("unknown")};}
   if(Name==TEXT("owner_stats_match_mark")){const auto V=S->OwnerStatsMatchMark();return {V.IsSet() && *V,V.IsSet()?TEXT("received sheet/maxima comparison"):TEXT("unknown")};}
   if(Name==TEXT("owner_stat_delta")){const auto V=S->OwnerStatDelta(A[0]);return {V.IsSet() && Compare<int32>(SignedOrder(*V,A[2]),A[1],0),V.IsSet()?*V:FString(TEXT("unknown"))};}
   if(Name==TEXT("item_operation")) { const auto V=S->LastOperation(); const FString E=V.IsSet() && uint8(*V)<UE_ARRAY_COUNT(Operations)?Operations[uint8(*V)]:TEXT("unknown"); return {V.IsSet() && E==A[1],E}; }
   if(Name==TEXT("enchant_outcome")) { const auto& V=S->GetEnchant();const FString E=V.IsSet() && uint8(V->Outcome)<UE_ARRAY_COUNT(Outcomes)?Outcomes[uint8(V->Outcome)]:TEXT("unknown");return {V.IsSet() && E==A[1],E}; }
   if(Name==TEXT("refine_outcome")){const auto& V=S->GetRefine();const FString E=V.IsSet()?(V->Outcome==EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_APPLIED?TEXT("applied"):TEXT("removed")):TEXT("unknown");return {V.IsSet() && E==A[1],E};}
   if(Name==TEXT("enchant_result_consistent"))return {S->EnchantConsistent(),S->GetEnchant().IsSet()?TEXT("received inventory/result coherence"):TEXT("unknown")};
   if(Name==TEXT("refined")) { const auto Id=S->Resolve(A[0]);const auto* I=Id.IsSet()?S->Find(*Id):nullptr;const auto& R=S->GetRefine();const bool Known=I && R.IsSet() && R->TargetInstanceId.Value==I->InstanceId.Value && R->InventoryRevision.Value<=S->Revision();return {Known && R->Outcome==EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_APPLIED && I->_augment_id._augment_idCase==EGrpcNightfallV1OwnedItem_augment_id::AugmentId && R->_augment_id._augment_idCase==EGrpcNightfallV1RefineResult_augment_id::AugmentId && I->_augment_id.AugmentId.Value==R->_augment_id.AugmentId.Value,Known?TEXT("received augment/result coherence"):TEXT("unknown")}; }
   if(Name==TEXT("autoshot_on"))return {S->AutoShot(uint32(FCString::Strtoui64(*A[0],nullptr,10))),TEXT("received toggle set")};
   TOptional<uint64> V;FString O=A[0];
   if(Name==TEXT("inventory_has")){V=S->Count(uint32(FCString::Strtoui64(*A[0],nullptr,10)));O=A[1];}
   else if(Name==TEXT("equipped") || Name==TEXT("enchant_level")){const auto* I=S->Equipped(NightfallItems::Slot(A[0]));if(I)V=Name==TEXT("equipped")?I->TemplateId.Value:I->Enchant.Value;else if(Name==TEXT("equipped"))V=uint64(0);O=A[1];}
   else if(Name==TEXT("inventory_revision"))V=S->Revision();
   else if(Name==TEXT("inventory_revision_since_mark"))V=S->RevisionSinceMark();
   else if(Name==TEXT("overload_level"))V=S->GetInventory().Penalties.OverloadLevel.Value;
   else if(Name==TEXT("grade_penalty")){V=A[0]==TEXT("weapon")?S->GetInventory().Penalties.WeaponLevel.Value:S->GetInventory().Penalties.ArmorLevel.Value;O=A[1];}
   else if(Name==TEXT("arrows_remaining")){const auto* I=S->Equipped(EGrpcNightfallV1EquipSlot::EQUIP_SLOT_LHAND);const auto* Def=I?S->Definition(I->TemplateId):nullptr; if(!I)V=0;else if(Def && NightfallItems::IsArrow(*Def))V=I->Count.Value;}
   else if(Name==TEXT("weight_pct")){const auto& W=S->GetInventory().Weight;if(W.MaxLoad<=0 || W.Weight<0)return {false,TEXT("unknown")};const int32 Order=DecimalOrder(Multiply(Multiply(Denominator,uint64(W.Weight)),100),Multiply(Numerator,uint64(W.MaxLoad)));return {Compare<int32>(Order,O,0),FString::Printf(TEXT("100*%lld/%lld percent"),W.Weight,W.MaxLoad)};}
   return {V.IsSet() && Compare<uint64>(*V,O,N),V.IsSet()?FString::Printf(TEXT("%llu"),*V):FString(TEXT("unknown"))};
  };
 }});
}
#if !UE_BUILD_SHIPPING
namespace ItemBotCommands
{
 FAutoConsoleCommandWithWorldAndArgs MarkItemState(TEXT("nf.MarkItemState"),TEXT("Save received inventory revision and owner stats"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();TArray<FString> Tokens={TEXT("nf.MarkItemState")};Tokens.Append(A);if(S && !S->Execute(Tokens))UE_LOG(LogTemp,Warning,TEXT("nf.MarkItemState: synchronized received inventory/stats required"));}));
 FAutoConsoleCommandWithWorldAndArgs Equip(TEXT("nf.Equip"),TEXT("E6 Phase 4 item command"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;TArray<FString>T;T.Add(TEXT("nf.Equip"));T.Append(A);auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();if(S && !S->Execute(T))UE_LOG(LogTemp,Warning,TEXT("nf.Equip: invalid arguments or unknown received item/target"));}));
 FAutoConsoleCommandWithWorldAndArgs Unequip(TEXT("nf.Unequip"),TEXT("E6 Phase 4 item command"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;TArray<FString>T;T.Add(TEXT("nf.Unequip"));T.Append(A);auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();if(S && !S->Execute(T))UE_LOG(LogTemp,Warning,TEXT("nf.Unequip: invalid arguments or unknown received item/target"));}));
 FAutoConsoleCommandWithWorldAndArgs UseItem(TEXT("nf.UseItem"),TEXT("E6 Phase 4 item command"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;TArray<FString>T;T.Add(TEXT("nf.UseItem"));T.Append(A);auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();if(S && !S->Execute(T))UE_LOG(LogTemp,Warning,TEXT("nf.UseItem: invalid arguments or unknown received item/target"));}));
 FAutoConsoleCommandWithWorldAndArgs Destroy(TEXT("nf.Destroy"),TEXT("E6 Phase 4 item command"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;TArray<FString>T;T.Add(TEXT("nf.Destroy"));T.Append(A);auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();if(S && !S->Execute(T))UE_LOG(LogTemp,Warning,TEXT("nf.Destroy: invalid arguments or unknown received item/target"));}));
 FAutoConsoleCommandWithWorldAndArgs Crystallize(TEXT("nf.Crystallize"),TEXT("E6 Phase 4 item command"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;TArray<FString>T;T.Add(TEXT("nf.Crystallize"));T.Append(A);auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();if(S && !S->Execute(T))UE_LOG(LogTemp,Warning,TEXT("nf.Crystallize: invalid arguments or unknown received item/target"));}));
 FAutoConsoleCommandWithWorldAndArgs Enchant(TEXT("nf.Enchant"),TEXT("E6 Phase 4 item command"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;TArray<FString>T;T.Add(TEXT("nf.Enchant"));T.Append(A);auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();if(S && !S->Execute(T))UE_LOG(LogTemp,Warning,TEXT("nf.Enchant: invalid arguments or unknown received item/target"));}));
 FAutoConsoleCommandWithWorldAndArgs AutoShot(TEXT("nf.AutoShot"),TEXT("E6 Phase 4 item command"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;TArray<FString>T;T.Add(TEXT("nf.AutoShot"));T.Append(A);auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();if(S && !S->Execute(T))UE_LOG(LogTemp,Warning,TEXT("nf.AutoShot: invalid arguments or unknown received item/target"));}));
 FAutoConsoleCommandWithWorldAndArgs Refine(TEXT("nf.Refine"),TEXT("E6 Phase 4 item command"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;TArray<FString>T;T.Add(TEXT("nf.Refine"));T.Append(A);auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();if(S && !S->Execute(T))UE_LOG(LogTemp,Warning,TEXT("nf.Refine: invalid arguments or unknown received item/target"));}));
 FAutoConsoleCommandWithWorldAndArgs CancelRefine(TEXT("nf.CancelRefine"),TEXT("E6 Phase 4 item command"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;TArray<FString>T;T.Add(TEXT("nf.CancelRefine"));T.Append(A);auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();if(S && !S->Execute(T))UE_LOG(LogTemp,Warning,TEXT("nf.CancelRefine: invalid arguments or unknown received item/target"));}));
 FAutoConsoleCommandWithWorldAndArgs ExchangeSA(TEXT("nf.ExchangeSA"),TEXT("E6 Phase 4 item command"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;TArray<FString>T;T.Add(TEXT("nf.ExchangeSA"));T.Append(A);auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();if(S && !S->Execute(T))UE_LOG(LogTemp,Warning,TEXT("nf.ExchangeSA: invalid arguments or unknown received item/target"));}));
 FAutoConsoleCommandWithWorldAndArgs GrantItem(TEXT("nf.GrantItem"),TEXT("E6 Phase 4 item command"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;TArray<FString>T;T.Add(TEXT("nf.GrantItem"));T.Append(A);auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();if(S && !S->Execute(T))UE_LOG(LogTemp,Warning,TEXT("nf.GrantItem: invalid arguments or unknown received item/target"));}));
 FAutoConsoleCommandWithWorldAndArgs ItemCatalog(TEXT("nf.ItemCatalog"),TEXT("E6 Phase 4 item command"),FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A,UWorld* W){if(!W || !W->GetGameInstance())return;TArray<FString>T;T.Add(TEXT("nf.ItemCatalog"));T.Append(A);auto* S=W->GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>();if(S && !S->Execute(T))UE_LOG(LogTemp,Warning,TEXT("nf.ItemCatalog: invalid arguments or unknown received item/target"));}));
}
#endif
