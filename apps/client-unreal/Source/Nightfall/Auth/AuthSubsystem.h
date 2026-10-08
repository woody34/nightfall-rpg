#pragma once

#include "CoreMinimal.h"
#include "GameFramework/SaveGame.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "AuthSubsystem.generated.h"

class USessionClient;

/** Where the login is. Driven only by UAuthSubsystem. */
UENUM(BlueprintType)
enum class EAuthState : uint8
{
	LoggedOut,
	/** Device code issued; the player has to approve it in a browser while we poll. */
	AwaitingApproval,
	/** Exchanging the stored refresh token. */
	Refreshing,
	LoggedIn,
};

DECLARE_DYNAMIC_MULTICAST_DELEGATE_TwoParams(FOnLoginRequired, const FString&, UserCode, const FString&, VerificationUri);
DECLARE_DYNAMIC_MULTICAST_DELEGATE(FOnLoggedIn);
DECLARE_DYNAMIC_MULTICAST_DELEGATE_OneParam(FOnLoginFailed, const FString&, Reason);

/** Persisted refresh token, sealed with NightfallAuthCrypto::Seal. See UAuthSubsystem. */
UCLASS()
class NIGHTFALL_API UNightfallAuthSaveGame : public USaveGame
{
	GENERATED_BODY()

public:
	UPROPERTY()
	int32 Version = 1;

	/** Random per-install salt; mixed with device material into the AES key. */
	UPROPERTY()
	TArray<uint8> Salt;

	/** IV || AES-256-CBC(refresh token). */
	UPROPERTY()
	TArray<uint8> SealedRefreshToken;
};

/** A form POST and its outcome. Status 0 means no HTTP response (DNS, refused, timeout). */
struct FAuthHttpResponse
{
	int32 Status = 0;
	FString Body;
};

/**
 * Seams for tests: HTTP, delayed calls, the clock and the developer token. The defaults use UE's
 * HTTP module, FTSTicker, FPlatformTime and the command line.
 */
struct FAuthEnvironment
{
	TFunction<void(const FString& Url, const FString& FormBody, TFunction<void(const FAuthHttpResponse&)> OnDone)> PostForm;
	TFunction<void(float DelaySeconds, TFunction<void()> Fn)> Schedule;
	TFunction<double()> NowSeconds;
	/** `-DevToken` / `-DevTokenFile`, or empty. Always empty in shipping builds. */
	TFunction<FString()> DevToken;
};

/**
 * OAuth 2.0 device authorization grant (RFC 8628) with PKCE S256 against Keycloak
 * (plan D1, Story 1.5). The player approves the login in any browser; the game never sees a
 * password and needs no embedded browser.
 *
 * Tokens: the access token lives in memory only and is pushed to USessionClient as the gRPC
 * bearer. The refresh token is kept in memory and persisted in save slot "NightfallAuth",
 * AES-encrypted under a key derived from FPlatformMisc device/login ids and a random per-install
 * salt. That is OBFUSCATION, not a platform keychain: it stops casual copying of the save file
 * to another machine or user, not malware running as this user. Production follow-up: libsecret
 * on Linux, DPAPI/Credential Manager on Windows, Keychain on macOS.
 *
 * Tokens and the user code are logged at Verbose at most.
 */
UCLASS()
class NIGHTFALL_API UAuthSubsystem : public UGameInstanceSubsystem
{
	GENERATED_BODY()

public:
	virtual void Initialize(FSubsystemCollectionBase& Collection) override;
	virtual void Deinitialize() override;

	/**
	 * Logs in. Order: the developer token (`-DevToken=<token>` or `-DevTokenFile=<path>` on the
	 * command line, non-shipping builds only), then the stored refresh token, then the device flow
	 * (fires OnLoginRequired with the code to show).
	 * No-op while a login is already in progress.
	 */
	UFUNCTION(BlueprintCallable, Category = "Nightfall|Auth")
	void StartLogin();

	/** Abandons a device flow in progress. */
	UFUNCTION(BlueprintCallable, Category = "Nightfall|Auth")
	void CancelLogin();

	/** Forgets all tokens, in memory and on disk. */
	UFUNCTION(BlueprintCallable, Category = "Nightfall|Auth")
	void Logout();

