#include "BotDiagnostics.h"
#include "Combat/CombatStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Dom/JsonObject.h"
#include "Serialization/JsonSerializer.h"
#include "Serialization/JsonWriter.h"

namespace
{
	TSharedRef<FJsonObject> Position(const FVector2D& Tiles)
	{
		TSharedRef<FJsonObject> Out = MakeShared<FJsonObject>();
		Out->SetNumberField(TEXT("x"), Tiles.X);
		Out->SetNumberField(TEXT("y"), Tiles.Y);
		return Out;
	}
	// Tick/XP/counts are uint64. Strings keep their exact value in JSON/JavaScript beyond 2^53.
	FString Exact(uint64 Value) { return FString::Printf(TEXT("%llu"), Value); }
}

FBotDiagnostics::FBotDiagnostics()
{
	for (const auto& Entry : NightfallContractWire::Catalogue()) Counts.FindOrAdd(Entry.Key).Add(Entry.Value, 0);
	// Close codes are WebSocket/application protocol constants, not protobuf enums (API guidelines).
	for (const FString& Code : { TEXT("4400"), TEXT("4408"), TEXT("4409"), TEXT("4429") })
		Counts.FindOrAdd(TEXT("close_codes")).Add(Code, 0);
}

FBotDiagnostics::~FBotDiagnostics() { Unbind(); }

void FBotDiagnostics::Bind(UNetClientSubsystem* Net)
{
	Unbind();
	if (!Net) return;
	BoundNet = Net;
	SentHandle = Net->OnWireSent.AddLambda([this](const TArray<uint8>& Bytes) { ObserveFrame(true, Bytes, FPlatformTime::Seconds()); });
	ReceivedHandle = Net->OnWireReceived.AddLambda([this](const TArray<uint8>& Bytes) { ObserveFrame(false, Bytes, FPlatformTime::Seconds()); });
	ClosedHandle = Net->OnWireClosed.AddRaw(this, &FBotDiagnostics::ObserveClose);
}

void FBotDiagnostics::Unbind()
{
	if (UNetClientSubsystem* Net = BoundNet.Get())
	{
		Net->OnWireSent.Remove(SentHandle);
		Net->OnWireReceived.Remove(ReceivedHandle);
		Net->OnWireClosed.Remove(ClosedHandle);
	}
	BoundNet.Reset();
}

void FBotDiagnostics::ObserveFrame(bool bClient, const TArray<uint8>& Bytes, double ArrivalSeconds)
{
	FNightfallContractObservation Observation;
	if (!NightfallContractWire::Inspect(bClient, Bytes, Observation)) return;
	if (bClient) ++ClientFrames; else ++ServerFrames;
	for (const auto& Entry : Observation.Cases) ++Counts.FindOrAdd(Entry.Key).FindOrAdd(Entry.Value);
	if (bClient) return;
	if (Observation.Tick.IsSet())
	{
		if (!FirstTick.IsSet()) FirstTick = Observation.Tick;
		LastTick = FMath::Max(LastTick, Observation.Tick.GetValue());
	}
	Events.Add(FEvent{ Observation.MessageType, Observation.DecodedFields, Observation.Tick, LastTick, ArrivalSeconds });
	if (Events.Num() > MaxEvents) Events.RemoveAt(0, Events.Num() - MaxEvents, EAllowShrinking::No);
}

void FBotDiagnostics::ObserveClose(int32 Code) { ++Counts.FindOrAdd(TEXT("close_codes")).FindOrAdd(FString::FromInt(Code)); }

void FBotDiagnostics::BeginMove(double NowSeconds)
{
	MoveStartSeconds = NowSeconds;
	PositionHistory.Reset();
}

void FBotDiagnostics::ResetIteration()
{
	Events.Reset();
	PositionHistory.Reset();
	FirstTick.Reset();
	LastTick = 0;
	MoveStartSeconds = 0.0;
}

void FBotDiagnostics::ObservePosition(const FBotContext& Context, double NowSeconds)
{
	if (MoveStartSeconds == 0.0) return;
	FVector2D Tiles;
	if (!BotPredicates::OwnPosition(Context, Tiles)) return;
	if (PositionHistory.IsEmpty() || PositionHistory.Last().Tick != LastTick || PositionHistory.Last().Tiles != Tiles)
		PositionHistory.Add(FPosition{ NowSeconds, LastTick, Tiles });
}

