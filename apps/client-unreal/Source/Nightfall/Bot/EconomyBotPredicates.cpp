#include "EconomyBotPredicates.h"
#include "BotPredicates.h"
#include "SkillBotPredicates.h"
#include "Economy/EconomyStateSubsystem.h"
#include "Items/InventoryStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"
#include "HAL/IConsoleManager.h"

namespace EconomyBotDetail
{
 using EOp = EGrpcNightfallV1ItemOperation;
 // phase5-grammar.py PREDICATES order; `rejected` is the Phase 1 builtin and is not re-registered.
 const TCHAR* const Predicates[] = { TEXT("ground_item_exists"), TEXT("ground_item_kind"), TEXT("ground_loot_owner"), TEXT("ground_drop_protection"), TEXT("ground_drop_protection_none"), TEXT("ground_protection_expired"), TEXT("ground_item_count_between"), TEXT("ground_item_gone"), TEXT("ground_despawn_on_grid"), TEXT("ground_item_any"), TEXT("ground_item_count"),
  TEXT("economy_result"), TEXT("economy_result_count"), TEXT("economy_export_marker"), TEXT("own_level_between"), TEXT("rules_rate_multiplier"), TEXT("herb_skill_early_abort"), TEXT("pk_drop_decays"),
  TEXT("party_member_count"), TEXT("loot_mode"), TEXT("party_invite_received"), TEXT("loot_recipient_in_party"), TEXT("loot_recipient"), TEXT("by_turn_cursor_advanced"), TEXT("loot_distributed"), TEXT("loot_distributed_mode"), TEXT("loot_distributed_count"), TEXT("loot_distributed_spoil"), TEXT("own_out_of_party_range"),
  TEXT("corpse_spoiled"), TEXT("corpse_sweepable"), TEXT("corpse_swept"), TEXT("corpse_until_delta"), TEXT("sweep_recipient_in_party"), TEXT("sweep_outcome_received"), TEXT("party_sweep_done"), TEXT("adena_split_share_between"), TEXT("adena_split_origin"), TEXT("loot_vote_phase"),
  TEXT("xp_gained_events"), TEXT("xp_gained_last"), TEXT("sp_gained_last"), TEXT("xp_gained_last_between"), TEXT("xp_share_matches_export"), TEXT("sp_gained_events_for"), TEXT("entity_gone"), TEXT("own_pk_count"), TEXT("pk_drop_count_between"), TEXT("pk_dropped_excluded"), TEXT("pk_unequipped_before_roll"),
  TEXT("merchant_known"), TEXT("adena_delta"), TEXT("refund_tab_count"), TEXT("refund_tab_oldest_destroyed"), TEXT("vendor_stock"), TEXT("vendor_restock_due_delta"), TEXT("exchanger_known"), TEXT("enchant_of"),
  TEXT("craft_result_any"), TEXT("craft_result"), TEXT("craft_materials_consumed"), TEXT("craft_masterwork"), TEXT("craft_fee_before_roll"), TEXT("store_state"), TEXT("store_state_of"), TEXT("manufacture_gate_ticks"),
  TEXT("own_trade_confirmed"), TEXT("trade_state"), TEXT("trade_transfer_pairs"), TEXT("trade_request_received"), TEXT("trade_cancel_reason"), TEXT("store_entries_skipped"),
  TEXT("warehouse_keeper_known"), TEXT("container_fee"), TEXT("warehouse_has"), TEXT("container_busy_retry_observed"), TEXT("freight_capacity"), TEXT("freight_has"), TEXT("cod_credit_received"), TEXT("mail_received"), TEXT("mail_expiry_hours"), TEXT("mail_gone"), TEXT("mail_has_attachments"),
  TEXT("auctioneer_known"), TEXT("auction_state"), TEXT("auction_highest_bid"), TEXT("auction_award_to_warehouse"), TEXT("auction_leader"),
  TEXT("manor_manager_known"), TEXT("corpse_seeded_or_sow_failed"), TEXT("harvest_done"), TEXT("manor_drops_unchanged"), TEXT("manor_fee_equals_p_div_20"), TEXT("manor_result"), TEXT("epoch_reconcile_rows_before_admission"), TEXT("recipe_registered") };
 const TCHAR* const Commands[] = { TEXT("nf.PickUp"), TEXT("nf.Buy"), TEXT("nf.Sell"), TEXT("nf.Refund"), TEXT("nf.Multisell"), TEXT("nf.RegisterRecipe"), TEXT("nf.DeleteRecipe"), TEXT("nf.Craft"), TEXT("nf.ManufactureOrder"), TEXT("nf.TradeRequest"), TEXT("nf.TradeAnswer"), TEXT("nf.TradeAdd"), TEXT("nf.TradeConfirm"), TEXT("nf.TradeCancel"), TEXT("nf.StoreSell"), TEXT("nf.StoreBuy"), TEXT("nf.StoreManufacture"), TEXT("nf.StoreClose"), TEXT("nf.StoreBuyFrom"), TEXT("nf.StoreSellTo"), TEXT("nf.PartyInvite"), TEXT("nf.PartyAnswer"), TEXT("nf.PartyLeave"), TEXT("nf.PartyKick"), TEXT("nf.PartyLeader"), TEXT("nf.PartyLootVote"), TEXT("nf.PartyLootAnswer"), TEXT("nf.Deposit"), TEXT("nf.Withdraw"), TEXT("nf.FreightSend"), TEXT("nf.FreightWithdraw"), TEXT("nf.MailSend"), TEXT("nf.MailTake"), TEXT("nf.MailReject"), TEXT("nf.MailCancel"), TEXT("nf.MailDelete"), TEXT("nf.MailRead"), TEXT("nf.AuctionBid"), TEXT("nf.AuctionCancel"), TEXT("nf.ManorBuySeed"), TEXT("nf.ManorSellCrop") };
 const TCHAR* const LootModes[] = { TEXT("FINDERS_KEEPERS"), TEXT("RANDOM"), TEXT("RANDOM_INCLUDING_SPOIL"), TEXT("BY_TURN"), TEXT("BY_TURN_INCLUDING_SPOIL") };
 const TCHAR* const StoreKinds[] = { TEXT("NONE"), TEXT("SELL"), TEXT("PACKAGE_SELL"), TEXT("BUY"), TEXT("MANUFACTURE") };
 const TCHAR* const TradePhases[] = { TEXT("STARTED"), TEXT("ITEM_ADDED"), TEXT("PARTNER_CONFIRMED"), TEXT("COMPLETED"), TEXT("CANCELLED") };
 const TCHAR* const VotePhases[] = { TEXT("REQUESTED"), TEXT("APPLIED"), TEXT("CANCELLED") };
 const TCHAR* const AuctionStates[] = { TEXT("CREATED"), TEXT("STARTED"), TEXT("FINISHED") };
 const TCHAR* const CancelReasons[] = { TEXT("departure"), TEXT("out_of_range"), TEXT("use_item"), TEXT("death") };
 template<size_t N> bool In(const FString& S, const TCHAR* const (&Set)[N]) { for (const TCHAR* V : Set) if (S == V) return true; return false; }
 bool Num(const FString& S, uint64 Max = MAX_uint64, bool Zero = false) { uint64 N = 0; return SkillBotPredicates::ParseUnsigned(S, N) && (N || Zero) && N <= Max; }
 uint64 Value(const FString& S) { uint64 N = 0; SkillBotPredicates::ParseUnsigned(S, N); return N; }
 bool Guid(const FString& S) { FGuid G; return FGuid::ParseExact(S, EGuidFormats::DigitsWithHyphens, G) && G.IsValid(); }
 bool Role(const FString& S) { return S == TEXT("a") || S == TEXT("b") || S == TEXT("c"); }
 bool Item(const FString& S) { return S.StartsWith(TEXT("instance:")) ? Num(S.Mid(9), MAX_int64) : Num(S.StartsWith(TEXT("template:")) ? S.Mid(9) : S, MAX_uint32); }
 bool TemplateRef(const FString& S) { return S.StartsWith(TEXT("template:")) && Num(S.Mid(9), MAX_uint32); }
 bool NpcRef(const FString& S) { return S.StartsWith(TEXT("template:npc_")) && Num(S.Mid(13), 9999999999ull) && S.Mid(13).Len() <= 10 && S[13] != TEXT('0'); }
 FString NpcTemplate(const FString& S) { return S.Mid(9); } // `template:npc_<id>` -> `npc_<id>`, the overlay template id (ResolveTemplateTarget's string)
 bool Op(const FString& S) { return S == TEXT("==") || S == TEXT("!=") || S == TEXT("<") || S == TEXT("<=") || S == TEXT(">") || S == TEXT(">="); }
 bool Signed(const FString& S) { return Num(S.StartsWith(TEXT("-")) ? S.Mid(1) : S, MAX_int64, true); }
 bool Subject(const FString& S) { if (S.IsEmpty() || S.Len() > 128) return false; for (TCHAR C : S) if (!FChar::IsAlnum(C) && C != TEXT('_') && C != TEXT('-')) return false; return true; }
 bool Decimal(const FString& S) { FString L, R; if (S.Split(TEXT("."), &L, &R)) return Num(L, MAX_uint64, true) && Num(R, MAX_uint64, true); return Num(S, MAX_uint64, true); }
 template<typename T> bool Compare(T V, const FString& O, T N) { if (O == TEXT("==")) return V == N; if (O == TEXT("!=")) return V != N; if (O == TEXT("<")) return V < N; if (O == TEXT("<=")) return V <= N; if (O == TEXT(">")) return V > N; return V >= N; }
 /** `template:<id>:<a>[:<b>...]` lists with `Parts` numeric fields; bare `<id>:<a>` when allowed. */
 bool Entries(const FString& Text, int32 Parts, bool bAllowBare = false)
 {
  if (Text.IsEmpty()) return false; TArray<FString> Rows; Text.ParseIntoArray(Rows, TEXT(","), true);
  for (const FString& Row : Rows)
  {
   TArray<FString> F; Row.ParseIntoArray(F, TEXT(":"), false);
   if (F.Num() == 2 + Parts && F[0] == TEXT("template") && Num(F[1], MAX_uint32)) { bool Ok = true; for (int32 I = 2; I < F.Num(); ++I) Ok = Ok && Num(F[I], MAX_int64, true); if (Ok) continue; }
   if (bAllowBare && F.Num() == 1 + Parts && Num(F[0], MAX_uint32)) { bool Ok = true; for (int32 I = 1; I < F.Num(); ++I) Ok = Ok && Num(F[I], MAX_int64); if (Ok) continue; }
   return false;
  }
  return true;
 }
 template<typename E> FString EnumName(E V, const TCHAR* Prefix) { FString N = StaticEnum<E>()->GetNameStringByValue(int64(V)); if (N.IsEmpty()) return FString::Printf(TEXT("%lld"), int64(V)); N.RemoveFromStart(Prefix); return N; }
 UEconomyStateSubsystem* Economy(const FBotContext& C) { return C.GameInstance ? C.GameInstance->GetSubsystem<UEconomyStateSubsystem>() : nullptr; }
 UInventoryStateSubsystem* Items(const FBotContext& C) { return C.GameInstance ? C.GameInstance->GetSubsystem<UInventoryStateSubsystem>() : nullptr; }
 FBotPredicateValue Unknown(const TCHAR* Why = TEXT("unknown")) { return { false, Why }; }
 FBotPredicateValue Number(TOptional<uint64> V, const FString& O, uint64 N) { return { V.IsSet() && Compare<uint64>(*V, O, N), V.IsSet() ? FString::Printf(TEXT("%llu"), *V) : FString(TEXT("unknown")) }; }
 FBotPredicateValue Between(TOptional<uint64> V, uint64 Lo, uint64 Hi) { return { V.IsSet() && *V >= Lo && *V <= Hi, V.IsSet() ? FString::Printf(TEXT("%llu"), *V) : FString(TEXT("unknown")) }; }
 FBotPredicateValue Named(const FString& Observed, const FString& Expected, bool bKnown = true) { return { bKnown && Observed == Expected, bKnown ? Observed : FString(TEXT("unknown")) }; }
 FBotPredicateValue Flag(bool bHolds, const TCHAR* Yes, const TCHAR* No = TEXT("unknown")) { return { bHolds, bHolds ? Yes : No }; }
 const FGrpcNightfallV1CraftOutcome* Craft(const UEconomyStateSubsystem* E, bool bManufactureOnly = false)
 {
  for (int32 I = E->Receipts().Num() - 1; I >= 0; --I) { const auto& R = E->Receipts()[I].Result; if (R.Outcome.OutcomeCase == EGrpcNightfallV1EconomyResultOutcome::Craft && R.Outcome.Craft.IsValid() && (!bManufactureOnly || R.Operation == EOp::ITEM_OPERATION_PRIVATE_MANUFACTURE)) return R.Outcome.Craft.Get(); }
  return nullptr;
 }
 const FGrpcNightfallV1PickupOutcome* Pickup(const UEconomyStateSubsystem* E) { const auto* R = E->LastReceiptOf(EOp::ITEM_OPERATION_PICK_UP_ITEM); return R && R->Result.Outcome.OutcomeCase == EGrpcNightfallV1EconomyResultOutcome::Pickup && R->Result.Outcome.Pickup.IsValid() ? R->Result.Outcome.Pickup.Get() : nullptr; }
 template<typename T> const T* Outcome(const UEconomyStateSubsystem::FReceipt* R, EGrpcNightfallV1EconomyResultOutcome Case, const TSharedPtr<T> FGrpcNightfallV1EconomyResultOutcome::* Member) { return R && R->Result.Outcome.OutcomeCase == Case && (R->Result.Outcome.*Member).IsValid() ? (R->Result.Outcome.*Member).Get() : nullptr; }
 const UEconomyStateSubsystem::FReceipt* LastOfAny(const UEconomyStateSubsystem* E, std::initializer_list<EOp> Ops) { for (int32 I = E->Receipts().Num() - 1; I >= 0; --I) for (EOp O : Ops) if (E->Receipts()[I].Result.Operation == O) return &E->Receipts()[I]; return nullptr; }
 const FGrpcNightfallV1MailHeader* Header(const TOptional<FGrpcNightfallV1ListMailResponse>& View, const FString& Subject) { if (!View.IsSet()) return nullptr; for (const auto& H : View->Messages) if (H.Subject == Subject) return &H; return nullptr; }
}

