#include "NightfallLoginScreen.h"
#include "Nightfall.h"
#include "Character/ClassStateSubsystem.h"
#include "Character/CharacterCreation.h"
#include "Auth/AuthSubsystem.h"
#include "Game/LoginFlowSubsystem.h"
#include "Blueprint/WidgetTree.h"
#include "Components/Border.h"
#include "Components/Button.h"
#include "Components/ComboBoxString.h"
#include "Components/EditableTextBox.h"
#include "Components/HorizontalBox.h"
#include "Components/HorizontalBoxSlot.h"
#include "Components/ProgressBar.h"
#include "Components/ScrollBox.h"
#include "Components/TextBlock.h"
#include "Components/VerticalBox.h"
#include "Components/VerticalBoxSlot.h"
#include "Engine/GameInstance.h"
#include "HAL/PlatformApplicationMisc.h"
#include "HAL/PlatformProcess.h"

#define LOCTEXT_NAMESPACE "NightfallLogin"

namespace
{

	template <typename TSubsystem>
	TSubsystem* GetSubsystemFrom(const UUserWidget* Widget)
	{
		const UGameInstance* GameInstance = Widget->GetGameInstance();
		return GameInstance != nullptr ? GameInstance->GetSubsystem<TSubsystem>() : nullptr;
	}
}

void UNightfallRaceCardHandler::HandleClicked()
{
	if (Screen.IsValid()) Screen->SelectRace(RaceId);
}

void UNightfallCharacterRowHandler::HandleClicked()
{
	if (Screen.IsValid())
	{
		Screen->SelectCharacter(CharacterId);
	}
}

void UNightfallLoginScreen::NativeOnInitialized()
{
	Super::NativeOnInitialized();
	// A Blueprint that lays out its own widgets binds at least LoginButton; otherwise (an empty
	// or placeholder tree) the screen replaces the tree with the default layout.
	EnsureLayout();

	if (LoginButton) LoginButton->OnClicked.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleLoginClicked);
	if (CopyUrlButton) CopyUrlButton->OnClicked.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleCopyUrlClicked);
	if (OpenUrlButton) OpenUrlButton->OnClicked.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleOpenUrlClicked);
	if (CreateButton) CreateButton->OnClicked.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleCreateClicked);
	if (ClassInput) ClassInput->OnSelectionChanged.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleClassSelected);
	if (RaceInput) RaceInput->OnSelectionChanged.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleRaceSelected);

	if (UAuthSubsystem* Auth = GetSubsystemFrom<UAuthSubsystem>(this))
	{
		Auth->OnLoginRequired.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleLoginRequired);
		Auth->OnLoggedIn.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleLoggedIn);
		Auth->OnLoginFailed.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleLoginFailed);
		if (Auth->IsLoggedIn())
		{
			ShowLoggedIn();
			LoadCatalogue();
			RefreshCharacters();
		}
		else
		{
			ShowLoggedOut();
			SetStatus(TEXT("Press Login to sign in with your browser."));
		}
	}
	if (ULoginFlowSubsystem* Flow = GetSubsystemFrom<ULoginFlowSubsystem>(this))
	{
		Flow->OnStatus.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleFlowStatus);
	}
}

void UNightfallLoginScreen::NativeDestruct()
{
	if (UAuthSubsystem* Auth = GetSubsystemFrom<UAuthSubsystem>(this))
	{
		Auth->OnLoginRequired.RemoveAll(this);
		Auth->OnLoggedIn.RemoveAll(this);
		Auth->OnLoginFailed.RemoveAll(this);
	}
	if (ULoginFlowSubsystem* Flow = GetSubsystemFrom<ULoginFlowSubsystem>(this))
	{
		Flow->OnStatus.RemoveAll(this);
	}
	Super::NativeDestruct();
}

TOptional<FUIInputConfig> UNightfallLoginScreen::GetDesiredInputConfig() const
{
	return FUIInputConfig(ECommonInputMode::Menu, EMouseCaptureMode::NoCapture);
}

UButton* UNightfallLoginScreen::MakeButton(UPanelWidget* Parent, const FText& Label)
{
	UButton* Button = WidgetTree->ConstructWidget<UButton>();
	UTextBlock* ButtonLabel = MakeText(nullptr, Label, 18);
	ButtonLabel->SetColorAndOpacity(FSlateColor(FLinearColor(0.015f, 0.02f, 0.03f)));
	Button->AddChild(ButtonLabel);
	if (Parent != nullptr)
	{
		Parent->AddChild(Button);
	}
	return Button;
}

