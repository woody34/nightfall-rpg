#include "NightfallClassDialog.h"
#include "Character/ClassStateSubsystem.h"
#include "Blueprint/WidgetTree.h"
#include "Components/Border.h"
#include "Components/Button.h"
#include "Components/ScrollBox.h"
#include "Components/SizeBox.h"
#include "Components/TextBlock.h"
#include "Components/VerticalBox.h"
#include "Engine/GameInstance.h"

namespace
{
	FString SkillLabel(const FString& Ref, uint32 Id)
	{
		return !Ref.IsEmpty() && !Ref.StartsWith(TEXT("l2.")) ? Ref : FString::Printf(TEXT("Skill %u"), Id);
	}
}

void UNightfallClassChoiceHandler::HandleClicked() { if (Dialog.IsValid()) Dialog->Select(ClassId); }

UTextBlock* UNightfallClassDialog::Text(UPanelWidget* Parent, const FString& Label, int32 Size)
{
	UTextBlock* T = WidgetTree->ConstructWidget<UTextBlock>();
	T->SetText(FText::FromString(Label));
	T->SetAutoWrapText(true);
	FSlateFontInfo Font = T->GetFont();
	Font.Size = Size;
	T->SetFont(Font);
	if (Parent) Parent->AddChild(T);
	return T;
}

UButton* UNightfallClassDialog::Button(UPanelWidget* Parent, const FString& Label)
{
	UButton* B = WidgetTree->ConstructWidget<UButton>();
	UTextBlock* ButtonLabel = Text(nullptr, Label);
	ButtonLabel->SetColorAndOpacity(FSlateColor(FLinearColor(0.015f, 0.02f, 0.03f)));
	B->AddChild(ButtonLabel);
	if (Parent) Parent->AddChild(B);
	return B;
}

void UNightfallClassDialog::EnsureLayout()
{
	if (!WidgetTree) WidgetTree = NewObject<UWidgetTree>(this, TEXT("WidgetTree"), RF_Transient);
	if (WidgetTree->RootWidget) return;
	UBorder* Root = WidgetTree->ConstructWidget<UBorder>();
	Root->SetBrushColor(FLinearColor(0.015f, 0.02f, 0.04f, 0.96f));
	Root->SetHorizontalAlignment(HAlign_Center);
	Root->SetVerticalAlignment(VAlign_Fill);
	Root->SetPadding(FMargin(24.f));
	WidgetTree->RootWidget = Root;
	USizeBox* Width = WidgetTree->ConstructWidget<USizeBox>();
	Width->SetWidthOverride(680.f);
	Root->SetContent(Width);
	UVerticalBox* Column = WidgetTree->ConstructWidget<UVerticalBox>();
	UScrollBox* DialogScroll = WidgetTree->ConstructWidget<UScrollBox>();
	Width->AddChild(DialogScroll);
	DialogScroll->AddChild(Column);
	Text(Column, TEXT("Class Master"), 26);
	Summary = Text(Column, TEXT("Loading your class path..."), 16);
	Status = Text(Column, TEXT(""));
	USizeBox* Height = WidgetTree->ConstructWidget<USizeBox>();
	Height->SetHeightOverride(280.f);
	Column->AddChild(Height);
	UScrollBox* Scroll = WidgetTree->ConstructWidget<UScrollBox>();
	Height->AddChild(Scroll);
	Tree = WidgetTree->ConstructWidget<UVerticalBox>();
	Scroll->AddChild(Tree);
	USizeBox* DetailsHeight = WidgetTree->ConstructWidget<USizeBox>();
	DetailsHeight->SetHeightOverride(100.f);
	Column->AddChild(DetailsHeight);
	UScrollBox* DetailsScroll = WidgetTree->ConstructWidget<UScrollBox>();
	DetailsHeight->AddChild(DetailsScroll);
	SkillDetails = Text(DetailsScroll, TEXT("Select any class to inspect its learning metadata. Skill effects are not available yet."));
	Confirmation = Text(Column, TEXT("Choose an eligible next class. Transfers are permanent."));
	ConfirmButton = Button(Column, TEXT("Confirm class transfer"));
	ConfirmButton->SetIsEnabled(false);
	ConfirmButton->OnClicked.AddDynamic(this, &UNightfallClassDialog::HandleConfirm);
	Button(Column, TEXT("Refresh options"))->OnClicked.AddDynamic(this, &UNightfallClassDialog::HandleRefresh);
	Button(Column, TEXT("Close"))->OnClicked.AddDynamic(this, &UNightfallClassDialog::HandleClose);
}

void UNightfallClassDialog::NativeOnInitialized()
{
	Super::NativeOnInitialized();
	EnsureLayout();
	BindState(GetState());
}

void UNightfallClassDialog::NativeDestruct()
{
	if (UClassStateSubsystem* Classes = GetState()) Classes->OnChanged.Remove(StateHandle);
	Super::NativeDestruct();
}

TOptional<FUIInputConfig> UNightfallClassDialog::GetDesiredInputConfig() const
{
	return FUIInputConfig(ECommonInputMode::Menu, EMouseCaptureMode::NoCapture);
}