TSharedRef<FJsonObject> FBotDiagnostics::Coverage() const
{
	TSharedRef<FJsonObject> Out = MakeShared<FJsonObject>();
	Out->SetNumberField(TEXT("schema_version"), 1);
	Out->SetStringField(TEXT("catalogue_source"), TEXT("compiled protobuf descriptors; application close codes from API guidelines"));
	Out->SetStringField(TEXT("client_frames"), Exact(ClientFrames));
	Out->SetStringField(TEXT("server_frames"), Exact(ServerFrames));
	TSharedRef<FJsonObject> Groups = MakeShared<FJsonObject>();
	for (const auto& Group : Counts)
	{
		TSharedRef<FJsonObject> Entries = MakeShared<FJsonObject>();
		for (const auto& Entry : Group.Value) Entries->SetStringField(Entry.Key, Exact(Entry.Value));
		Groups->SetObjectField(Group.Key, Entries);
	}
	Out->SetObjectField(TEXT("counts"), Groups);
	TSharedRef<FJsonObject> Numbers = MakeShared<FJsonObject>();
	for (const auto& Group : NightfallContractWire::Numbers())
	{
		TSharedRef<FJsonObject> Entries = MakeShared<FJsonObject>();
		for (const auto& Entry : Group.Value) Entries->SetNumberField(Entry.Key, Entry.Value);
		Numbers->SetObjectField(Group.Key, Entries);
	}
	Out->SetObjectField(TEXT("wire_numbers"), Numbers);
	return Out;
}

