#include "Misc/AutomationTest.h"
#include "AuthTestListener.h"
#include "TestGameInstance.h"
#include "Auth/AuthCrypto.h"
#include "Auth/AuthSubsystem.h"
#include "Net/NetSettings.h"
#include "Net/SessionClientSubsystem.h"
#include "GenericPlatform/GenericPlatformHttp.h"
#include "Kismet/GameplayStatics.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"

#if WITH_DEV_AUTOMATION_TESTS

// Unit-level: the device-flow state machine against a scripted identity provider. No network,
// no real timers: every HTTP call and every delay goes through FMockIdp.

namespace
{
	constexpr EAutomationTestFlags AuthTestFlags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;
	const TCHAR* const TestSlot = TEXT("NightfallAuthTest");

	FString Hex(const TArray<uint8>& Bytes)
	{
		return BytesToHex(Bytes.GetData(), Bytes.Num()).ToLower();
	}

	/** Parses an application/x-www-form-urlencoded body. */
	TMap<FString, FString> ParseForm(const FString& Body)
	{
		TMap<FString, FString> Out;
		TArray<FString> Pairs;
		Body.ParseIntoArray(Pairs, TEXT("&"));
		for (const FString& Pair : Pairs)
		{
			FString Key, Value;
			if (Pair.Split(TEXT("="), &Key, &Value))
			{
				Out.Add(FGenericPlatformHttp::UrlDecode(Key), FGenericPlatformHttp::UrlDecode(Value));
			}
		}
		return Out;
	}

	/** An unsigned JWT-shaped token with the given audience. The client only reads `aud`. */
	FString FakeJwt(const FString& Subject, const FString& Audience = TEXT("nightfall-api"))
	{
		auto Part = [](const FString& Json)
		{
			const FTCHARToUTF8 Utf8(*Json);
			return NightfallAuthCrypto::Base64UrlEncode(TArray<uint8>(reinterpret_cast<const uint8*>(Utf8.Get()), Utf8.Length()));
		};
		return Part(TEXT("{\"alg\":\"RS256\",\"typ\":\"JWT\"}")) + TEXT(".")
			+ Part(FString::Printf(TEXT("{\"sub\":\"%s\",\"aud\":[\"%s\",\"account\"]}"), *Subject, *Audience)) + TEXT(".sig");
	}

	FString TokenBody(const FString& Access, const FString& Refresh, int32 ExpiresIn = 900)
	{
		return FString::Printf(TEXT("{\"access_token\":\"%s\",\"refresh_token\":\"%s\",\"expires_in\":%d,\"token_type\":\"Bearer\"}"),
			*Access, *Refresh, ExpiresIn);
	}

	const TCHAR* const DeviceBody = TEXT("{\"device_code\":\"dc-1\",\"user_code\":\"WXYZ-ABCD\",\"verification_uri\":\"http://idp.test/device\","
		"\"verification_uri_complete\":\"http://idp.test/device?user_code=WXYZ-ABCD\",\"expires_in\":600,\"interval\":5}");

	FString OAuthError(const TCHAR* Error)
	{
		return FString::Printf(TEXT("{\"error\":\"%s\"}"), Error);
	}

	/** Scripted IdP + manual clock. */
	struct FMockIdp
	{
		struct FCall
		{
			FString Url;
			TMap<FString, FString> Form;
			TFunction<void(const FAuthHttpResponse&)> OnDone;
		};

		TArray<FCall> Calls;
		TArray<TPair<float, TFunction<void()>>> Timers;
		double Now = 1000.0;

		FAuthEnvironment Environment()
		{
			FAuthEnvironment Env;
			Env.PostForm = [this](const FString& Url, const FString& Body, TFunction<void(const FAuthHttpResponse&)> OnDone)
			{
				Calls.Add({ Url, ParseForm(Body), MoveTemp(OnDone) });
			};
			Env.Schedule = [this](float Delay, TFunction<void()> Fn) { Timers.Add({ Delay, MoveTemp(Fn) }); };
			Env.NowSeconds = [this]() { return Now; };
			Env.DevToken = []() { return FString(); };   // immune to -DevToken on the test command line
			return Env;
		}

		/** Answers the oldest pending call. */
		void Reply(int32 Status, const FString& Body)
		{
			if (Calls.IsEmpty())
			{
				UE_LOG(LogTemp, Error, TEXT("FMockIdp: reply %d with no pending request"), Status);   // fails the test
				return;
			}
			FCall Call = MoveTemp(Calls[0]);
			Calls.RemoveAt(0);
			Call.OnDone({ Status, Body });
		}