UTextBlock* UNightfallLoginScreen::MakeText(UPanelWidget* Parent, const FText& Text, int32 Size)
{
	UTextBlock* Block = WidgetTree->ConstructWidget<UTextBlock>();
	Block->SetText(Text);
	FSlateFontInfo Font = Block->GetFont();
	Font.Size = Size;
	Block->SetFont(Font);
	if (Parent != nullptr)
	{
		Parent->AddChild(Block);
	}
	return Block;
}

void UNightfallLoginScreen::BuildDefaultLayout()
{
	UBorder* Root = WidgetTree->ConstructWidget<UBorder>(UBorder::StaticClass(), TEXT("Root"));
	Root->SetBrushColor(FLinearColor(0.02f, 0.02f, 0.04f, 1.f));
	Root->SetPadding(FMargin(32.f));
	WidgetTree->RootWidget = Root;

	UVerticalBox* Column = WidgetTree->ConstructWidget<UVerticalBox>();
	UScrollBox* Scroll = WidgetTree->ConstructWidget<UScrollBox>();
	Root->SetContent(Scroll);
	Scroll->AddChild(Column);
	auto Pad = [](UWidget* Widget, float Bottom)
	{
		if (UVerticalBoxSlot* BoxSlot = Cast<UVerticalBoxSlot>(Widget->Slot))
		{
			BoxSlot->SetPadding(FMargin(0.f, 0.f, 0.f, Bottom));
			BoxSlot->SetHorizontalAlignment(HAlign_Left);
		}
	};

	Pad(MakeText(Column, LOCTEXT("Title", "NIGHTFALL"), 40), 24.f);
	StatusText = MakeText(Column, FText::GetEmpty(), 18);
	Pad(StatusText, 24.f);
	LoginButton = MakeButton(Column, LOCTEXT("Login", "Login"));
	Pad(LoginButton, 24.f);

	UVerticalBox* Code = WidgetTree->ConstructWidget<UVerticalBox>();
	Column->AddChild(Code);
	Pad(Code, 24.f);
	CodePanel = Code;
	MakeText(Code, LOCTEXT("CodeHint", "Your login code:"), 18);
	UserCodeText = MakeText(Code, FText::GetEmpty(), 64);
	VerificationUriText = MakeText(Code, FText::GetEmpty(), 16);
	UHorizontalBox* UrlButtons = WidgetTree->ConstructWidget<UHorizontalBox>();
	Code->AddChild(UrlButtons);
	CopyUrlButton = MakeButton(UrlButtons, LOCTEXT("CopyUrl", "Copy verification URL"));
	OpenUrlButton = MakeButton(UrlButtons, LOCTEXT("OpenUrl", "Open in browser"));
	if (UHorizontalBoxSlot* BoxSlot = Cast<UHorizontalBoxSlot>(CopyUrlButton->Slot))
	{
		BoxSlot->SetPadding(FMargin(0.f, 8.f, 16.f, 0.f));
	}

	UVerticalBox* Characters = WidgetTree->ConstructWidget<UVerticalBox>();
	Column->AddChild(Characters);
	CharacterPanel = Characters;
	MakeText(Characters, LOCTEXT("Characters", "Characters"), 24);
	CharacterList = WidgetTree->ConstructWidget<UVerticalBox>();
	Characters->AddChild(CharacterList);
	Pad(CharacterList, 16.f);
	MakeText(Characters, LOCTEXT("CreateTitle", "Create character"), 20);
	UHorizontalBox* Form = WidgetTree->ConstructWidget<UHorizontalBox>();
	Characters->AddChild(Form);
	NameInput = WidgetTree->ConstructWidget<UEditableTextBox>();
	NameInput->SetHintText(LOCTEXT("NameHint", "Name (3-16 letters)"));
	NameInput->SetMinDesiredWidth(240.f);
	Form->AddChild(NameInput);
	RaceCards = WidgetTree->ConstructWidget<UHorizontalBox>();
	Characters->AddChild(RaceCards);
	ClassInput = WidgetTree->ConstructWidget<UComboBoxString>();
	Form->AddChild(ClassInput);
	SexInput = WidgetTree->ConstructWidget<UComboBoxString>();
	SexInput->AddOption(TEXT("Male"));
	SexInput->AddOption(TEXT("Female"));
	SexInput->SetSelectedIndex(0);
	Form->AddChild(SexInput);
	UHorizontalBox* Appearance = WidgetTree->ConstructWidget<UHorizontalBox>();
	Characters->AddChild(Appearance);
	MakeText(Appearance, LOCTEXT("Appearance", "Prototype appearance: "), 14);
	HairStyleInput = WidgetTree->ConstructWidget<UComboBoxString>();
	HairColorInput = WidgetTree->ConstructWidget<UComboBoxString>();
	FaceInput = WidgetTree->ConstructWidget<UComboBoxString>();
	Appearance->AddChild(HairStyleInput);
	Appearance->AddChild(HairColorInput);
	Appearance->AddChild(FaceInput);
	MakeText(Characters, LOCTEXT("PrototypeArt", "Existing mannequin body is shared by races and sexes during this prototype."), 12);
	TraitText = MakeText(Characters, FText::GetEmpty(), 14);
	TraitText->SetAutoWrapText(true);
	PreviewLabel = MakeText(Characters, LOCTEXT("CataloguePending", "Loading the character catalogue..."), 14);
	for (int32 I = 0; I < 6; ++I)
	{
		UHorizontalBox* Row = WidgetTree->ConstructWidget<UHorizontalBox>();
		Characters->AddChild(Row);
		StatLabels.Add(MakeText(Row, FText::GetEmpty(), 12));
		UProgressBar* Bar = WidgetTree->ConstructWidget<UProgressBar>();
		Row->AddChild(Bar);
		if (UHorizontalBoxSlot* Slot = Cast<UHorizontalBoxSlot>(Bar->Slot)) Slot->SetSize(FSlateChildSize(ESlateSizeRule::Fill));
		StatBars.Add(Bar);
	}
	CreateButton = MakeButton(Form, LOCTEXT("Create", "Create"));
	for (UWidget* Child : { static_cast<UWidget*>(NameInput), static_cast<UWidget*>(ClassInput) })
	{
		if (UHorizontalBoxSlot* BoxSlot = Cast<UHorizontalBoxSlot>(Child->Slot))
		{
			BoxSlot->SetPadding(FMargin(0.f, 0.f, 12.f, 0.f));
		}
	}
}

