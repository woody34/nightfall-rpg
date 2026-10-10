#pragma once

#include "CoreMinimal.h"
#include "CommonActivatableWidget.h"
#include "SNightfallV1/GameMessage.h"
#include "NightfallLoginScreen.generated.h"

class UButton;
class UComboBoxString;
class UEditableTextBox;
class UPanelWidget;
class UHorizontalBox;
class UProgressBar;
class UTextBlock;
class UVerticalBox;
class UNightfallLoginScreen;

/** Routes one character row's click to the screen (UButton::OnClicked carries no payload). */
UCLASS()
class NIGHTFALL_API UNightfallCharacterRowHandler : public UObject
{
	GENERATED_BODY()

public:
	FString CharacterId;
	TWeakObjectPtr<UNightfallLoginScreen> Screen;

	UFUNCTION()
	void HandleClicked();
};

/** Payload for a catalogue race card. */
UCLASS()
class NIGHTFALL_API UNightfallRaceCardHandler : public UObject
{
	GENERATED_BODY()
public:
	uint32 RaceId = 0;
	TWeakObjectPtr<UNightfallLoginScreen> Screen;
	UFUNCTION() void HandleClicked();
};

/**
 * Login screen (Story 6.2): Login button, the device-flow user code in large text, "Copy URL" /
 * "Open browser", a status line, and once logged in the account's characters with a
 * create-character form. Picking a character requests a play ticket and connects; the login
 * flow subsystem travels to the world map when the socket is up.
 *
 * WBP_Login (Content/UI) is a Blueprint subclass. Widgets are optional bindings: a designer can
 * lay them out in the Blueprint with these names; if the Blueprint binds no LoginButton the
 * screen builds a plain default layout in code (that is what WBP_Login does today).
 */
UCLASS(Blueprintable)
class NIGHTFALL_API UNightfallLoginScreen : public UCommonActivatableWidget
{
	GENERATED_BODY()

public:
	void SelectCharacter(const FString& CharacterId);
	void SelectRace(uint32 RaceId);
	/** Exposed for UI automation without a player. */
	void EnsureLayout();
	void ApplyCatalogue();
	bool CanCreate() const;
	int32 NumRaceCards() const { return RaceCardHandlers.Num(); }
	int32 NumClassChoices() const { return SelectedClassIds.Num(); }

protected:
	virtual void NativeOnInitialized() override;
	virtual void NativeDestruct() override;
	virtual TOptional<FUIInputConfig> GetDesiredInputConfig() const override;

	UPROPERTY(BlueprintReadOnly, meta = (BindWidgetOptional), Category = "Nightfall|Login")
	TObjectPtr<UTextBlock> StatusText;

	UPROPERTY(BlueprintReadOnly, meta = (BindWidgetOptional), Category = "Nightfall|Login")
	TObjectPtr<UButton> LoginButton;

	/** Holds UserCodeText, VerificationUriText and the copy/open buttons. */
	UPROPERTY(BlueprintReadOnly, meta = (BindWidgetOptional), Category = "Nightfall|Login")
	TObjectPtr<UPanelWidget> CodePanel;

	UPROPERTY(BlueprintReadOnly, meta = (BindWidgetOptional), Category = "Nightfall|Login")
	TObjectPtr<UTextBlock> UserCodeText;

	UPROPERTY(BlueprintReadOnly, meta = (BindWidgetOptional), Category = "Nightfall|Login")
	TObjectPtr<UTextBlock> VerificationUriText;

	UPROPERTY(BlueprintReadOnly, meta = (BindWidgetOptional), Category = "Nightfall|Login")
	TObjectPtr<UButton> CopyUrlButton;

	UPROPERTY(BlueprintReadOnly, meta = (BindWidgetOptional), Category = "Nightfall|Login")
	TObjectPtr<UButton> OpenUrlButton;

	/** Holds CharacterList and the create form; shown once logged in. */
	UPROPERTY(BlueprintReadOnly, meta = (BindWidgetOptional), Category = "Nightfall|Login")
	TObjectPtr<UPanelWidget> CharacterPanel;

	UPROPERTY(BlueprintReadOnly, meta = (BindWidgetOptional), Category = "Nightfall|Login")
	TObjectPtr<UVerticalBox> CharacterList;

	UPROPERTY(BlueprintReadOnly, meta = (BindWidgetOptional), Category = "Nightfall|Login")
	TObjectPtr<UEditableTextBox> NameInput;

	UPROPERTY(BlueprintReadOnly, meta = (BindWidgetOptional), Category = "Nightfall|Login")
	TObjectPtr<UComboBoxString> RaceInput;

	UPROPERTY(BlueprintReadOnly, meta = (BindWidgetOptional), Category = "Nightfall|Login")
	TObjectPtr<UButton> CreateButton;

private:
	void BuildDefaultLayout();
	UButton* MakeButton(UPanelWidget* Parent, const FText& Label);
	UTextBlock* MakeText(UPanelWidget* Parent, const FText& Text, int32 Size);

	void LoadCatalogue();
	void UpdatePreview();
	void SetStatus(const FString& Status);
	UFUNCTION() void HandleClassSelected(FString Selected, ESelectInfo::Type Type);
	UFUNCTION() void HandleRaceSelected(FString Selected, ESelectInfo::Type Type);
	void ShowLoggedOut();
	void ShowLoggedIn();
	void RefreshCharacters();
	void SetCharacterRowsEnabled(bool bEnabled);

	UFUNCTION()
	void HandleLoginClicked();

	UFUNCTION()
	void HandleCopyUrlClicked();

	UFUNCTION()
	void HandleOpenUrlClicked();

	UFUNCTION()
	void HandleCreateClicked();

	UFUNCTION()
	void HandleLoginRequired(const FString& Code, const FString& Uri);

	UFUNCTION()
	void HandleLoggedIn();

	UFUNCTION()
	void HandleLoginFailed(const FString& Reason);

	UFUNCTION()
	void HandleFlowStatus(const FString& Status);

	UPROPERTY()
	TArray<TObjectPtr<UNightfallCharacterRowHandler>> RowHandlers;

	UPROPERTY()
	TArray<TObjectPtr<UButton>> RowButtons;

	UPROPERTY() TObjectPtr<UHorizontalBox> RaceCards;
	UPROPERTY() TObjectPtr<UComboBoxString> ClassInput;
	UPROPERTY() TObjectPtr<UComboBoxString> SexInput;
	UPROPERTY() TObjectPtr<UComboBoxString> HairStyleInput;
	UPROPERTY() TObjectPtr<UComboBoxString> HairColorInput;
	UPROPERTY() TObjectPtr<UComboBoxString> FaceInput;
	UPROPERTY() TObjectPtr<UTextBlock> PreviewLabel;
	UPROPERTY() TObjectPtr<UTextBlock> TraitText;
	UPROPERTY() TArray<TObjectPtr<UProgressBar>> StatBars;
	UPROPERTY() TArray<TObjectPtr<UTextBlock>> StatLabels;
	UPROPERTY() TArray<TObjectPtr<UNightfallRaceCardHandler>> RaceCardHandlers;
	UPROPERTY() TArray<TObjectPtr<UButton>> RaceCardButtons;
	TArray<uint32> SelectedClassIds;
	TArray<uint32> CatalogueRaceIds;
	uint32 SelectedRace = 0;
	FString VerificationUrl;
};