const TArray<FString>& EconomyBotPredicates::PredicateNames() { static const TArray<FString> N = [] { TArray<FString> A; for (const TCHAR* P : EconomyBotDetail::Predicates) A.Add(P); return A; }(); return N; }
const TArray<FString>& EconomyBotPredicates::CommandNames() { static const TArray<FString> N = [] { TArray<FString> A; for (const TCHAR* P : EconomyBotDetail::Commands) A.Add(P); return A; }(); return N; }

bool EconomyBotPredicates::ValidateCommand(const TArray<FString>& T, FString& Error)
{
 using namespace EconomyBotDetail;
 if (T.IsEmpty()) return true; const FString Name = T[0].ToLower(); bool bKnown = false; for (const TCHAR* C : Commands) bKnown = bKnown || Name == FString(C).ToLower(); if (!bKnown) return true;
 TArray<FString> A = T; A.RemoveAt(0); if (!A.IsEmpty() && Guid(A.Last())) A.Pop(); const int32 N = A.Num(); bool V = false;
 auto Mode = [](const FString& S) { return In(S, LootModes); };
 if (Name == TEXT("nf.pickup")) V = N == 1 && (A[0] == TEXT("nearest") || TemplateRef(A[0]));
 else if (Name == TEXT("nf.buy")) V = N == 4 && A[0] == TEXT("merchant") && Num(A[1]) && Num(A[2], MAX_uint32) && Num(A[3], MAX_int64);
 else if (Name == TEXT("nf.sell")) V = N == 4 && A[0] == TEXT("merchant") && Num(A[1]) && Item(A[2]) && Num(A[3], MAX_int64);
 else if (Name == TEXT("nf.refund")) V = N == 3 && A[0] == TEXT("merchant") && Num(A[1]) && Num(A[2], 11, true);
 else if (Name == TEXT("nf.multisell")) V = (N == 4 || N == 5) && A[0] == TEXT("exchanger") && Num(A[1], MAX_uint32) && Num(A[2], MAX_uint32, true) && Num(A[3], 5000) && (N == 4 || (A[4].StartsWith(TEXT("enchant:")) && Num(A[4].Mid(8), MAX_uint32, true)));
 else if (Name == TEXT("nf.registerrecipe")) V = N == 1 && Item(A[0]);
 else if (Name == TEXT("nf.deleterecipe") || Name == TEXT("nf.craft")) V = N == 1 && Num(A[0], MAX_uint32);
 else if (Name == TEXT("nf.manufactureorder")) V = N == 2 && Role(A[0]) && Num(A[1], MAX_uint32);
 else if (Name == TEXT("nf.traderequest") || Name == TEXT("nf.partykick") || Name == TEXT("nf.partyleader")) V = N == 1 && Role(A[0]);
 else if (Name == TEXT("nf.tradeanswer") || Name == TEXT("nf.partylootanswer")) V = N == 1 && (A[0] == TEXT("accept") || A[0] == TEXT("decline"));
 else if (Name == TEXT("nf.tradeadd")) V = N == 2 && Item(A[0]) && Num(A[1], MAX_int64, true);
 else if (Name == TEXT("nf.tradeconfirm") || Name == TEXT("nf.tradecancel") || Name == TEXT("nf.storeclose") || Name == TEXT("nf.partyleave")) V = N == 0;
 else if (Name == TEXT("nf.storesell")) V = (N == 2 || N == 3) && (N == 2 || A[1] == TEXT("package")) && Entries(A.Last(), 2);
 else if (Name == TEXT("nf.storebuy")) V = N == 2 && Entries(A[1], 3);
 else if (Name == TEXT("nf.storemanufacture")) { V = N == 2; if (V) { TArray<FString> Rows; A[1].ParseIntoArray(Rows, TEXT(","), true); V = !Rows.IsEmpty(); for (const FString& R : Rows) { TArray<FString> F; R.ParseIntoArray(F, TEXT(":"), false); V = V && F.Num() == 2 && Num(F[0], MAX_uint32) && Num(F[1], MAX_int64); } } }
 else if (Name == TEXT("nf.storebuyfrom") || Name == TEXT("nf.storesellto")) V = N == 2 && Role(A[0]) && Entries(A[1], 2);
 else if (Name == TEXT("nf.partyinvite")) V = (N == 1 || N == 2) && Role(A[0]) && (N == 1 || Mode(A[1]));
 else if (Name == TEXT("nf.partyanswer")) V = N == 1 && (A[0] == TEXT("-1") || A[0] == TEXT("0") || A[0] == TEXT("1"));
 else if (Name == TEXT("nf.partylootvote")) V = N == 1 && Mode(A[0]);
 else if (Name == TEXT("nf.deposit")) V = N == 1 && Entries(A[0], 1);
 else if (Name == TEXT("nf.withdraw") || Name == TEXT("nf.freightwithdraw")) V = N == 1 && Entries(A[0], 1, true);
 else if (Name == TEXT("nf.freightsend")) V = N == 2 && Role(A[0]) && Entries(A[1], 1);
 else if (Name == TEXT("nf.mailsend"))
 {
  V = N >= 2 && Role(A[0]) && Subject(A[1]); int32 I = 2;
  if (V && A.IsValidIndex(I) && A[I] == TEXT("cod")) { V = A.IsValidIndex(I + 1) && Num(A[I + 1], 99900000000ull, true); I += 2; }
  if (V) V = I == N || (I + 1 == N && Entries(A[I], 1));
 }
 else if (Name == TEXT("nf.mailtake") || Name == TEXT("nf.mailreject") || Name == TEXT("nf.mailcancel") || Name == TEXT("nf.mailread")) V = N == 1 && A[0] == TEXT("latest");
 else if (Name == TEXT("nf.maildelete")) V = (N == 1 || N == 2) && A[0] == TEXT("latest") && (N == 1 || A[1] == TEXT("sent"));
 else if (Name == TEXT("nf.auctionbid")) V = N == 2 && Num(A[0], MAX_uint32) && Num(A[1], 100000000000ull);
 else if (Name == TEXT("nf.auctioncancel")) V = N == 1 && Num(A[0], MAX_uint32);
 else if (Name == TEXT("nf.manorbuyseed")) V = N == 4 && A[0] == TEXT("manor_manager") && Num(A[1], 9) && Num(A[2], MAX_uint32) && Num(A[3], MAX_int64);
 else if (Name == TEXT("nf.manorsellcrop")) V = N == 5 && A[0] == TEXT("manor_manager") && Item(A[1]) && Num(A[2], MAX_uint32) && Num(A[3], 9) && (A[4] == TEXT("all") || Num(A[4], MAX_int64));
 if (!V) Error = TEXT("invalid Phase 5 command arguments"); return V;
}