void UNightfallLoginScreen::SetStatus(const FString& Status)
{
	if (StatusText)
	{
		StatusText->SetText(FText::FromString(Status));
	}
}

void UNightfallLoginScreen::ShowLoggedOut()
{
	if (LoginButton) LoginButton->SetVisibility(ESlateVisibility::Visible);
	if (CodePanel) CodePanel->SetVisibility(ESlateVisibility::Collapsed);
	if (CharacterPanel) CharacterPanel->SetVisibility(ESlateVisibility::Collapsed);
}

void UNightfallLoginScreen::ShowLoggedIn()
{
	if (LoginButton) LoginButton->SetVisibility(ESlateVisibility::Collapsed);
	if (CodePanel) CodePanel->SetVisibility(ESlateVisibility::Collapsed);
	if (CharacterPanel) CharacterPanel->SetVisibility(ESlateVisibility::Visible);
}

void UNightfallLoginScreen::HandleLoginClicked()
{
	if (UAuthSubsystem* Auth = GetSubsystemFrom<UAuthSubsystem>(this))
	{
		SetStatus(TEXT("Contacting the login server..."));
		if (LoginButton) LoginButton->SetIsEnabled(false);
		Auth->StartLogin();
	}
}

void UNightfallLoginScreen::HandleLoginRequired(const FString& Code, const FString& Uri)
{
	VerificationUrl = Uri;
	if (LoginButton) LoginButton->SetVisibility(ESlateVisibility::Collapsed);
	if (UserCodeText) UserCodeText->SetText(FText::FromString(Code));
	if (VerificationUriText) VerificationUriText->SetText(FText::FromString(Uri));
	if (CodePanel) CodePanel->SetVisibility(ESlateVisibility::Visible);
	SetStatus(TEXT("Open the URL in any browser, sign in and approve. Waiting for approval..."));
}

void UNightfallLoginScreen::HandleLoggedIn()
{
	VerificationUrl.Empty();
	if (LoginButton) LoginButton->SetIsEnabled(true);
	ShowLoggedIn();
	LoadCatalogue();
	RefreshCharacters();
}

void UNightfallLoginScreen::HandleLoginFailed(const FString& Reason)
{
	VerificationUrl.Empty();
	if (LoginButton) LoginButton->SetIsEnabled(true);
	ShowLoggedOut();
	SetStatus(Reason);
}

