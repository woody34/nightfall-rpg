#include "AuthSubsystem.h"
#include "AuthCrypto.h"
#include "Nightfall.h"
#include "Net/NetSettings.h"
#include "Net/SessionClientSubsystem.h"
#include "Containers/Ticker.h"
#include "Dom/JsonObject.h"
#include "GenericPlatform/GenericPlatformHttp.h"
#include "HttpModule.h"
#include "Interfaces/IHttpRequest.h"
#include "Interfaces/IHttpResponse.h"
#include "Kismet/GameplayStatics.h"
#include "Misc/CommandLine.h"
#include "Misc/FileHelper.h"
#include "Misc/Parse.h"
#include "Serialization/JsonReader.h"
#include "Serialization/JsonSerializer.h"

namespace
{
	constexpr int32 SaveUserIndex = 0;
	const TCHAR* const DeviceGrantType = TEXT("urn:ietf:params:oauth:grant-type:device_code");

	FString FormEncode(std::initializer_list<TPair<const TCHAR*, FString>> Fields)
	{
		FString Out;
		for (const TPair<const TCHAR*, FString>& Field : Fields)
		{
			if (!Out.IsEmpty())
			{
				Out.AppendChar(TEXT('&'));
			}
			Out += FGenericPlatformHttp::UrlEncode(Field.Key);
			Out.AppendChar(TEXT('='));
			Out += FGenericPlatformHttp::UrlEncode(Field.Value);
		}
		return Out;
	}

	TSharedPtr<FJsonObject> ParseJson(const FString& Body)
	{
		TSharedPtr<FJsonObject> Object;
		const TSharedRef<TJsonReader<>> Reader = TJsonReaderFactory<>::Create(Body);
		if (!FJsonSerializer::Deserialize(Reader, Object))
		{
			return nullptr;
		}
		return Object;
	}

	/** A string field, or empty when the object or field is missing. */
	FString JsonString(const TSharedPtr<FJsonObject>& Object, const TCHAR* Field)
	{
		FString Value;
		if (Object.IsValid())
		{
			Object->TryGetStringField(Field, Value);
		}
		return Value;
	}

	/** True when the JWT's `aud` (string or array) contains Audience. False for a non-JWT. */
	bool JwtHasAudience(const FString& Jwt, const FString& Audience)
	{
		TArray<FString> Parts;
		Jwt.ParseIntoArray(Parts, TEXT("."), /*InCullEmpty=*/false);
		TArray<uint8> PayloadBytes;
		if (Parts.Num() != 3 || !NightfallAuthCrypto::Base64UrlDecode(Parts[1], PayloadBytes))
		{
			return false;
		}
		const FString Payload(FUTF8ToTCHAR(reinterpret_cast<const ANSICHAR*>(PayloadBytes.GetData()), PayloadBytes.Num()));
		const TSharedPtr<FJsonObject> Claims = ParseJson(Payload);
		if (!Claims.IsValid())
		{
			return false;
		}
		FString Single;
		if (Claims->TryGetStringField(TEXT("aud"), Single))
		{
			return Single == Audience;
		}
		const TArray<TSharedPtr<FJsonValue>>* Many = nullptr;
		if (Claims->TryGetArrayField(TEXT("aud"), Many))
		{
			for (const TSharedPtr<FJsonValue>& Value : *Many)
			{
				if (Value.IsValid() && Value->AsString() == Audience)
				{
					return true;
				}
			}
		}
		return false;
	}

