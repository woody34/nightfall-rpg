#include "LoginFlowSubsystem.h"
#include "Bot/BotCharacterName.h"
#include "Character/ClassStateSubsystem.h"
#include "Nightfall.h"
#include "Auth/AuthSubsystem.h"
#include "Net/NetSettings.h"
#include "Engine/Engine.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"
#include "HAL/IConsoleManager.h"
#include "Kismet/GameplayStatics.h"
#include "Misc/Guid.h"
#include "Misc/Parse.h"

namespace
{
	FNetResult NotLoggedIn()
	{
		FNetResult Result;
		Result.Error = ENetError::Unauthenticated;
		Result.Message = TEXT("Not logged in");
		return Result;
	}

	/** Errors after which asking for another ticket cannot help. */
	bool IsFatalTicketError(ENetError Error)
	{
		return Error == ENetError::Unauthenticated || Error == ENetError::PermissionDenied
			|| Error == ENetError::NotFound || Error == ENetError::InvalidArgument;
	}
}

void ULoginFlowSubsystem::Initialize(FSubsystemCollectionBase& Collection)
{
	Super::Initialize(Collection);
	Session = Collection.InitializeDependency<USessionClient>();
	Auth = Collection.InitializeDependency<UAuthSubsystem>();
	Net = Collection.InitializeDependency<UNetClientSubsystem>();
	if (Net != nullptr)
	{
		Net->OnConnected.AddDynamic(this, &ULoginFlowSubsystem::HandleConnected);
		Net->OnDisconnected.AddDynamic(this, &ULoginFlowSubsystem::HandleDisconnected);
	}
}

void ULoginFlowSubsystem::Deinitialize()
{
	if (Net != nullptr)
	{
		Net->OnConnected.RemoveAll(this);
		Net->OnDisconnected.RemoveAll(this);
		Net->SetTicketProvider(nullptr);
	}
	Super::Deinitialize();
}

FString ULoginFlowSubsystem::NewIdempotencyKey()
{
	return FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower);
}

void ULoginFlowSubsystem::WithFreshToken(TFunction<void()> Call, FResultCallback Fail)
{
	if (Auth == nullptr)
	{
		Fail(NotLoggedIn());
		return;
	}
	Auth->RefreshIfNeeded([Call = MoveTemp(Call), Fail = MoveTemp(Fail)](bool bOk)
	{
		if (bOk)
		{
			Call();
		}
		else
		{
			Fail(NotLoggedIn());
		}
	});
}

void ULoginFlowSubsystem::ListCharacters(USessionClient::FCharacterListCallback Callback)
{
	WithFreshToken(
		[Weak = TWeakObjectPtr<ULoginFlowSubsystem>(this), Callback]()
		{
			if (Weak.IsValid() && Weak->Session != nullptr)
			{
				Weak->Session->ListMyCharacters(Callback);
			}
		},
		[Callback](const FNetResult& Error) { Callback(Error, {}); });
}

void ULoginFlowSubsystem::CreateCharacter(const FString& Name, EGrpcNightfallV1Race Race, USessionClient::FCharacterCallback Callback)
{
	FGrpcNightfallV1CreateCharacterRequest Request;
	Request.IdempotencyKey = NewIdempotencyKey();   // one per attempt
	Request.Name = Name;
	Request.Race = Race;
	CreateCharacter(Request, MoveTemp(Callback));
}

void ULoginFlowSubsystem::CreateCharacter(const FGrpcNightfallV1CreateCharacterRequest& InRequest, USessionClient::FCharacterCallback Callback)
{
	FGrpcNightfallV1CreateCharacterRequest Request = InRequest;
	Request.IdempotencyKey = NewIdempotencyKey();
	WithFreshToken(
		[Weak = TWeakObjectPtr<ULoginFlowSubsystem>(this), Request, Callback]()
		{
			if (Weak.IsValid() && Weak->Session != nullptr)
			{
				Weak->Session->CreateCharacter(Request, Callback);
			}
		},
		[Callback](const FNetResult& Error) { Callback(Error, FGrpcNightfallV1Character()); });
}