void UNightfallLoginScreen::HandleCopyUrlClicked()
{
	if (!VerificationUrl.IsEmpty())
	{
		FPlatformApplicationMisc::ClipboardCopy(*VerificationUrl);
		SetStatus(TEXT("URL copied. Paste it into a browser, sign in and approve."));
	}
}

void UNightfallLoginScreen::HandleOpenUrlClicked()
{
	if (!VerificationUrl.IsEmpty())
	{
		FString Error;
		FPlatformProcess::LaunchURL(*VerificationUrl, nullptr, &Error);
		if (!Error.IsEmpty())
		{
			SetStatus(FString::Printf(TEXT("Could not open a browser (%s); copy the URL instead."), *Error));
		}
	}
}

void UNightfallLoginScreen::HandleFlowStatus(const FString& Status)
{
	SetStatus(Status);
}

void UNightfallLoginScreen::RefreshCharacters()
{
	ULoginFlowSubsystem* Flow = GetSubsystemFrom<ULoginFlowSubsystem>(this);
	if (Flow == nullptr)
	{
		return;
	}
	SetStatus(TEXT("Loading characters..."));
	Flow->ListCharacters([Weak = TWeakObjectPtr<UNightfallLoginScreen>(this)](const FNetResult& Result, const TArray<FGrpcNightfallV1Character>& Characters)
	{
		UNightfallLoginScreen* Self = Weak.Get();
		if (Self == nullptr)
		{
			return;
		}
		if (!Result.IsOk())
		{
			Self->SetStatus(FString::Printf(TEXT("Could not load characters: %s"), *Result.Message));
			return;
		}
		if (Self->CharacterList)
		{
			Self->CharacterList->ClearChildren();
		}
		Self->RowHandlers.Reset();
		Self->RowButtons.Reset();
		for (const FGrpcNightfallV1Character& Character : Characters)
		{
			const UClassStateSubsystem* Classes = GetSubsystemFrom<UClassStateSubsystem>(Self);
			const FGrpcNightfallV1ClassInfo* Class = Classes ? Classes->FindClass(Character.ClassId.Value) : nullptr;
			const FText Label = FText::FromString(FString::Printf(TEXT("%s  -  %s  -  level %u"),
				*Character.Name, Class ? *Class->DisplayName : TEXT("Character"), Character.Level.Value));
			UButton* Row = Self->MakeButton(Self->CharacterList, Label);
			if (UVerticalBoxSlot* BoxSlot = Cast<UVerticalBoxSlot>(Row->Slot))
			{
				BoxSlot->SetPadding(FMargin(0.f, 4.f));
				BoxSlot->SetHorizontalAlignment(HAlign_Left);
			}
			UNightfallCharacterRowHandler* Handler = NewObject<UNightfallCharacterRowHandler>(Self);
			Handler->CharacterId = Character.Id;
			Handler->Screen = Self;
			Row->OnClicked.AddDynamic(Handler, &UNightfallCharacterRowHandler::HandleClicked);
			Self->RowHandlers.Add(Handler);
			Self->RowButtons.Add(Row);
		}
		Self->SetStatus(Characters.IsEmpty()
			? FString(TEXT("No characters yet. Create one below."))
			: FString(TEXT("Pick a character to enter the world.")));
	});
}

void UNightfallLoginScreen::HandleCreateClicked()
{
	ULoginFlowSubsystem* Flow = GetSubsystemFrom<ULoginFlowSubsystem>(this);
	if (Flow == nullptr || NameInput == nullptr)
	{
		return;
	}
	const FString Name = NameInput->GetText().ToString().TrimStartAndEnd();
	UClassStateSubsystem* Classes = GetSubsystemFrom<UClassStateSubsystem>(this);
	const int32 ClassIndex = ClassInput ? ClassInput->GetSelectedIndex() : INDEX_NONE;
	if (!Classes || Classes->IsBusy() || !Classes->HasCatalogue() || !SelectedClassIds.IsValidIndex(ClassIndex)) return;
	FGrpcNightfallV1CreateCharacterRequest Request = NightfallCreation::Request(Name, SelectedRace, SelectedClassIds[ClassIndex], SexInput && SexInput->GetSelectedIndex() == 1 ? 2u : 1u,
		HairStyleInput ? HairStyleInput->GetSelectedIndex() : 0,
		HairColorInput ? HairColorInput->GetSelectedIndex() : 0,
		FaceInput ? FaceInput->GetSelectedIndex() : 0);
	if (CreateButton) CreateButton->SetIsEnabled(false);
	SetStatus(FString::Printf(TEXT("Creating %s..."), *Name));
	Classes->Create(Request, [Weak = TWeakObjectPtr<UNightfallLoginScreen>(this)](const FNetResult& Result)
	{
		UNightfallLoginScreen* Self = Weak.Get();
		if (Self == nullptr)
		{
			return;
		}
		if (Self->CreateButton) Self->CreateButton->SetIsEnabled(true);
		if (!Result.IsOk())
		{
			Self->SetStatus(FString::Printf(TEXT("Could not create the character: %s"), *Result.Message));
			return;
		}
		if (Self->NameInput) Self->NameInput->SetText(FText::GetEmpty());
		Self->RefreshCharacters();
	});
}

