#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Auth/AuthSubsystem.h"
#include "Game/LoginFlowSubsystem.h"
#include "Items/InventoryStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Net/ProtoCodec.h"
#include "Net/SessionClientSubsystem.h"
#include "UI/NightfallHud.h"
#include "UI/NightfallInventoryDialog.h"
#include "Components/Button.h"
#include "Components/ComboBoxString.h"
#include "Components/UniformGridPanel.h"
#include "Components/VerticalBox.h"
#include "Components/TextBlock.h"
#include "Containers/Ticker.h"
#include "Dom/JsonObject.h"
#include "HAL/FileManager.h"
#include "HAL/PlatformMisc.h"
#include "HAL/PlatformProcess.h"
#include "Misc/FileHelper.h"
#include "Misc/Guid.h"
#include "Misc/Paths.h"
#include "Serialization/JsonReader.h"
#include "Serialization/JsonSerializer.h"
#include "TurboLinkGrpcManager.h"

#if WITH_DEV_AUTOMATION_TESTS

// Live Phase 4 (V6) item automation (decision D115, latent-stub audit B7; plan stories 5.2/5.4).
// Needs an API started with PHASE4_ENABLED=1 AUTH_DEV_TOKENS=1 (a durable V6 zone). Without a
// reachable API the tests skip with a warning, and fail under -RequireLiveApi
// (`run-tests.sh --require-live-api`). A reachable API that is not V6 (dev GrantItem refused) is a
// failure under -RequireLiveApi too: these tests say nothing unless the item path is live.
//
// Every test provisions a fresh character through the dev GrantItem RPC before it enters the world
// (harness-inbox "API (E3) 2026-10-13 — T5"): the first grant materializes the human fighter's
// starter kit (Squire's Sword 2369 equipped in rhand, shirt/pants equipped), plus five Lesser
// Healing Potions (1060) and 200 No-Grade Soulshots (1835).
//
// Nightfall.Item.Enrichment.EndToEnd also needs the replay tool and the API's JetStream:
//   NIGHTFALL_REPLAY_BIN  nightfall-replay binary (default <repo>/target/release/nightfall-replay,
//                         then target/debug); build with `cargo build --release -p nightfall-api
//                         --bin nightfall-replay`
//   NATS_URL              the API's NATS (default nats://localhost:4222)
//   NIGHTFALL_ZONE_ID     the zone the character enters (default 1)
namespace ItemLiveTests
{
	constexpr EAutomationTestFlags Flags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;
	constexpr uint32 Sword = 2369, Potion = 1060, Soulshot = 1835;
	constexpr uint64 PotionCount = 5, SoulshotCount = 200;

	FString Lower(const TArray<uint8>& Bytes) { return BytesToHex(Bytes.GetData(), Bytes.Num()).ToLower(); }

	enum class EStart : uint8 { Ready, Skipped, Failed };

	struct FSession
	{
		FAutomationTestBase& Test;
		FScopedTestGameInstance Scope;
		USessionClient* Session = nullptr;
		UAuthSubsystem* Auth = nullptr;
		ULoginFlowSubsystem* Flow = nullptr;
		UNetClientSubsystem* Net = nullptr;
		UInventoryStateSubsystem* Items = nullptr;
		UNightfallInventoryDialog* Dialog = nullptr;
		FString Character, HotbarFile;
		TArray<TArray<uint8>> Received;
		int32 InventorySyncs = 0;
		TArray<FGrpcNightfallV1ItemOperationResult> Results;
		TMap<uint32, FIntentRejected> Rejected;
		TArray<FGrpcNightfallV1EquipmentChanged> OwnEquipment;
		bool bEntered = false;

		explicit FSession(FAutomationTestBase& InTest) : Test(InTest)
		{
			NightfallTest::AllowApiUnavailableLogs(Test);
			// The offline-grant probe may be refused before the first admission (handled below).
			Test.AddExpectedError(TEXT("paused skill checkpoint unavailable"), EAutomationExpectedErrorFlags::Contains, -1);
			Session = Scope.Get<USessionClient>();
			Auth = Scope.Get<UAuthSubsystem>();
			Flow = Scope.Get<ULoginFlowSubsystem>();
			Net = Scope.Get<UNetClientSubsystem>();
			Items = Scope.Get<UInventoryStateSubsystem>();
			Flow->bTravelOnConnect = false;
			HotbarFile = FPaths::Combine(FPaths::ProjectSavedDir(), TEXT("Tests"), FGuid::NewGuid().ToString() + TEXT("-live-items.ini"));
			Items->SetHotbarConfigForTesting(HotbarFile);
		}

