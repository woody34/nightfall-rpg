#include "ClassBotPredicates.h"
#include "BotPredicates.h"
#include "BotCharacterName.h"
#include "BotScenarioRunner.h"
#include "Character/ClassStateSubsystem.h"
#include "Character/CharacterCreation.h"
#include "Combat/CombatStateSubsystem.h"
#include "NightfallPlayerController.h"
#include "Net/NetClientSubsystem.h"
#include "World/WorldProxySubsystem.h"
#include "World/RemoteEntityActor.h"
#include "UI/NightfallClassDialog.h"
#include "UnrealClient.h"
#include "Nightfall.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"
#include "HAL/IConsoleManager.h"

namespace
{
	TArray<uint64> CurrentClassState(const FBotContext& C)
	{
		const auto* GI = C.GameInstance;
		const auto* S = GI ? GI->GetSubsystem<UClassStateSubsystem>() : nullptr;
		const auto* N = C.Net(); const auto* Combat = C.Combat();
		const auto* Entity = N && Combat ? Combat->FindEntity(N->GetOwnEntityId()) : nullptr;
		if (!S || !S->GetOwnClassId().IsSet() || !S->HasOptions() || !Entity || !Combat->GetOwn().bCpKnown) return {};
		const auto& Own = Combat->GetOwn(); const auto& Options = S->GetOptions();
		return { S->GetOwnClassId().GetValue(), S->GetCharacter().ClassId.Value, Options.CurrentClassId.Value,
			Own.ClassId, Entity->Hp, Entity->MaxHp, Own.Mp, Own.MaxMp, Own.Cp, Own.MaxCp, Own.Sp, Own.Xp,
			Own.TokenTier1Count, Own.TokenTier2Count, Options.TokenTier1Count.Value, Options.TokenTier2Count.Value,
			static_cast<uint64>(S->GetOwnClassEventCount()), static_cast<uint64>(C.Observations ? C.Observations->OwnClassWireEvents : 0) };
	}
	UClassStateSubsystem* State(const FBotContext& C) { return C.GameInstance ? C.GameInstance->GetSubsystem<UClassStateSubsystem>() : nullptr; }
	FString ErrorName(ENetError Error)
	{
		FString Name = UEnum::GetValueAsString(Error);
		Name.RemoveFromStart(TEXT("ENetError::"));
		return Name.ToLower();
	}
	uint32 Budget(const FGrpcNightfallV1BaseStats& S) { return S.Str.Value + S.Dex.Value + S.Con.Value + S.Int.Value + S.Wit.Value + S.Men.Value; }
}