void UNightfallLoginScreen::SelectCharacter(const FString& CharacterId)
{
	ULoginFlowSubsystem* Flow = GetSubsystemFrom<ULoginFlowSubsystem>(this);
	if (Flow == nullptr)
	{
		return;
	}
	SetCharacterRowsEnabled(false);
	Flow->EnterWorld(CharacterId, [Weak = TWeakObjectPtr<UNightfallLoginScreen>(this)](const FNetResult& Result)
	{
		if (!Result.IsOk() && Weak.IsValid())
		{
			Weak->SetCharacterRowsEnabled(true);
		}
	});
}

void UNightfallLoginScreen::SetCharacterRowsEnabled(bool bEnabled)
{
	for (UButton* Row : RowButtons)
	{
		if (Row) Row->SetIsEnabled(bEnabled);
	}
}

void UNightfallLoginScreen::EnsureLayout()
{
	if (!WidgetTree) WidgetTree = NewObject<UWidgetTree>(this, TEXT("WidgetTree"), RF_Transient);
	if (!LoginButton) BuildDefaultLayout();
}

void UNightfallLoginScreen::LoadCatalogue()
{
	if (CreateButton) CreateButton->SetIsEnabled(false);
	if (UClassStateSubsystem* Classes = GetSubsystemFrom<UClassStateSubsystem>(this))
	{
		Classes->LoadCatalogue([Weak = TWeakObjectPtr<UNightfallLoginScreen>(this)](const FNetResult& R)
		{
			if (!Weak.IsValid()) return;
			if (R.IsOk()) Weak->ApplyCatalogue();
			else Weak->SetStatus(FString::Printf(TEXT("Could not load character choices: %s"), *R.Message));
		});
	}
}

void UNightfallLoginScreen::ApplyCatalogue()
{
	const UClassStateSubsystem* Classes = GetSubsystemFrom<UClassStateSubsystem>(this);
	if (!Classes) return;
	if (RaceCards) RaceCards->ClearChildren();
	if (RaceInput) RaceInput->ClearOptions();
	RaceCardHandlers.Reset();
	RaceCardButtons.Reset();
	CatalogueRaceIds.Reset();
	for (const auto& Race : Classes->GetCatalogue().Races)
	{
		CatalogueRaceIds.Add(static_cast<uint32>(Race.Race));
		if (RaceInput) RaceInput->AddOption(Race.DisplayName);
		if (RaceCards)
		{
			UButton* Card = MakeButton(RaceCards, FText::FromString(Race.DisplayName));
			UNightfallRaceCardHandler* Handler = NewObject<UNightfallRaceCardHandler>(this);
			Handler->RaceId = static_cast<uint32>(Race.Race);
			Handler->Screen = this;
			Card->OnClicked.AddDynamic(Handler, &UNightfallRaceCardHandler::HandleClicked);
			RaceCardHandlers.Add(Handler);
			RaceCardButtons.Add(Card);
		}
	}
	if (!CatalogueRaceIds.IsEmpty()) SelectRace(CatalogueRaceIds[0]);
}