void ULoginFlowSubsystem::RequestTicket(const FString& CharacterId, FTicketResultCallback Callback)
{
	FGrpcNightfallV1IssuePlayTicketRequest Request;
	Request.IdempotencyKey = NewIdempotencyKey();   // never reuse: a retry would return a consumed ticket
	Request.CharacterId = CharacterId;
	WithFreshToken(
		[Weak = TWeakObjectPtr<ULoginFlowSubsystem>(this), Request, Callback]()
		{
			if (!Weak.IsValid() || Weak->Session == nullptr)
			{
				return;
			}
			Weak->Session->IssuePlayTicket(Request, [Callback](const FNetResult& Result, const FGrpcNightfallV1IssuePlayTicketResponse& Response)
			{
				FPlayTicket Ticket;
				Ticket.WsUrl = Response.WsUrl;
				Ticket.Ticket = Response.Ticket;
				Callback(Result, Ticket);
			});
		},
		[Callback](const FNetResult& Error) { Callback(Error, FPlayTicket()); });
}

void ULoginFlowSubsystem::EnterWorld(const FString& CharacterId, FResultCallback OnTicket)
{
	SelectedCharacterId = CharacterId;
	SetStatus(TEXT("Requesting a play ticket..."));
	RequestTicket(CharacterId, [Weak = TWeakObjectPtr<ULoginFlowSubsystem>(this), CharacterId, OnTicket](const FNetResult& Result, const FPlayTicket& Ticket)
	{
		if (!Weak.IsValid() || Weak->Net == nullptr || Weak->SelectedCharacterId != CharacterId)
		{
			return;
		}
		if (!Result.IsOk())
		{
			Weak->SetStatus(FString::Printf(TEXT("Could not enter the world: %s"), *Result.Message));
			OnTicket(Result);
			return;
		}

		// The server's entity id for a player is the character id.
		Weak->Net->SetOwnEntityId(CharacterId);

		// Reconnects ask for a brand-new ticket each time (plan §8 #9).
		Weak->Net->SetTicketProvider([Weak, CharacterId](UNetClientSubsystem::FTicketCallback OnNewTicket)
		{
			if (!Weak.IsValid())
			{
				return;
			}
			Weak->SetStatus(TEXT("Reconnecting..."));
			Weak->RequestTicket(CharacterId, [Weak, OnNewTicket](const FNetResult& R, const FPlayTicket& T)
			{
				if (!Weak.IsValid())
				{
					return;
				}
				if (R.IsOk())
				{
					OnNewTicket(true, T);
				}
				else if (IsFatalTicketError(R.Error))
				{
					Weak->SetStatus(FString::Printf(TEXT("Disconnected: %s"), *R.Message));
					Weak->Net->Disconnect();
				}
				else
				{
					OnNewTicket(false, T);
				}
			});
		});
		Weak->SetStatus(TEXT("Connecting..."));
		Weak->Net->Connect(Ticket.WsUrl, Ticket.Ticket);
		OnTicket(Result);
	});
}

void ULoginFlowSubsystem::LeaveWorld()
{
	if (GetGameInstance())
	{
		if (auto* Classes = GetGameInstance()->GetSubsystem<UClassStateSubsystem>()) Classes->ResetAccount();
	}
	SelectedCharacterId.Empty();
	if (Net != nullptr)
	{
		Net->SetOwnEntityId(FString());
		Net->SetTicketProvider(nullptr);
		Net->Disconnect();
	}
}

void ULoginFlowSubsystem::SetStatus(const FString& Status)
{
	UE_LOG(LogNightfall, Log, TEXT("Login: %s"), *Status);
	OnStatus.Broadcast(Status);
	// Status line in-world until a HUD exists (plan Story 6.2); key 1 replaces the previous line.
	if (GEngine != nullptr)
	{
		GEngine->AddOnScreenDebugMessage(/*Key=*/1, 5.f, FColor::White, Status);
	}
}

void ULoginFlowSubsystem::HandleConnected()
{
	SetStatus(TEXT("Connected"));
	if (!bTravelOnConnect || SelectedCharacterId.IsEmpty())   // only connections EnterWorld started
	{
		return;
	}
	UWorld* World = GetGameInstance() != nullptr ? GetGameInstance()->GetWorld() : nullptr;
	const FSoftObjectPath& WorldMap = GetDefault<UNetSettings>()->WorldMap;
	if (World == nullptr || WorldMap.IsNull())
	{
		return;
	}
	const FString Current = UWorld::RemovePIEPrefix(World->GetOutermost()->GetName());
	if (Current != WorldMap.GetLongPackageName())
	{
		UGameplayStatics::OpenLevelBySoftObjectPtr(World, TSoftObjectPtr<UWorld>(WorldMap));
	}
}