		~FSession()
		{
			if (bEntered) Flow->LeaveWorld();
			IFileManager::Get().Delete(*HotbarFile);
		}

		bool Pump(TFunctionRef<bool()> Done, double Seconds)
		{
			const double End = FPlatformTime::Seconds() + Seconds;
			while (!Done() && FPlatformTime::Seconds() < End)
			{
				Session->GetGrpcManager()->Tick(0.01f);
				FTSTicker::GetCoreTicker().Tick(0.01f);
				FPlatformProcess::Sleep(0.01f);
			}
			return Done();
		}

		/** Dev GrantItem for the (offline) character; false with a test error or skip on refusal. */
		bool Grant(uint32 Template, uint64 Count, bool& bSkipped, FString* Refusal = nullptr)
		{
			FGrpcNightfallV1GrantItemRequest Q;
			Q.CharacterId = NightfallItems::UuidBytes(Character);
			Q.TemplateId = Template;
			Q.Count = Count;
			Q.IdempotencyKey = FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower);
			bool bDone = false;
			FNetResult Result;
			Session->GrantItem(Q, [&](const FNetResult& R, const FGrpcNightfallV1GrantItemResponse&) { Result = R; bDone = true; });
			if (!Pump([&] { return bDone; }, 20.0))
			{
				Test.AddError(FString::Printf(TEXT("GrantItem %u x%llu timed out"), Template, Count));
				return false;
			}
			if (Result.IsOk()) return true;
			if (Refusal && Result.Message.Contains(TEXT("checkpoint")))
			{
				*Refusal = Result.Message;
				return false;
			}
			// A V5 API (PHASE4_ENABLED unset) or one without AUTH_DEV_TOKENS refuses the dev grant.
			bSkipped = NightfallTest::SkipLive(Test, FString::Printf(
				TEXT("SKIPPED: the API refused the dev GrantItem (%u x%llu): %s; run it with PHASE4_ENABLED=1 AUTH_DEV_TOKENS=1"),
				Template, Count, *Result.Message));
			return false;
		}

		EStart Start(const TCHAR* Prefix)
		{
			bool bDone = false;
			FNetResult Result;
			Session->Ping([&](const FNetResult& R, const auto&) { Result = R; bDone = true; });
			if (!Pump([&] { return bDone; }, 10.0)) { Test.AddError(TEXT("Ping callback timed out; not an offline skip")); return EStart::Failed; }
			if (Result.Error == ENetError::Unavailable)
			{
				return NightfallTest::SkipLive(Test, FString::Printf(TEXT("SKIPPED: no API listening at %s; no live Phase 4 item coverage"), *Session->GetEndpoint()))
					? EStart::Skipped : EStart::Failed;
			}
			if (!Result.IsOk()) { Test.AddError(TEXT("Reachable API Ping failed: ") + Result.Message); return EStart::Failed; }
			// A fresh dev account per test (GrantItem needs AUTH_DEV_TOKENS=1 anyway): the run-wide
			// -DevTokenFile account has seven character slots shared by every live test.
			Auth->LoginWithDevToken(TEXT("test:") + FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower));
			if (!Auth->IsLoggedIn()) { Test.AddError(TEXT("Reachable API requires valid test authentication (AUTH_DEV_TOKENS=1)")); return EStart::Failed; }

			const FString Hex = FGuid::NewGuid().ToString(EGuidFormats::Digits);
			FString Name = Prefix;
			for (int32 I = 0; I < 10; ++I) Name.AppendChar(TEXT('a') + static_cast<TCHAR>(FParse::HexDigit(Hex[I])));
			bDone = false;
			Flow->CreateCharacter(Name, EGrpcNightfallV1Race::RACE_HUMAN, [&](const FNetResult& R, const auto& C) { Result = R; Character = C.Id.ToLower(); bDone = true; });
			if (!Pump([&] { return bDone; }, 10.0) || !Result.IsOk() || Character.IsEmpty())
			{
				Test.AddError(TEXT("Live item character creation failed: ") + Result.Message);
				return EStart::Failed;
			}