void UNightfallLoginScreen::SelectRace(uint32 RaceId)
{
	const UClassStateSubsystem* Classes = GetSubsystemFrom<UClassStateSubsystem>(this);
	const FGrpcNightfallV1RaceInfo* Race = Classes ? Classes->FindRace(RaceId) : nullptr;
	if (!Race || !ClassInput) return;
	SelectedRace = RaceId;
	for (int32 I = 0; I < RaceCardHandlers.Num(); ++I)
	{
		if (!RaceCardButtons.IsValidIndex(I)) continue;
		const bool bSelected = RaceCardHandlers[I]->RaceId == RaceId;
		UButton* Card = RaceCardButtons[I];
		Card->SetBackgroundColor(bSelected ? FLinearColor(0.12f, 0.28f, 0.38f) : FLinearColor::White);
		if (UTextBlock* Label = Cast<UTextBlock>(Card->GetChildAt(0)))
		{
			const auto* Info = Classes->FindRace(RaceCardHandlers[I]->RaceId);
			Label->SetText(FText::FromString((bSelected ? TEXT("Selected: ") : TEXT("")) + (Info ? Info->DisplayName : FString())));
			Label->SetColorAndOpacity(FSlateColor(bSelected ? FLinearColor::White : FLinearColor(0.015f, 0.02f, 0.03f)));
		}
	}
	ClassInput->ClearOptions();
	SelectedClassIds.Reset();
	for (const FUInt32 Id : Race->BaseClassIds)
	{
		const FGrpcNightfallV1ClassInfo* C = Classes->FindClass(Id.Value);
		if (!C || C->Tier.Value != 0) continue;
		SelectedClassIds.Add(Id.Value);
		ClassInput->AddOption(C->DisplayName);
	}
	ClassInput->SetSelectedIndex(0);
	auto Fill = [](UComboBoxString* Input, uint32 Count, const FString& Label)
	{
		if (!Input) return;
		Input->ClearOptions();
		for (uint32 I = 0; I < Count; ++I) Input->AddOption(Count == 1 ? Label + TEXT(" default") : FString::Printf(TEXT("%s %u"), *Label, I + 1));
		Input->SetSelectedIndex(0);
	};
	Fill(HairStyleInput, Race->HairStyleCount.Value, TEXT("Hair"));
	Fill(HairColorInput, Race->HairColorCount.Value, TEXT("Color"));
	Fill(FaceInput, Race->FaceCount.Value, TEXT("Face"));
	TArray<FString> Traits;
	for (const auto& Passive : Race->Passives) Traits.Add(Passive.DisplayName + TEXT(": ") + Passive.Description + (Passive.Implemented ? TEXT("") : TEXT(" (planned)")));
	if (TraitText) TraitText->SetText(FText::FromString(FString::Join(Traits, TEXT("  |  "))));
	if (CreateButton) CreateButton->SetIsEnabled(!SelectedClassIds.IsEmpty() && !Classes->IsBusy());
	UpdatePreview();
}

void UNightfallLoginScreen::HandleRaceSelected(FString Selected, ESelectInfo::Type Type)
{
	if (RaceInput && CatalogueRaceIds.IsValidIndex(RaceInput->GetSelectedIndex())) SelectRace(CatalogueRaceIds[RaceInput->GetSelectedIndex()]);
}

void UNightfallLoginScreen::HandleClassSelected(FString Selected, ESelectInfo::Type Type) { UpdatePreview(); }

void UNightfallLoginScreen::UpdatePreview()
{
	const UClassStateSubsystem* Classes = GetSubsystemFrom<UClassStateSubsystem>(this);
	const int32 Index = ClassInput ? ClassInput->GetSelectedIndex() : INDEX_NONE;
	const FGrpcNightfallV1ClassInfo* C = Classes && SelectedClassIds.IsValidIndex(Index) ? Classes->FindClass(SelectedClassIds[Index]) : nullptr;
	if (!C) return;
	const uint32 Values[] = { C->BaseStats.Str.Value, C->BaseStats.Dex.Value, C->BaseStats.Con.Value, C->BaseStats.Int.Value, C->BaseStats.Wit.Value, C->BaseStats.Men.Value };
	const TCHAR* Labels[] = { TEXT("STR"), TEXT("DEX"), TEXT("CON"), TEXT("INT"), TEXT("WIT"), TEXT("MEN") };
	uint32 Total = 0;
	for (int32 I = 0; I < 6; ++I)
	{
		Total += Values[I];
		if (StatLabels.IsValidIndex(I)) StatLabels[I]->SetText(FText::FromString(FString::Printf(TEXT("%s %u  "), Labels[I], Values[I])));
		if (StatBars.IsValidIndex(I)) StatBars[I]->SetPercent(static_cast<float>(Values[I]) / 170.f);
	}
	if (PreviewLabel) PreviewLabel->SetText(FText::FromString(FString::Printf(TEXT("%s — Base stat budget %u / 170"), *C->DisplayName, Total)));
}

#undef LOCTEXT_NAMESPACE

bool UNightfallLoginScreen::CanCreate() const { return CreateButton && CreateButton->GetIsEnabled(); }
