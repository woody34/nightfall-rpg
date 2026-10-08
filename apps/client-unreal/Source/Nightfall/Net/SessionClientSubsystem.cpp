#include "SessionClientSubsystem.h"
#include "Nightfall.h"
#include "NetSettings.h"
#include "Engine/GameInstance.h"
#include "Misc/App.h"
#include "TurboLinkGrpcConfig.h"
#include "TurboLinkGrpcManager.h"
#include "SNightfallV1/GameClient.h"
#include "SNightfallV1/GameService.h"
#include "SNightfallV1/SessionClient.h"
#include "SNightfallV1/SessionService.h"

namespace
{
	const TCHAR* const GameServiceName = TEXT("GameService");
	const TCHAR* const SessionServiceName = TEXT("SessionService");

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
	GetMutableDefault<UTurboLinkGrpcConfig>()->ServiceEndPoint.Add(SessionServiceName, Endpoint);

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
	Client->OnListMyCharactersResponse.AddDynamic(this, &USessionClient::HandleListMyCharacters);

	SessionService = Cast<USessionService>(Manager->MakeService(SessionServiceName));
	if (SessionService == nullptr)
	{
		UE_LOG(LogNightfall, Error, TEXT("SessionClient: could not create SessionService"));
		return;
	}
	SessionService->Connect();
	SessionServiceClient = SessionService->MakeClient();
	SessionServiceClient->OnIssuePlayTicketResponse.AddDynamic(this, &USessionClient::HandleIssuePlayTicket);
	UE_LOG(LogNightfall, Log, TEXT("SessionClient: GameService and SessionService at %s"), *Endpoint);
}