	FAuthEnvironment MakeDefaultEnvironment()
	{
		FAuthEnvironment Env;
		Env.PostForm = [](const FString& Url, const FString& FormBody, TFunction<void(const FAuthHttpResponse&)> OnDone)
		{
			const TSharedRef<IHttpRequest, ESPMode::ThreadSafe> Request = FHttpModule::Get().CreateRequest();
			Request->SetURL(Url);
			Request->SetVerb(TEXT("POST"));
			Request->SetHeader(TEXT("Content-Type"), TEXT("application/x-www-form-urlencoded"));
			Request->SetHeader(TEXT("Accept"), TEXT("application/json"));
			Request->SetContentAsString(FormBody);
			Request->SetTimeout(15.f);
			Request->OnProcessRequestComplete().BindLambda(
				[OnDone = MoveTemp(OnDone)](FHttpRequestPtr, FHttpResponsePtr HttpResponse, bool bConnected)
				{
					FAuthHttpResponse Response;
					if (bConnected && HttpResponse.IsValid())
					{
						Response.Status = HttpResponse->GetResponseCode();
						Response.Body = HttpResponse->GetContentAsString();
					}
					OnDone(Response);
				});
			Request->ProcessRequest();
		};
		Env.Schedule = [](float DelaySeconds, TFunction<void()> Fn)
		{
			FTSTicker::GetCoreTicker().AddTicker(FTickerDelegate::CreateLambda([Fn = MoveTemp(Fn)](float)
			{
				Fn();
				return false;   // one shot
			}), DelaySeconds);
		};
		Env.NowSeconds = []() { return FPlatformTime::Seconds(); };
		Env.DevToken = []() { return UAuthSubsystem::GetCommandLineDevToken(); };
		return Env;
	}
}

void UAuthSubsystem::Initialize(FSubsystemCollectionBase& Collection)
{
	Super::Initialize(Collection);
	Session = Collection.InitializeDependency<USessionClient>();
	Env = MakeDefaultEnvironment();
}

void UAuthSubsystem::Deinitialize()
{
	++FlowGeneration;   // pending HTTP replies and timers become no-ops
	RefreshWaiters.Empty();
	Session = nullptr;
	Super::Deinitialize();
}

FString UAuthSubsystem::DeviceEndpoint() const
{
	return GetDefault<UNetSettings>()->OidcIssuer + TEXT("/protocol/openid-connect/auth/device");
}

FString UAuthSubsystem::TokenEndpoint() const
{
	return GetDefault<UNetSettings>()->OidcIssuer + TEXT("/protocol/openid-connect/token");
}

void UAuthSubsystem::StartLogin()
{
	if (State == EAuthState::AwaitingApproval || State == EAuthState::Refreshing)
	{
		return;
	}
#if !UE_BUILD_SHIPPING
	const FString DevToken = Env.DevToken ? Env.DevToken() : FString();
	if (!DevToken.IsEmpty())
	{
		LoginWithDevToken(DevToken);
		return;
	}
#endif
	if (State == EAuthState::LoggedIn)
	{
		OnLoggedIn.Broadcast();
		return;
	}
	LoadRefreshToken();
	if (!RefreshToken.IsEmpty())
	{
		StartRefresh(/*bFallBackToDeviceFlow=*/true);
	}
	else
	{
		StartDeviceFlow();
	}
}

void UAuthSubsystem::CancelLogin()
{
	if (State != EAuthState::AwaitingApproval)
	{
		return;
	}
	++FlowGeneration;
	State = EAuthState::LoggedOut;
	DeviceCode.Empty();
	CodeVerifier.Empty();
	UserCode.Empty();
	VerificationUri.Empty();
	VerificationUriComplete.Empty();
}

void UAuthSubsystem::Logout()
{
	++FlowGeneration;
	ClearTokens();
	State = EAuthState::LoggedOut;
	UE_LOG(LogNightfall, Log, TEXT("Auth: logged out"));
	FinishRefreshWaiters(false);
}

#if !UE_BUILD_SHIPPING
void UAuthSubsystem::LoginWithDevToken(const FString& Token)
{
	++FlowGeneration;
	UE_LOG(LogNightfall, Warning, TEXT("Auth: using a developer token (-DevToken); the identity provider is bypassed"));
	AccessToken = Token;
	RefreshToken.Empty();   // never persisted
	AccessExpiresAt = TNumericLimits<double>::Max();
	bDevToken = true;
	if (Session != nullptr)
	{
		Session->SetBearerToken(AccessToken);
	}
	SetLoggedIn();
}
#endif