void ClassBotPredicates::Register(FBotPredicateRegistry& R)
{
	R.RegisterFlag(TEXT("class_rpc_done"), TEXT("The newest class/catalogue/creation RPC completed"), [](const FBotContext& C) { const auto* S = State(C); return S && !S->IsBusy() && !S->GetLastOperation().IsEmpty(); });
	R.RegisterEquality(TEXT("class_rpc_error"), TEXT("class_rpc_error == <none|invalidargument|failedprecondition|...>"), TEXT("Canonical status of newest completed class RPC; pending never matches"),
		[](const FString& V, FString& E)
		{
			for (int32 I = 0; I <= 16; ++I) if (V.Equals(ErrorName(static_cast<ENetError>(I)), ESearchCase::IgnoreCase)) return true;
			E = TEXT("unknown canonical gRPC status"); return false;
		}, [](const FBotContext& C, const FString& V, FString& O)
		{
			const auto* S = State(C);
			O = !S || S->IsBusy() ? TEXT("pending") : ErrorName(S->GetLastResult().Error) + TEXT(": ") + S->GetLastResult().Message;
			return S && !S->IsBusy() && V.Equals(ErrorName(S->GetLastResult().Error), ESearchCase::IgnoreCase);
		});
	auto Number = [&R](const FString& Name, const FString& Desc, TFunction<double(const UClassStateSubsystem&)> Read)
	{
		R.RegisterNumber(Name, Desc, [Read](const FBotContext& C) -> TOptional<double> { const auto* S = State(C); return S ? TOptional<double>(Read(*S)) : TOptional<double>(); });
	};
	Number(TEXT("catalogue_races"), TEXT("Server catalogue race count"), [](const auto& S) { return S.GetCatalogue().Races.Num(); });
	Number(TEXT("catalogue_base_classes"), TEXT("Server catalogue tier-zero class count"), [](const auto& S) { int32 N = 0; for (const auto& C : S.GetCatalogue().Classes) N += C.Tier.Value == 0; return N; });
	Number(TEXT("creation_count"), TEXT("Successful creations in this game instance"), [](const auto& S) { return S.GetCreationCount(); });
	Number(TEXT("creation_race"), TEXT("Race of most recent successful creation"), [](const auto& S) { return static_cast<uint32>(S.GetLastCreated().Race); });
	Number(TEXT("creation_class"), TEXT("Class id of most recent successful creation"), [](const auto& S) { return S.GetLastCreated().ClassId.Value; });
	Number(TEXT("creation_sex"), TEXT("Sex id of most recent successful creation"), [](const auto& S) { return static_cast<uint32>(S.GetLastCreated().Sex); });
	Number(TEXT("creation_budget"), TEXT("Base-stat sum returned by creation"), [](const auto& S) { return Budget(S.GetLastCreated().Stats); });
	Number(TEXT("transfer_granted_skill_keys"), TEXT("Actual newly granted/upgraded keys returned by latest successful transfer"), [](const auto& S) { return S.GetLastGrantedSkillKeys().Num(); });
	R.Register({ TEXT("transfer_granted_keys"), TEXT("transfer_granted_keys <key>..."), TEXT("Exact metadata key set in the latest successful transfer receipt"), [](const TArray<FString>& A, FString& E) -> FBotPredicateFn
	{
		TArray<FString> Expected = A; Expected.Sort();
		if (Expected.IsEmpty()) { E = TEXT("needs at least one key; use transfer_granted_skill_keys == 0 for an empty delta"); return nullptr; }
		for (int32 I = 1; I < Expected.Num(); ++I) if (Expected[I] == Expected[I - 1]) { E = TEXT("duplicate expected key"); return nullptr; }
		return [Expected](const FBotContext& C) -> FBotPredicateValue
		{
			const auto* S = State(C); TArray<FString> Actual = S ? S->GetLastGrantedSkillKeys() : TArray<FString>(); Actual.Sort();
			return { S && Actual == Expected, FString::Join(Actual, TEXT(", ")) };
		};
	} });
	Number(TEXT("observed_class_transfers"), TEXT("Admitted public class changes received for other players"), [](const auto& S) { return S.GetObservedTransferCount(); });
	Number(TEXT("own_class_events"), TEXT("Admitted owner ClassChanged advances since newest admission"), [](const auto& S) { return S.GetOwnClassEventCount(); });
	R.RegisterNumber(TEXT("own_class_wire_events"), TEXT("Every received owner ClassChanged envelope, including stale/duplicate events before projection filtering"), [](const FBotContext& C) -> TOptional<double> { return C.Observations ? TOptional<double>(C.Observations->OwnClassWireEvents) : TOptional<double>(); });
	Number(TEXT("transfer_receipt_class"), TEXT("Class in immutable last successful RPC receipt; may be historical"), [](const auto& S) { return S.GetLastTransferResponse().Character.ClassId.Value; });
	Number(TEXT("transfer_receipt_tier2_tokens"), TEXT("Tier2 balance frozen in last RPC receipt"), [](const auto& S) { return S.GetLastTransferResponse().TokenTier2Count.Value; });
	R.RegisterFlag(TEXT("class_state_matches_mark"), TEXT("Live class/tree/current Character and owner resources unchanged from nf.MarkClassState"), [](const FBotContext& C) { const auto Now = CurrentClassState(C); return C.Observations && !Now.IsEmpty() && Now == C.Observations->ClassStateMark; });
	Number(TEXT("private_stats_leaks"), TEXT("Private stats erroneously received for other entities"), [](const auto& S) { return S.GetPrivateStatsLeakCount(); });
	Number(TEXT("own_move_milli_speed"), TEXT("Authoritative owner wire speed rounded to milli-tiles/s to compare float transport fairly"), [](const auto& S) { return FMath::RoundToInt(S.GetOwnMoveSpeed() * 1000.f); });
	Number(TEXT("own_move_speed"), TEXT("Most recent authoritative owner EntityMove speed in tiles/s"), [](const auto& S) { return S.GetOwnMoveSpeed(); });
	R.RegisterNumber(TEXT("other_player_class"), TEXT("Highest admitted other-player class id also presented by its actual world proxy"), [](const FBotContext& C) -> TOptional<double>
	{
		const auto* Net = C.Net(); const auto* Proxies = C.World() ? C.World()->GetSubsystem<UWorldProxySubsystem>() : nullptr;
		if (!Net || !Proxies) return {};
		TOptional<double> Class;
		for (const auto& Entry : Net->GetKnownEntities())
		{
			if (Entry.Value.Kind != 1 || Net->IsOwnEntity(Entry.Key)) continue;
			const auto* Actor = Proxies->GetProxies().Find(Entry.Key);
			if (Actor && *Actor && (*Actor)->GetClassId() == Entry.Value.ClassId) Class = Class.IsSet() ? FMath::Max(Class.GetValue(), double(Entry.Value.ClassId)) : double(Entry.Value.ClassId);
		}
		return Class;
	});
	R.RegisterFlag(TEXT("catalogue_learning_metadata"), TEXT("Catalogue exposes source trees/proficiencies with effects explicitly unavailable"), [](const FBotContext& C)
	{
		const auto* S = State(C); if (!S) return false; int32 Rows = 0, Trees = 0, Proficiencies = 0;
		for (const auto& Class : S->GetCatalogue().Classes)
		{
			Trees += Class.SkillTreePopulated;
			for (const auto& Skill : Class.SkillTree) { if (Skill.EffectImplemented || Skill.SkillId.Value == 0) return false; ++Rows; }
			for (const auto& P : Class.Proficiencies) { if (P.EffectImplemented || P.SkillId.Value == 0) return false; ++Proficiencies; }
		}
		return Trees == 39 && Rows >= 6000 && Proficiencies > 0;
	});
	Number(TEXT("transfer_count"), TEXT("Successful class transfers"), [](const auto& S) { return S.GetTransferCount(); });
	Number(TEXT("character_class"), TEXT("Active class read back by GetCharacter"), [](const auto& S) { return S.GetCharacter().ClassId.Value; });
	Number(TEXT("character_base_class"), TEXT("Lineage root read back by GetCharacter"), [](const auto& S) { return S.GetCharacter().BaseClassId.Value; });
	Number(TEXT("options_current_class"), TEXT("Active class returned by TransferOptions"), [](const auto& S) { return S.GetOptions().CurrentClassId.Value; });
	Number(TEXT("eligible_options"), TEXT("Eligible immediate transfer options"), [](const auto& S) { int32 N = 0; for (const auto& O : S.GetOptions().Options) N += O.Eligible; return N; });
	Number(TEXT("options_tier1_tokens"), TEXT("Tier1 tokens from TransferOptions"), [](const auto& S) { return S.GetOptions().TokenTier1Count.Value; });
	Number(TEXT("options_tier2_tokens"), TEXT("Tier2 tokens from TransferOptions"), [](const auto& S) { return S.GetOptions().TokenTier2Count.Value; });
	R.RegisterNumber(TEXT("world_class"), TEXT("Own class projected solely from EntitySpawn/ClassChanged"), [](const FBotContext& C) -> TOptional<double> { const auto* S = State(C); return S && S->GetOwnClassId().IsSet() ? TOptional<double>(S->GetOwnClassId().GetValue()) : TOptional<double>(); });
	R.RegisterFlag(TEXT("creation_matches_catalogue"), TEXT("Creation returned precisely the selected server base-class stat array"), [](const FBotContext& C) { const auto* S = State(C); return S && S->CreatedMatchesCatalogue(); });
	R.RegisterFlag(TEXT("creation_default_appearance"), TEXT("The server persisted prototype appearance index0"), [](const FBotContext& C) { const auto* S = State(C); const auto* A = S ? &S->GetLastCreated() : nullptr; return A && !A->Id.IsEmpty() && A->HairStyle.Value == 0 && A->HairColor.Value == 0 && A->Face.Value == 0; });
	R.RegisterFlag(TEXT("character_matches_world"), TEXT("Persisted character class and sex match the reconstructed world spawn/class projection"), [](const FBotContext& C)
	{
		const auto* S = State(C); const auto* N = C.Net();
		const auto* Spawn = N ? N->GetKnownEntities().Find(N->GetOwnEntityId()) : nullptr;
		return S && Spawn && S->GetOwnClassId().IsSet() && S->GetCharacter().Id.Equals(Spawn->EntityId, ESearchCase::IgnoreCase)
			&& S->GetCharacter().ClassId.Value == S->GetOwnClassId().GetValue() && static_cast<uint32>(S->GetCharacter().Sex) == Spawn->Sex
			&& S->GetCharacter().HairStyle.Value == Spawn->HairStyle && S->GetCharacter().HairColor.Value == Spawn->HairColor && S->GetCharacter().Face.Value == Spawn->Face;
	});
	R.RegisterFlag(TEXT("at_class_master"), TEXT("Authoritative movement samples place the player within the catalogue master radius"), [](const FBotContext& C) { const auto* S = State(C); return S && S->IsAtMaster(); });
	R.RegisterNumber(TEXT("own_cp"), TEXT("Owner CP from StatsChanged"), [](const FBotContext& C) -> TOptional<double> { const auto* Combat = C.Combat(); return Combat && Combat->GetOwn().bCpKnown ? TOptional<double>(Combat->GetOwn().Cp) : TOptional<double>(); });
	R.RegisterNumber(TEXT("own_max_cp"), TEXT("Owner maxCP from StatsChanged"), [](const FBotContext& C) -> TOptional<double> { const auto* Combat = C.Combat(); return Combat && Combat->GetOwn().bCpKnown ? TOptional<double>(Combat->GetOwn().MaxCp) : TOptional<double>(); });
	R.RegisterNumber(TEXT("own_tier1_tokens"), TEXT("Owner Tier 1 tokens from StatsChanged"), [](const FBotContext& C) -> TOptional<double> { const auto* Combat = C.Combat(); return Combat && Combat->GetOwn().bCpKnown ? TOptional<double>(Combat->GetOwn().TokenTier1Count) : TOptional<double>(); });
	R.RegisterNumber(TEXT("own_tier2_tokens"), TEXT("Owner Tier 2 tokens from StatsChanged"), [](const FBotContext& C) -> TOptional<double> { const auto* Combat = C.Combat(); return Combat && Combat->GetOwn().bCpKnown ? TOptional<double>(Combat->GetOwn().TokenTier2Count) : TOptional<double>(); });
	R.RegisterNumber(TEXT("initial_tier1_tokens"), TEXT("First-owner Tier 1 tokens observed on first admission StatsChanged in current transport scope"), [](const FBotContext& C) -> TOptional<double>
	{
		if (!C.Observations) return TOptional<double>();
		const TOptional<uint32> Tokens = C.Observations->GetInitialTier1Tokens(C.Net());
		return Tokens.IsSet() ? TOptional<double>(Tokens.GetValue()) : TOptional<double>();
	});
	R.RegisterNumber(TEXT("initial_tier2_tokens"), TEXT("First-owner Tier 2 tokens observed on first admission StatsChanged in current transport scope"), [](const FBotContext& C) -> TOptional<double>
	{
		if (!C.Observations) return TOptional<double>();
		const TOptional<uint32> Tokens = C.Observations->GetInitialTier2Tokens(C.Net());
		return Tokens.IsSet() ? TOptional<double>(Tokens.GetValue()) : TOptional<double>();
	});
	R.Register({ TEXT("transfer_option"), TEXT("transfer_option <class id> <eligible|blocked>"), TEXT("Exact candidate eligibility returned by the server"), [](const TArray<FString>& A, FString& E) -> FBotPredicateFn
	{
		if (A.Num() != 2 || !A[0].IsNumeric() || (A[1] != TEXT("eligible") && A[1] != TEXT("blocked"))) { E = TEXT("needs class id and eligible|blocked"); return nullptr; }
		const uint32 Id = FCString::Strtoui64(*A[0], nullptr, 10); const bool Eligible = A[1] == TEXT("eligible");
		return [Id, Eligible](const FBotContext& C) -> FBotPredicateValue
		{
			const auto* S = State(C); const auto* O = S ? S->GetOptions().Options.FindByPredicate([Id](const auto& V) { return V.ClassId.Value == Id; }) : nullptr;
			return { O && O->Eligible == Eligible, O ? FString(O->Eligible ? TEXT("eligible") : TEXT("blocked: ")) + FString::Join(O->Unmet, TEXT(", ")) : FString(TEXT("candidate absent")) };
		};
	} });
}