void ULoginFlowSubsystem::HandleDisconnected(const FString& Reason)
{
	SetStatus(Reason.IsEmpty() ? FString(TEXT("Disconnected")) : FString::Printf(TEXT("Disconnected: %s"), *Reason));
}

#if !UE_BUILD_SHIPPING
// Developer console commands: drive the login flow without the UI (headless -game runs, quick
// manual checks). `-ExecCmds="nf.Login, nf.EnterWorld"` with -DevTokenFile logs in and connects.
namespace
{
	ULoginFlowSubsystem* FlowFrom(UWorld* World)
	{
		UGameInstance* GameInstance = World != nullptr ? World->GetGameInstance() : nullptr;
		return GameInstance != nullptr ? GameInstance->GetSubsystem<ULoginFlowSubsystem>() : nullptr;
	}

	FAutoConsoleCommandWithWorld LoginCommand(TEXT("nf.Login"), TEXT("UAuthSubsystem::StartLogin"),
		FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* World)
		{
			UGameInstance* GameInstance = World != nullptr ? World->GetGameInstance() : nullptr;
			if (UAuthSubsystem* Auth = GameInstance != nullptr ? GameInstance->GetSubsystem<UAuthSubsystem>() : nullptr)
			{
				Auth->StartLogin();
			}
		}));

	FAutoConsoleCommandWithWorld LogoutCommand(TEXT("nf.Logout"), TEXT("Forget all tokens (memory and disk) and disconnect"),
		FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* World)
		{
			if (ULoginFlowSubsystem* Flow = FlowFrom(World))
			{
				Flow->LeaveWorld();
			}
			UGameInstance* GameInstance = World != nullptr ? World->GetGameInstance() : nullptr;
			if (UAuthSubsystem* Auth = GameInstance != nullptr ? GameInstance->GetSubsystem<UAuthSubsystem>() : nullptr)
			{
				Auth->Logout();
			}
		}));

	FAutoConsoleCommandWithWorld CharactersCommand(TEXT("nf.Characters"), TEXT("Log ListMyCharacters"),
		FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* World)
		{
			if (ULoginFlowSubsystem* Flow = FlowFrom(World))
			{
				Flow->ListCharacters([](const FNetResult& Result, const TArray<FGrpcNightfallV1Character>& Characters)
				{
					UE_LOG(LogNightfall, Display, TEXT("nf.Characters: %s, %d"), *UEnum::GetValueAsString(Result.Error), Characters.Num());
					for (const FGrpcNightfallV1Character& Character : Characters)
					{
						UE_LOG(LogNightfall, Display, TEXT("  %s %s level %u"), *Character.Id, *Character.Name, Character.Level.Value);
					}
				});
			}
		}));

	FAutoConsoleCommandWithWorldAndArgs CreateCommand(TEXT("nf.CreateCharacter"), TEXT("nf.CreateCharacter <Name> [1-5 race, default 1 human]"),
		FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& Args, UWorld* World)
		{
			ULoginFlowSubsystem* Flow = FlowFrom(World);
			if (Flow == nullptr || Args.IsEmpty())
			{
				return;
			}
			const int32 Race = Args.Num() > 1 ? FMath::Clamp(FCString::Atoi(*Args[1]), 1, 5) : 1;
			Flow->CreateCharacter(Args[0], static_cast<EGrpcNightfallV1Race>(Race), [](const FNetResult& Result, const FGrpcNightfallV1Character& Character)
			{
				UE_LOG(LogNightfall, Display, TEXT("nf.CreateCharacter: %s %s %s"), *UEnum::GetValueAsString(Result.Error), *Character.Id, *Result.Message);
			});
		}));

	FAutoConsoleCommandWithWorldAndArgs EnterCommand(TEXT("nf.EnterWorld"), TEXT("nf.EnterWorld [character name; default the first, or a new human when the account has none]"),
		FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& Args, UWorld* World)
		{
			ULoginFlowSubsystem* Flow = FlowFrom(World);
			if (Flow == nullptr)
			{
				return;
			}
			const FString Wanted = Args.IsEmpty() ? FString() : Args[0];
			Flow->ListCharacters([Weak = TWeakObjectPtr<ULoginFlowSubsystem>(Flow), Wanted](const FNetResult& Result, const TArray<FGrpcNightfallV1Character>& Characters)
			{
				const FGrpcNightfallV1Character* Pick = Characters.FindByPredicate([&](const FGrpcNightfallV1Character& C)
				{
					return Wanted.IsEmpty() || C.Name.Equals(Wanted, ESearchCase::IgnoreCase);
				});
				if (Weak.IsValid() && Result.IsOk() && Characters.IsEmpty() && Wanted.IsEmpty())
				{
					// A fresh account (a bot's test:<uuid>): create a character with a unique
					// letters-only name (3-16, the server's rule) and enter with it.
					const FString Name = BotCharacterName::FromGuid(FGuid::NewGuid());
					UE_LOG(LogNightfall, Display, TEXT("nf.EnterWorld: no characters; creating %s"), *Name);
					Weak->CreateCharacter(Name, EGrpcNightfallV1Race::RACE_HUMAN, [Weak](const FNetResult& Created, const FGrpcNightfallV1Character& Character)
					{
						if (!Weak.IsValid() || !Created.IsOk())
						{
							UE_LOG(LogNightfall, Warning, TEXT("nf.EnterWorld: could not create a character (%s)"), *Created.Message);
							return;
						}
						Weak->EnterWorld(Character.Id, [](const FNetResult& Entered)
						{
							if (!Entered.IsOk()) UE_LOG(LogNightfall, Warning, TEXT("nf.EnterWorld: no play ticket (%s)"), *Entered.Message);
						});
					});
					return;
				}
				if (!Weak.IsValid() || Pick == nullptr)
				{
					UE_LOG(LogNightfall, Warning, TEXT("nf.EnterWorld: no such character (%s)"), *Result.Message);
					return;
				}
				UE_LOG(LogNightfall, Display, TEXT("nf.EnterWorld: %s"), *Pick->Name);
				Weak->EnterWorld(Pick->Id, [](const FNetResult&) {});
			});
		}));
}
#endif