void EconomyBotPredicates::Register(FBotPredicateRegistry& R)
{
 using namespace EconomyBotDetail;
 for (const FString& Name : PredicateNames()) R.Register({ Name, Name + TEXT(" (E5 phase5-grammar.py)"), TEXT("Received Phase 5 economy facts only"), [Name](const TArray<FString>& A, FString& Error) -> FBotPredicateFn
 {
  const int32 N = A.Num(); bool Valid = false;
  auto Flag0 = [&] { return N == 0; };
  if (Name == TEXT("ground_drop_protection_none") || Name == TEXT("ground_item_any") || Name == TEXT("economy_export_marker") || Name == TEXT("party_invite_received") || Name == TEXT("loot_recipient_in_party") || Name == TEXT("by_turn_cursor_advanced") || Name == TEXT("loot_distributed") || Name == TEXT("own_out_of_party_range") || Name == TEXT("sweep_recipient_in_party") || Name == TEXT("sweep_outcome_received") || Name == TEXT("xp_share_matches_export") || Name == TEXT("pk_unequipped_before_roll") || Name == TEXT("merchant_known") || Name == TEXT("refund_tab_oldest_destroyed") || Name == TEXT("exchanger_known") || Name == TEXT("craft_result_any") || Name == TEXT("craft_fee_before_roll") || Name == TEXT("own_trade_confirmed") || Name == TEXT("trade_request_received") || Name == TEXT("warehouse_keeper_known") || Name == TEXT("container_busy_retry_observed") || Name == TEXT("auctioneer_known") || Name == TEXT("manor_manager_known") || Name == TEXT("manor_drops_unchanged") || Name == TEXT("manor_fee_equals_p_div_20") || Name == TEXT("epoch_reconcile_rows_before_admission")) Valid = Flag0();
  else if (Name == TEXT("ground_item_exists") || Name == TEXT("ground_protection_expired") || Name == TEXT("ground_item_gone") || Name == TEXT("ground_despawn_on_grid") || Name == TEXT("herb_skill_early_abort") || Name == TEXT("auction_award_to_warehouse") || Name == TEXT("craft_materials_consumed") || Name == TEXT("recipe_registered")) Valid = N == 1 && Num(A[0], MAX_uint32);
  else if (Name == TEXT("corpse_spoiled") || Name == TEXT("corpse_sweepable") || Name == TEXT("corpse_swept") || Name == TEXT("party_sweep_done") || Name == TEXT("entity_gone") || Name == TEXT("corpse_seeded_or_sow_failed") || Name == TEXT("harvest_done")) Valid = N == 1 && NpcRef(A[0]);
  else if (Name == TEXT("ground_item_kind")) Valid = N == 3 && Num(A[0], MAX_uint32) && A[1] == TEXT("==") && A[2] == TEXT("3");
  else if (Name == TEXT("ground_loot_owner") || Name == TEXT("ground_drop_protection")) Valid = N == 3 && Num(A[0], MAX_uint32) && (A[1] == TEXT("==") || A[1] == TEXT("!=")) && A[2] == TEXT("self");
  else if (Name == TEXT("ground_item_count_between")) Valid = N == 3 && Num(A[0], MAX_uint32) && Num(A[1], MAX_uint64, true) && Num(A[2], MAX_uint64, true) && Value(A[1]) <= Value(A[2]);
  else if (Name == TEXT("own_level_between") || Name == TEXT("xp_gained_last_between") || Name == TEXT("pk_drop_count_between") || Name == TEXT("adena_split_share_between")) Valid = N == 2 && Num(A[0], MAX_uint64, true) && Num(A[1], MAX_uint64, true) && Value(A[0]) <= Value(A[1]);
  else if (Name == TEXT("economy_result")) Valid = N == 2 && A[0] == TEXT("==") && UEconomyStateSubsystem::ParseOperation(A[1]).IsSet();
  else if (Name == TEXT("economy_result_count")) Valid = N == 3 && UEconomyStateSubsystem::ParseOperation(A[0]).IsSet() && Op(A[1]) && Num(A[2], MAX_uint64, true);
  else if (Name == TEXT("rules_rate_multiplier")) Valid = N == 2 && A[0] == TEXT("==") && Decimal(A[1]);
  else if (Name == TEXT("pk_drop_decays") || Name == TEXT("loot_distributed_spoil") || Name == TEXT("craft_masterwork")) Valid = N == 2 && A[0] == TEXT("==") && (A[1] == TEXT("true") || A[1] == TEXT("false"));
  else if (Name == TEXT("loot_mode") || Name == TEXT("loot_distributed_mode")) Valid = N == 2 && A[0] == TEXT("==") && In(A[1], LootModes);
  else if (Name == TEXT("loot_recipient") || Name == TEXT("auction_leader")) Valid = N == 2 && A[0] == TEXT("!=") && A[1] == TEXT("self");
  else if (Name == TEXT("loot_vote_phase")) Valid = N == 2 && A[0] == TEXT("==") && In(A[1], VotePhases);
  else if (Name == TEXT("trade_state")) Valid = N == 2 && A[0] == TEXT("==") && In(A[1], TradePhases);
  else if (Name == TEXT("trade_cancel_reason")) Valid = N == 2 && A[0] == TEXT("==") && In(A[1], CancelReasons);
  else if (Name == TEXT("store_state")) Valid = N == 2 && A[0] == TEXT("==") && In(A[1], StoreKinds);
  else if (Name == TEXT("store_state_of")) Valid = N == 3 && Role(A[0]) && A[1] == TEXT("==") && In(A[2], StoreKinds);
  else if (Name == TEXT("auction_state")) Valid = N == 2 && A[0] == TEXT("==") && In(A[1], AuctionStates);
  else if (Name == TEXT("adena_split_origin")) Valid = N == 2 && A[0] == TEXT("==") && A[1] == TEXT("npc");
  else if (Name == TEXT("craft_result")) Valid = N == 2 && A[0] == TEXT("==") && (A[1] == TEXT("success") || A[1] == TEXT("fail") || A[1] == TEXT("masterwork"));
  else if (Name == TEXT("adena_delta")) Valid = N == 2 && A[0] == TEXT("==") && Signed(A[1]);
  else if (Name == TEXT("corpse_until_delta") || Name == TEXT("sp_gained_events_for")) Valid = N == 3 && NpcRef(A[0]) && A[1] == TEXT("==") && Num(A[2], MAX_uint64, true);
  else if (Name == TEXT("vendor_stock") || Name == TEXT("vendor_restock_due_delta")) Valid = N == 4 && Num(A[0]) && Num(A[1], MAX_uint32) && A[2] == TEXT("==") && Num(A[3], MAX_uint64, true);
  else if (Name == TEXT("enchant_of") || Name == TEXT("warehouse_has") || Name == TEXT("freight_has")) Valid = N == 3 && Num(A[0], MAX_uint32) && A[1] == TEXT("==") && Num(A[2], MAX_uint64, true);
  else if (Name == TEXT("cod_credit_received")) Valid = N == 1 && Num(A[0]);
  else if (Name == TEXT("mail_received") || Name == TEXT("mail_gone") || Name == TEXT("mail_has_attachments")) Valid = N == 1 && Subject(A[0]);
  else if (Name == TEXT("manor_result")) Valid = N == 4 && A[0] == TEXT("granted") && Num(A[1], MAX_uint32) && A[2] == TEXT(">=") && Num(A[3], MAX_uint64, true);
  else if (Name == TEXT("xp_gained_last") || Name == TEXT("sp_gained_last") || Name == TEXT("refund_tab_count") || Name == TEXT("manufacture_gate_ticks") || Name == TEXT("trade_transfer_pairs") || Name == TEXT("store_entries_skipped") || Name == TEXT("container_fee") || Name == TEXT("freight_capacity") || Name == TEXT("mail_expiry_hours") || Name == TEXT("auction_highest_bid") || Name == TEXT("pk_dropped_excluded")) Valid = N == 2 && A[0] == TEXT("==") && Num(A[1], MAX_uint64, true) && (Name != TEXT("pk_dropped_excluded") || A[1] == TEXT("0"));
  else if (Name == TEXT("ground_item_count") || Name == TEXT("party_member_count") || Name == TEXT("loot_distributed_count") || Name == TEXT("own_pk_count") || Name == TEXT("xp_gained_events")) Valid = N == 2 && Op(A[0]) && Num(A[1], MAX_uint64, true);
  if (!Valid) { Error = TEXT("invalid Phase 5 predicate arguments"); return nullptr; }
  return [Name, A](const FBotContext& C) -> FBotPredicateValue
  {
   UEconomyStateSubsystem* E = Economy(C); UInventoryStateSubsystem* Inv = Items(C); UNetClientSubsystem* Net = C.Net(); if (!E || !Net) return Unknown();
   const FString Own = Net->GetOwnEntityId();
   auto Self = [&](const FString& Id) { return !Id.IsEmpty() && Id.Equals(Own, ESearchCase::IgnoreCase); };
   auto V = [&](int32 I) { return Value(A[I]); };
   // Export/sidecar-bound predicates (reset column `export`/sidecar in phase5-grammar.py): the client registers the grammar and
   // reports UNKNOWN; the C14 export and the oracle sidecars decide them in the live gate (stories 3.15, 6.2, 6.6).
   if (Name == TEXT("rules_rate_multiplier")) return Unknown(TEXT("export-bound: InventorySync.rules_hash is resolved by the Phase5Rules manifest (6.2)"));
   if (Name == TEXT("xp_share_matches_export") || Name == TEXT("vendor_restock_due_delta") || Name == TEXT("epoch_reconcile_rows_before_admission")) return Unknown(TEXT("export-bound (C14 export, story 6.2)"));
   if (Name == TEXT("pk_dropped_excluded") || Name == TEXT("craft_materials_consumed") || Name == TEXT("manor_drops_unchanged")) return Unknown(TEXT("sidecar-bound (oracle sidecar, story 6.6)"));
   if (Name == TEXT("adena_split_origin")) return Unknown(TEXT("no position carrier on LootDistributed/EconomyResult (P5-Q10)"));
   // ground items
   if (Name == TEXT("ground_item_exists") || Name == TEXT("ground_item_kind")) { const auto G = E->GroundOf(uint32(V(0))); return Flag(!G.IsEmpty(), Name == TEXT("ground_item_kind") ? TEXT("3") : TEXT("received ground entity"), TEXT("none")); }
   if (Name == TEXT("ground_loot_owner") || Name == TEXT("ground_drop_protection")) { const auto G = E->GroundOf(uint32(V(0))); if (G.IsEmpty()) return Unknown(); const FString& O = Name == TEXT("ground_loot_owner") ? G[0]->Detail.LootOwner : G[0]->Detail.DropProtectOwner; const bool bSelf = Self(O); return { A[1] == TEXT("==") ? bSelf : !bSelf, O.IsEmpty() ? TEXT("none") : O }; }
   if (Name == TEXT("ground_drop_protection_none")) return E->PkDropsReceived() ? Flag(E->PkDropsAllUnowned(), TEXT("every received PK drop unowned"), TEXT("an owned PK drop")) : Unknown(TEXT("no PK drop received"));
   if (Name == TEXT("ground_protection_expired")) { const auto G = E->GroundOf(uint32(V(0))); if (G.IsEmpty()) return Unknown(); const uint64 T = Net->GetLatestReceivedTick(); return { T >= G[0]->Detail.LootUntilTick.Value && T >= G[0]->Detail.DropProtectUntilTick.Value, FString::Printf(TEXT("tick %llu loot_until %llu drop_protect_until %llu"), T, G[0]->Detail.LootUntilTick.Value, G[0]->Detail.DropProtectUntilTick.Value) }; }
   if (Name == TEXT("ground_item_count_between")) { const auto G = E->GroundOf(uint32(V(0))); return Between(G.IsEmpty() ? TOptional<uint64>() : TOptional<uint64>(G[0]->Detail.Count.Value), V(1), V(2)); }
   if (Name == TEXT("ground_item_gone") || Name == TEXT("ground_despawn_on_grid")) { const UEconomyStateSubsystem::FGoneGroundItem* Gone = nullptr; for (const auto& P : E->GoneGroundItems()) if (P.Value.Detail.TemplateId.Value == V(0) && (!Gone || P.Value.DespawnTick > Gone->DespawnTick)) Gone = &P.Value; if (!Gone) return Unknown(TEXT("no despawn received")); if (Name == TEXT("ground_item_gone")) return { true, TEXT("received despawn") }; return { Gone->DespawnTick % 50 == 0 && Gone->DespawnTick > Gone->Detail.DroppedTick.Value, FString::Printf(TEXT("despawn tick %llu dropped %llu (limit from the sidecar, 6.6)"), Gone->DespawnTick, Gone->Detail.DroppedTick.Value) }; }
   if (Name == TEXT("ground_item_any")) return Flag(!E->GroundItems().IsEmpty(), TEXT("received ground entity"), TEXT("none"));
   if (Name == TEXT("ground_item_count")) return Number(uint64(E->GroundItems().Num()), A[0], V(1));
   // receipts
   if (Name == TEXT("economy_result")) { const auto* L = E->LastReceipt(); return Named(L ? UEconomyStateSubsystem::OperationName(L->Result.Operation) : FString(), A[1], L != nullptr); }
   if (Name == TEXT("economy_result_count")) return Number(uint64(E->ReceiptCount(*UEconomyStateSubsystem::ParseOperation(A[0]))), A[1], V(2));
   if (Name == TEXT("economy_export_marker")) return Flag(E->EconomyExportMarker(), TEXT("received EconomyResult or own kill"), TEXT("none"));
   if (Name == TEXT("own_level_between")) return Between(E->OwnStats().IsSet() ? TOptional<uint64>(E->OwnStats()->Level) : TOptional<uint64>(), V(0), V(1));
   if (Name == TEXT("herb_skill_early_abort")) { const auto T = E->HerbPickupTemplate(); if (!T.IsSet() || *T != V(0)) return Unknown(TEXT("no herb pickup receipt")); return { E->OwnEffectsAfterHerb() == 0, FString::Printf(TEXT("%d own effects after the herb"), E->OwnEffectsAfterHerb()) }; }
   if (Name == TEXT("pk_drop_decays")) { if (!E->PkDropsReceived()) return Unknown(TEXT("no PK drop received")); const bool bDecayed = E->PkDropDespawned(); return { (A[1] == TEXT("true")) == bDecayed, bDecayed ? TEXT("true") : TEXT("false") }; }
   // party
   const auto& Party = E->Party();
   if (Name == TEXT("party_member_count")) return Number(Party.IsSet() ? TOptional<uint64>(Party->Members.Num()) : TOptional<uint64>(), A[0], V(1));
   if (Name == TEXT("loot_mode")) return Named(Party.IsSet() ? EnumName(Party->Mode, TEXT("LOOT_MODE_")) : FString(), A[1], Party.IsSet());
   if (Name == TEXT("party_invite_received")) return Flag(E->Invite().IsSet(), TEXT("received invite"), TEXT("none"));
   if (Name == TEXT("loot_recipient_in_party")) { const auto* P = Pickup(E); if (!P || !Party.IsSet()) return Unknown(); return { E->InParty(P->Recipient), P->Recipient }; }
   if (Name == TEXT("loot_recipient")) { if (E->LootLog().IsEmpty()) return Unknown(); const FString& Rcp = E->LootLog().Last().Recipient; return { !Self(Rcp), Rcp }; }
   if (Name == TEXT("by_turn_cursor_advanced")) { TSet<FString> Rcp; for (const auto& L : E->LootLog()) Rcp.Add(L.Recipient.ToLower()); for (const auto& Rc : E->Receipts()) if (Rc.Result.Outcome.OutcomeCase == EGrpcNightfallV1EconomyResultOutcome::Pickup && Rc.Result.Outcome.Pickup.IsValid()) Rcp.Add(Rc.Result.Outcome.Pickup->Recipient.ToLower()); return { Rcp.Num() >= 2, FString::Printf(TEXT("%d distinct recipients"), Rcp.Num()) }; }
   if (Name == TEXT("loot_distributed")) return Flag(!E->LootLog().IsEmpty(), TEXT("received LootDistributed"), TEXT("none"));
   if (Name == TEXT("loot_distributed_mode")) return Named(E->LootLog().IsEmpty() ? FString() : EnumName(E->LootLog().Last().Mode, TEXT("LOOT_MODE_")), A[1], !E->LootLog().IsEmpty());
   if (Name == TEXT("loot_distributed_count")) return Number(uint64(E->LootLog().Num()), A[0], V(1));
   if (Name == TEXT("loot_distributed_spoil")) { if (E->LootLog().IsEmpty()) return Unknown(); const bool S = E->LootLog().Last().Spoil; return { (A[1] == TEXT("true")) == S, S ? TEXT("true") : TEXT("false") }; }
   if (Name == TEXT("own_out_of_party_range")) { const auto X = E->OwnOutOfPartyRange(); return X.IsSet() ? Flag(*X, TEXT("beyond PartyRange2"), TEXT("within PartyRange2")) : Unknown(TEXT("leader position unknown")); }
   // corpses
   if (Name == TEXT("corpse_spoiled") || Name == TEXT("corpse_sweepable") || Name == TEXT("corpse_swept") || Name == TEXT("party_sweep_done") || Name == TEXT("corpse_until_delta") || Name == TEXT("corpse_seeded_or_sow_failed") || Name == TEXT("harvest_done"))
   {
    const auto* Corpse = E->CorpseOf(NpcTemplate(A[0]));
    if (Name == TEXT("corpse_seeded_or_sow_failed")) { const int32 Sow = E->CastResults(2097); if (!Corpse && !Sow) return Unknown(TEXT("no corpse state and no Sow result")); return { (Corpse && Corpse->State.Seeded) || Sow > 0, Corpse && Corpse->State.Seeded ? TEXT("seeded") : TEXT("Sow result received") }; }
    if (Name == TEXT("harvest_done")) { const auto T = E->LastCastResultTick(2098); if (!T.IsSet()) return Unknown(TEXT("no Harvest result")); return { E->InventoryTick() >= *T, FString::Printf(TEXT("harvest tick %llu inventory tick %llu"), *T, E->InventoryTick()) }; }
    if (!Corpse) return Unknown(TEXT("no corpse state received"));
    if (Name == TEXT("corpse_spoiled")) return Flag(Corpse->State.Spoiled, TEXT("spoiled"), TEXT("not spoiled"));
    if (Name == TEXT("corpse_sweepable")) return Flag(Corpse->State.Sweepable, TEXT("sweepable"), TEXT("not sweepable"));
    if (Name == TEXT("corpse_swept") || Name == TEXT("party_sweep_done")) return Flag(Corpse->State.Swept, TEXT("swept"), TEXT("not swept"));
    if (!Corpse->DespawnTick.IsSet() || *Corpse->DespawnTick < Corpse->FirstTick) return Unknown(TEXT("corpse not despawned"));
    return Number(*Corpse->DespawnTick - Corpse->FirstTick, TEXT("=="), V(2));
   }
   if (Name == TEXT("sweep_recipient_in_party") || Name == TEXT("sweep_outcome_received"))
   {
    const FGrpcNightfallV1LootDistributed* Spoil = nullptr; for (const auto& L : E->LootLog()) if (L.Spoil) Spoil = &L;
    const TOptional<uint64> Rev = Inv ? Inv->RevisionSinceMark() : TOptional<uint64>(); const bool bOwnAdvanced = Rev.IsSet() && *Rev > 0;
    if (!Spoil && !bOwnAdvanced) return Unknown(TEXT("no spoil LootDistributed and no owner revision since the mark"));
    if (Name == TEXT("sweep_outcome_received")) return { true, Spoil ? TEXT("received spoil LootDistributed") : TEXT("owner inventory revision advanced") };
    return Spoil ? FBotPredicateValue{ E->InParty(Spoil->Recipient), Spoil->Recipient } : FBotPredicateValue{ E->InParty(Own), TEXT("owner (revision advanced)") };
   }
   if (Name == TEXT("adena_split_share_between") || Name == TEXT("adena_delta")) { const auto D = E->AdenaDelta(); if (!D.IsSet()) return Unknown(TEXT("no mark or baseline")); if (Name == TEXT("adena_delta")) return { *D == FCString::Atoi64(*A[1]), FString::Printf(TEXT("%lld"), *D) }; return { *D >= 0 && uint64(*D) >= V(0) && uint64(*D) <= V(1), FString::Printf(TEXT("%lld"), *D) }; }
   if (Name == TEXT("loot_vote_phase")) return Named(E->LootVote().IsSet() ? EnumName(E->LootVote()->Phase, TEXT("LOOT_VOTE_PHASE_")) : FString(), A[1], E->LootVote().IsSet());
   // xp / sp / pk
   if (Name == TEXT("xp_gained_events")) return Number(uint64(E->XpEvents()), A[0], V(1));
   if (Name == TEXT("xp_gained_last")) return Number(E->LastXpAmount(), TEXT("=="), V(1));
   if (Name == TEXT("sp_gained_last")) return Number(E->LastSpAmount(), TEXT("=="), V(1));
   if (Name == TEXT("xp_gained_last_between")) return Between(E->LastXpAmount(), V(0), V(1));
   if (Name == TEXT("sp_gained_events_for")) return Number(uint64(E->SpEventsAfterKill(NpcTemplate(A[0]))), TEXT("=="), V(2));
   if (Name == TEXT("entity_gone")) return Flag(E->EntityGone(NpcTemplate(A[0])), TEXT("received despawn"), TEXT("none"));
   if (Name == TEXT("own_pk_count")) return Number(E->OwnStats().IsSet() ? TOptional<uint64>(E->OwnStats()->PkCount) : TOptional<uint64>(), A[0], V(1));
   if (Name == TEXT("pk_drop_count_between")) { const auto Nn = E->PkDropCount(); return Between(Nn.IsSet() ? TOptional<uint64>(uint64(*Nn)) : TOptional<uint64>(), V(0), V(1)); }
   if (Name == TEXT("pk_unequipped_before_roll")) { const auto X = E->PkUnequippedBeforeRoll(); return X.IsSet() ? Flag(*X, TEXT("equipment cleared before the removal"), TEXT("removal first")) : Unknown(TEXT("no death removal received")); }
   // NPC presence
   if (Name == TEXT("merchant_known") || Name == TEXT("exchanger_known") || Name == TEXT("warehouse_keeper_known") || Name == TEXT("auctioneer_known") || Name == TEXT("manor_manager_known")) { FString Who = Name; Who.RemoveFromEnd(TEXT("_known")); const FString Id = E->NpcNamed(Who); return { !Id.IsEmpty(), Id.IsEmpty() ? FString(TEXT("unknown")) : Id }; }
   // vendors
   if (Name == TEXT("refund_tab_count") || Name == TEXT("refund_tab_oldest_destroyed") || Name == TEXT("vendor_stock"))
   {
    if (!E->Buylist().IsSet()) { E->RefreshBuylist(); return Unknown(TEXT("ListBuylist view pending")); }
    const auto& B = *E->Buylist();
    if (Name == TEXT("refund_tab_count")) return Number(uint64(B.RefundTab.Num()), TEXT("=="), V(1));
    if (Name == TEXT("refund_tab_oldest_destroyed")) { if (E->SoldInstances().Num() < 13) return Unknown(TEXT("fewer than 13 sells")); const uint64 First = E->SoldInstances()[0]; bool bListed = false; for (const auto& I : B.RefundTab) bListed = bListed || I.InstanceId.Value == First; return { !bListed, bListed ? TEXT("first sold entry still listed") : TEXT("first sold entry absent") }; }
    if (B.ListId.Value != V(0)) return Unknown(TEXT("other buylist"));
    for (const auto& Entry : B.Entries) if (Entry.TemplateId.Value == V(1)) return Number(Entry.Stock.Value, TEXT("=="), V(3)); return Unknown(TEXT("template not listed"));
   }
   if (Name == TEXT("enchant_of")) { const auto Id = Inv ? Inv->Resolve(FString::Printf(TEXT("template:%llu"), V(0))) : TOptional<uint64>(); const auto* I = Id.IsSet() ? Inv->Find(*Id) : nullptr; return Number(I ? TOptional<uint64>(I->Enchant.Value) : TOptional<uint64>(), TEXT("=="), V(2)); }
   // crafting
   if (Name == TEXT("craft_result_any") || Name == TEXT("craft_result") || Name == TEXT("craft_masterwork") || Name == TEXT("craft_fee_before_roll"))
   {
    const auto* Cr = Craft(E, Name == TEXT("craft_fee_before_roll")); if (!Cr) return Unknown(TEXT("no craft receipt"));
    if (Name == TEXT("craft_result_any")) return { true, TEXT("received craft receipt") };
    const FString Observed = Cr->Masterwork ? TEXT("masterwork") : Cr->Success ? TEXT("success") : TEXT("fail");
    if (Name == TEXT("craft_result")) return { Observed == A[1], Observed };
    if (Name == TEXT("craft_masterwork")) return { (A[1] == TEXT("true")) == Cr->Masterwork, Cr->Masterwork ? TEXT("true") : TEXT("false") };
    return { !Cr->Success && Cr->Fee.Value > 0, FString::Printf(TEXT("fee %llu %s"), Cr->Fee.Value, Observed == TEXT("fail") ? TEXT("on a failed craft") : TEXT("on a successful craft")) };
   }
   if (Name == TEXT("store_state") || Name == TEXT("store_state_of")) { const FString Owner = Name == TEXT("store_state") ? Own : E->ResolveRole(A[0]); const auto* S = Owner.IsEmpty() ? nullptr : E->Store(Owner); return Named(S ? EnumName(S->Kind, TEXT("STORE_KIND_")) : FString(), A.Last(), S != nullptr); }
   if (Name == TEXT("manufacture_gate_ticks")) return Number(E->ManufactureGateTicks(), TEXT("=="), V(1));
   // trade
   const auto& Trade = E->Trade();
   if (Name == TEXT("own_trade_confirmed")) return Trade.IsSet() ? Flag(Trade->OwnConfirmed, TEXT("own confirmed"), TEXT("not confirmed")) : Unknown();
   if (Name == TEXT("trade_state")) return Named(Trade.IsSet() ? EnumName(Trade->Phase, TEXT("TRADE_PHASE_")) : FString(), A[1], Trade.IsSet());
   if (Name == TEXT("trade_transfer_pairs")) { const auto* T = Outcome(E->LastReceiptOf(EOp::ITEM_OPERATION_TRADE_CONFIRM), EGrpcNightfallV1EconomyResultOutcome::Trade, &FGrpcNightfallV1EconomyResultOutcome::Trade); return Number(T ? TOptional<uint64>(uint64(T->Given.Num() + T->Received.Num())) : TOptional<uint64>(), TEXT("=="), V(1)); }
   if (Name == TEXT("trade_request_received")) return Flag(E->TradeRequest().IsSet(), TEXT("received trade request"), TEXT("none"));
   if (Name == TEXT("trade_cancel_reason")) return Named(Trade.IsSet() && Trade->Phase == EGrpcNightfallV1TradePhase::TRADE_PHASE_CANCELLED ? Trade->CancelReason : FString(), A[1], Trade.IsSet() && Trade->Phase == EGrpcNightfallV1TradePhase::TRADE_PHASE_CANCELLED);
   if (Name == TEXT("store_entries_skipped")) { const auto* S = Outcome(LastOfAny(E, { EOp::ITEM_OPERATION_STORE_BUY, EOp::ITEM_OPERATION_STORE_SELL }), EGrpcNightfallV1EconomyResultOutcome::Store, &FGrpcNightfallV1EconomyResultOutcome::Store); return Number(S ? TOptional<uint64>(uint64(S->Skipped.Num())) : TOptional<uint64>(), TEXT("=="), V(1)); }
   // containers
   if (Name == TEXT("container_fee")) { const auto* Ct = Outcome(LastOfAny(E, { EOp::ITEM_OPERATION_WAREHOUSE_DEPOSIT, EOp::ITEM_OPERATION_WAREHOUSE_WITHDRAW, EOp::ITEM_OPERATION_FREIGHT_SEND, EOp::ITEM_OPERATION_FREIGHT_WITHDRAW }), EGrpcNightfallV1EconomyResultOutcome::Container, &FGrpcNightfallV1EconomyResultOutcome::Container); return Number(Ct ? TOptional<uint64>(Ct->Fee.Value) : TOptional<uint64>(), TEXT("=="), V(1)); }
   if (Name == TEXT("warehouse_has") || Name == TEXT("auction_award_to_warehouse")) { const auto Nn = E->WarehouseCount(uint32(V(0))); if (!Nn.IsSet()) { E->RefreshWarehouse(); return Unknown(TEXT("GetWarehouse view pending")); } return Name == TEXT("warehouse_has") ? Number(Nn, TEXT("=="), V(2)) : FBotPredicateValue{ *Nn > 0, FString::Printf(TEXT("%llu"), *Nn) }; }
   if (Name == TEXT("container_busy_retry_observed")) return Flag(E->ContainerBusyRetryObserved(), TEXT("CONTAINER_BUSY then an accepted retry"), TEXT("none"));
   if (Name == TEXT("freight_capacity") || Name == TEXT("freight_has")) { if (!E->FreightCapacity().IsSet()) { E->RefreshFreight(); return Unknown(TEXT("GetFreight view pending")); } return Name == TEXT("freight_capacity") ? Number(uint64(*E->FreightCapacity()), TEXT("=="), V(1)) : Number(E->FreightCount(uint32(V(0))), TEXT("=="), V(2)); }
   if (Name == TEXT("cod_credit_received")) { for (uint64 Inc : E->AdenaIncrementsSinceMark()) if (Inc == V(0)) return { true, FString::Printf(TEXT("%llu"), Inc) }; return Unknown(E->AdenaIncrementsSinceMark().IsEmpty() ? TEXT("no adena credit since the mark") : TEXT("other credit amounts")); }
   // mail
   if (Name == TEXT("mail_received") || Name == TEXT("mail_has_attachments") || Name == TEXT("mail_gone") || Name == TEXT("mail_expiry_hours"))
   {
    const UEconomyStateSubsystem::FMailNotice* Notice = nullptr; for (const auto& Mn : E->MailNotices()) if (Name == TEXT("mail_expiry_hours") || Mn.Notice.Subject == A[0]) Notice = &Mn;
    const FGrpcNightfallV1MailHeader* H = Name == TEXT("mail_expiry_hours") ? nullptr : Header(E->Inbox(), A[0]);
    if (Name == TEXT("mail_received")) { if (!Notice && !H) { E->RefreshMail(); return Unknown(TEXT("no notification or inbox header")); } return { true, Notice ? TEXT("received MailNotification") : TEXT("listed in the inbox") }; }
    if (Name == TEXT("mail_has_attachments")) { if (!Notice && !H) return Unknown(); return Flag(Notice ? Notice->Notice.HasAttachments : H->HasAttachments, TEXT("has attachments"), TEXT("no attachments")); }
    if (Name == TEXT("mail_gone")) { if (!Notice && !H) return Unknown(TEXT("subject never seen")); if (!E->Inbox().IsSet() || !E->Outbox().IsSet()) { E->RefreshMail(); return Unknown(TEXT("ListMail views pending")); } return Flag(!Header(E->Inbox(), A[0]) && !Header(E->Outbox(), A[0]), TEXT("no longer listed"), TEXT("still listed")); }
    if (!Notice) return Unknown(TEXT("no notification"));
    const FGrpcNightfallV1MailHeader* Hdr = nullptr; if (E->Inbox().IsSet()) for (const auto& Row : E->Inbox()->Messages) if (Row.MessageId == Notice->Notice.MessageId) Hdr = &Row;
    if (!Hdr) { E->RefreshMail(); return Unknown(TEXT("inbox header pending")); }
    const double Hours = double(Hdr->ExpiresUnixMs - Notice->ReceivedUnixMs) / 3600000.0; const int64 Rounded = int64(FMath::RoundHalfFromZero(Hours));
    return { Rounded >= 0 && uint64(Rounded) == V(1), FString::Printf(TEXT("%.3f hours from the notification"), Hours) };
   }
   // auctions
   if (Name == TEXT("auction_state") || Name == TEXT("auction_highest_bid")) { const auto* Au = E->LastAuction(); if (!Au) return Unknown(TEXT("no ItemAuctionStatusChanged")); return Name == TEXT("auction_state") ? Named(EnumName(Au->State, TEXT("AUCTION_STATE_")), A[1]) : Number(Au->HighestBid.Value, TEXT("=="), V(1)); }
   if (Name == TEXT("auction_leader")) { if (!E->AuctionView().IsSet()) { E->RefreshAuctions(); return Unknown(TEXT("ListItemAuctions view pending")); } const auto& Cur = E->AuctionView()->Current; return { Cur.OwnBid.Value > 0 && Cur.OwnBid.Value < Cur.HighestBid.Value, FString::Printf(TEXT("own bid %llu highest %llu"), Cur.OwnBid.Value, Cur.HighestBid.Value) }; }
   // manor
   if (Name == TEXT("manor_fee_equals_p_div_20") || Name == TEXT("manor_result"))
   {
    const auto* Mo = Outcome(LastOfAny(E, { EOp::ITEM_OPERATION_MANOR_SELL_CROP, EOp::ITEM_OPERATION_MANOR_BUY_SEED }), EGrpcNightfallV1EconomyResultOutcome::Manor, &FGrpcNightfallV1EconomyResultOutcome::Manor); if (!Mo) return Unknown(TEXT("no manor receipt"));
    if (Name == TEXT("manor_fee_equals_p_div_20")) return { Mo->Fee.Value == Mo->Adena.Value / 20, FString::Printf(TEXT("fee %llu adena %llu"), Mo->Fee.Value, Mo->Adena.Value) };
    uint64 Granted = 0; for (const auto& G : Mo->Granted) if (G.TemplateId.Value == V(1)) Granted += G.Count.Value; return Number(Granted, TEXT(">="), V(3));
   }
   if (Name == TEXT("recipe_registered")) { if (!E->RecipeBook().IsSet()) return Unknown(TEXT("no RecipeBookSync baseline")); const uint32 Id = uint32(V(0)); bool bListed = false; for (const auto& Rc : E->RecipeBook()->Dwarven) bListed = bListed || Rc.Value == Id; for (const auto& Rc : E->RecipeBook()->Common) bListed = bListed || Rc.Value == Id; return Flag(bListed, TEXT("listed"), TEXT("not listed")); }
   return Unknown();
  };
 } });
}