void USessionClient::Deinitialize()
{
	// Callers may hold references captured in their callbacks; drop them without invoking.
	PendingPing.Empty();
	PendingGetCharacter.Empty();
	PendingCreateCharacter.Empty();
	PendingListMyCharacters.Empty();
	PendingIssuePlayTicket.Empty();

	if (SessionServiceClient != nullptr)
	{
		SessionServiceClient->Shutdown();
		if (SessionService != nullptr)
		{
			SessionService->RemoveClient(SessionServiceClient);
		}
	}
	if (Manager != nullptr && SessionService != nullptr)
	{
		Manager->ReleaseService(SessionService);
	}
	SessionServiceClient = nullptr;
	SessionService = nullptr;

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

FGrpcMetaData USessionClient::MakeMetaData(bool bAuthenticated) const
{
	FGrpcMetaData MetaData;
	if (bAuthenticated && !BearerToken.IsEmpty())
	{
		// gRPC metadata keys are lowercase on the wire.
		MetaData.MetaData.Add(TEXT("authorization"), FString::Printf(TEXT("Bearer %s"), *BearerToken));
	}
	return MetaData;
}

bool USessionClient::EnsureClient(const UObject* ServiceClient, const TFunctionRef<void(const FNetResult&)>& Fail)
{
	if (ServiceClient != nullptr)
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
	if (!EnsureClient(Client, [&](const FNetResult& R) { Callback(R, FGrpcNightfallV1PingResponse()); }))
	{
		return;
	}
	FGrpcNightfallV1PingRequest Request;
	Request.ClientVersion = FString::Printf(TEXT("nightfall-unreal/%s"), FApp::GetBuildVersion());

	const FGrpcContextHandle Handle = Client->InitPing();
	PendingPing.Add(Handle.Value, MoveTemp(Callback));
	// Ping is the one unauthenticated RPC; it never carries the token.
	Client->Ping(Handle, Request, MakeMetaData(/*bAuthenticated=*/false), CallTimeoutSeconds);
}

void USessionClient::GetCharacter(const FString& CharacterId, FCharacterCallback Callback)
{
	if (!EnsureClient(Client, [&](const FNetResult& R) { Callback(R, FGrpcNightfallV1Character()); }))
	{
		return;
	}
	FGrpcNightfallV1GetCharacterRequest Request;
	Request.CharacterId = CharacterId;

	const FGrpcContextHandle Handle = Client->InitGetCharacter();
	PendingGetCharacter.Add(Handle.Value, MoveTemp(Callback));
	Client->GetCharacter(Handle, Request, MakeMetaData(/*bAuthenticated=*/true), CallTimeoutSeconds);
}

void USessionClient::CreateCharacter(const FGrpcNightfallV1CreateCharacterRequest& Request, FCharacterCallback Callback)
{
	if (!EnsureClient(Client, [&](const FNetResult& R) { Callback(R, FGrpcNightfallV1Character()); }))
	{
		return;
	}
	const FGrpcContextHandle Handle = Client->InitCreateCharacter();
	PendingCreateCharacter.Add(Handle.Value, MoveTemp(Callback));
	Client->CreateCharacter(Handle, Request, MakeMetaData(/*bAuthenticated=*/true), CallTimeoutSeconds);
}

void USessionClient::ListMyCharacters(FCharacterListCallback Callback)
{
	if (!EnsureClient(Client, [&](const FNetResult& R) { Callback(R, {}); }))
	{
		return;
	}
	const FGrpcContextHandle Handle = Client->InitListMyCharacters();
	PendingListMyCharacters.Add(Handle.Value, MoveTemp(Callback));
	Client->ListMyCharacters(Handle, FGrpcNightfallV1ListMyCharactersRequest(), MakeMetaData(/*bAuthenticated=*/true), CallTimeoutSeconds);
}

void USessionClient::IssuePlayTicket(const FGrpcNightfallV1IssuePlayTicketRequest& Request, FPlayTicketCallback Callback)
{
	if (!EnsureClient(SessionServiceClient, [&](const FNetResult& R) { Callback(R, FGrpcNightfallV1IssuePlayTicketResponse()); }))
	{
		return;
	}
	const FGrpcContextHandle Handle = SessionServiceClient->InitIssuePlayTicket();
	PendingIssuePlayTicket.Add(Handle.Value, MoveTemp(Callback));
	SessionServiceClient->IssuePlayTicket(Handle, Request, MakeMetaData(/*bAuthenticated=*/true), CallTimeoutSeconds);
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

void USessionClient::K2_ListMyCharacters(FOnNetCharacterListDone OnDone)
{
	ListMyCharacters([OnDone](const FNetResult& Result, const TArray<FGrpcNightfallV1Character>& Characters)
	{
		OnDone.ExecuteIfBound(Result, Characters);
	});
}

void USessionClient::K2_IssuePlayTicket(const FGrpcNightfallV1IssuePlayTicketRequest& Request, FOnNetPlayTicketDone OnDone)
{
	IssuePlayTicket(Request, [OnDone](const FNetResult& Result, const FGrpcNightfallV1IssuePlayTicketResponse& Ticket)
	{
		OnDone.ExecuteIfBound(Result, Ticket);
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

void USessionClient::HandleListMyCharacters(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1ListMyCharactersResponse& Response)
{
	FCharacterListCallback Callback;
	if (!PendingListMyCharacters.RemoveAndCopyValue(Handle.Value, Callback))
	{
		return;
	}
	const FNetResult NetResult = FNetResult::FromGrpc(Result);
	// The generated response holds shared pointers (repeated message field); hand out values.
	TArray<FGrpcNightfallV1Character> Characters;
	if (NetResult.IsOk())
	{
		for (const TSharedPtr<FGrpcNightfallV1Character>& Character : Response.Characters)
		{
			if (Character.IsValid())
			{
				Characters.Add(*Character);
			}
		}
	}
	Callback(NetResult, Characters);
}

void USessionClient::HandleIssuePlayTicket(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1IssuePlayTicketResponse& Response)
{
	Complete(PendingIssuePlayTicket, Handle.Value, Result, Response);
}
