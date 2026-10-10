#include "NightfallHud.h"
#include "Nightfall.h"
#include "Character/ClassStateSubsystem.h"
#include "NightfallPlayerController.h"
#include "Game/LoginFlowSubsystem.h"
#include "World/RemoteEntityActor.h"
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
				Label += FString::Printf(TEXT("  SP %llu  Tokens %u / %u"), Own.Sp, Own.TokenTier1Count, Own.TokenTier2Count);
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
		FVector2D Screen;
		if (!Actor || !E || !E->bCombatant || E->bDead || E->MaxHp == 0
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
			New.Name = MakeText(New.Box, TEXT(""), 11);
			New.Bar = MakeBar(New.Box, E->Spawn.Kind == 1 ? MpColor : NpcHpColor, 70.f, 8.f);
			if (UCanvasPanelSlot* Slot = FloatLayer->AddChildToCanvas(New.Box))
			{
				Slot->SetAlignment(FVector2D(0.5f, 1.f));
				Slot->SetSize(FVector2D(80.f, 28.f));
			}
			BarBoxes.Add(K, New.Box);
			Bar = &Bars.Add(K, New);
		}
		Bar->Name->SetText(FText::FromString(Actor->GetNameplate()));
		Bar->Name->SetColorAndOpacity(FSlateColor(Actor->HasTransferCue() ? FLinearColor(1.f, 0.8f, 0.2f) : FLinearColor::White));
		Bar->Bar->SetPercent(E->MaxHp == 0 ? 0.f : FMath::Clamp(static_cast<float>(E->Hp) / static_cast<float>(E->MaxHp), 0.f, 1.f));
		Bar->Bar->SetFillColorAndOpacity(E->Spawn.Kind == 1 ? FLinearColor(0.2f, 0.7f, 0.25f) : NpcHpColor);
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
	const UGameInstance* GI = GetGameInstance();
	if (const UCombatStateSubsystem* Combat = GI ? GI->GetSubsystem<UCombatStateSubsystem>() : nullptr)
	{
		ApplyModel(Combat->BuildHudModel());
	}
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