		/** Advances the clock by the oldest timer's delay and fires it. Returns the delay, or -1. */
		float RunNextTimer()
		{
			if (Timers.IsEmpty())
			{
				return -1.f;
			}
			TPair<float, TFunction<void()>> Timer = MoveTemp(Timers[0]);
			Timers.RemoveAt(0);
			Now += Timer.Key;
			Timer.Value();
			return Timer.Key;
		}
	};

	/** A game instance whose UAuthSubsystem talks to Idp and saves to the test slot. */
	struct FAuthFixture
	{
		FScopedTestGameInstance Instance;
		UAuthSubsystem* Auth = nullptr;
		USessionClient* Session = nullptr;
		UNightfallAuthTestListener* Listener = nullptr;

		explicit FAuthFixture(FMockIdp& Idp)
		{
			Auth = Instance.Get<UAuthSubsystem>();
			Session = Instance.Get<USessionClient>();
			Auth->SetEnvironmentForTesting(Idp.Environment());
			Auth->SetSaveSlotForTesting(TestSlot);
			Listener = NewObject<UNightfallAuthTestListener>(Instance.GameInstance);
			Auth->OnLoginRequired.AddDynamic(Listener, &UNightfallAuthTestListener::HandleLoginRequired);
			Auth->OnLoggedIn.AddDynamic(Listener, &UNightfallAuthTestListener::HandleLoggedIn);
			Auth->OnLoginFailed.AddDynamic(Listener, &UNightfallAuthTestListener::HandleLoginFailed);
		}
	};

	void DeleteTestSlot()
	{
		if (UGameplayStatics::DoesSaveGameExist(TestSlot, 0))
		{
			UGameplayStatics::DeleteGameInSlot(TestSlot, 0);
		}
	}

	/** Drives StartLogin -> device code -> one pending poll -> tokens. */
	void LoginThroughDeviceFlow(FMockIdp& Idp, UAuthSubsystem* Auth, const FString& Access, const FString& Refresh)
	{
		Auth->StartLogin();
		Idp.Reply(200, DeviceBody);
		Idp.RunNextTimer();
		Idp.Reply(200, TokenBody(Access, Refresh));
	}
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FAuthCryptoTest, "Nightfall.Auth.Crypto", AuthTestFlags)

bool FAuthCryptoTest::RunTest(const FString& Parameters)
{
	using namespace NightfallAuthCrypto;
	// FIPS 180-4 examples, including the two-block padding case (56 bytes).
	TestEqual(TEXT("sha256('')"), Hex(Sha256(FString())), FString(TEXT("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")));
	TestEqual(TEXT("sha256('abc')"), Hex(Sha256(FString(TEXT("abc")))), FString(TEXT("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")));
	TestEqual(TEXT("sha256(448-bit message)"), Hex(Sha256(FString(TEXT("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")))),
		FString(TEXT("248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1")));
	TArray<uint8> Million;
	Million.Init('a', 1'000'000);
	TestEqual(TEXT("sha256(1M x 'a')"), Hex(Sha256(Million.GetData(), Million.Num())),
		FString(TEXT("cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0")));

	// RFC 7636 appendix B.
	TestEqual(TEXT("PKCE S256 challenge"), PkceChallenge(TEXT("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk")),
		FString(TEXT("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")));
	const FString Verifier = NewPkceVerifier();
	TestEqual(TEXT("verifier length"), Verifier.Len(), 43);
	TestFalse(TEXT("verifier is base64url without padding"), Verifier.Contains(TEXT("=")) || Verifier.Contains(TEXT("+")) || Verifier.Contains(TEXT("/")));
	TestNotEqual(TEXT("verifiers differ"), Verifier, NewPkceVerifier());

	const FString Secret = TEXT("refresh-token-sentinel-0123456789abcdef");
	const TArray<uint8> Salt = RandomBytes(16);
	const TArray<uint8> Sealed = Seal(Secret, TEXT("device-a"), Salt);
	FString Opened;
	TestTrue(TEXT("round trip opens"), Open(Sealed, TEXT("device-a"), Salt, Opened));
	TestEqual(TEXT("round trip value"), Opened, Secret);
	TestFalse(TEXT("other device material cannot open"), Open(Sealed, TEXT("device-b"), Salt, Opened));
	TestFalse(TEXT("other salt cannot open"), Open(Sealed, TEXT("device-a"), RandomBytes(16), Opened));
	TestNotEqual(TEXT("random IV: two seals differ"), Hex(Sealed), Hex(Seal(Secret, TEXT("device-a"), Salt)));
	const FTCHARToUTF8 SecretUtf8(*Secret);
	TestFalse(TEXT("ciphertext does not contain the plaintext"),
		Hex(Sealed).Contains(BytesToHex(reinterpret_cast<const uint8*>(SecretUtf8.Get()), 8).ToLower()));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FAuthDeviceFlowTest, "Nightfall.Auth.DeviceFlow.Polling", AuthTestFlags)

