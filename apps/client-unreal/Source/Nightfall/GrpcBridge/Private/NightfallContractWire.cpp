#include "NightfallContractWire.h"
#include "nightfall/v1/world.pb.h"
#include "google/protobuf/descriptor.h"

namespace
{
	FString Text(const std::string& Value) { return FString(UTF8_TO_TCHAR(Value.c_str())); }
	void AddOneof(const google::protobuf::Descriptor* Descriptor, const char* Name, const TCHAR* Group,
		TArray<TPair<FString, FString>>& Out)
	{
		const google::protobuf::OneofDescriptor* Oneof = Descriptor->FindOneofByName(Name);
		if (!Oneof) return;
		for (int32 I = 0; I < Oneof->field_count(); ++I) Out.Emplace(Group, Text(Oneof->field(I)->name()));
	}
	void InspectOneof(const google::protobuf::Message& Message, const char* Name, const TCHAR* Group,
		FNightfallContractObservation& Out)
	{
		const auto* Oneof = Message.GetDescriptor()->FindOneofByName(Name);
		const auto* Field = Message.GetReflection()->GetOneofFieldDescriptor(Message, Oneof);
		if (!Field) return;
		Out.Cases.Emplace(Group, Text(Field->name()));
		if (Field->cpp_type() != google::protobuf::FieldDescriptor::CPPTYPE_MESSAGE) return;
		const auto& Payload = Message.GetReflection()->GetMessage(Message, Field);
		Out.MessageType = Text(Payload.GetDescriptor()->full_name());
		Out.DecodedFields = Text(Payload.ShortDebugString());
		if (const auto* Tick = Payload.GetDescriptor()->FindFieldByName("tick"))
		{
			if (Tick->cpp_type() == google::protobuf::FieldDescriptor::CPPTYPE_UINT64)
				Out.Tick = Payload.GetReflection()->GetUInt64(Payload, Tick);
		}
	}
}

TArray<TPair<FString, FString>> NightfallContractWire::Catalogue()
{
	TArray<TPair<FString, FString>> Out;
	AddOneof(::nightfall::v1::ClientMessage::descriptor(), "intent", TEXT("intents"), Out);
	AddOneof(::nightfall::v1::ServerMessage::descriptor(), "payload", TEXT("payloads"), Out);
	AddOneof(::nightfall::v1::WorldEvent::descriptor(), "event", TEXT("events"), Out);
	const auto* Reasons = ::nightfall::v1::RejectReason_descriptor();
	for (int32 I = 0; I < Reasons->value_count(); ++I) Out.Emplace(TEXT("reasons"), Text(Reasons->value(I)->name()));
	return Out;
}

TMap<FString, TMap<FString, int32>> NightfallContractWire::Numbers()
{
	TMap<FString, TMap<FString, int32>> Out;
	auto Add = [&Out](const google::protobuf::Descriptor* Descriptor, const char* OneofName, const TCHAR* Group)
	{
		const auto* Oneof = Descriptor->FindOneofByName(OneofName);
		for (int32 I = 0; I < Oneof->field_count(); ++I)
			Out.FindOrAdd(Group).Add(Text(Oneof->field(I)->name()), Oneof->field(I)->number());
	};
	Add(::nightfall::v1::ClientMessage::descriptor(), "intent", TEXT("intents"));
	Add(::nightfall::v1::ServerMessage::descriptor(), "payload", TEXT("payloads"));
	Add(::nightfall::v1::WorldEvent::descriptor(), "event", TEXT("events"));
	const auto* Reasons = ::nightfall::v1::RejectReason_descriptor();
	for (int32 I = 0; I < Reasons->value_count(); ++I)
		Out.FindOrAdd(TEXT("reasons")).Add(Text(Reasons->value(I)->name()), Reasons->value(I)->number());
	return Out;
}

bool NightfallContractWire::Inspect(bool bClient, const TArray<uint8>& Bytes, FNightfallContractObservation& Out)
{
	Out = FNightfallContractObservation();
	if (bClient)
	{
		::nightfall::v1::ClientMessage Message;
		if (!Message.ParseFromArray(Bytes.GetData(), Bytes.Num())) return false;
		InspectOneof(Message, "intent", TEXT("intents"), Out);
		if (Out.MessageType.IsEmpty())
		{
			Out.MessageType = TEXT("nightfall.v1.ClientMessage");
			Out.DecodedFields = Text(Message.ShortDebugString());
		}
		return true;
	}
	::nightfall::v1::ServerMessage Message;
	if (!Message.ParseFromArray(Bytes.GetData(), Bytes.Num())) return false;
	InspectOneof(Message, "payload", TEXT("payloads"), Out);
	if (Message.has_event()) InspectOneof(Message.event(), "event", TEXT("events"), Out);
	if (Message.has_rejected())
	{
		const int32 Reason = static_cast<int32>(Message.rejected().reason());
		const auto* Descriptor = ::nightfall::v1::RejectReason_descriptor()->FindValueByNumber(Reason);
		Out.Cases.Emplace(TEXT("reasons"), Descriptor ? Text(Descriptor->name()) : FString::Printf(TEXT("unknown_%d"), Reason));
	}
	return true;
}
