#include "NightfallHud.h"
#include "Nightfall.h"
#include "Skills/SkillStateSubsystem.h"
#include "Items/InventoryStateSubsystem.h"
#include "Effects/EffectStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Components/HorizontalBox.h"
#include "Character/ClassStateSubsystem.h"
#include "NightfallPlayerController.h"
#include "Game/LoginFlowSubsystem.h"
#include "World/RemoteEntityActor.h"
#include "World/ClassMasterActor.h"
#include "World/WorldProxySubsystem.h"
#include "Blueprint/WidgetLayoutLibrary.h"
#include "Blueprint/WidgetTree.h"
#include "Components/Border.h"
#include "Components/Button.h"
#include "Components/CanvasPanel.h"
#include "Components/CanvasPanelSlot.h"
#include "Components/ProgressBar.h"
#include "Components/SizeBox.h"
#include "Components/TextBlock.h"
#include "Components/VerticalBox.h"
#include "Components/VerticalBoxSlot.h"
#include "Engine/GameInstance.h"
#include "GameFramework/PlayerController.h"
#include "GameFramework/Pawn.h"

namespace
{
	const FLinearColor HpColor(0.78f, 0.12f, 0.12f, 1.f);
	const FLinearColor MpColor(0.15f, 0.35f, 0.85f, 1.f);
	const FLinearColor NpcHpColor(0.85f, 0.2f, 0.15f, 1.f);
}

void UNightfallHud::NativeOnInitialized()
{
	Super::NativeOnInitialized();
	EnsureLayout();
}

void UNightfallHud::EnsureLayout()
{
	if (WidgetTree == nullptr)
	{
		WidgetTree = NewObject<UWidgetTree>(this, TEXT("WidgetTree"), RF_Transient);
	}
	if (WidgetTree->RootWidget == nullptr)
	{
		BuildLayout();
	}
}

UTextBlock* UNightfallHud::MakeText(UPanelWidget* Parent, const FString& Text, int32 Size)
{
	UTextBlock* Block = WidgetTree->ConstructWidget<UTextBlock>();
	Block->SetText(FText::FromString(Text));
	FSlateFontInfo Font = Block->GetFont();
	Font.Size = Size;
	Block->SetFont(Font);
	Block->SetShadowOffset(FVector2D(1.f, 1.f));
	Block->SetShadowColorAndOpacity(FLinearColor::Black);
	if (Parent != nullptr) Parent->AddChild(Block);
	return Block;
}

UProgressBar* UNightfallHud::MakeBar(UPanelWidget* Parent, const FLinearColor& Fill, float Width, float Height)
{
	UProgressBar* Bar = WidgetTree->ConstructWidget<UProgressBar>();
	Bar->SetFillColorAndOpacity(Fill);
	Bar->SetPercent(0.f);
	USizeBox* Sized = WidgetTree->ConstructWidget<USizeBox>();
	Sized->SetWidthOverride(Width);
	Sized->SetHeightOverride(Height);
	Sized->AddChild(Bar);
	if (Parent != nullptr) Parent->AddChild(Sized);
	return Bar;
}