FString UAuthSubsystem::GetCommandLineDevToken()
{
#if UE_BUILD_SHIPPING
	return FString();
#else
	FString Token;
	if (FParse::Value(FCommandLine::Get(), TEXT("DevToken="), Token) && !Token.IsEmpty())
	{
		return Token;
	}
	FString Path;
	if (FParse::Value(FCommandLine::Get(), TEXT("DevTokenFile="), Path) && FFileHelper::LoadFileToString(Token, *Path))
	{
		return Token.TrimStartAndEnd();
	}
	return FString();
#endif
}

void UAuthSubsystem::StartDeviceFlow()
{
	const uint32 Gen = ++FlowGeneration;
	State = EAuthState::AwaitingApproval;
	CodeVerifier = NightfallAuthCrypto::NewPkceVerifier();
	PollIntervalSeconds = 5.f;

	const UNetSettings* Settings = GetDefault<UNetSettings>();
	const FString Body = FormEncode({
		{ TEXT("client_id"), Settings->OidcClientId },
		{ TEXT("scope"), Settings->OidcScope },
		{ TEXT("code_challenge"), NightfallAuthCrypto::PkceChallenge(CodeVerifier) },
		{ TEXT("code_challenge_method"), TEXT("S256") },
	});
	UE_LOG(LogNightfall, Log, TEXT("Auth: starting device login at %s"), *Settings->OidcIssuer);
	Env.PostForm(DeviceEndpoint(), Body, [Weak = TWeakObjectPtr<UAuthSubsystem>(this), Gen](const FAuthHttpResponse& Response)
	{
		if (Weak.IsValid() && Weak->FlowGeneration == Gen)
		{
			Weak->HandleDeviceResponse(Response);
		}
	});
}

void UAuthSubsystem::HandleDeviceResponse(const FAuthHttpResponse& Response)
{
	const TSharedPtr<FJsonObject> Json = ParseJson(Response.Body);
	if (Response.Status == 0)
	{
		Fail(TEXT("Could not reach the login server."));
		return;
	}
	if (Response.Status != 200 || !Json.IsValid() || !Json->TryGetStringField(TEXT("device_code"), DeviceCode))
	{
		const FString Error = JsonString(Json, TEXT("error_description"));
		Fail(FString::Printf(TEXT("The login server refused to start a login (HTTP %d) %s"), Response.Status, *Error).TrimEnd());
		return;
	}
	UserCode = JsonString(Json, TEXT("user_code"));
	VerificationUri = JsonString(Json, TEXT("verification_uri"));
	VerificationUriComplete = JsonString(Json, TEXT("verification_uri_complete"));
	int32 ExpiresIn = 600;
	int32 Interval = 5;
	Json->TryGetNumberField(TEXT("expires_in"), ExpiresIn);
	Json->TryGetNumberField(TEXT("interval"), Interval);
	PollIntervalSeconds = static_cast<float>(FMath::Max(1, Interval));
	DeviceCodeExpiresAt = Env.NowSeconds() + ExpiresIn;

	UE_LOG(LogNightfall, Log, TEXT("Auth: waiting for approval at %s (code valid %d s, polling every %.0f s)"),
		*VerificationUri, ExpiresIn, PollIntervalSeconds);
	UE_LOG(LogNightfall, Verbose, TEXT("Auth: user code %s"), *UserCode);
	OnLoginRequired.Broadcast(UserCode, VerificationUriComplete.IsEmpty() ? VerificationUri : VerificationUriComplete);
	SchedulePoll();
}

void UAuthSubsystem::SchedulePoll()
{
	Env.Schedule(PollIntervalSeconds, [Weak = TWeakObjectPtr<UAuthSubsystem>(this), Gen = FlowGeneration]()
	{
		if (Weak.IsValid() && Weak->FlowGeneration == Gen)
		{
			Weak->Poll();
		}
	});
}

