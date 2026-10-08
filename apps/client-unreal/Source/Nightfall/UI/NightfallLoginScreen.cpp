#include "NightfallLoginScreen.h"
#include "Nightfall.h"
#include "Auth/AuthSubsystem.h"
#include "Game/LoginFlowSubsystem.h"
#include "Blueprint/WidgetTree.h"
#include "Components/Border.h"
#include "Components/Button.h"
#include "Components/ComboBoxString.h"
#include "Components/EditableTextBox.h"
#include "Components/HorizontalBox.h"
#include "Components/HorizontalBoxSlot.h"
#include "Components/TextBlock.h"
#include "Components/VerticalBox.h"
#include "Components/VerticalBoxSlot.h"
#include "Engine/GameInstance.h"
#include "HAL/PlatformApplicationMisc.h"
#include "HAL/PlatformProcess.h"

#define LOCTEXT_NAMESPACE "NightfallLogin"

namespace
{
	struct FRaceOption
	{
		const TCHAR* Label;
		EGrpcNightfallV1Race Race;
	};

	const FRaceOption RaceOptions[] = {
		{ TEXT("Human"), EGrpcNightfallV1Race::RACE_HUMAN },
		{ TEXT("Elf"), EGrpcNightfallV1Race::RACE_ELF },
		{ TEXT("Dark Elf"), EGrpcNightfallV1Race::RACE_DARK_ELF },
		{ TEXT("Orc"), EGrpcNightfallV1Race::RACE_ORC },
		{ TEXT("Dwarf"), EGrpcNightfallV1Race::RACE_DWARF },
	};

	FString RaceLabel(EGrpcNightfallV1Race Race)
	{
		for (const FRaceOption& Option : RaceOptions)
		{
			if (Option.Race == Race) return Option.Label;
		}
		return TEXT("?");
	}

	template <typename TSubsystem>
	TSubsystem* GetSubsystemFrom(const UUserWidget* Widget)
	{
		const UGameInstance* GameInstance = Widget->GetGameInstance();
		return GameInstance != nullptr ? GameInstance->GetSubsystem<TSubsystem>() : nullptr;
	}
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
	if (WidgetTree != nullptr && LoginButton == nullptr)
	{
		BuildDefaultLayout();
	}

	if (LoginButton) LoginButton->OnClicked.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleLoginClicked);
	if (CopyUrlButton) CopyUrlButton->OnClicked.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleCopyUrlClicked);
	if (OpenUrlButton) OpenUrlButton->OnClicked.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleOpenUrlClicked);
	if (CreateButton) CreateButton->OnClicked.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleCreateClicked);
	if (RaceInput && RaceInput->GetOptionCount() == 0)
	{
		for (const FRaceOption& Option : RaceOptions)
		{
			RaceInput->AddOption(Option.Label);
		}
		RaceInput->SetSelectedIndex(0);
	}

	if (UAuthSubsystem* Auth = GetSubsystemFrom<UAuthSubsystem>(this))
	{
		Auth->OnLoginRequired.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleLoginRequired);
		Auth->OnLoggedIn.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleLoggedIn);
		Auth->OnLoginFailed.AddUniqueDynamic(this, &UNightfallLoginScreen::HandleLoginFailed);
		if (Auth->IsLoggedIn())
		{
			ShowLoggedIn();
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
	Button->AddChild(MakeText(nullptr, Label, 18));
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
	Root->SetPadding(FMargin(64.f));
	WidgetTree->RootWidget = Root;

	UVerticalBox* Column = WidgetTree->ConstructWidget<UVerticalBox>();
	Root->SetContent(Column);
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
	RaceInput = WidgetTree->ConstructWidget<UComboBoxString>();
	Form->AddChild(RaceInput);
	CreateButton = MakeButton(Form, LOCTEXT("Create", "Create"));
	for (UWidget* Child : { static_cast<UWidget*>(NameInput), static_cast<UWidget*>(RaceInput) })
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
			const FText Label = FText::FromString(FString::Printf(TEXT("%s  -  %s  -  level %u"),
				*Character.Name, *RaceLabel(Character.Race), Character.Level.Value));
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
	const int32 RaceIndex = RaceInput ? RaceInput->GetSelectedIndex() : 0;
	const EGrpcNightfallV1Race Race = RaceOptions[FMath::Clamp(RaceIndex, 0, int32(UE_ARRAY_COUNT(RaceOptions)) - 1)].Race;
	if (CreateButton) CreateButton->SetIsEnabled(false);
	SetStatus(FString::Printf(TEXT("Creating %s..."), *Name));
	Flow->CreateCharacter(Name, Race, [Weak = TWeakObjectPtr<UNightfallLoginScreen>(this)](const FNetResult& Result, const FGrpcNightfallV1Character& Character)
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

#undef LOCTEXT_NAMESPACE
