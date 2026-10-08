#include "SessionClientSubsystem.h"
#include "Nightfall.h"
#include "NetSettings.h"
#include "Engine/GameInstance.h"
#include "Misc/App.h"
#include "TurboLinkGrpcConfig.h"
#include "TurboLinkGrpcManager.h"
#include "SNightfallV1/GameClient.h"
#include "SNightfallV1/GameService.h"

namespace
{
	const TCHAR* const GameServiceName = TEXT("GameService");

	ENetError ToNetError(EGrpcResultCode Code)
	{
		switch (Code)
		{
		case EGrpcResultCode::Ok: return ENetError::None;
		case EGrpcResultCode::Cancelled: return ENetError::Cancelled;
		case EGrpcResultCode::InvalidArgument: return ENetError::InvalidArgument;
		case EGrpcResultCode::DeadlineExceeded: return ENetError::DeadlineExceeded;
		case EGrpcResultCode::NotFound: return ENetError::NotFound;
		case EGrpcResultCode::AlreadyExists: return ENetError::AlreadyExists;
		case EGrpcResultCode::PermissionDenied: return ENetError::PermissionDenied;
		case EGrpcResultCode::ResourceExhausted: return ENetError::ResourceExhausted;
		case EGrpcResultCode::FailedPrecondition: return ENetError::FailedPrecondition;
		case EGrpcResultCode::Aborted: return ENetError::Aborted;
		case EGrpcResultCode::OutOfRange: return ENetError::OutOfRange;
		case EGrpcResultCode::Unimplemented: return ENetError::Unimplemented;
		case EGrpcResultCode::Internal: return ENetError::Internal;
		case EGrpcResultCode::Unavailable: return ENetError::Unavailable;
		case EGrpcResultCode::DataLoss: return ENetError::DataLoss;
		case EGrpcResultCode::Unauthenticated: return ENetError::Unauthenticated;
		// TurboLink-only: the channel never connected. Same meaning to callers as UNAVAILABLE.
		case EGrpcResultCode::ConnectionFailed: return ENetError::Unavailable;
		case EGrpcResultCode::Unknown:
		case EGrpcResultCode::NotDefined:
		default:
			return ENetError::Unknown;
		}
	}

	template <typename TCallback, typename TResponse>
	void Complete(TMap<uint32, TCallback>& Pending, uint32 Handle, const FGrpcResult& Result, const TResponse& Response)
	{
		TCallback Callback;
		if (!Pending.RemoveAndCopyValue(Handle, Callback))
		{
			return;   // not ours, or already completed
		}
		const FNetResult NetResult = FNetResult::FromGrpc(Result);
		// TurboLink passes a default-constructed response on failure; never hand back partial data.
		Callback(NetResult, NetResult.IsOk() ? Response : TResponse());
	}
}

FNetResult FNetResult::FromGrpc(const FGrpcResult& Result)
{
	FNetResult Out;
	Out.Error = ToNetError(Result.Code);
	Out.Message = Result.Message;
	return Out;
}

void USessionClient::Initialize(FSubsystemCollectionBase& Collection)
{
	Super::Initialize(Collection);
	Manager = Collection.InitializeDependency<UTurboLinkGrpcManager>();

	const UNetSettings* Settings = GetDefault<UNetSettings>();
	Endpoint = Settings->GrpcEndpoint;
	CallTimeoutSeconds = Settings->CallTimeoutSeconds;

	// TurboLink reads endpoints from its own settings object when a service connects. Our ini
	// section is the source of truth, so mirror it there before connecting.
	GetMutableDefault<UTurboLinkGrpcConfig>()->ServiceEndPoint.Add(GameServiceName, Endpoint);

	if (Manager == nullptr)
	{
		UE_LOG(LogNightfall, Error, TEXT("SessionClient: TurboLink gRPC manager unavailable"));
		return;
	}
	Service = Cast<UGameService>(Manager->MakeService(GameServiceName));
	if (Service == nullptr)
	{
		UE_LOG(LogNightfall, Error, TEXT("SessionClient: could not create GameService"));
		return;
	}
	Service->Connect();
	Client = Service->MakeClient();
	Client->OnPingResponse.AddDynamic(this, &USessionClient::HandlePing);
	Client->OnGetCharacterResponse.AddDynamic(this, &USessionClient::HandleGetCharacter);
	Client->OnCreateCharacterResponse.AddDynamic(this, &USessionClient::HandleCreateCharacter);
	UE_LOG(LogNightfall, Log, TEXT("SessionClient: GameService at %s"), *Endpoint);
}