void UNightfallHud::BuildLayout()
{
	Root = WidgetTree->ConstructWidget<UCanvasPanel>(UCanvasPanel::StaticClass(), TEXT("Root"));
	WidgetTree->RootWidget = Root;

	// Own stats, top-left.
	OwnPanel = WidgetTree->ConstructWidget<UVerticalBox>();
	OwnPanel->SetVisibility(ESlateVisibility::HitTestInvisible);
	OwnLabel = MakeText(OwnPanel, TEXT(""), 16);
	OwnCpBar = MakeBar(OwnPanel, FLinearColor(0.9f, 0.7f, 0.15f), 260.f, 10.f);
	OwnCpText = MakeText(OwnPanel, TEXT(""), 12);
	OwnHpBar = MakeBar(OwnPanel, HpColor, 260.f, 18.f);
	OwnHpText = MakeText(OwnPanel, TEXT(""), 12);
	OwnMpBar = MakeBar(OwnPanel, MpColor, 260.f, 12.f);
	OwnMpText = MakeText(OwnPanel, TEXT(""), 12);
	ClassText = MakeText(OwnPanel, TEXT(""), 12);
	XpText = MakeText(OwnPanel, TEXT(""), 12);
	AttackText = MakeText(OwnPanel, TEXT(""), 12);
	if (UCanvasPanelSlot* Slot = Root->AddChildToCanvas(OwnPanel))
	{
		Slot->SetPosition(FVector2D(16.f, 16.f));
		Slot->SetSize(FVector2D(260.f, 185.f));
	}

	UButton* ClassesButton = WidgetTree->ConstructWidget<UButton>();
	ClassesButton->AddChild(MakeText(nullptr, TEXT("Class path / Master"), 14));
	ClassesButton->OnClicked.AddDynamic(this, &UNightfallHud::HandleClassesClicked);
	if (UCanvasPanelSlot* Slot = Root->AddChildToCanvas(ClassesButton))
	{
		Slot->SetPosition(FVector2D(16.f, 210.f));
		Slot->SetSize(FVector2D(260.f, 28.f));
	}

	UButton* SkillsButton=WidgetTree->ConstructWidget<UButton>();SkillsButton->AddChild(MakeText(nullptr,TEXT("Skill trainer"),14));
 SkillsButton->OnClicked.AddDynamic(this,&UNightfallHud::HandleSkillsClicked);
 auto* InventoryButton=WidgetTree->ConstructWidget<UButton>();InventoryButton->AddChild(MakeText(nullptr,TEXT("Inventory (I)"),14));SkillsPanel->AddChild(InventoryButton);InventoryButton->OnClicked.AddDynamic(this,&UNightfallHud::HandleInventoryClicked);
 if(auto* Slot=Root->AddChildToCanvas(SkillsButton)){Slot->SetPosition(FVector2D(16,242));Slot->SetSize(FVector2D(260,28));}
 auto* SkillsPanel=WidgetTree->ConstructWidget<UVerticalBox>();
 SkillResources=MakeText(SkillsPanel,TEXT("SP —"),14); EffectsText=MakeText(SkillsPanel,TEXT(""),12);EffectsText->SetAutoWrapText(true);
 CastLabel=MakeText(SkillsPanel,TEXT(""),14);CastBar=MakeBar(SkillsPanel,FLinearColor(0.55f,0.3f,0.85f),700,8);
 auto* Row=WidgetTree->ConstructWidget<UHorizontalBox>();SkillsPanel->AddChild(Row);
 for(int32 I=0;I<9;++I)
 {
  auto* Col=WidgetTree->ConstructWidget<UVerticalBox>();Row->AddChild(Col);
  auto* B=WidgetTree->ConstructWidget<UButton>();auto* Label=MakeText(nullptr,FString::FromInt(I+1),12);Label->SetColorAndOpacity(FSlateColor(FLinearColor::Black));Label->SetAutoWrapText(true);B->AddChild(Label);
  auto* Size=WidgetTree->ConstructWidget<USizeBox>();Size->SetWidthOverride(78);Size->SetHeightOverride(54);Size->AddChild(B);Col->AddChild(Size);
  auto* H=NewObject<UNightfallHotbarClick>(this);H->Index=I;HotbarClicks.Add(H);B->OnClicked.AddDynamic(H,&UNightfallHotbarClick::Click);
  HotbarButtons.Add(B);HotbarLabels.Add(Label);Cooldowns.Add(MakeBar(Col,FLinearColor(0.2f,0.6f,1.f),78,5));
 }
 auto* ItemsRow=WidgetTree->ConstructWidget<UHorizontalBox>();SkillsPanel->AddChild(ItemsRow);
 for(int32 I=0;I<9;++I){auto* B=WidgetTree->ConstructWidget<UButton>();auto* L=MakeText(nullptr,FString::Printf(TEXT("F%d empty"),I+1),11);B->AddChild(L);ItemsRow->AddChild(B);ItemButtons.Add(B);ItemLabels.Add(L);auto* H=NewObject<UNightfallHotbarClick>(this);H->Index=I;H->bItem=true;ItemClicks.Add(H);B->OnClicked.AddDynamic(H,&UNightfallHotbarClick::Click);}
 if(auto* Slot=Root->AddChildToCanvas(SkillsPanel)){Slot->SetAnchors(FAnchors(0.5f,1));Slot->SetAlignment(FVector2D(0.5f,1));Slot->SetPosition(FVector2D(0,-65));Slot->SetSize(FVector2D(720,225));}
 // Target frame, top-centre.
	TargetPanel = WidgetTree->ConstructWidget<UVerticalBox>();
	TargetPanel->SetVisibility(ESlateVisibility::Collapsed);
	TargetLabel = MakeText(TargetPanel, TEXT(""), 16);
	TargetHpBar = MakeBar(TargetPanel, NpcHpColor, 280.f, 18.f);
	TargetHpText = MakeText(TargetPanel, TEXT(""), 12);
	if (UCanvasPanelSlot* Slot = Root->AddChildToCanvas(TargetPanel))
	{
		Slot->SetAnchors(FAnchors(0.5f, 0.f));
		Slot->SetAlignment(FVector2D(0.5f, 0.f));
		Slot->SetPosition(FVector2D(0.f, 16.f));
		Slot->SetSize(FVector2D(280.f, 70.f));
	}

	// Status line, bottom-centre.
	StatusText = MakeText(nullptr, TEXT(""), 16);
	StatusText->SetVisibility(ESlateVisibility::HitTestInvisible);
	if (UCanvasPanelSlot* Slot = Root->AddChildToCanvas(StatusText))
	{
		Slot->SetAnchors(FAnchors(0.5f, 1.f));
		Slot->SetAlignment(FVector2D(0.5f, 1.f));
		Slot->SetPosition(FVector2D(0.f, -32.f));
		Slot->SetAutoSize(true);
	}

	// Floating bars and damage numbers live above the frames and below the dead overlay.
	FloatLayer = WidgetTree->ConstructWidget<UCanvasPanel>();
	FloatLayer->SetVisibility(ESlateVisibility::HitTestInvisible);
	if (UCanvasPanelSlot* Slot = Root->AddChildToCanvas(FloatLayer))
	{
		Slot->SetAnchors(FAnchors(0.f, 0.f, 1.f, 1.f));
		Slot->SetOffsets(FMargin(0.f));
	}

	// Dead overlay: swallows clicks, offers Respawn.
	DeadOverlay = WidgetTree->ConstructWidget<UBorder>();
	DeadOverlay->SetBrushColor(FLinearColor(0.f, 0.f, 0.f, 0.6f));
	DeadOverlay->SetHorizontalAlignment(HAlign_Center);
	DeadOverlay->SetVerticalAlignment(VAlign_Center);
	UVerticalBox* DeadColumn = WidgetTree->ConstructWidget<UVerticalBox>();
	DeadOverlay->SetContent(DeadColumn);
	MakeText(DeadColumn, TEXT("You have died"), 32);
	OfferLabel=MakeText(DeadColumn,TEXT(""),16);
 AcceptOffer=WidgetTree->ConstructWidget<UButton>();AcceptOffer->AddChild(MakeText(nullptr,TEXT("Accept resurrection"),18));DeadColumn->AddChild(AcceptOffer);
 DeclineOffer=WidgetTree->ConstructWidget<UButton>();DeclineOffer->AddChild(MakeText(nullptr,TEXT("Decline resurrection"),18));DeadColumn->AddChild(DeclineOffer);
 Cast<UTextBlock>(AcceptOffer->GetChildAt(0))->SetColorAndOpacity(FSlateColor(FLinearColor::Black));
 Cast<UTextBlock>(DeclineOffer->GetChildAt(0))->SetColorAndOpacity(FSlateColor(FLinearColor::Black));
 AcceptOffer->OnClicked.AddDynamic(this,&UNightfallHud::AcceptResurrect);DeclineOffer->OnClicked.AddDynamic(this,&UNightfallHud::DeclineResurrect);
 RespawnButton = WidgetTree->ConstructWidget<UButton>();
	RespawnLabel = MakeText(nullptr, TEXT("Respawn"), 20);
	RespawnButton->AddChild(RespawnLabel);
	DeadColumn->AddChild(RespawnButton);
	DeadOverlay->SetVisibility(ESlateVisibility::Collapsed);
	if (UCanvasPanelSlot* Slot = Root->AddChildToCanvas(DeadOverlay))
	{
		Slot->SetAnchors(FAnchors(0.f, 0.f, 1.f, 1.f));
		Slot->SetOffsets(FMargin(0.f));
	}
}