#if !UE_BUILD_SHIPPING
namespace EconomyBotCommands
{
 void Run(const TCHAR* Command, const TArray<FString>& A, UWorld* W)
 {
  if (!W || !W->GetGameInstance()) return; TArray<FString> T; T.Add(Command); T.Append(A);
  auto* S = W->GetGameInstance()->GetSubsystem<UEconomyStateSubsystem>();
  if (S && !S->Execute(T)) UE_LOG(LogTemp, Warning, TEXT("%s: invalid arguments or unknown received binding (role, NPC, item, store entry or view)"), Command);
 }
#define NIGHTFALL_ECONOMY_COMMAND(Var, Text) FAutoConsoleCommandWithWorldAndArgs Var(TEXT(Text), TEXT("E5 Phase 5 economy command (phase5-grammar.py)"), FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A, UWorld* W) { Run(TEXT(Text), A, W); }));
 NIGHTFALL_ECONOMY_COMMAND(PickUp, "nf.PickUp") NIGHTFALL_ECONOMY_COMMAND(Buy, "nf.Buy") NIGHTFALL_ECONOMY_COMMAND(Sell, "nf.Sell") NIGHTFALL_ECONOMY_COMMAND(Refund, "nf.Refund") NIGHTFALL_ECONOMY_COMMAND(Multisell, "nf.Multisell")
 NIGHTFALL_ECONOMY_COMMAND(RegisterRecipe, "nf.RegisterRecipe") NIGHTFALL_ECONOMY_COMMAND(DeleteRecipe, "nf.DeleteRecipe") NIGHTFALL_ECONOMY_COMMAND(Craft, "nf.Craft") NIGHTFALL_ECONOMY_COMMAND(ManufactureOrder, "nf.ManufactureOrder")
 NIGHTFALL_ECONOMY_COMMAND(TradeRequest, "nf.TradeRequest") NIGHTFALL_ECONOMY_COMMAND(TradeAnswer, "nf.TradeAnswer") NIGHTFALL_ECONOMY_COMMAND(TradeAdd, "nf.TradeAdd") NIGHTFALL_ECONOMY_COMMAND(TradeConfirm, "nf.TradeConfirm") NIGHTFALL_ECONOMY_COMMAND(TradeCancel, "nf.TradeCancel")
 NIGHTFALL_ECONOMY_COMMAND(StoreSell, "nf.StoreSell") NIGHTFALL_ECONOMY_COMMAND(StoreBuy, "nf.StoreBuy") NIGHTFALL_ECONOMY_COMMAND(StoreManufacture, "nf.StoreManufacture") NIGHTFALL_ECONOMY_COMMAND(StoreClose, "nf.StoreClose") NIGHTFALL_ECONOMY_COMMAND(StoreBuyFrom, "nf.StoreBuyFrom") NIGHTFALL_ECONOMY_COMMAND(StoreSellTo, "nf.StoreSellTo")
 NIGHTFALL_ECONOMY_COMMAND(PartyInvite, "nf.PartyInvite") NIGHTFALL_ECONOMY_COMMAND(PartyAnswer, "nf.PartyAnswer") NIGHTFALL_ECONOMY_COMMAND(PartyLeave, "nf.PartyLeave") NIGHTFALL_ECONOMY_COMMAND(PartyKick, "nf.PartyKick") NIGHTFALL_ECONOMY_COMMAND(PartyLeader, "nf.PartyLeader") NIGHTFALL_ECONOMY_COMMAND(PartyLootVote, "nf.PartyLootVote") NIGHTFALL_ECONOMY_COMMAND(PartyLootAnswer, "nf.PartyLootAnswer")
 NIGHTFALL_ECONOMY_COMMAND(Deposit, "nf.Deposit") NIGHTFALL_ECONOMY_COMMAND(Withdraw, "nf.Withdraw") NIGHTFALL_ECONOMY_COMMAND(FreightSend, "nf.FreightSend") NIGHTFALL_ECONOMY_COMMAND(FreightWithdraw, "nf.FreightWithdraw")
 NIGHTFALL_ECONOMY_COMMAND(MailSend, "nf.MailSend") NIGHTFALL_ECONOMY_COMMAND(MailTake, "nf.MailTake") NIGHTFALL_ECONOMY_COMMAND(MailReject, "nf.MailReject") NIGHTFALL_ECONOMY_COMMAND(MailCancel, "nf.MailCancel") NIGHTFALL_ECONOMY_COMMAND(MailDelete, "nf.MailDelete") NIGHTFALL_ECONOMY_COMMAND(MailRead, "nf.MailRead")
 NIGHTFALL_ECONOMY_COMMAND(AuctionBid, "nf.AuctionBid") NIGHTFALL_ECONOMY_COMMAND(AuctionCancel, "nf.AuctionCancel") NIGHTFALL_ECONOMY_COMMAND(ManorBuySeed, "nf.ManorBuySeed") NIGHTFALL_ECONOMY_COMMAND(ManorSellCrop, "nf.ManorSellCrop")
#undef NIGHTFALL_ECONOMY_COMMAND
}
#endif