	/**
	 * Calls OnDone(true) once the access token is valid for at least another 60 s, refreshing it
	 * first when needed. OnDone(false) when not logged in or the refresh failed; a rejected
	 * refresh token also logs out and fires OnLoginFailed.
	 */
	void RefreshIfNeeded(TFunction<void(bool bOk)> OnDone = nullptr);

	UFUNCTION(BlueprintPure, Category = "Nightfall|Auth")
	EAuthState GetState() const { return State; }

	UFUNCTION(BlueprintPure, Category = "Nightfall|Auth")
	bool IsLoggedIn() const { return State == EAuthState::LoggedIn; }

	/** Short code the player types at the verification page. */
	UPROPERTY(BlueprintReadOnly, Category = "Nightfall|Auth")
	FString UserCode;

	/** Verification page without the code (show it with UserCode). */
	UPROPERTY(BlueprintReadOnly, Category = "Nightfall|Auth")
	FString VerificationUri;

	/** Verification page with the code filled in (open or copy this). */
	UPROPERTY(BlueprintReadOnly, Category = "Nightfall|Auth")
	FString VerificationUriComplete;

	UPROPERTY(BlueprintAssignable, Category = "Nightfall|Auth")
	FOnLoginRequired OnLoginRequired;

	UPROPERTY(BlueprintAssignable, Category = "Nightfall|Auth")
	FOnLoggedIn OnLoggedIn;

	UPROPERTY(BlueprintAssignable, Category = "Nightfall|Auth")
	FOnLoginFailed OnLoginFailed;

	/** The current access token. Treat as a secret: never log or display it. */
	const FString& GetAccessToken() const { return AccessToken; }

#if !UE_BUILD_SHIPPING
	/** Developer bypass: use Token as the access token, no IdP. Same as `-DevToken=<token>`. */
	void LoginWithDevToken(const FString& Token);
#endif

	/**
	 * The developer token from the command line: `-DevToken=<token>`, else the trimmed contents of
	 * `-DevTokenFile=<path>` (keeps the token out of `ps` and the log's command-line line).
	 * Empty when neither is given, and always in shipping builds.
	 */
	static FString GetCommandLineDevToken();

	// Test seams.
	void SetEnvironmentForTesting(FAuthEnvironment InEnv) { Env = MoveTemp(InEnv); }
	void SetSaveSlotForTesting(const FString& Slot) { SaveSlot = Slot; bLoadedSave = false; }
	double GetAccessExpiresAtForTesting() const { return AccessExpiresAt; }

	/** Seconds before expiry at which RefreshIfNeeded refreshes. */
	static constexpr double RefreshMarginSeconds = 60.0;

private:
	FString DeviceEndpoint() const;
	FString TokenEndpoint() const;

	void StartDeviceFlow();
	void HandleDeviceResponse(const FAuthHttpResponse& Response);
	void SchedulePoll();
	void Poll();
	void HandlePollResponse(const FAuthHttpResponse& Response);
	void StartRefresh(bool bFallBackToDeviceFlow);
	void HandleRefreshResponse(const FAuthHttpResponse& Response, bool bFallBackToDeviceFlow);

	/** Applies a token endpoint success body. Returns an error text or an empty string. */
	FString AcceptTokens(const FString& Body);
	void SetLoggedIn();
	void Fail(const FString& Reason);
	void ClearTokens();
	void FinishRefreshWaiters(bool bOk);

	void LoadRefreshToken();
	void SaveRefreshToken();

	FAuthEnvironment Env;

	UPROPERTY()
	TObjectPtr<USessionClient> Session;

	EAuthState State = EAuthState::LoggedOut;

	/** Bumped whenever a flow starts or is abandoned; stale HTTP replies and timers compare it. */
	uint32 FlowGeneration = 0;

	FString DeviceCode;
	FString CodeVerifier;
	float PollIntervalSeconds = 5.f;
	double DeviceCodeExpiresAt = 0.0;

	FString AccessToken;
	FString RefreshToken;
	double AccessExpiresAt = 0.0;   // NowSeconds() clock; MAX_dbl for dev tokens
	bool bDevToken = false;

	bool bRefreshInFlight = false;
	TArray<TFunction<void(bool)>> RefreshWaiters;

	FString SaveSlot = TEXT("NightfallAuth");
	bool bLoadedSave = false;
};