void UNightfallHud::NativeConstruct()
{
	Super::NativeConstruct();
	SetVisibility(ESlateVisibility::SelfHitTestInvisible);
	const UGameInstance* GI = GetGameInstance();
	if (UCombatStateSubsystem* Combat = GI ? GI->GetSubsystem<UCombatStateSubsystem>() : nullptr)
	{
		DamageHandle = Combat->OnDamageNumber.AddUObject(this, &UNightfallHud::SpawnNumber);
	}
	if (ULoginFlowSubsystem* Flow = GI ? GI->GetSubsystem<ULoginFlowSubsystem>() : nullptr)
	{
		Flow->OnStatus.AddDynamic(this, &UNightfallHud::HandleStatus);
	}
	if (RespawnButton) RespawnButton->OnClicked.AddDynamic(this, &UNightfallHud::HandleRespawnClicked);
}

void UNightfallHud::NativeDestruct()
{
	const UGameInstance* GI = GetGameInstance();
	if (UCombatStateSubsystem* Combat = GI ? GI->GetSubsystem<UCombatStateSubsystem>() : nullptr)
	{
		Combat->OnDamageNumber.Remove(DamageHandle);
	}
	if (ULoginFlowSubsystem* Flow = GI ? GI->GetSubsystem<ULoginFlowSubsystem>() : nullptr)
	{
		Flow->OnStatus.RemoveDynamic(this, &UNightfallHud::HandleStatus);
	}
	if (RespawnButton) RespawnButton->OnClicked.RemoveDynamic(this, &UNightfallHud::HandleRespawnClicked);
	Super::NativeDestruct();
}