void UAuthSubsystem::Poll()
{
	if (Env.NowSeconds() >= DeviceCodeExpiresAt)
	{
		Fail(TEXT("The login code expired. Press Login to get a new one."));
		return;
	}
	const FString Body = FormEncode({
		{ TEXT("grant_type"), DeviceGrantType },
		{ TEXT("client_id"), GetDefault<UNetSettings>()->OidcClientId },
		{ TEXT("device_code"), DeviceCode },
		{ TEXT("code_verifier"), CodeVerifier },
	});
	Env.PostForm(TokenEndpoint(), Body, [Weak = TWeakObjectPtr<UAuthSubsystem>(this), Gen = FlowGeneration](const FAuthHttpResponse& Response)
	{
		if (Weak.IsValid() && Weak->FlowGeneration == Gen)
		{
			Weak->HandlePollResponse(Response);
		}
	});
}

void UAuthSubsystem::HandlePollResponse(const FAuthHttpResponse& Response)
{
	if (Response.Status == 200)
	{
		const FString Error = AcceptTokens(Response.Body);
		if (Error.IsEmpty())
		{
			SetLoggedIn();
		}
		else
		{
			Fail(Error);
		}
		return;
	}
	if (Response.Status == 0 || Response.Status >= 500)
	{
		// Transient: keep polling until the device code itself expires.
		UE_LOG(LogNightfall, Warning, TEXT("Auth: token endpoint unavailable (HTTP %d); retrying"), Response.Status);
		SchedulePoll();
		return;
	}

	const TSharedPtr<FJsonObject> Json = ParseJson(Response.Body);
	const FString Error = JsonString(Json, TEXT("error"));
	if (Error == TEXT("authorization_pending"))
	{
		SchedulePoll();
	}
	else if (Error == TEXT("slow_down"))
	{
		PollIntervalSeconds += 5.f;   // RFC 8628 §3.5
		UE_LOG(LogNightfall, Log, TEXT("Auth: slow_down; polling every %.0f s"), PollIntervalSeconds);
		SchedulePoll();
	}
	else if (Error == TEXT("expired_token"))
	{
		Fail(TEXT("The login code expired. Press Login to get a new one."));
	}
	else if (Error == TEXT("access_denied"))
	{
		Fail(TEXT("The login was declined in the browser."));
	}
	else
	{
		const FString Description = JsonString(Json, TEXT("error_description"));
		Fail(FString::Printf(TEXT("Login failed (HTTP %d): %s %s"), Response.Status, *Error, *Description).TrimEnd());
	}
}

void UAuthSubsystem::RefreshIfNeeded(TFunction<void(bool bOk)> OnDone)
{
	if (State == EAuthState::LoggedIn && (bDevToken || Env.NowSeconds() < AccessExpiresAt - RefreshMarginSeconds))
	{
		if (OnDone) OnDone(true);
		return;
	}
	if (State == EAuthState::LoggedIn || State == EAuthState::Refreshing)
	{
		if (OnDone) RefreshWaiters.Add(MoveTemp(OnDone));
		if (!bRefreshInFlight)
		{
			StartRefresh(/*bFallBackToDeviceFlow=*/false);
		}
		return;
	}
	if (OnDone) OnDone(false);
}

void UAuthSubsystem::StartRefresh(bool bFallBackToDeviceFlow)
{
	if (RefreshToken.IsEmpty())
	{
		if (bFallBackToDeviceFlow)
		{
			StartDeviceFlow();
		}
		else
		{
			ClearTokens();
			Fail(TEXT("Your session expired. Log in again."));
		}
		return;
	}
	const uint32 Gen = ++FlowGeneration;
	if (State != EAuthState::LoggedIn)
	{
		State = EAuthState::Refreshing;
	}
	bRefreshInFlight = true;
	const FString Body = FormEncode({
		{ TEXT("grant_type"), TEXT("refresh_token") },
		{ TEXT("client_id"), GetDefault<UNetSettings>()->OidcClientId },
		{ TEXT("refresh_token"), RefreshToken },
	});
	UE_LOG(LogNightfall, Log, TEXT("Auth: refreshing the access token"));
	Env.PostForm(TokenEndpoint(), Body, [Weak = TWeakObjectPtr<UAuthSubsystem>(this), Gen, bFallBackToDeviceFlow](const FAuthHttpResponse& Response)
	{
		if (Weak.IsValid() && Weak->FlowGeneration == Gen)
		{
			Weak->HandleRefreshResponse(Response, bFallBackToDeviceFlow);
		}
	});
}

