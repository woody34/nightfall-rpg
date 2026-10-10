#include "ProtoCodec.h"
#include "NightfallWire.h"

// Maps the client-facing FNet* structs to TurboLink's generated nightfall.v1 USTRUCTs. The bytes
// themselves are produced and parsed by protoc-generated code behind NightfallWire.

namespace
{
	FNetVec2 FromGrpc(const FGrpcNightfallV1Position& P)
	{
		return FNetVec2{ P.X, P.Y };
	}

	FEntitySpawn FromGrpc(const FGrpcNightfallV1EntitySpawn& S)
	{
		FEntitySpawn Out;
		Out.EntityId = S.EntityId;
		Out.Name = S.Name;
		Out.Position = FromGrpc(S.Position);
		Out.Kind = static_cast<uint32>(S.Kind);
		Out.SessionGeneration = S.SessionGeneration;
		Out.bCombatant = S.Combatant;
		Out.TemplateId = S.TemplateId;
		Out.LifeIncarnation = S.LifeIncarnation;
		Out.bDead = S.Dead;
		Out.bAttackable = S.Attackable;
		Out.Hp = S.Hp;
		Out.MaxHp = S.MaxHp;
		Out.Level = S.Level;
		Out.Race = static_cast<uint32>(S.Race);
		Out.ClassId = S.ClassId;
		Out.Sex = static_cast<uint32>(S.Sex);
		Out.HairStyle = S.HairStyle;
		Out.HairColor = S.HairColor;
		Out.Face = S.Face;
		Out.StateTick = S.StateTick;
		return Out;
	}

	ENetAttackOutcome FromGrpc(EGrpcNightfallV1AttackOutcome O)
	{
		switch (O)
		{
		case EGrpcNightfallV1AttackOutcome::ATTACK_OUTCOME_MISS: return ENetAttackOutcome::Miss;
		case EGrpcNightfallV1AttackOutcome::ATTACK_OUTCOME_HIT: return ENetAttackOutcome::Hit;
		case EGrpcNightfallV1AttackOutcome::ATTACK_OUTCOME_CRIT: return ENetAttackOutcome::Crit;
		default: return ENetAttackOutcome::Unspecified;
		}
	}

	FEntityMove FromGrpc(const FGrpcNightfallV1EntityMove& M)
	{
		FEntityMove Out;
		Out.EntityId = M.EntityId;
		Out.Position = FromGrpc(M.Position);
		Out.Destination = FromGrpc(M.Destination);
		Out.Speed = M.Speed;
		Out.ServerTimeMs = M.ServerTimeMs;
		Out.Tick = M.Tick;
		return Out;
	}

	FWorldEvent FromGrpc(const FGrpcNightfallV1WorldEvent& E)
	{
		// The oneof's case enum defaults to its first member, so the payload pointer is what says
		// whether a member is actually present.
		FWorldEvent Out;
		const FGrpcNightfallV1WorldEventEvent& Ev = E.Event;
		switch (Ev.EventCase)
		{
		case EGrpcNightfallV1WorldEventEvent::Spawn:
			if (Ev.Spawn.IsValid()) { Out.Spawn = FromGrpc(*Ev.Spawn); }
			break;
		case EGrpcNightfallV1WorldEventEvent::Move:
			if (Ev.Move.IsValid()) { Out.Move = FromGrpc(*Ev.Move); }
			break;
		case EGrpcNightfallV1WorldEventEvent::Despawn:
			if (Ev.Despawn.IsValid()) { Out.Despawn = FEntityDespawn{ Ev.Despawn->EntityId }; }
			break;
		case EGrpcNightfallV1WorldEventEvent::AttackResult:
			if (const FGrpcNightfallV1AttackResult* R = Ev.AttackResult.Get())
			{
				Out.AttackResult = FAttackResult{ R->Attacker, R->Target, R->Tick, FromGrpc(R->Outcome), R->Damage, R->TargetHpAfter, R->TargetIncarnation };
			}
			break;
		case EGrpcNightfallV1WorldEventEvent::EntityDied:
			if (const FGrpcNightfallV1EntityDied* D = Ev.EntityDied.Get())
			{
				Out.EntityDied = FEntityDied{ D->Entity, D->Tick, D->Killer, D->Incarnation };
			}
			break;
		case EGrpcNightfallV1WorldEventEvent::EntityRespawned:
			if (const FGrpcNightfallV1EntityRespawned* R = Ev.EntityRespawned.Get())
			{
				Out.EntityRespawned = FEntityRespawned{ R->Entity, R->Tick, FromGrpc(R->Position), R->Hp, R->Incarnation };
			}
			break;
		case EGrpcNightfallV1WorldEventEvent::StatsChanged:
			if (const FGrpcNightfallV1StatsChanged* S = Ev.StatsChanged.Get())
			{
				Out.StatsChanged = FStatsChanged{ S->Entity, S->Hp, S->MaxHp, S->Mp, S->MaxMp, S->Level, S->Xp, S->Cp, S->MaxCp, S->ClassId, S->Sp, S->TokenTier1Count, S->TokenTier2Count, S->Tick };
			}
			break;
		case EGrpcNightfallV1WorldEventEvent::XpGained:
			if (const FGrpcNightfallV1XpGained* X = Ev.XpGained.Get())
			{
				Out.XpGained = FXpGained{ X->Entity, X->Amount, X->Total };
			}
			break;
		case EGrpcNightfallV1WorldEventEvent::LevelUp:
			if (const FGrpcNightfallV1LevelUp* L = Ev.LevelUp.Get())
			{
				Out.LevelUp = FLevelUp{ L->Entity, L->Level };
			}
			break;
		case EGrpcNightfallV1WorldEventEvent::ClassChanged:
			if (const FGrpcNightfallV1ClassChanged* C = Ev.ClassChanged.Get())
			{
				Out.ClassChanged = FClassChanged{ C->Entity, C->ClassId, C->Tick, C->SessionGeneration };
			}
			break;
		case EGrpcNightfallV1WorldEventEvent::TargetChanged:
			if (const FGrpcNightfallV1TargetChanged* T = Ev.TargetChanged.Get())
			{
				Out.TargetChanged = FTargetChanged{ T->Entity, T->Target };
			}
			break;
		default:
			break;
		}
		return Out;
	}
}