void UNightfallHud::HandleStatus(const FString& Status)
{
	if (StatusText)
	{
		StatusText->SetText(FText::FromString(Status));
		StatusAge = 0.f;
	}
}

void UNightfallHud::HandleRespawnClicked()
{
	const UGameInstance* GI = GetGameInstance();
	if (UCombatStateSubsystem* Combat = GI ? GI->GetSubsystem<UCombatStateSubsystem>() : nullptr)
	{
		Combat->RequestRespawn();
	}
}

void UNightfallHud::ApplyModel(const FCombatHudModel& M)
{
	if (!Root) return;
	OwnPanel->SetVisibility(M.bOwnKnown ? ESlateVisibility::HitTestInvisible : ESlateVisibility::Collapsed);
	OwnLabel->SetText(FText::FromString(FString::Printf(TEXT("%s  %s"), *M.OwnName, *M.LevelText)));
	OwnHpBar->SetPercent(M.OwnHpFraction);
	OwnHpText->SetText(FText::FromString(M.OwnHpText));
	OwnMpBar->SetPercent(M.OwnMpFraction);
	OwnMpText->SetText(FText::FromString(M.OwnMpText));
	OwnCpBar->SetPercent(M.OwnCpFraction);
	OwnCpText->SetText(FText::FromString(M.OwnCpText));
	if (const UGameInstance* GI = GetGameInstance())
	{
		if (const UClassStateSubsystem* Classes = GI->GetSubsystem<UClassStateSubsystem>())
		{
			FString Label = Classes->OwnClassLabel();
			if (const auto* Combat = GI->GetSubsystem<UCombatStateSubsystem>(); Combat && Combat->GetOwn().bCpKnown)
			{
				const auto& Own = Combat->GetOwn();
				const auto* Skills=GI->GetSubsystem<USkillStateSubsystem>();
    const uint64 Sp=Skills && Skills->GetSp().IsSet()?*Skills->GetSp():Own.Sp;
    Label += FString::Printf(TEXT("  SP %llu  Tokens %u / %u"), Sp, Own.TokenTier1Count, Own.TokenTier2Count);
			}
			ClassText->SetText(FText::FromString(Label));
		}
	}
	XpText->SetText(FText::FromString(M.XpText));
	AttackText->SetText(FText::FromString(M.AttackText));

	TargetPanel->SetVisibility(M.bTargetVisible ? ESlateVisibility::HitTestInvisible : ESlateVisibility::Collapsed);
	if (M.bTargetVisible)
	{
		TargetLabel->SetText(FText::FromString(FString::Printf(TEXT("%s  Lv %u"), *M.TargetName, M.TargetLevel)));
		TargetHpBar->SetPercent(M.TargetHpFraction);
		TargetHpText->SetText(FText::FromString(M.TargetHpText));
	}

	DeadOverlay->SetVisibility(M.bDeadOverlay ? ESlateVisibility::Visible : ESlateVisibility::Collapsed);
	RespawnButton->SetIsEnabled(!M.bRespawnPending);
}