			Net->OnWireReceived.AddLambda([this](const TArray<uint8>& Frame) { Received.Add(Frame); });
			Net->OnInventorySync.AddLambda([this](const FGrpcNightfallV1InventorySync&) { ++InventorySyncs; });
			Net->OnItemOperationResult.AddLambda([this](const FGrpcNightfallV1ItemOperationResult& R) { Results.Add(R); });
			Net->OnIntentRejected.AddLambda([this](const FIntentRejected& R) { Rejected.Add(R.Seq, R); });
			Net->OnEquipmentChanged.AddLambda([this](const FGrpcNightfallV1EquipmentChanged& E)
			{
				if (NightfallItems::UuidString(E.EntityId).Equals(Character, ESearchCase::IgnoreCase)) OwnEquipment.Add(E);
			});
			// Provisioning (harness-inbox T5): an offline dev grant before the first login, which
			// also materializes the starter kit. A character whose skills need a Phase 3 checkpoint
			// the server has not written yet is refused offline ("paused skill checkpoint
			// unavailable"); it is then provisioned online after admission (queued, applied on the
			// next tick, delivered as InventoryChanged).
			bool bSkipped = false;
			FString Refusal;
			if (Grant(Potion, PotionCount, bSkipped, &Refusal))
			{
				if (!Grant(Soulshot, SoulshotCount, bSkipped)) return bSkipped ? EStart::Skipped : EStart::Failed;
				if (!Enter()) return EStart::Failed;
			}
			else if (Refusal.Contains(TEXT("checkpoint")))
			{
				if (!Enter()) return EStart::Failed;
				if (!Grant(Potion, PotionCount, bSkipped) || !Grant(Soulshot, SoulshotCount, bSkipped))
				{
					return bSkipped ? EStart::Skipped : EStart::Failed;
				}
			}
			else
			{
				return bSkipped ? EStart::Skipped : EStart::Failed;
			}
			if (!Pump([&] { return Items->Count(Potion) == TOptional<uint64>(PotionCount) && Items->Count(Soulshot) == TOptional<uint64>(SoulshotCount)
				&& Items->Equipped(EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND) != nullptr && !Items->NeedsBaseline(); }, 20.0))
			{
				Test.AddError(FString::Printf(TEXT("granted items and the starter kit did not reach the received inventory (status '%s')"), *Items->Status()));
				return EStart::Failed;
			}
			Items->LoadCatalog();
			if (!Pump([&] { return Items->HasCatalog(); }, 10.0)) { Test.AddError(TEXT("item catalogue did not load: ") + Items->RpcResult().Message); return EStart::Failed; }
			Dialog = NewObject<UNightfallInventoryDialog>(Scope.GameInstance);
			Dialog->EnsureLayout();
			Dialog->BindState(Items, false);
			return EStart::Ready;
		}

		/** Ticket, WebSocket and V6 admission; waits for the own spawn and an InventorySync baseline. */
		bool Enter()
		{
			bool bDone = false;
			FNetResult Result;
			Flow->EnterWorld(Character, [&](const FNetResult& R) { Result = R; bDone = true; });
			if (!Pump([&] { return bDone; }, 10.0) || !Result.IsOk()) { Test.AddError(TEXT("V6 world admission refused: ") + Result.Message); return false; }
			bEntered = true;
			if (!Pump([&] { return Items->HasBaseline() && Net->GetKnownEntities().Contains(Character); }, 20.0))
			{
				Test.AddError(FString::Printf(TEXT("V6 admission did not deliver the own spawn and an InventorySync baseline (syncs %d, status '%s')"), InventorySyncs, *Items->Status()));
				return false;
			}
			return true;
		}

		/** Leaves and enters again: the second admission's baseline must carry the durable state. */
		bool Rejoin()
		{
			Flow->LeaveWorld();
			bEntered = false;
			Pump([&] { return !Net->IsConnected(); }, 5.0);
			Pump([] { return false; }, 1.0);
			InventorySyncs = 0;
			return Enter();
		}

		const FGrpcNightfallV1OwnedItem* ByTemplate(uint32 Template) const
		{
			for (const auto& Pair : Items->GetItems()) if (Pair.Value.TemplateId.Value == Template) return &Pair.Value;
			return nullptr;
		}

		/** Clicks the item's own button in the dialog grid (UNightfallItemChoice → Select). */
		bool ClickGridItem(uint64 Instance)
		{
			const auto& Rows = Items->GetInventory().Items;
			const int32 Index = Rows.IndexOfByPredicate([&](const auto& I) { return I.InstanceId.Value == Instance; });
			UButton* Button = Index == INDEX_NONE ? nullptr : Cast<UButton>(Dialog->GridControl()->GetChildAt(Index));
			if (!Test.TestNotNull(*FString::Printf(TEXT("grid button for instance %llu"), Instance), Button)) return false;
			Button->OnClicked.Broadcast();
			return true;
		}

		/** Clicks the paperdoll row for a slot (UNightfallItemChoice → SelectSlot). */
		bool ClickPaperdoll(const FString& Slot)
		{
			const FString Prefix = Slot + TEXT(": ");
			for (UWidget* Child : Dialog->PaperdollControl()->GetAllChildren())
			{
				auto* Button = Cast<UButton>(Child);
				const auto* Text = Button ? Cast<UTextBlock>(Button->GetChildAt(0)) : nullptr;
				if (Text && Text->GetText().ToString().StartsWith(Prefix)) { Button->OnClicked.Broadcast(); return true; }
			}
			Test.AddError(TEXT("no paperdoll row for ") + Slot);
			return false;
		}

		/** Clicks a dialog/HUD control that sends one intent; waits for its receipt or rejection. */
		bool Send(UButton* Control, EGrpcNightfallV1ItemOperation Op, const TCHAR* What)
		{
			if (!Test.TestTrue(*FString::Printf(TEXT("%s control enabled"), What), Control && Control->GetIsEnabled())) return false;
			const uint32 Before = Net->GetLastSentSeq();
			const int32 Seen = Results.Num();
			Control->OnClicked.Broadcast();
			const uint32 Seq = Net->GetLastSentSeq();
			if (!Test.TestEqual(*FString::Printf(TEXT("%s sends exactly one intent"), What), Seq, Before + 1)) return false;
			auto Done = [&]
			{
				if (Rejected.Contains(Seq)) return true;
				for (int32 I = Seen; I < Results.Num(); ++I) if (Results[I].Operation == Op) return true;
				return false;
			};
			if (!Test.TestTrue(*FString::Printf(TEXT("%s receipt or rejection within 15s"), What), Pump(Done, 15.0))) return false;
			if (const FIntentRejected* R = Rejected.Find(Seq))
			{
				Test.AddError(FString::Printf(TEXT("%s rejected: reason %u '%s'"), What, R->Reason, *R->Detail));
				return false;
			}
			Pump([&] { return !Items->IsBusy(); }, 5.0);
			return Test.TestFalse(*FString::Printf(TEXT("%s settled"), What), Items->IsBusy());
		}

		const FGrpcNightfallV1VisibleEquipment* PublicSlot(EGrpcNightfallV1EquipSlot Slot) const
		{
			const auto* Rows = Items->Visible(Character);
			return Rows ? Rows->FindByPredicate([&](const auto& V) { return V.Slot == Slot; }) : nullptr;
		}

		/** Unequip the kit sword through the paperdoll and re-equip it through the grid. */
		bool CycleWeapon()
		{
			const auto* Held = Items->Equipped(EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND);
			if (!Test.TestTrue(TEXT("kit sword starts in rhand"), Held && Held->TemplateId.Value == Sword)) return false;
			const uint64 Instance = Held->InstanceId.Value;
			const uint64 Revision = Items->Revision();
			const int32 PublicBefore = OwnEquipment.Num();

			if (!ClickPaperdoll(TEXT("rhand"))) return false;
			if (!Send(Dialog->UnequipControl(), EGrpcNightfallV1ItemOperation::ITEM_OPERATION_UNEQUIP, TEXT("dialog unequip"))) return false;
			Pump([&] { return !PublicSlot(EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND); }, 5.0);
			Test.TestNull(TEXT("received inventory has rhand empty"), Items->Equipped(EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND));
			Test.TestEqual(TEXT("paperdoll shows rhand empty"), Dialog->PaperdollLabel(TEXT("rhand")), FString(TEXT("rhand: empty")));
			Test.TestTrue(TEXT("unequip advanced the inventory revision"), Items->Revision() > Revision);
			Test.TestTrue(TEXT("own public EquipmentChanged after unequip"), OwnEquipment.Num() > PublicBefore);
			Test.TestNull(TEXT("public equipment no longer shows the sword"), PublicSlot(EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND));
			const auto* Stored = Items->Find(Instance);
			Test.TestTrue(TEXT("sword is back in the inventory"), Stored && Stored->Slot == EGrpcNightfallV1EquipSlot::EQUIP_SLOT_UNSPECIFIED);

			const int32 PublicMiddle = OwnEquipment.Num();
			if (!ClickGridItem(Instance)) return false;
			if (!Send(Dialog->EquipControl(), EGrpcNightfallV1ItemOperation::ITEM_OPERATION_EQUIP, TEXT("dialog equip"))) return false;
			Pump([&] { return PublicSlot(EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND) != nullptr; }, 5.0);
			const auto* Again = Items->Equipped(EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND);
			Test.TestTrue(TEXT("received inventory has the sword in rhand"), Again && Again->InstanceId.Value == Instance);
			Test.TestTrue(TEXT("paperdoll shows the sword in rhand"), Dialog->PaperdollLabel(TEXT("rhand")).StartsWith(TEXT("rhand: ")) && !Dialog->PaperdollLabel(TEXT("rhand")).EndsWith(TEXT("empty")));
			Test.TestTrue(TEXT("own public EquipmentChanged after equip"), OwnEquipment.Num() > PublicMiddle);
			const auto* Public = PublicSlot(EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND);
			Test.TestTrue(TEXT("public equipment shows the sword template"), Public && Public->TemplateId.Value == Sword);
			return !Test.HasAnyErrors();
		}
	};

	FString RepoRoot() { return FPaths::ConvertRelativePathToFull(FPaths::Combine(FPaths::ProjectDir(), TEXT("../.."))); }

	FString ReplayBinary()
	{
		const FString Env = FPlatformMisc::GetEnvironmentVariable(TEXT("NIGHTFALL_REPLAY_BIN"));
		if (!Env.IsEmpty()) return Env;
		for (const TCHAR* Profile : { TEXT("release"), TEXT("debug") })
		{
			const FString Path = FPaths::Combine(RepoRoot(), TEXT("target"), Profile, TEXT("nightfall-replay"));
			if (FPaths::FileExists(Path)) return Path;
		}
		return {};
	}

	FString EnvOr(const TCHAR* Name, const TCHAR* Default)
	{
		const FString V = FPlatformMisc::GetEnvironmentVariable(Name);
		return V.IsEmpty() ? FString(Default) : V;
	}
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemBaselineLiveTest, "Nightfall.Item.Baseline.LiveAdmission", ItemLiveTests::Flags)
bool FItemBaselineLiveTest::RunTest(const FString& Parameters)
{
	using namespace ItemLiveTests;
	FSession S(*this);
	const EStart Started = S.Start(TEXT("Bag"));
	if (Started != EStart::Ready) return Started == EStart::Skipped;

	// Second login: the V6 admission baseline itself must carry the durable kit and grants.
	const uint64 Revision = S.Items->Revision();
	TestTrue(TEXT("granted revision is positive"), Revision > 0);
	if (!S.Rejoin()) return false;
	TestTrue(TEXT("an InventorySync baseline arrived on the WebSocket at the second admission"), S.InventorySyncs >= 1);
	TestEqual(TEXT("baseline revision equals the committed revision"), S.Items->Revision(), Revision);
	const auto Potions = S.Items->Count(Potion);
	const auto Shots = S.Items->Count(Soulshot);
	TestTrue(TEXT("granted potions in the baseline"), Potions.IsSet() && *Potions == PotionCount);
	TestTrue(TEXT("granted soulshots in the baseline"), Shots.IsSet() && *Shots == SoulshotCount);
	const auto* Weapon = S.Items->Equipped(EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND);
	TestTrue(TEXT("starter kit sword equipped in rhand"), Weapon && Weapon->TemplateId.Value == Sword);
	int32 Equipped = 0;
	for (const auto& Pair : S.Items->GetItems()) Equipped += Pair.Value.Slot != EGrpcNightfallV1EquipSlot::EQUIP_SLOT_UNSPECIFIED;
	TestTrue(TEXT("kit paperdoll holds the equipped kit items"), Equipped >= 3);
	TestTrue(TEXT("own spawn carries public visible equipment (D26 field 29)"), S.PublicSlot(EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND) != nullptr
		&& S.PublicSlot(EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND)->TemplateId.Value == Sword);
	TestEqual(TEXT("dialog grid renders every received item"), S.Dialog->GridControl()->GetChildrenCount(), S.Items->GetInventory().Items.Num());
	TestTrue(TEXT("dialog paperdoll renders the received sword"), !S.Dialog->PaperdollLabel(TEXT("rhand")).IsEmpty() && !S.Dialog->PaperdollLabel(TEXT("rhand")).EndsWith(TEXT("empty")));
	return !HasAnyErrors();
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemDialogLiveTest, "Nightfall.Item.Dialog.EndToEnd", ItemLiveTests::Flags)
bool FItemDialogLiveTest::RunTest(const FString& Parameters)
{
	using namespace ItemLiveTests;
	FSession S(*this);
	const EStart Started = S.Start(TEXT("Doll"));
	if (Started != EStart::Ready) return Started == EStart::Skipped;
	S.CycleWeapon();
	const int32 Equips = S.Results.FilterByPredicate([](const auto& R) { return R.Operation == EGrpcNightfallV1ItemOperation::ITEM_OPERATION_EQUIP; }).Num();
	const int32 Unequips = S.Results.FilterByPredicate([](const auto& R) { return R.Operation == EGrpcNightfallV1ItemOperation::ITEM_OPERATION_UNEQUIP; }).Num();
	TestEqual(TEXT("one Equip receipt"), Equips, 1);
	TestEqual(TEXT("one Unequip receipt"), Unequips, 1);
	return !HasAnyErrors();
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemConsumableLiveTest, "Nightfall.Item.Consumable.EndToEnd", ItemLiveTests::Flags)
bool FItemConsumableLiveTest::RunTest(const FString& Parameters)
{
	using namespace ItemLiveTests;
	FSession S(*this);
	const EStart Started = S.Start(TEXT("Sip"));
	if (Started != EStart::Ready) return Started == EStart::Skipped;

	// Use: select the potion stack in the grid, press Use.
	const auto* Stack = S.ByTemplate(Potion);
	if (!TestNotNull(TEXT("potion stack received"), Stack) || !S.ClickGridItem(Stack->InstanceId.Value)) return false;
	if (!S.Send(S.Dialog->UseControl(), EGrpcNightfallV1ItemOperation::ITEM_OPERATION_USE, TEXT("dialog use"))) return false;
	S.Pump([&] { return S.Items->Count(Potion) == TOptional<uint64>(PotionCount - 1); }, 5.0);
	TestTrue(TEXT("use consumed exactly one potion"), S.Items->Count(Potion) == TOptional<uint64>(PotionCount - 1));

	// Auto-shots: the dialog assigns the soulshot to F1 as an auto-shot toggle; the HUD button toggles it.
	const auto* Shots = S.ByTemplate(Soulshot);
	if (!TestNotNull(TEXT("soulshot stack received"), Shots) || !S.ClickGridItem(Shots->InstanceId.Value)) return false;
	S.Dialog->HotbarSlotControl()->SetSelectedIndex(0);
	S.Dialog->HotbarModeControl()->SetSelectedIndex(1);
	S.Dialog->AssignControl()->OnClicked.Broadcast();
	const auto Slots = S.Items->Hotbar();
	TestTrue(TEXT("dialog assigned the soulshot auto-toggle to F1"), Slots.IsValidIndex(0) && Slots[0].Key == Soulshot && Slots[0].Value);
	auto* Hud = NewObject<UNightfallHud>(S.Scope.GameInstance);
	Hud->EnsureLayout();
	Hud->RefreshPresentation();
	TestTrue(TEXT("HUD shows AUTO OFF"), Hud->ItemLabel(0).Contains(TEXT("AUTO OFF")));
	if (!S.Send(Hud->ItemControl(0), EGrpcNightfallV1ItemOperation::ITEM_OPERATION_TOGGLE_AUTO_SHOT, TEXT("HUD auto-shot on"))) return false;
	S.Pump([&] { return S.Items->AutoShot(Soulshot); }, 5.0);
	TestTrue(TEXT("received auto-shot enabled"), S.Items->AutoShot(Soulshot));
	Hud->RefreshPresentation();
	TestTrue(TEXT("HUD shows AUTO ON"), Hud->ItemLabel(0).Contains(TEXT("AUTO ON")));
	const int32 Toggles = S.Results.Num();
	if (!S.Send(Hud->ItemControl(0), EGrpcNightfallV1ItemOperation::ITEM_OPERATION_TOGGLE_AUTO_SHOT, TEXT("HUD auto-shot off"))) return false;
	S.Pump([&] { return !S.Items->AutoShot(Soulshot); }, 5.0);
	TestTrue(TEXT("second toggle has its own receipt"), S.Results.Num() > Toggles);
	TestFalse(TEXT("received auto-shot disabled"), S.Items->AutoShot(Soulshot));
	return !HasAnyErrors();
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemEnrichmentLiveTest, "Nightfall.Item.Enrichment.EndToEnd", ItemLiveTests::Flags)
bool FItemEnrichmentLiveTest::RunTest(const FString& Parameters)
{
	using namespace ItemLiveTests;
	const FString Binary = ReplayBinary();
	FSession S(*this);
	const EStart Started = S.Start(TEXT("Wire"));
	if (Started != EStart::Ready) return Started == EStart::Skipped;
	if (Binary.IsEmpty() || !FPaths::FileExists(Binary))
	{
		AddError(TEXT("nightfall-replay not found (set NIGHTFALL_REPLAY_BIN or `cargo build --release -p nightfall-api --bin nightfall-replay`)"));
		return false;
	}
	// Equip changes produce owner StatsChanged weight/penalties/revision (D26 tag 41).
	if (!S.CycleWeapon()) return false;
	S.Pump([] { return false; }, 1.0);   // late frames of the last tick
	S.Flow->LeaveWorld();
	S.bEntered = false;
	S.Pump([] { return false; }, 2.0);   // the despawn tick reaches the log

	const FString Dir = FPaths::ConvertRelativePathToFull(FPaths::Combine(FPaths::ProjectSavedDir(), TEXT("Tests"), TEXT("ItemLive-") + FGuid::NewGuid().ToString()));
	IFileManager::Get().MakeDirectory(*Dir, true);
	const FString Recording = FPaths::Combine(Dir, TEXT("live.nfr")), Frames = FPaths::Combine(Dir, TEXT("replayed-frames.jsonl"));
	auto Run = [&](const FString& Args, const TCHAR* What)
	{
		int32 Code = -1;
		FString Out, Err;
		FPlatformProcess::ExecProcess(*Binary, *Args, &Code, &Out, &Err, *RepoRoot());
		AddInfo(FString::Printf(TEXT("%s: exit %d\n%s%s"), What, Code, *Out, *Err));
		UE_LOG(LogTemp, Display, TEXT("ItemLive %s: exit %d: %s %s"), What, Code, *Out.TrimStartAndEnd(), *Err.TrimStartAndEnd());
		return Code;
	};
	const FString Zone = EnvOr(TEXT("NIGHTFALL_ZONE_ID"), TEXT("1"));
	const FString Nats = EnvOr(TEXT("NATS_URL"), TEXT("nats://localhost:4222"));
	int32 Code = -1;
	for (int32 Attempt = 0; Attempt < 5 && Code != 0; ++Attempt)
	{
		if (Attempt > 0) FPlatformProcess::Sleep(1.0f);
		IFileManager::Get().Delete(*Recording);
		Code = Run(FString::Printf(TEXT("export --zone %s --latest --live --session %s --out \"%s\" --nats %s"), *Zone, *S.Character, *Recording, *Nats), TEXT("export live recording"));
	}
	if (!TestEqual(TEXT("live recording exported from the API's event log"), Code, 0)) return false;
	if (!TestEqual(TEXT("replay is byte-exact and writes the replayed session frames"),
		Run(FString::Printf(TEXT("frames --file \"%s\" --session %s --out \"%s\""), *Recording, *S.Character, *Frames), TEXT("replay frames")), 0)) return false;

	TArray<FString> Lines;
	FFileHelper::LoadFileToStringArray(Lines, *Frames);
	TSet<FString> Live;
	for (const TArray<uint8>& Frame : S.Received) Live.Add(Lower(Frame));
	TSet<FString> ReplayedStats;
	int32 Spawns = 0, Stats = 0, Missing = 0, Total = 0;
	for (const FString& Line : Lines)
	{
		TSharedPtr<FJsonObject> Json;
		if (Line.IsEmpty() || !FJsonSerializer::Deserialize(TJsonReaderFactory<>::Create(Line), Json) || !Json.IsValid()) { AddError(TEXT("unparseable frames line: ") + Line); continue; }
		++Total;
		if (!Json->GetBoolField(TEXT("enriched"))) continue;
		const FString Kind = Json->GetStringField(TEXT("kind"));
		const FString Hex = Json->GetStringField(TEXT("frame_hex")).ToLower();
		TArray<uint8> Bytes;
		Bytes.SetNumUninitialized(Hex.Len() / 2);
		HexToBytes(Hex, Bytes.GetData());
		FServerMessage Decoded;
		const bool bDecoded = NightfallProto::Decode(Bytes.GetData(), Bytes.Num(), Decoded) && Decoded.Event.IsSet();
		if (Kind == TEXT("spawn"))
		{
			++Spawns;
			TestTrue(TEXT("replayed enriched frame decodes as EntitySpawn"), bDecoded && Decoded.Event->Spawn.IsSet());
		}
		else if (Kind == TEXT("stats"))
		{
			++Stats;
			ReplayedStats.Add(Hex);
			TestTrue(TEXT("replayed enriched frame decodes as StatsChanged with item weight"), bDecoded && Decoded.Event->StatsChanged.IsSet() && Decoded.Event->StatsChanged->ItemWeight.IsSet());
		}
		if (!Live.Contains(Hex))
		{
			++Missing;
			AddError(FString::Printf(TEXT("replayed enriched %s frame (tick %d) was not received live byte-for-byte: %s"), *Kind, static_cast<int32>(Json->GetNumberField(TEXT("tick"))), *Hex));
		}
	}
	// The reverse: every enriched owner StatsChanged the client received live is a replayed frame.
	int32 LiveStats = 0;
	for (const TArray<uint8>& Frame : S.Received)
	{
		FServerMessage Decoded;
		if (!NightfallProto::Decode(Frame.GetData(), Frame.Num(), Decoded) || !Decoded.Event.IsSet() || !Decoded.Event->StatsChanged.IsSet() || !Decoded.Event->StatsChanged->ItemWeight.IsSet()) continue;
		++LiveStats;
		if (!ReplayedStats.Contains(Lower(Frame))) AddError(TEXT("live enriched StatsChanged frame is absent from the replay: ") + Lower(Frame));
	}
	const FString Summary = FString::Printf(TEXT("D26: %d replayed frames; enriched spawn %d, enriched stats %d (live enriched stats %d, live frames %d); %d missing live"), Total, Spawns, Stats, LiveStats, S.Received.Num(), Missing);
	AddInfo(Summary);
	UE_LOG(LogTemp, Display, TEXT("ItemLive %s"), *Summary);
	TestTrue(TEXT("replay carries enriched owner StatsChanged for the equip changes"), Stats >= 2);
	TestTrue(TEXT("replay carries the enriched own EntitySpawn (equipped spawn hydration)"), Spawns >= 1);
	TestEqual(TEXT("every replayed enriched frame matched a live frame byte-for-byte"), Missing, 0);
	IFileManager::Get().DeleteDirectory(*Dir, false, true);
	return !HasAnyErrors();
}

#endif