#if !UE_BUILD_SHIPPING
namespace
{
	UClassStateSubsystem* State(UWorld* World)
	{
		UGameInstance* GI = World ? World->GetGameInstance() : nullptr;
		return GI ? GI->GetSubsystem<UClassStateSubsystem>() : nullptr;
	}
	FAutoConsoleCommandWithWorld OpenDialogCommand(TEXT("nf.OpenClassDialog"), TEXT("Open the actual class master dialog for rendered developer smoke"), FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* W) { if (auto* PC = W ? Cast<ANightfallPlayerController>(W->GetFirstPlayerController()) : nullptr) PC->OpenClassDialog(); }));
	FAutoConsoleCommandWithWorld MarkClassCommand(TEXT("nf.MarkClassState"), TEXT("Record live class/resources before an idempotent receipt retry"), FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* W)
	{
		if (auto* GI = W ? W->GetGameInstance() : nullptr) if (auto* Runner = GI->GetSubsystem<UBotScenarioRunner>())
		{
			Runner->MarkClassState(CurrentClassState(Runner->MakeContext()));
		}
	}));
	FAutoConsoleCommandWithWorld InventoryCommand(TEXT("nf.ProxyInventory"), TEXT("List authoritative visible entity IDs and their actual native proxy classes"), FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* W)
	{
		const auto* GI = W ? W->GetGameInstance() : nullptr;
		const auto* Net = GI ? GI->GetSubsystem<UNetClientSubsystem>() : nullptr;
		const auto* Proxies = W ? W->GetSubsystem<UWorldProxySubsystem>() : nullptr;
		if (!Net || !Proxies) return;
		for (const auto& Pair : Net->GetKnownEntities())
		{
			const auto* Proxy = Proxies->GetProxies().Find(Pair.Key);
			const FString ActorClass = Proxy && *Proxy ? (*Proxy)->GetClass()->GetName() : (Net->IsOwnEntity(Pair.Key) ? TEXT("controlled pawn") : TEXT("none"));
			const auto& Spawn = Pair.Value;
			UE_LOG(LogNightfall, Display, TEXT("proxy inventory entity=%s name=%s template=%s kind=%u actor=%s tile=(%.2f,%.2f) combatant=%d attackable=%d"), *Pair.Key, *Spawn.Name, *Spawn.TemplateId, Spawn.Kind, *ActorClass, Spawn.Position.X, Spawn.Position.Y, Spawn.bCombatant, Spawn.bAttackable);
		}
	}));
	FAutoConsoleCommandWithWorldAndArgs PreviewCommand(TEXT("nf.PreviewClass"), TEXT("nf.PreviewClass <id>: select metadata/confirmation in the open real dialog"), FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A, UWorld* W)
	{
		auto* PC = W ? Cast<ANightfallPlayerController>(W->GetFirstPlayerController()) : nullptr;
		if (PC && PC->GetClassDialog() && A.Num() == 1 && A[0].IsNumeric()) PC->GetClassDialog()->Select(FCString::Strtoui64(*A[0], nullptr, 10));
	}));
	FAutoConsoleCommandWithWorldAndArgs CaptureCommand(TEXT("nf.CaptureUi"), TEXT("nf.CaptureUi <safe-name>: screenshot real rendered UI including HUD"), FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A, UWorld* W)
	{
		if (A.Num() != 1 || A[0].IsEmpty()) return;
		for (TCHAR Ch : A[0]) if (!FChar::IsAlnum(Ch) && Ch != TEXT('-')) return;
		FScreenshotRequest::RequestScreenshot(A[0], true, false);
	}));
	FAutoConsoleCommandWithWorld CatalogueCommand(TEXT("nf.ClassCatalogue"), TEXT("Fetch the authenticated server race/class catalogue"), FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* W) { if (auto* S = State(W)) S->LoadCatalogue(); }));
	FAutoConsoleCommandWithWorld EnterCommand(TEXT("nf.EnterCreated"), TEXT("Enter with the most recent successful explicit creation"), FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* W) { if (auto* S = State(W)) S->EnterCreated(); }));
	FAutoConsoleCommandWithWorld OptionsCommand(TEXT("nf.TransferOptions"), TEXT("Query selected character's server transfer requirements"), FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* W) { if (auto* S = State(W)) S->LoadOptions(); }));
	FAutoConsoleCommandWithWorld CharacterCommand(TEXT("nf.Character"), TEXT("Read selected character back from persistent storage"), FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* W) { if (auto* S = State(W)) S->RefreshCharacter(); }));
	FAutoConsoleCommandWithWorldAndArgs CreateCommand(TEXT("nf.CreateClass"), TEXT("nf.CreateClass <race> <base class> <sex> [hair style] [hair color] [face]"), FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A, UWorld* W)
	{
		auto* S = State(W);
		if (!S || A.Num() < 3 || A.Num() > 6) return;
		for (const FString& V : A) if (!V.IsNumeric()) return;
		auto N = [&A](int32 I) -> uint32 { return A.IsValidIndex(I) ? FCString::Strtoui64(*A[I], nullptr, 10) : 0; };
		S->Create(NightfallCreation::Request(BotCharacterName::FromGuid(FGuid::NewGuid()), N(0), N(1), N(2), N(3), N(4), N(5)));
	}));
	FAutoConsoleCommandWithWorldAndArgs TransferCommand(TEXT("nf.ChangeClass"), TEXT("nf.ChangeClass <target class id> [idempotency UUID]; uses authenticated RPC"), FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& A, UWorld* W)
	{
		FGuid Key;
		if (A.Num() < 1 || A.Num() > 2 || !A[0].IsNumeric() || (A.Num() == 2 && !FGuid::ParseExact(A[1], EGuidFormats::DigitsWithHyphens, Key))) return;
		if (auto* S = State(W)) S->Transfer(FCString::Strtoui64(*A[0], nullptr, 10), nullptr, A.Num() == 2 ? A[1] : FString());
	}));
	FAutoConsoleCommandWithWorld MoveMasterCommand(TEXT("nf.MoveMaster"), TEXT("Walk through the ordinary click path to the server catalogue class master"), FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* W)
	{
		auto* S = State(W); auto* PC = W ? Cast<ANightfallPlayerController>(W->GetFirstPlayerController()) : nullptr;
		if (!S || !PC || !S->HasCatalogue()) return;
		const auto& P = S->GetCatalogue().ClassMaster.Position;
		PC->ClickGroundLocation(FVector(P.X * 100.f, P.Y * 100.f, 0.f));
	}));
}
#endif