void UNightfallClassDialog::Refresh()
{
	if (const UClassStateSubsystem* Classes = GetState(); Classes && Classes->IsBusy()) return;
	SelectedTarget.Reset();
	if (ConfirmButton) ConfirmButton->SetIsEnabled(false);
	if (UClassStateSubsystem* Classes = GetState())
	{
		Classes->LoadOptions([Weak = TWeakObjectPtr<UNightfallClassDialog>(this)](const FNetResult& R)
		{
			if (!Weak.IsValid()) return;
			Weak->Status->SetText(FText::FromString(R.IsOk() ? TEXT("Server requirements checked.") : *R.Message));
			Weak->Render();
		});
	}
}

void UNightfallClassDialog::Render()
{
	UClassStateSubsystem* Classes = GetState();
	if (!Classes || !Tree) return;
	const auto& Options = Classes->GetOptions();
	const auto& Catalogue = Classes->GetCatalogue();
	const FGrpcNightfallV1ClassInfo* Current = Classes->HasOptions() ? Classes->FindClass(Options.CurrentClassId.Value) : nullptr;
	Tree->ClearChildren();
	Handlers.Reset();
	if (!Current)
	{
		SelectedTarget.Reset();
		if (ConfirmButton) ConfirmButton->SetIsEnabled(false);
		if (Confirmation) Confirmation->SetText(FText::FromString(TEXT("Server options are unavailable. Refresh before choosing a transfer.")));
		return;
	}
	const TArray<uint32> Path = Classes->PathTo(Current->ClassId.Value);
	TArray<FString> PathNames;
	for (uint32 Id : Path) if (const auto* C = Classes->FindClass(Id)) PathNames.Add(C->DisplayName);
	Summary->SetText(FText::FromString(FString::Printf(TEXT("%s\nTier 1 tokens: %u   Tier 2 tokens: %u\n%s: (%.0f, %.0f) tiles, interaction radius %.0f tiles. %s"), *FString::Join(PathNames, TEXT(" > ")), Options.TokenTier1Count.Value, Options.TokenTier2Count.Value, *Catalogue.ClassMaster.Name, Catalogue.ClassMaster.Position.X, Catalogue.ClassMaster.Position.Y, Catalogue.ClassMaster.InteractionRadius, Classes->IsAtMaster() ? TEXT("You are nearby.") : TEXT("Travel to the master to transfer."))));
	for (const auto& C : Catalogue.Classes)
	{
		const TArray<uint32> CandidatePath = Classes->PathTo(C.ClassId.Value);
		if (Path.IsEmpty() || CandidatePath.IsEmpty() || Path[0] != CandidatePath[0]) continue;
		const auto* O = Options.Options.FindByPredicate([&](const auto& Entry) { return Entry.ClassId.Value == C.ClassId.Value; });
				FString Label = FString::ChrN(C.Tier.Value * 2, TEXT(' ')) + C.DisplayName;
		Label += FString::Printf(TEXT("  (level %u)"), C.MinLevel.Value);
		if (C.ClassId.Value == Current->ClassId.Value) Label += TEXT(" [current]");
		else if (Path.Contains(C.ClassId.Value)) Label += TEXT(" [your path]");
		else if (C.Tier.Value > Catalogue.MaxTransferTier.Value) Label += TEXT(" [not available yet]");
		if (O && !O->Unmet.IsEmpty()) Label += TEXT(" — ") + FString::Join(O->Unmet, TEXT(", "));
		UButton* Choice = Button(Tree, Label);
		TArray<FString> Skills;
		for (const auto& Skill : C.SkillTree) Skills.Add(FString::Printf(TEXT("%s %u: level %u, SP %llu%s"), *SkillLabel(Skill.L2Ref, Skill.SkillId.Value), Skill.SkillLevel.Value, Skill.RequiredLevel.Value, Skill.SpCost.Value, Skill.AutoGet ? TEXT(" (automatic)") : TEXT("")));
		for (const auto& Proficiency : C.Proficiencies) Skills.Add(FString::Printf(TEXT("%s %u (proficiency metadata, effect unavailable)"), *SkillLabel(Proficiency.L2Ref, Proficiency.SkillId.Value), Proficiency.SkillLevel.Value));
		Choice->SetToolTipText(FText::FromString((C.SkillTreePopulated ? TEXT("Skills — effects are not available yet:\n") : TEXT("Learning tree unavailable. Skill effects are not available yet.\n")) + FString::Join(Skills, TEXT("\n"))));
		if (Path.Contains(C.ClassId.Value))
		{
			Choice->SetBackgroundColor(FLinearColor(0.12f, 0.28f, 0.38f));
			if (UTextBlock* LabelText = Cast<UTextBlock>(Choice->GetChildAt(0))) LabelText->SetColorAndOpacity(FSlateColor(FLinearColor::White));
		}
		Choice->SetIsEnabled(!Classes->IsBusy());
		UNightfallClassChoiceHandler* Handler = NewObject<UNightfallClassChoiceHandler>(this);
		Handler->ClassId = C.ClassId.Value;
		Handler->Dialog = this;
		Choice->OnClicked.AddDynamic(Handler, &UNightfallClassChoiceHandler::HandleClicked);
		Handlers.Add(Handler);
	}
	const bool bSelectedEligible = SelectedTarget.IsSet() && Options.Options.ContainsByPredicate([this](const auto& O) { return O.ClassId.Value == SelectedTarget.GetValue() && O.Eligible; });
	if (ConfirmButton) ConfirmButton->SetIsEnabled(bSelectedEligible && !Classes->IsBusy());
}