bool UNightfallHud::ProjectToCanvas(const FVector& World, FVector2D& OutPosition) const
{
	APlayerController* PC = GetOwningPlayer();
	return PC && UWidgetLayoutLibrary::ProjectWorldLocationToWidgetPosition(PC, World, OutPosition, /*bPlayerViewportRelative=*/false);
}

void UNightfallHud::UpdateFloatingBars()
{
	const UWorld* World = GetWorld();
	UWorldProxySubsystem* Proxies = World ? World->GetSubsystem<UWorldProxySubsystem>() : nullptr;
	const UGameInstance* GI = GetGameInstance();
	UCombatStateSubsystem* Combat = GI ? GI->GetSubsystem<UCombatStateSubsystem>() : nullptr;
	if (!Proxies || !Combat) return;

	TSet<FString> Live;
	for (const TPair<FString, TObjectPtr<ARemoteEntityActor>>& Pair : Proxies->GetProxies())
	{
		const ARemoteEntityActor* Actor = Pair.Value;
		const FCombatEntity* E = Combat->FindEntity(Pair.Key);
		const bool bMaster = Cast<AClassMasterActor>(Actor) != nullptr;
		FVector2D Screen;
		if (!Actor || (!bMaster && (!E || !E->bCombatant || E->bDead || E->MaxHp == 0))
			|| !ProjectToCanvas(Actor->GetActorLocation() + FVector(0.f, 0.f, BarHeightCm), Screen))
		{
			continue;
		}
		const FString K = Pair.Key.ToLower();
		Live.Add(K);
		FFloatingBar* Bar = Bars.Find(K);
		if (!Bar)
		{
			FFloatingBar New;
			New.Box = WidgetTree->ConstructWidget<UVerticalBox>();
			New.Box->SetVisibility(ESlateVisibility::HitTestInvisible);
			New.Name = MakeText(New.Box, TEXT(""), bMaster ? 16 : 11);
			New.Name->SetJustification(ETextJustify::Center);
			New.Bar = MakeBar(New.Box, E && E->Spawn.Kind == 1 ? MpColor : NpcHpColor, 70.f, 8.f);
			if (UCanvasPanelSlot* Slot = FloatLayer->AddChildToCanvas(New.Box))
			{
				Slot->SetAlignment(FVector2D(0.5f, 1.f));
				Slot->SetSize(FVector2D(bMaster ? 160.f : 80.f, 28.f));
			}
			BarBoxes.Add(K, New.Box);
			Bar = &Bars.Add(K, New);
		}
		Bar->Name->SetText(FText::FromString(Actor->GetNameplate()));
		Bar->Name->SetColorAndOpacity(FSlateColor(bMaster || Actor->HasTransferCue() ? FLinearColor(1.f, 0.8f, 0.2f) : FLinearColor::White));
		Bar->Bar->SetVisibility(bMaster ? ESlateVisibility::Collapsed : ESlateVisibility::HitTestInvisible);
		if (!bMaster)
		{
			Bar->Bar->SetPercent(E->MaxHp == 0 ? 0.f : FMath::Clamp(static_cast<float>(E->Hp) / static_cast<float>(E->MaxHp), 0.f, 1.f));
			Bar->Bar->SetFillColorAndOpacity(E->Spawn.Kind == 1 ? FLinearColor(0.2f, 0.7f, 0.25f) : NpcHpColor);
		}
		if (UCanvasPanelSlot* Slot = Cast<UCanvasPanelSlot>(Bar->Box->Slot)) Slot->SetPosition(Screen);
	}
	for (auto It = Bars.CreateIterator(); It; ++It)
	{
		if (!Live.Contains(It.Key()))
		{
			It.Value().Box->RemoveFromParent();
			BarBoxes.Remove(It.Key());
			It.RemoveCurrent();
		}
	}
}