namespace NightfallProto
{
	bool IsStaleSpawn(const FEntitySpawn& Known, const FEntitySpawn& New)
	{
		if (New.SessionGeneration < Known.SessionGeneration) return true;
		if (New.SessionGeneration == Known.SessionGeneration && New.StateTick < Known.StateTick) return true;
		return New.SessionGeneration == Known.SessionGeneration
			&& Known.LifeIncarnation != 0 && New.LifeIncarnation != 0 && New.LifeIncarnation < Known.LifeIncarnation;
	}

	void Encode(const FClientMessage& In, TArray<uint8>& Out)
	{
		FGrpcNightfallV1ClientMessage Msg;
		Msg.Seq = In.Seq;
		if (In.MoveTo.IsSet())
		{
			TSharedPtr<FGrpcNightfallV1MoveToRequest> MoveTo = MakeShared<FGrpcNightfallV1MoveToRequest>();
			MoveTo->Destination.X = In.MoveTo->Destination.X;
			MoveTo->Destination.Y = In.MoveTo->Destination.Y;
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::MoveTo;
			Msg.Intent.MoveTo = MoveTo;
		}
		else if (In.SetTarget.IsSet())
		{
			TSharedPtr<FGrpcNightfallV1SetTargetRequest> Req = MakeShared<FGrpcNightfallV1SetTargetRequest>();
			Req->EntityId = In.SetTarget->EntityId;
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::SetTarget;
			Msg.Intent.SetTarget = Req;
		}
		else if (In.bAttack)
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::Attack;
			Msg.Intent.Attack = MakeShared<FGrpcNightfallV1AttackRequest>();
		}
		else if (In.bStopAttack)
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::StopAttack;
			Msg.Intent.StopAttack = MakeShared<FGrpcNightfallV1StopAttackRequest>();
		}
		else if (In.bRespawn)
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::Respawn;
			Msg.Intent.Respawn = MakeShared<FGrpcNightfallV1RespawnRequest>();
		}
		NightfallWire::EncodeClientMessage(Msg, Out);
	}

	bool Decode(const uint8* Data, int32 Size, FServerMessage& Out)
	{
		Out = FServerMessage();
		FGrpcNightfallV1ServerMessage Msg;
		if (!NightfallWire::DecodeServerMessage(Data, Size, Msg))
		{
			return false;
		}

		const FGrpcNightfallV1ServerMessagePayload& Payload = Msg.Payload;
		switch (Payload.PayloadCase)
		{
		case EGrpcNightfallV1ServerMessagePayload::Ack:
			if (Payload.Ack.IsValid()) { Out.Ack = FAck{ Payload.Ack->Seq, Payload.Ack->Tick }; }
			break;
		case EGrpcNightfallV1ServerMessagePayload::Event:
			if (Payload.Event.IsValid()) { Out.Event = FromGrpc(*Payload.Event); }
			break;
		case EGrpcNightfallV1ServerMessagePayload::Rejected:
			if (Payload.Rejected.IsValid())
			{
				Out.Rejected = FIntentRejected{ Payload.Rejected->Seq, static_cast<uint32>(Payload.Rejected->Reason), Payload.Rejected->Detail };
			}
			break;
		}
		return true;
	}
}