void UAuthSubsystem::HandleRefreshResponse(const FAuthHttpResponse& Response, bool bFallBackToDeviceFlow)
{
	bRefreshInFlight = false;
	if (Response.Status == 200)
	{
		const FString Error = AcceptTokens(Response.Body);
		if (Error.IsEmpty())
		{
			SetLoggedIn();
			FinishRefreshWaiters(true);
			return;
		}
		UE_LOG(LogNightfall, Warning, TEXT("Auth: refresh returned unusable tokens: %s"), *Error);
	}
	else if (Response.Status == 0 || Response.Status >= 500)
	{
		// Transient. Keep the refresh token (memory and disk) for the next attempt.
		UE_LOG(LogNightfall, Warning, TEXT("Auth: refresh failed, login server unavailable (HTTP %d)"), Response.Status);
		if (State == EAuthState::LoggedIn)
		{
			FinishRefreshWaiters(false);
		}
		else
		{
			Fail(TEXT("Could not reach the login server."));
		}
		return;
	}

	// The IdP rejected the refresh token (invalid_grant: revoked, expired, or session ended).
	UE_LOG(LogNightfall, Log, TEXT("Auth: stored login no longer valid (HTTP %d)"), Response.Status);
	ClearTokens();
	if (bFallBackToDeviceFlow)
	{
		FinishRefreshWaiters(false);
		StartDeviceFlow();
	}
	else
	{
		Fail(TEXT("Your session expired. Log in again."));
	}
}

FString UAuthSubsystem::AcceptTokens(const FString& Body)
{
	const TSharedPtr<FJsonObject> Json = ParseJson(Body);
	FString NewAccess;
	if (!Json.IsValid() || !Json->TryGetStringField(TEXT("access_token"), NewAccess) || NewAccess.IsEmpty())
	{
		return TEXT("The login server returned no access token.");
	}
	const FString& Audience = GetDefault<UNetSettings>()->OidcAudience;
	if (!Audience.IsEmpty() && !JwtHasAudience(NewAccess, Audience))
	{
		return FString::Printf(TEXT("The access token is not valid for %s; check OidcAudience and the realm's audience mapper."), *Audience);
	}
	int32 ExpiresIn = 300;
	Json->TryGetNumberField(TEXT("expires_in"), ExpiresIn);
	FString NewRefresh;
	if (Json->TryGetStringField(TEXT("refresh_token"), NewRefresh) && !NewRefresh.IsEmpty())
	{
		RefreshToken = NewRefresh;   // Keycloak rotates refresh tokens
	}

	AccessToken = NewAccess;
	AccessExpiresAt = Env.NowSeconds() + ExpiresIn;
	bDevToken = false;
	UE_LOG(LogNightfall, Verbose, TEXT("Auth: access token %s"), *AccessToken);
	if (Session != nullptr)
	{
		Session->SetBearerToken(AccessToken);
	}
	SaveRefreshToken();

	// Refresh proactively so calls never wait on it. RefreshIfNeeded still covers sleep/suspend.
	const float RefreshIn = static_cast<float>(FMath::Max(1.0, ExpiresIn - RefreshMarginSeconds + 1.0));
	Env.Schedule(RefreshIn, [Weak = TWeakObjectPtr<UAuthSubsystem>(this), Gen = FlowGeneration]()
	{
		if (Weak.IsValid() && Weak->FlowGeneration == Gen)
		{
			Weak->RefreshIfNeeded();
		}
	});
	return FString();
}