void UNightfallHud::SpawnNumber(const FDamageNumber& N)
{
	if (!Root) return;
	const UWorld* World = GetWorld();
	FVector Anchor = FVector::ZeroVector;
	bool bFound = false;
	if (N.bTargetIsOwn)
	{
		if (const APlayerController* PC = GetOwningPlayer(); PC && PC->GetPawn())
		{
			Anchor = PC->GetPawn()->GetActorLocation() + FVector(0.f, 0.f, 120.f);
			bFound = true;
		}
	}
	else if (const UWorldProxySubsystem* Proxies = World ? World->GetSubsystem<UWorldProxySubsystem>() : nullptr)
	{
		for (const TPair<FString, TObjectPtr<ARemoteEntityActor>>& Pair : Proxies->GetProxies())
		{
			if (Pair.Key.Equals(N.TargetId, ESearchCase::IgnoreCase) && Pair.Value)
			{
				Anchor = Pair.Value->GetActorLocation() + FVector(0.f, 0.f, BarHeightCm - 40.f);
				bFound = true;
				break;
			}
		}
	}
	if (!bFound) return;   // target not on screen/world: nothing to float over

	FString Label;
	FLinearColor Color = FLinearColor::White;
	int32 Size = 20;
	switch (N.Outcome)
	{
	case ENetAttackOutcome::Miss: Label = TEXT("Miss"); Color = FLinearColor(0.7f, 0.7f, 0.7f); Size = 14; break;
	case ENetAttackOutcome::Crit: Label = FString::Printf(TEXT("%u!"), N.Damage); Color = FLinearColor(1.f, 0.85f, 0.1f); Size = 30; break;
	default: Label = FString::FromInt(N.Damage); Color = N.bTargetIsOwn ? FLinearColor(1.f, 0.3f, 0.3f) : FLinearColor::White; break;
	}
	FFloatingNumber F;
	F.Text = MakeText(nullptr, Label, Size);
	F.Text->SetColorAndOpacity(FSlateColor(Color));
	F.Text->SetVisibility(ESlateVisibility::HitTestInvisible);
	F.Anchor = Anchor + FVector(0.f, FMath::FRandRange(-30.f, 30.f), 0.f);   // fan out simultaneous numbers
	if (UCanvasPanelSlot* Slot = FloatLayer->AddChildToCanvas(F.Text))
	{
		Slot->SetAutoSize(true);
		Slot->SetAlignment(FVector2D(0.5f, 1.f));
		Slot->SetZOrder(10);
	}
	Numbers.Add(F);
}

void UNightfallHud::UpdateNumbers(float DeltaSeconds)
{
	for (int32 I = Numbers.Num() - 1; I >= 0; --I)
	{
		FFloatingNumber& F = Numbers[I];
		F.Age += DeltaSeconds;
		FVector2D Screen;
		if (F.Age >= DamageNumberSeconds || !ProjectToCanvas(F.Anchor + FVector(0.f, 0.f, 120.f * F.Age), Screen))
		{
			F.Text->RemoveFromParent();
			Numbers.RemoveAtSwap(I);
			continue;
		}
		F.Text->SetRenderOpacity(1.f - FMath::Clamp((F.Age - 0.6f * DamageNumberSeconds) / (0.4f * DamageNumberSeconds), 0.f, 1.f));
		if (UCanvasPanelSlot* Slot = Cast<UCanvasPanelSlot>(F.Text->Slot)) Slot->SetPosition(Screen);
	}
}

void UNightfallHud::NativeTick(const FGeometry& MyGeometry, float InDeltaTime)
{
	Super::NativeTick(MyGeometry, InDeltaTime);
	RefreshPresentation();
	UpdateFloatingBars();
	UpdateNumbers(InDeltaTime);
	if (StatusText)
	{
		StatusAge += InDeltaTime;
		if (StatusAge > StatusSeconds) StatusText->SetText(FText::GetEmpty());
	}
}

void UNightfallHud::HandleClassesClicked()
{
	if (ANightfallPlayerController* PC = Cast<ANightfallPlayerController>(GetOwningPlayer())) PC->OpenClassDialog();
}