void USessionClient::Deinitialize()
{
	// Callers may hold references captured in their callbacks; drop them without invoking.
	PendingPing.Empty();
	PendingGetCharacter.Empty();
	PendingCreateCharacter.Empty();

	if (Client != nullptr)
	{
		Client->Shutdown();
		if (Service != nullptr)
		{
			Service->RemoveClient(Client);
		}
	}
	if (Manager != nullptr && Service != nullptr)
	{
		Manager->ReleaseService(Service);
	}
	Client = nullptr;
	Service = nullptr;
	Manager = nullptr;
	Super::Deinitialize();
}

FGrpcMetaData USessionClient::MakeMetaData() const
{
	FGrpcMetaData MetaData;
	if (!BearerToken.IsEmpty())
	{
		// gRPC metadata keys are lowercase on the wire.
		MetaData.MetaData.Add(TEXT("authorization"), FString::Printf(TEXT("Bearer %s"), *BearerToken));
	}
	return MetaData;
}

bool USessionClient::EnsureClient(const TFunctionRef<void(const FNetResult&)>& Fail) const
{
	if (Client != nullptr)
	{
		return true;
	}
	FNetResult Result;
	Result.Error = ENetError::Unavailable;
	Result.Message = TEXT("gRPC client not initialised");
	Fail(Result);
	return false;
}

void USessionClient::Ping(FPingCallback Callback)
{
	if (!EnsureClient([&](const FNetResult& R) { Callback(R, FGrpcNightfallV1PingResponse()); }))
	{
		return;
	}
	FGrpcNightfallV1PingRequest Request;
	Request.ClientVersion = FString::Printf(TEXT("nightfall-unreal/%s"), FApp::GetBuildVersion());

	const FGrpcContextHandle Handle = Client->InitPing();
	PendingPing.Add(Handle.Value, MoveTemp(Callback));
	Client->Ping(Handle, Request, MakeMetaData(), CallTimeoutSeconds);
}

void USessionClient::GetCharacter(const FString& CharacterId, FCharacterCallback Callback)
{
	if (!EnsureClient([&](const FNetResult& R) { Callback(R, FGrpcNightfallV1Character()); }))
	{
		return;
	}
	FGrpcNightfallV1GetCharacterRequest Request;
	Request.CharacterId = CharacterId;

	const FGrpcContextHandle Handle = Client->InitGetCharacter();
	PendingGetCharacter.Add(Handle.Value, MoveTemp(Callback));
	Client->GetCharacter(Handle, Request, MakeMetaData(), CallTimeoutSeconds);
}

void USessionClient::CreateCharacter(const FGrpcNightfallV1CreateCharacterRequest& Request, FCharacterCallback Callback)
{
	if (!EnsureClient([&](const FNetResult& R) { Callback(R, FGrpcNightfallV1Character()); }))
	{
		return;
	}
	const FGrpcContextHandle Handle = Client->InitCreateCharacter();
	PendingCreateCharacter.Add(Handle.Value, MoveTemp(Callback));
	Client->CreateCharacter(Handle, Request, MakeMetaData(), CallTimeoutSeconds);
}

void USessionClient::K2_Ping(FOnNetPingDone OnDone)
{
	Ping([OnDone](const FNetResult& Result, const FGrpcNightfallV1PingResponse& Response)
	{
		OnDone.ExecuteIfBound(Result, Response);
	});
}

void USessionClient::K2_GetCharacter(const FString& CharacterId, FOnNetCharacterDone OnDone)
{
	GetCharacter(CharacterId, [OnDone](const FNetResult& Result, const FGrpcNightfallV1Character& Character)
	{
		OnDone.ExecuteIfBound(Result, Character);
	});
}

void USessionClient::K2_CreateCharacter(const FGrpcNightfallV1CreateCharacterRequest& Request, FOnNetCharacterDone OnDone)
{
	CreateCharacter(Request, [OnDone](const FNetResult& Result, const FGrpcNightfallV1Character& Character)
	{
		OnDone.ExecuteIfBound(Result, Character);
	});
}

void USessionClient::HandlePing(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1PingResponse& Response)
{
	Complete(PendingPing, Handle.Value, Result, Response);
}

void USessionClient::HandleGetCharacter(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1Character& Response)
{
	Complete(PendingGetCharacter, Handle.Value, Result, Response);
}

void USessionClient::HandleCreateCharacter(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1Character& Response)
{
	Complete(PendingCreateCharacter, Handle.Value, Result, Response);
}