void ULoginFlowSubsystem::ListClasses(USessionClient::FCatalogueCallback Callback)
{
	WithFreshToken([Weak = TWeakObjectPtr<ULoginFlowSubsystem>(this), Callback]()
	{
		if (Weak.IsValid() && Weak->Session) Weak->Session->ListClasses(Callback);
	}, [Callback](const FNetResult& R) { Callback(R, FGrpcNightfallV1ListClassesResponse()); });
}

void ULoginFlowSubsystem::TransferOptions(const FString& CharacterId, USessionClient::FTransferOptionsCallback Callback)
{
	WithFreshToken([Weak = TWeakObjectPtr<ULoginFlowSubsystem>(this), CharacterId, Callback]()
	{
		if (Weak.IsValid() && Weak->Session) Weak->Session->TransferOptions(CharacterId, Callback);
	}, [Callback](const FNetResult& R) { Callback(R, FGrpcNightfallV1TransferOptionsResponse()); });
}

void ULoginFlowSubsystem::ChangeClass(const FString& CharacterId, uint32 TargetClassId, USessionClient::FChangeClassCallback Callback)
{
	FGrpcNightfallV1ChangeClassRequest Request;
	Request.CharacterId = CharacterId;
	Request.TargetClassId = TargetClassId;
	Request.IdempotencyKey = NewIdempotencyKey();
	WithFreshToken([Weak = TWeakObjectPtr<ULoginFlowSubsystem>(this), Request, Callback]()
	{
		if (Weak.IsValid() && Weak->Session) Weak->Session->ChangeClass(Request, Callback);
	}, [Callback](const FNetResult& R) { Callback(R, FGrpcNightfallV1ChangeClassResponse()); });
}

void ULoginFlowSubsystem::GetCharacter(const FString& CharacterId, USessionClient::FCharacterCallback Callback)
{
	WithFreshToken([Weak = TWeakObjectPtr<ULoginFlowSubsystem>(this), CharacterId, Callback]()
	{
		if (Weak.IsValid() && Weak->Session) Weak->Session->GetCharacter(CharacterId, Callback);
	}, [Callback](const FNetResult& R) { Callback(R, FGrpcNightfallV1Character()); });
}