namespace NightfallSkillHud
{
 FLinearColor FlagColor(const FString& Flag){return Flag==TEXT("chaotic")?FLinearColor::Red:Flag==TEXT("pvp")?FLinearColor(0.7f,0.25f,1.f):FLinearColor::White;}
}
void UNightfallHotbarClick::Click()
{
 auto* Hud=Cast<UNightfallHud>(GetOuter());auto* GI=Hud?Hud->GetGameInstance():nullptr;if(!GI)return;
 auto* C=GI->GetSubsystem<UCombatStateSubsystem>();if(C && C->IsOwnDead())return;
 if(bItem){if(auto* S=GI->GetSubsystem<UInventoryStateSubsystem>())S->ActivateHotbar(Index);}
 else if(auto* S=GI->GetSubsystem<USkillStateSubsystem>())S->CastSlot(Index);
}
void UNightfallHud::HandleSkillsClicked(){if(auto* PC=Cast<ANightfallPlayerController>(GetOwningPlayer()))PC->OpenSkillDialog();}
void UNightfallHud::AcceptResurrect()
{
 auto* GI=GetGameInstance();auto* S=GI?GI->GetSubsystem<USkillStateSubsystem>():nullptr;
 if(S && S->OfferPending())GI->GetSubsystem<UNetClientSubsystem>()->SendAcceptResurrect(S->GetOffer()->OfferId,true);
}
void UNightfallHud::DeclineResurrect()
{
 auto* GI=GetGameInstance();auto* S=GI?GI->GetSubsystem<USkillStateSubsystem>():nullptr;
 if(S && S->OfferPending())GI->GetSubsystem<UNetClientSubsystem>()->SendAcceptResurrect(S->GetOffer()->OfferId,false);
}
void UNightfallHud::UpdateSkills()
{
 auto* GI=GetGameInstance();auto* S=GI?GI->GetSubsystem<USkillStateSubsystem>():nullptr;auto* N=GI?GI->GetSubsystem<UNetClientSubsystem>():nullptr;
 auto* C=GI?GI->GetSubsystem<UCombatStateSubsystem>():nullptr;if(!S || !N || !C || !SkillResources)return;
 SkillResources->SetText(FText::FromString(S->GetSp().IsSet()?FString::Printf(TEXT("SP %llu   Death penalty %u"),*S->GetSp(),S->GetDeathPenalty()):FString(TEXT("SP —"))));
 const auto Slots=S->Hotbar();
 for(int32 I=0;I<HotbarButtons.Num();++I)
 {
  const bool Valid=Slots.IsValidIndex(I) && Slots[I]!=0;const auto Remaining=Valid?S->ReuseRemaining(Slots[I]):TOptional<uint64>();
  const auto* Skill=Valid?S->FindSkill(Slots[I]):nullptr;
  HotbarLabels[I]->SetText(FText::FromString(Valid?FString::Printf(TEXT("%d %s\nr%u%s"),I+1,Skill?*Skill->DisplayName:TEXT("Skill"),S->GetLearned().FindRef(Slots[I]),Remaining.IsSet() && *Remaining?*FString::Printf(TEXT(" [%llu ticks]"),*Remaining):TEXT("")):FString::Printf(TEXT("%d —"),I+1)));
  HotbarButtons[I]->SetIsEnabled(Valid && Remaining.IsSet() && *Remaining==0 && !C->IsOwnDead());
  Cooldowns[I]->SetPercent(Valid?S->CooldownFraction(Slots[I]):0.f);
 }
 const auto* V=S->FindCast(N->GetOwnEntityId());
 if(V && V->Cast.IsSet())
 {
  const auto& Cast=*V->Cast;const auto* Skill=S->FindSkill(Cast.SkillId);
  CastLabel->SetText(FText::FromString(FString::Printf(TEXT("Casting %s [%llu ticks]"),Skill?*Skill->DisplayName:TEXT("skill"),Cast.CompleteTick>S->GetTick()?Cast.CompleteTick-S->GetTick():0)));
  const uint64 Duration=Cast.CompleteTick>Cast.StartTick?Cast.CompleteTick-Cast.StartTick:0;
  CastBar->SetPercent(Duration?FMath::Clamp(float(S->GetTick()>Cast.StartTick?S->GetTick()-Cast.StartTick:0)/float(Duration),0.f,1.f):1.f);
 }else{CastLabel->SetText(FText::FromString(V && V->TerminalId?(V->CancelReason?TEXT("Cast cancelled"):TEXT("Cast completed")):TEXT("")));CastBar->SetPercent(0);}
 TArray<FString> Effects;
 if(auto* E=GI->GetSubsystem<UEffectStateSubsystem>())for(const auto& Effect:E->Active(N->GetOwnEntityId()))
 {
  const auto* Skill=S->FindSkill(Effect.SkillId);const TCHAR* Pool=Effect.Pool==3?TEXT("Debuff"):Effect.Pool==2?TEXT("Song/dance"):TEXT("Buff");
  Effects.Add(FString::Printf(TEXT("%s: %s r%u [%s]"),Pool,Skill?*Skill->DisplayName:*FString::FromInt(Effect.SkillId),Effect.SkillLevel,Effect.EndTick?*FString::Printf(TEXT("%llu ticks"),Effect.EndTick>S->GetTick()?Effect.EndTick-S->GetTick():0):TEXT("permanent")));
 }
 EffectsText->SetText(FText::FromString(FString::Join(Effects,TEXT(" • "))));
 OwnLabel->SetColorAndOpacity(FSlateColor(NightfallSkillHud::FlagColor(S->Flag(N->GetOwnEntityId()))));
 const FString DisplayTarget = S->CorpseTarget().Get(C->GetTargetId());
 if (S->CorpseTarget().IsSet())
 {
  const auto* Corpse = C->FindEntity(DisplayTarget);
  TargetPanel->SetVisibility(ESlateVisibility::Visible);
  TargetLabel->SetText(FText::FromString(FString::Printf(TEXT("%s [corpse]"), Corpse ? *Corpse->Spawn.Name : *DisplayTarget)));
  TargetHpBar->SetPercent(0.f); TargetHpText->SetText(FText::FromString(TEXT("Dead")));
 }
 TargetLabel->SetColorAndOpacity(FSlateColor(NightfallSkillHud::FlagColor(S->Flag(DisplayTarget))));
 if(const auto* P=S->FindPvp(DisplayTarget);P && !DisplayTarget.IsEmpty())TargetLabel->SetText(FText::FromString(TargetLabel->GetText().ToString()+FString::Printf(TEXT("   Karma %u"),P->Karma)));
 const bool Pending=S->OfferPending();AcceptOffer->SetVisibility(Pending?ESlateVisibility::Visible:ESlateVisibility::Collapsed);DeclineOffer->SetVisibility(Pending?ESlateVisibility::Visible:ESlateVisibility::Collapsed);
 OfferLabel->SetText(FText::FromString(Pending?FString::Printf(TEXT("Resurrection offered — recovery %.2f%%, recoverable XP %llu, expires in %llu ticks"),double(S->GetOffer()->RestorePercentQ)/1000000.0,S->GetOffer()->RecoverableXp,S->GetOffer()->ExpiryTick-S->GetTick()):FString(TEXT(""))));
}