void UNightfallClassDialog::Select(uint32 ClassId)
{
	UClassStateSubsystem* Classes = GetState();
	const auto* C = Classes ? Classes->FindClass(ClassId) : nullptr;
	const auto* O = Classes ? Classes->GetOptions().Options.FindByPredicate([&](const auto& Entry) { return Entry.ClassId.Value == ClassId && Entry.Eligible; }) : nullptr;
	if (!C || Classes->IsBusy()) return;
	TArray<FString> Details;
	Details.Add(C->DisplayName + (C->SkillTreePopulated ? TEXT(" — skills. Skill effects are not available yet.") : TEXT(" — learning tree unavailable. Skill effects are not available yet.")));
	for (const auto& Skill : C->SkillTree) Details.Add(FString::Printf(TEXT("%s %u: level %u, SP %llu%s%s"), *SkillLabel(Skill.L2Ref, Skill.SkillId.Value), Skill.SkillLevel.Value, Skill.RequiredLevel.Value, Skill.SpCost.Value, Skill.AutoGet ? TEXT(" automatic") : TEXT(""), Skill.LearnedByNpc ? TEXT(" trainer") : TEXT("")));
	for (const auto& P : C->Proficiencies) Details.Add(FString::Printf(TEXT("%s %u: proficiency metadata, level %u"), *SkillLabel(P.L2Ref, P.SkillId.Value), P.SkillLevel.Value, P.MinLevel.Value));
	SkillDetails->SetText(FText::FromString(FString::Join(Details, TEXT("\n"))));
	if (!O)
	{
		SelectedTarget.Reset();
		ConfirmButton->SetIsEnabled(false);
		Confirmation->SetText(FText::FromString(TEXT("This class is unavailable for transfer. Inspect its metadata or select an eligible next class.")));
		return;
	}
	SelectedTarget = ClassId;
	Confirmation->SetText(FText::FromString(FString::Printf(TEXT("Transfer permanently to %s and consume one tier %u token?"), *C->DisplayName, C->Tier.Value)));
	ConfirmButton->SetIsEnabled(true);
}

void UNightfallClassDialog::HandleConfirm()
{
	UClassStateSubsystem* Classes = GetState();
	if (!Classes || !SelectedTarget.IsSet() || Classes->IsBusy() || !Classes->HasOptions()) return;
	if (!Classes->GetOptions().Options.ContainsByPredicate([this](const auto& Option) { return Option.ClassId.Value == SelectedTarget.GetValue() && Option.Eligible; }))
	{
		SelectedTarget.Reset(); ConfirmButton->SetIsEnabled(false); return;
	}
	ConfirmButton->SetIsEnabled(false);
	Classes->Transfer(SelectedTarget.GetValue(), [Weak = TWeakObjectPtr<UNightfallClassDialog>(this)](const FNetResult& R)
	{
		if (!Weak.IsValid()) return;
		Weak->Status->SetText(FText::FromString(R.IsOk() ? FString::Printf(TEXT("Class transfer completed. %d skills unlocked. Skill effects are not available yet."), Weak->GetState()->GetLastGrantedSkillKeys().Num()) : R.Message));
		Weak->Refresh();
	});
}

void UNightfallClassDialog::HandleRefresh() { Refresh(); }
void UNightfallClassDialog::HandleClose() { RemoveFromParent(); }

UClassStateSubsystem* UNightfallClassDialog::GetState() const
{
	if (BoundState) return BoundState;
	const UGameInstance* GI = GetGameInstance();
	return GI ? GI->GetSubsystem<UClassStateSubsystem>() : nullptr;
}

void UNightfallClassDialog::BindState(UClassStateSubsystem* InState, bool bLoadOptions)
{
	if (BoundState) BoundState->OnChanged.Remove(StateHandle);
	BoundState = InState;
	if (BoundState) StateHandle = BoundState->OnChanged.AddUObject(this, &UNightfallClassDialog::Render);
	if (bLoadOptions) Refresh();
	else Render();
}

bool UNightfallClassDialog::CanConfirm() const { return ConfirmButton && ConfirmButton->GetIsEnabled(); }
FString UNightfallClassDialog::ConfirmationText() const { return Confirmation ? Confirmation->GetText().ToString() : FString(); }

FString UNightfallClassDialog::SkillDetailsText() const { return SkillDetails ? SkillDetails->GetText().ToString() : FString(); }