bool FAuthDeviceFlowTest::RunTest(const FString& Parameters)
{
	DeleteTestSlot();
	ON_SCOPE_EXIT { DeleteTestSlot(); };
	FMockIdp Idp;
	FAuthFixture F(Idp);

	F.Auth->StartLogin();
	if (!TestEqual(TEXT("one device authorization request"), Idp.Calls.Num(), 1))
	{
		return false;
	}
	const FString& Issuer = GetDefault<UNetSettings>()->OidcIssuer;
	TestTrue(FString::Printf(TEXT("issuer is a realm URL (%s); quote it in the ini"), *Issuer),
		Issuer.StartsWith(TEXT("http")) && Issuer.Contains(TEXT("://")) && Issuer.Contains(TEXT("/realms/")));
	TestEqual(TEXT("device endpoint"), Idp.Calls[0].Url, Issuer + TEXT("/protocol/openid-connect/auth/device"));
	TestEqual(TEXT("client_id"), Idp.Calls[0].Form.FindRef(TEXT("client_id")), FString(TEXT("nightfall-client")));
	TestEqual(TEXT("PKCE method"), Idp.Calls[0].Form.FindRef(TEXT("code_challenge_method")), FString(TEXT("S256")));
	TestTrue(TEXT("scope asks for offline_access"), Idp.Calls[0].Form.FindRef(TEXT("scope")).Contains(TEXT("offline_access")));
	const FString Challenge = Idp.Calls[0].Form.FindRef(TEXT("code_challenge"));

	Idp.Reply(200, DeviceBody);
	TestEqual(TEXT("state awaiting approval"), F.Auth->GetState(), EAuthState::AwaitingApproval);
	TestEqual(TEXT("OnLoginRequired fired once"), F.Listener->LoginRequiredCount, 1);
	TestEqual(TEXT("user code"), F.Listener->LastCode, FString(TEXT("WXYZ-ABCD")));
	TestEqual(TEXT("complete verification URI"), F.Listener->LastUri, FString(TEXT("http://idp.test/device?user_code=WXYZ-ABCD")));
	TestEqual(TEXT("no poll before the interval"), Idp.Calls.Num(), 0);
	TestEqual(TEXT("first poll after the server's interval"), Idp.RunNextTimer(), 5.f);

	if (!TestEqual(TEXT("one poll"), Idp.Calls.Num(), 1))
	{
		return false;
	}
	TestEqual(TEXT("token endpoint"), Idp.Calls[0].Url, Issuer + TEXT("/protocol/openid-connect/token"));
	TestEqual(TEXT("device grant"), Idp.Calls[0].Form.FindRef(TEXT("grant_type")), FString(TEXT("urn:ietf:params:oauth:grant-type:device_code")));
	TestEqual(TEXT("device_code echoed"), Idp.Calls[0].Form.FindRef(TEXT("device_code")), FString(TEXT("dc-1")));
	TestEqual(TEXT("code_verifier matches the challenge"),
		NightfallAuthCrypto::PkceChallenge(Idp.Calls[0].Form.FindRef(TEXT("code_verifier"))), Challenge);

	Idp.Reply(400, OAuthError(TEXT("authorization_pending")));
	TestEqual(TEXT("pending: same interval"), Idp.RunNextTimer(), 5.f);
	Idp.Reply(400, OAuthError(TEXT("slow_down")));
	TestEqual(TEXT("slow_down: interval + 5 s"), Idp.RunNextTimer(), 10.f);
	Idp.Reply(503, TEXT("busy"));
	TestEqual(TEXT("5xx: keep polling at the slowed interval"), Idp.RunNextTimer(), 10.f);
	TestEqual(TEXT("still not logged in"), F.Listener->LoggedInCount, 0);

	const FString Access = FakeJwt(TEXT("player-1"));
	Idp.Reply(200, TokenBody(Access, TEXT("rt-1")));
	TestEqual(TEXT("logged in"), F.Auth->GetState(), EAuthState::LoggedIn);
	TestEqual(TEXT("OnLoggedIn fired once"), F.Listener->LoggedInCount, 1);
	TestEqual(TEXT("access token kept"), F.Auth->GetAccessToken(), Access);
	TestTrue(TEXT("user code cleared"), F.Auth->UserCode.IsEmpty());
	TestEqual(TEXT("no more polling"), Idp.Calls.Num(), 0);
	TestEqual(TEXT("only the proactive refresh is scheduled"), Idp.Timers.Num(), 1);
	if (Idp.Timers.Num() == 1)
	{
		TestEqual(TEXT("proactive refresh 59 s before expiry"), Idp.Timers[0].Key, 841.f);
	}

	// The bearer reaches gRPC metadata for authenticated calls but never for Ping.
	TestTrue(TEXT("session has a bearer"), F.Session->HasBearerToken());
	TestEqual(TEXT("authenticated metadata"), F.Session->MakeMetaData(true).MetaData.FindRef(TEXT("authorization")), TEXT("Bearer ") + Access);
	TestFalse(TEXT("Ping metadata carries no token"), F.Session->MakeMetaData(false).MetaData.Contains(TEXT("authorization")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FAuthDeviceFlowFailuresTest, "Nightfall.Auth.DeviceFlow.Failures", AuthTestFlags)

bool FAuthDeviceFlowFailuresTest::RunTest(const FString& Parameters)
{
	DeleteTestSlot();
	ON_SCOPE_EXIT { DeleteTestSlot(); };
	FMockIdp Idp;
	FAuthFixture F(Idp);

	// expired_token from the server ends the flow.
	F.Auth->StartLogin();
	Idp.Reply(200, DeviceBody);
	Idp.RunNextTimer();
	Idp.Reply(400, OAuthError(TEXT("expired_token")));
	TestEqual(TEXT("expired_token: failed"), F.Listener->FailedCount, 1);
	TestTrue(TEXT("expired_token: reason mentions expiry"), F.Listener->LastFailure.Contains(TEXT("expired")));
	TestEqual(TEXT("expired_token: logged out"), F.Auth->GetState(), EAuthState::LoggedOut);
	TestEqual(TEXT("expired_token: no further polling"), Idp.Timers.Num(), 0);

	// The device code's own lifetime is enforced locally too, without another request.
	F.Auth->StartLogin();
	Idp.Reply(200, DeviceBody);
	Idp.Now += 601.0;
	Idp.RunNextTimer();
	TestEqual(TEXT("local expiry: no request"), Idp.Calls.Num(), 0);
	TestEqual(TEXT("local expiry: failed"), F.Listener->FailedCount, 2);

	// access_denied: the player clicked No.
	F.Auth->StartLogin();
	Idp.Reply(200, DeviceBody);
	Idp.RunNextTimer();
	Idp.Reply(400, OAuthError(TEXT("access_denied")));
	TestEqual(TEXT("access_denied: failed"), F.Listener->FailedCount, 3);
	TestTrue(TEXT("access_denied: reason"), F.Listener->LastFailure.Contains(TEXT("declined")));

	// A token for some other audience is refused before it is ever used.
	F.Auth->StartLogin();
	Idp.Reply(200, DeviceBody);
	Idp.RunNextTimer();
	Idp.Reply(200, TokenBody(FakeJwt(TEXT("p"), TEXT("some-other-api")), TEXT("rt")));
	TestEqual(TEXT("wrong audience: failed"), F.Listener->FailedCount, 4);
	TestFalse(TEXT("wrong audience: no bearer"), F.Session->HasBearerToken());

	// Cancel drops a late reply.
	F.Auth->StartLogin();
	Idp.Reply(200, DeviceBody);
	F.Auth->CancelLogin();
	Idp.RunNextTimer();
	TestEqual(TEXT("cancelled: timer does not poll"), Idp.Calls.Num(), 0);
	TestEqual(TEXT("cancelled: logged out"), F.Auth->GetState(), EAuthState::LoggedOut);

	// Device endpoint unreachable.
	F.Auth->StartLogin();
	Idp.Reply(0, FString());
	TestEqual(TEXT("unreachable: failed"), F.Listener->FailedCount, 5);
	TestEqual(TEXT("never logged in"), F.Listener->LoggedInCount, 0);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FAuthRefreshTest, "Nightfall.Auth.Refresh", AuthTestFlags)

bool FAuthRefreshTest::RunTest(const FString& Parameters)
{
	DeleteTestSlot();
	ON_SCOPE_EXIT { DeleteTestSlot(); };
	FMockIdp Idp;
	FAuthFixture F(Idp);
	LoginThroughDeviceFlow(Idp, F.Auth, FakeJwt(TEXT("p1")), TEXT("rt-1"));
	if (!TestTrue(TEXT("logged in"), F.Auth->IsLoggedIn()))
	{
		return false;
	}
	const double ExpiresAt = F.Auth->GetAccessExpiresAtForTesting();

	int32 Ok = 0, NotOk = 0;
	auto Count = [&](bool bOk) { bOk ? ++Ok : ++NotOk; };

	Idp.Now = ExpiresAt - 61.0;
	F.Auth->RefreshIfNeeded(Count);
	TestEqual(TEXT("61 s left: no refresh"), Idp.Calls.Num(), 0);
	TestEqual(TEXT("61 s left: ok immediately"), Ok, 1);

	Idp.Now = ExpiresAt - 59.0;
	F.Auth->RefreshIfNeeded(Count);
	F.Auth->RefreshIfNeeded(Count);
	if (!TestEqual(TEXT("59 s left: one refresh for two callers"), Idp.Calls.Num(), 1))
	{
		return false;
	}
	TestEqual(TEXT("refresh grant"), Idp.Calls[0].Form.FindRef(TEXT("grant_type")), FString(TEXT("refresh_token")));
	TestEqual(TEXT("refresh token sent"), Idp.Calls[0].Form.FindRef(TEXT("refresh_token")), FString(TEXT("rt-1")));
	const FString Access2 = FakeJwt(TEXT("p1-again"));
	Idp.Reply(200, TokenBody(Access2, TEXT("rt-2")));
	TestEqual(TEXT("both callers told ok"), Ok, 3);
	TestEqual(TEXT("new access token"), F.Auth->GetAccessToken(), Access2);
	TestEqual(TEXT("bearer updated"), F.Session->MakeMetaData(true).MetaData.FindRef(TEXT("authorization")), TEXT("Bearer ") + Access2);
	TestEqual(TEXT("refresh does not re-announce login"), F.Listener->LoggedInCount, 1);

	// The IdP rejects the (rotated) refresh token: logged out everywhere.
	Idp.Now = F.Auth->GetAccessExpiresAtForTesting() - 10.0;
	F.Auth->RefreshIfNeeded(Count);
	TestEqual(TEXT("rotated token sent"), Idp.Calls.Num() == 1 ? Idp.Calls[0].Form.FindRef(TEXT("refresh_token")) : FString(), FString(TEXT("rt-2")));
	Idp.Reply(400, OAuthError(TEXT("invalid_grant")));
	TestEqual(TEXT("rejected: caller told not ok"), NotOk, 1);
	TestEqual(TEXT("rejected: logged out"), F.Auth->GetState(), EAuthState::LoggedOut);
	TestEqual(TEXT("rejected: OnLoginFailed"), F.Listener->FailedCount, 1);
	TestFalse(TEXT("rejected: bearer cleared"), F.Session->HasBearerToken());
	TestFalse(TEXT("rejected: stored login deleted"), UGameplayStatics::DoesSaveGameExist(TestSlot, 0));

	F.Auth->RefreshIfNeeded(Count);
	TestEqual(TEXT("logged out: not ok, no request"), NotOk + Idp.Calls.Num(), 2);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FAuthPersistenceTest, "Nightfall.Auth.Persistence", AuthTestFlags)

bool FAuthPersistenceTest::RunTest(const FString& Parameters)
{
	DeleteTestSlot();
	ON_SCOPE_EXIT { DeleteTestSlot(); };
	const FString Refresh = TEXT("refresh-token-sentinel-6f1c2a9e");
	{
		FMockIdp Idp;
		FAuthFixture F(Idp);
		LoginThroughDeviceFlow(Idp, F.Auth, FakeJwt(TEXT("p1")), Refresh);
		TestTrue(TEXT("first run logged in"), F.Auth->IsLoggedIn());
	}

	TArray<uint8> FileBytes;
	const FString SavePath = FPaths::Combine(FPaths::ProjectSavedDir(), TEXT("SaveGames"), FString(TestSlot) + TEXT(".sav"));
	if (TestTrue(TEXT("save file written"), FFileHelper::LoadFileToArray(FileBytes, *SavePath)))
	{
		const FTCHARToUTF8 Needle(*Refresh);
		const bool bPlain = FileBytes.Num() >= Needle.Length() && [&]
		{
			for (int32 I = 0; I + Needle.Length() <= FileBytes.Num(); ++I)
			{
				if (FMemory::Memcmp(FileBytes.GetData() + I, Needle.Get(), Needle.Length()) == 0) return true;
			}
			return false;
		}();
		TestFalse(TEXT("refresh token is not stored in plain text"), bPlain);
	}

	{
		// "Next launch": a fresh game instance resumes with the stored token, no device flow.
		FMockIdp Idp;
		FAuthFixture F(Idp);
		F.Auth->StartLogin();
		if (!TestEqual(TEXT("resume: one request"), Idp.Calls.Num(), 1))
		{
			return false;
		}
		TestEqual(TEXT("resume: refresh grant"), Idp.Calls[0].Form.FindRef(TEXT("grant_type")), FString(TEXT("refresh_token")));
		TestEqual(TEXT("resume: decrypted token"), Idp.Calls[0].Form.FindRef(TEXT("refresh_token")), Refresh);
		TestEqual(TEXT("resume: refreshing"), F.Auth->GetState(), EAuthState::Refreshing);
		Idp.Reply(200, TokenBody(FakeJwt(TEXT("p1")), TEXT("rt-next")));
		TestEqual(TEXT("resume: logged in"), F.Auth->GetState(), EAuthState::LoggedIn);
		TestEqual(TEXT("resume: no code shown"), F.Listener->LoginRequiredCount, 0);
	}

	{
		// A stored token the IdP no longer accepts falls back to the device flow.
		FMockIdp Idp;
		FAuthFixture F(Idp);
		F.Auth->StartLogin();
		Idp.Reply(400, OAuthError(TEXT("invalid_grant")));
		if (TestEqual(TEXT("fallback: one new request"), Idp.Calls.Num(), 1))
		{
			TestTrue(TEXT("fallback: device authorization"), Idp.Calls[0].Url.EndsWith(TEXT("/auth/device")));
		}
		TestEqual(TEXT("fallback: awaiting approval"), F.Auth->GetState(), EAuthState::AwaitingApproval);
		TestFalse(TEXT("fallback: rejected token forgotten"), UGameplayStatics::DoesSaveGameExist(TestSlot, 0));
	}

	{
		// Logout forgets everything.
		FMockIdp Idp;
		FAuthFixture F(Idp);
		LoginThroughDeviceFlow(Idp, F.Auth, FakeJwt(TEXT("p1")), TEXT("rt-x"));
		TestTrue(TEXT("logout: saved before"), UGameplayStatics::DoesSaveGameExist(TestSlot, 0));
		F.Auth->Logout();
		TestFalse(TEXT("logout: save deleted"), UGameplayStatics::DoesSaveGameExist(TestSlot, 0));
		TestFalse(TEXT("logout: no bearer"), F.Session->HasBearerToken());
	}
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FAuthDevTokenTest, "Nightfall.Auth.DevToken", AuthTestFlags)

bool FAuthDevTokenTest::RunTest(const FString& Parameters)
{
	DeleteTestSlot();
	ON_SCOPE_EXIT { DeleteTestSlot(); };
	FMockIdp Idp;
	FAuthFixture F(Idp);
	FAuthEnvironment Env = Idp.Environment();
	Env.DevToken = []() { return FString(TEXT("test:00000000-0000-0000-0000-00000000d3v0")); };
	F.Auth->SetEnvironmentForTesting(MoveTemp(Env));

	F.Auth->StartLogin();
	TestEqual(TEXT("logged in without the IdP"), F.Auth->GetState(), EAuthState::LoggedIn);
	TestEqual(TEXT("no HTTP"), Idp.Calls.Num(), 0);
	TestEqual(TEXT("OnLoggedIn"), F.Listener->LoggedInCount, 1);
	TestEqual(TEXT("bearer is the dev token"), F.Session->MakeMetaData(true).MetaData.FindRef(TEXT("authorization")),
		FString(TEXT("Bearer test:00000000-0000-0000-0000-00000000d3v0")));
	TestFalse(TEXT("dev token never persisted"), UGameplayStatics::DoesSaveGameExist(TestSlot, 0));

	bool bOk = false;
	Idp.Now += 1.0e9;
	F.Auth->RefreshIfNeeded([&](bool bResult) { bOk = bResult; });
	TestTrue(TEXT("dev token never refreshes"), bOk && Idp.Calls.IsEmpty());
	return true;
}

#endif