void UNightfallHud::RefreshPresentation()
{
 EnsureLayout();
 const auto* GI = GetGameInstance();
 if (const auto* Combat = GI ? GI->GetSubsystem<UCombatStateSubsystem>() : nullptr) ApplyModel(Combat->BuildHudModel());
 UpdateSkills(); UpdateItems();
}
float UNightfallHud::CastProgress() const { return CastBar ? CastBar->GetPercent() : 0.f; }
FString UNightfallHud::EffectLabel() const { return EffectsText ? EffectsText->GetText().ToString() : FString(); }
FString UNightfallHud::CastStatus() const { return CastLabel ? CastLabel->GetText().ToString() : FString(); }
FLinearColor UNightfallHud::OwnFlagColor() const { return OwnLabel ? OwnLabel->GetColorAndOpacity().GetSpecifiedColor() : FLinearColor::White; }
FLinearColor UNightfallHud::TargetFlagColor() const { return TargetLabel ? TargetLabel->GetColorAndOpacity().GetSpecifiedColor() : FLinearColor::White; }

void UNightfallHud::HandleInventoryClicked(){if(auto* PC=Cast<ANightfallPlayerController>(GetOwningPlayer()))PC->OpenInventoryDialog();}
void UNightfallHud::UpdateItems()
{
 const auto* GI=GetGameInstance();const auto* S=GI?GI->GetSubsystem<UInventoryStateSubsystem>():nullptr;if(!S)return;const auto Slots=S->Hotbar();
 for(int32 I=0;I<ItemButtons.Num();++I){const auto& Slot=Slots[I];const auto* D=S->Definition(Slot.Key);const auto Count=S->Count(Slot.Key);ItemLabels[I]->SetText(FText::FromString(FString::Printf(TEXT("F%d %s
x%llu%s"),I+1,D?*D->Name:TEXT("empty"),Count.IsSet()?*Count:0,Slot.Value?(S->AutoShot(Slot.Key)?TEXT(" AUTO ON"):TEXT(" AUTO OFF")):TEXT(""))));ItemButtons[I]->SetIsEnabled(S->HasBaseline() && D && Count.IsSet() && (Slot.Value || *Count>0));}
}