TSharedRef<FJsonObject> FBotDiagnostics::FailureBundle(const FString& Scenario, const FString& OwnEntity,
	const FBotContext& Context, const FBotFailedStep& Step, const FString& Message) const
{
	TSharedRef<FJsonObject> Out = MakeShared<FJsonObject>();
	Out->SetNumberField(TEXT("schema_version"), FailureSchemaVersion);
	Out->SetStringField(TEXT("scenario"), Scenario);
	Out->SetStringField(TEXT("own_entity_id"), OwnEntity);
	Out->SetStringField(TEXT("message"), Message);
	Out->SetStringField(TEXT("tick_first"), FirstTick.IsSet() ? Exact(FirstTick.GetValue()) : FString());
	Out->SetStringField(TEXT("tick_last"), FirstTick.IsSet() ? Exact(LastTick) : FString());
	Out->SetStringField(TEXT("replay_session"), OwnEntity);
	TSharedRef<FJsonObject> Failed = MakeShared<FJsonObject>();
	Failed->SetNumberField(TEXT("line"), Step.Line);
	Failed->SetStringField(TEXT("step"), Step.Source);
	Failed->SetStringField(TEXT("predicate"), Step.Predicate);
	Failed->SetStringField(TEXT("last_value"), Step.Observed);
	Failed->SetNumberField(TEXT("wait_seconds"), Step.WaitSeconds);
	Failed->SetNumberField(TEXT("elapsed_seconds"), Step.ElapsedSeconds);
	Out->SetObjectField(TEXT("failure"), Failed);
	TArray<TSharedPtr<FJsonValue>> Recent;
	for (const FEvent& Event : Events)
	{
		TSharedRef<FJsonObject> Item = MakeShared<FJsonObject>();
		Item->SetStringField(TEXT("type"), Event.Type);
		Item->SetStringField(TEXT("decoded_fields"), Event.Fields);
		if (Event.Tick.IsSet()) Item->SetStringField(TEXT("server_tick"), Exact(Event.Tick.GetValue()));
		else Item->SetField(TEXT("server_tick"), MakeShared<FJsonValueNull>());
		Item->SetStringField(TEXT("nearest_preceding_tick"), Exact(Event.PrecedingTick));
		Item->SetNumberField(TEXT("arrival_monotonic_seconds"), Event.ArrivalSeconds);
		Recent.Add(MakeShared<FJsonValueObject>(Item));
	}
	Out->SetArrayField(TEXT("events"), Recent);
	TSharedRef<FJsonObject> Snapshot = MakeShared<FJsonObject>();
	UCombatStateSubsystem* Combat = Context.Combat();
	UNetClientSubsystem* Net = Context.Net();
	TSharedRef<FJsonObject> Own = MakeShared<FJsonObject>();
	for (const FString& Field : { TEXT("hp"), TEXT("max_hp"), TEXT("mp"), TEXT("max_mp"), TEXT("xp"), TEXT("level"), TEXT("dead"), TEXT("position"), TEXT("target"), TEXT("attack_state") })
		Own->SetField(Field, MakeShared<FJsonValueNull>());
	Own->SetBoolField(TEXT("hp_known"), Combat && Combat->FindOwnEntity());
	Own->SetBoolField(TEXT("mp_known"), false);
	Own->SetBoolField(TEXT("xp_known"), false);
	if (Combat)
	{
		const FOwnCombatState& State = Combat->GetOwn();
		Own->SetBoolField(TEXT("xp_known"), State.bXpKnown);
		if (State.bXpKnown) Own->SetStringField(TEXT("xp"), Exact(State.Xp));
		Own->SetBoolField(TEXT("mp_known"), State.bMpKnown);
		if (State.bMpKnown)
		{
			Own->SetNumberField(TEXT("mp"), State.Mp);
			Own->SetNumberField(TEXT("max_mp"), State.MaxMp);
		}
		Own->SetStringField(TEXT("target"), State.TargetId);
		const TCHAR* Attack = Combat->GetAttackState() == EAttackState::Active ? TEXT("active") :
			Combat->GetAttackState() == EAttackState::Pending ? TEXT("pending") : TEXT("idle");
		Own->SetStringField(TEXT("attack_state"), Attack);
		if (const FCombatEntity* Entity = Combat->FindOwnEntity())
		{
			Own->SetNumberField(TEXT("hp"), Entity->Hp);
			Own->SetNumberField(TEXT("max_hp"), Entity->MaxHp);
			Own->SetNumberField(TEXT("level"), Entity->Level);
			Own->SetBoolField(TEXT("dead"), Entity->bDead);
		}
	}
	FVector2D OwnTiles;
	if (BotPredicates::OwnPosition(Context, OwnTiles)) Own->SetObjectField(TEXT("position"), Position(OwnTiles));
	Snapshot->SetObjectField(TEXT("own"), Own);
	TArray<TSharedPtr<FJsonValue>> Proxies;
	if (Net)
	{
		TArray<FString> Ids;
		Net->GetKnownEntities().GetKeys(Ids);
		Ids.Sort();
		for (const FString& Id : Ids)
		{
			if (Net->IsOwnEntity(Id)) continue;
			const FEntitySpawn& Spawn = Net->GetKnownEntities().FindChecked(Id);
			TSharedRef<FJsonObject> Proxy = MakeShared<FJsonObject>();
			Proxy->SetStringField(TEXT("entity_id"), Id);
			Proxy->SetStringField(TEXT("name"), Spawn.Name);
			FVector2D Tiles;
			if (BotPredicates::EntityPosition(Context, Id, Tiles)) Proxy->SetObjectField(TEXT("position"), Position(Tiles));
			const FCombatEntity* Entity = Combat ? Combat->FindEntity(Id) : nullptr;
			Proxy->SetNumberField(TEXT("hp"), Entity ? Entity->Hp : Spawn.Hp);
			Proxy->SetNumberField(TEXT("incarnation"), Entity ? Entity->Incarnation : Spawn.LifeIncarnation);
			Proxies.Add(MakeShared<FJsonValueObject>(Proxy));
		}
	}
	Snapshot->SetArrayField(TEXT("proxies"), Proxies);
	Out->SetObjectField(TEXT("projection"), Snapshot);
	TArray<TSharedPtr<FJsonValue>> Positions;
	for (const FPosition& Sample : PositionHistory)
	{
		TSharedRef<FJsonObject> Item = Position(Sample.Tiles);
		Item->SetStringField(TEXT("server_tick"), Exact(Sample.Tick));
		Item->SetNumberField(TEXT("arrival_monotonic_seconds"), Sample.ArrivalSeconds);
		Positions.Add(MakeShared<FJsonValueObject>(Item));
	}
	Out->SetNumberField(TEXT("last_click_move_monotonic_seconds"), MoveStartSeconds);
	Out->SetArrayField(TEXT("own_position_history"), Positions);
	return Out;
}

FString FBotDiagnostics::Json(const TSharedRef<FJsonObject>& Object)
{
	FString Out;
	FJsonSerializer::Serialize(Object, TJsonWriterFactory<>::Create(&Out));
	return Out;
}

void FBotDiagnostics::AddJUnitProperties(TArray<TPair<FString, FString>>& Properties) const
{
	for (const auto& Group : Counts)
	{
		int32 Seen = 0;
		TArray<FString> Missing;
		for (const auto& Entry : Group.Value)
		{
			Properties.Emplace(TEXT("contract.") + Group.Key + TEXT(".") + Entry.Key, Exact(Entry.Value));
			if (Entry.Value > 0) ++Seen; else Missing.Add(Entry.Key);
		}
		Missing.Sort();
		Properties.Emplace(TEXT("contract.") + Group.Key + TEXT(".covered"), FString::Printf(TEXT("%d/%d"), Seen, Group.Value.Num()));
		Properties.Emplace(TEXT("contract.") + Group.Key + TEXT(".missing"), FString::Join(Missing, TEXT(", ")));
	}
}