void UAuthSubsystem::SetLoggedIn()
{
	const bool bWasLoggedIn = State == EAuthState::LoggedIn;
	State = EAuthState::LoggedIn;
	DeviceCode.Empty();
	CodeVerifier.Empty();
	UserCode.Empty();
	VerificationUri.Empty();
	VerificationUriComplete.Empty();
	if (!bWasLoggedIn)
	{
		UE_LOG(LogNightfall, Log, TEXT("Auth: logged in"));
		OnLoggedIn.Broadcast();
	}
}

void UAuthSubsystem::Fail(const FString& Reason)
{
	++FlowGeneration;
	bRefreshInFlight = false;
	if (State != EAuthState::LoggedIn)
	{
		State = EAuthState::LoggedOut;
	}
	DeviceCode.Empty();
	CodeVerifier.Empty();
	UserCode.Empty();
	VerificationUri.Empty();
	VerificationUriComplete.Empty();
	UE_LOG(LogNightfall, Warning, TEXT("Auth: %s"), *Reason);
	OnLoginFailed.Broadcast(Reason);
	FinishRefreshWaiters(false);
}

void UAuthSubsystem::ClearTokens()
{
	AccessToken.Empty();
	RefreshToken.Empty();
	AccessExpiresAt = 0.0;
	bDevToken = false;
	if (State == EAuthState::LoggedIn)
	{
		State = EAuthState::LoggedOut;
	}
	if (Session != nullptr)
	{
		Session->SetBearerToken(FString());
	}
	SaveRefreshToken();
}

void UAuthSubsystem::FinishRefreshWaiters(bool bOk)
{
	TArray<TFunction<void(bool)>> Waiters = MoveTemp(RefreshWaiters);
	RefreshWaiters.Reset();
	for (TFunction<void(bool)>& Waiter : Waiters)
	{
		Waiter(bOk);
	}
}

void UAuthSubsystem::LoadRefreshToken()
{
	if (bLoadedSave)
	{
		return;
	}
	bLoadedSave = true;
	if (!UGameplayStatics::DoesSaveGameExist(SaveSlot, SaveUserIndex))
	{
		return;
	}
	const UNightfallAuthSaveGame* Save = Cast<UNightfallAuthSaveGame>(UGameplayStatics::LoadGameFromSlot(SaveSlot, SaveUserIndex));
	FString Plain;
	if (Save != nullptr && NightfallAuthCrypto::Open(Save->SealedRefreshToken, NightfallAuthCrypto::LocalDeviceMaterial(), Save->Salt, Plain))
	{
		RefreshToken = Plain;
		UE_LOG(LogNightfall, Log, TEXT("Auth: found a stored login"));
	}
	else
	{
		UE_LOG(LogNightfall, Warning, TEXT("Auth: stored login unreadable (other machine or user?); discarding it"));
		UGameplayStatics::DeleteGameInSlot(SaveSlot, SaveUserIndex);
	}
}

void UAuthSubsystem::SaveRefreshToken()
{
	bLoadedSave = true;   // memory is now the source of truth
	if (RefreshToken.IsEmpty())
	{
		if (UGameplayStatics::DoesSaveGameExist(SaveSlot, SaveUserIndex))
		{
			UGameplayStatics::DeleteGameInSlot(SaveSlot, SaveUserIndex);
		}
		return;
	}
	UNightfallAuthSaveGame* Save = NewObject<UNightfallAuthSaveGame>();
	Save->Salt = NightfallAuthCrypto::RandomBytes(16);
	Save->SealedRefreshToken = NightfallAuthCrypto::Seal(RefreshToken, NightfallAuthCrypto::LocalDeviceMaterial(), Save->Salt);
	if (!UGameplayStatics::SaveGameToSlot(Save, SaveSlot, SaveUserIndex))
	{
		UE_LOG(LogNightfall, Warning, TEXT("Auth: could not save the login; you will have to log in again next time"));
	}
}
