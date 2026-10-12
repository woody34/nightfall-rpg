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


	FEffectiveStats FromGrpc(const FGrpcNightfallV1EffectiveStats& S)
	{
		FEffectiveStats Out;
		Out.PAtkQ = S.PAtkQ;
		Out.PDefQ = S.PDefQ;
		Out.MAtkQ = S.MAtkQ;
		Out.MDefQ = S.MDefQ;
		Out.AttackSpeed = S.AttackSpeed;
		Out.CastSpeed = S.CastSpeed;
		Out.SpeedQ = S.SpeedQ;
		Out.AccuracyQ = S.AccuracyQ;
		Out.EvasionQ = S.EvasionQ;
		Out.CritPermille = S.CritPermille;
		Out.ShieldDefQ = S.ShieldDefQ;
		Out.ShieldRateQ = S.ShieldRateQ;
		Out.ShieldAngleDegrees = S.ShieldAngleDegrees;
		Out.PhysicalReachMillitiles = S.PhysicalReachMillitiles;
		Out.PhysicalAngleDegrees = S.PhysicalAngleDegrees;

		return Out;
	}

	FAttackStarted FromGrpc(const FGrpcNightfallV1AttackStarted& S)
	{
		FAttackStarted Out;
		Out.Attacker = S.Attacker;
		Out.Target = S.Target;
		Out.Tick = S.Tick;
		Out.ImpactTick = S.ImpactTick;
		Out.ReadyTick = S.ReadyTick;
		Out.TargetIncarnation = S.TargetIncarnation;
		Out.bBaseline = S.Baseline;
		Out.BaselineTick = S.BaselineTick;
		if (S._attacker_incarnation._attacker_incarnationCase == EGrpcNightfallV1AttackStarted_attacker_incarnation::AttackerIncarnation) Out.AttackerIncarnation = S._attacker_incarnation.AttackerIncarnation;
		return Out;
	}

	FAttackCancelled FromGrpc(const FGrpcNightfallV1AttackCancelled& S)
	{
		FAttackCancelled Out;
		Out.Attacker = S.Attacker;
		Out.Target = S.Target;
		Out.Tick = S.Tick;
		Out.Reason = static_cast<uint32>(S.Reason);
		return Out;
	}

	FActiveEffect FromGrpc(const FGrpcNightfallV1ActiveEffect& S)
	{
		FActiveEffect Out;
		Out.SkillId = S.SkillId;
		Out.SkillLevel = S.SkillLevel;
		Out.RemainingTicks = S.RemainingTicks;
		Out.bIsDebuff = S.IsDebuff;
		if (S._caster._casterCase == EGrpcNightfallV1ActiveEffect_caster::Caster) Out.Caster = S._caster.Caster;
		Out.EffectId = S.EffectId;
		if (S._caster_incarnation._caster_incarnationCase == EGrpcNightfallV1ActiveEffect_caster_incarnation::CasterIncarnation) Out.CasterIncarnation = S._caster_incarnation.CasterIncarnation;
		Out.TargetIncarnation = S.TargetIncarnation;
		return Out;
	}

	FSkillCastStarted FromGrpc(const FGrpcNightfallV1SkillCastStarted& S)
	{
		FSkillCastStarted Out;
		Out.Caster = S.Caster;
		if (S._target._targetCase == EGrpcNightfallV1SkillCastStarted_target::Target) Out.Target = S._target.Target;
		Out.SkillId = S.SkillId;
		Out.SkillLevel = S.SkillLevel;
		Out.CastId = S.CastId;
		Out.Tick = S.Tick;
		Out.CompleteTick = S.CompleteTick;
		Out.CasterIncarnation = S.CasterIncarnation;
		if (S._target_incarnation._target_incarnationCase == EGrpcNightfallV1SkillCastStarted_target_incarnation::TargetIncarnation) Out.TargetIncarnation = S._target_incarnation.TargetIncarnation;
		Out.bBaseline = S.Baseline;
		Out.BaselineTick = S.BaselineTick;
		Out.LaunchTick = S.LaunchTick;
		Out.StartTick = S.StartTick;
		return Out;
	}

	FSkillTargetResult FromGrpc(const FGrpcNightfallV1SkillTargetResult& S)
	{
		FSkillTargetResult Out;
		Out.Target = S.Target;
		Out.TargetIncarnation = S.TargetIncarnation;
		Out.Tick = S.Tick;
		Out.ResolutionKind = static_cast<uint32>(S.ResolutionKind);
		Out.Rolled = S.Rolled;
		Out.CpAbsorbed = S.CpAbsorbed;
		Out.HpRemoved = S.HpRemoved;
		Out.Heal = S.Heal;
		Out.Outcome = static_cast<uint32>(S.Outcome);
		Out.TargetHpAfter = S.TargetHpAfter;
		for (const auto& Row : S.AppliedEffectIds) { Out.AppliedEffectIds.Add(Row); }
		return Out;
	}

	FSkillCastResult FromGrpc(const FGrpcNightfallV1SkillCastResult& S)
	{
		FSkillCastResult Out;
		Out.Caster = S.Caster;
		Out.CasterIncarnation = S.CasterIncarnation;
		Out.CastId = S.CastId;
		Out.Tick = S.Tick;
		Out.SkillId = S.SkillId;
		Out.SkillLevel = S.SkillLevel;
		for (const auto& Row : S.Targets) { Out.Targets.Add(FromGrpc(Row)); }
		if (S._effect_id._effect_idCase == EGrpcNightfallV1SkillCastResult_effect_id::EffectId) Out.EffectId = S._effect_id.EffectId;
		return Out;
	}

	FSkillCastCancelled FromGrpc(const FGrpcNightfallV1SkillCastCancelled& S)
	{
		FSkillCastCancelled Out;
		Out.Caster = S.Caster;
		Out.CastId = S.CastId;
		Out.Reason = static_cast<uint32>(S.Reason);
		Out.Tick = S.Tick;
		Out.CasterIncarnation = S.CasterIncarnation;
		return Out;
	}

	FEffectApplied FromGrpc(const FGrpcNightfallV1EffectApplied& S)
	{
		FEffectApplied Out;
		Out.Target = S.Target;
		Out.TargetIncarnation = S.TargetIncarnation;
		if (S._caster._casterCase == EGrpcNightfallV1EffectApplied_caster::Caster) Out.Caster = S._caster.Caster;
		if (S._caster_incarnation._caster_incarnationCase == EGrpcNightfallV1EffectApplied_caster_incarnation::CasterIncarnation) Out.CasterIncarnation = S._caster_incarnation.CasterIncarnation;
		Out.EffectId = S.EffectId;
		Out.SkillId = S.SkillId;
		Out.SkillLevel = S.SkillLevel;
		Out.Pool = static_cast<uint32>(S.Pool);
		Out.Tick = S.Tick;
		Out.EndTick = S.EndTick;
		Out.AbnormalType = S.AbnormalType;
		Out.AbnormalLevel = S.AbnormalLevel;
		Out.StatusFlags = S.StatusFlags;
		Out.bBaseline = S.Baseline;
		Out.BaselineTick = S.BaselineTick;
		Out.AppliedTick = S.AppliedTick;
		return Out;
	}

	FEffectRemoved FromGrpc(const FGrpcNightfallV1EffectRemoved& S)
	{
		FEffectRemoved Out;
		Out.Target = S.Target;
		Out.TargetIncarnation = S.TargetIncarnation;
		Out.EffectId = S.EffectId;
		Out.Reason = static_cast<uint32>(S.Reason);
		Out.Tick = S.Tick;
		return Out;
	}

	FSkillReuse FromGrpc(const FGrpcNightfallV1SkillReuse& S)
	{
		FSkillReuse Out;
		Out.Entity = S.Entity;
		Out.SkillId = S.SkillId;
		Out.ReadyTick = S.ReadyTick;
		Out.Tick = S.Tick;
		Out.Incarnation = S.Incarnation;
		return Out;
	}

	FSkillLearned FromGrpc(const FGrpcNightfallV1SkillLearned& S)
	{
		FSkillLearned Out;
		Out.Entity = S.Entity;
		Out.SkillId = S.SkillId;
		Out.SkillLevel = S.SkillLevel;
		Out.SpCost = S.SpCost;
		Out.SpRemaining = S.SpRemaining;
		Out.IdempotencyKey = S.IdempotencyKey;
		Out.Tick = S.Tick;
		Out.Incarnation = S.Incarnation;
		return Out;
	}

	FPvpStateChanged FromGrpc(const FGrpcNightfallV1PvpStateChanged& S)
	{
		FPvpStateChanged Out;
		Out.Entity = S.Entity;
		Out.FlagUntilTick = S.FlagUntilTick;
		Out.Karma = S.Karma;
		Out.PkCount = S.PkCount;
		Out.PvpCount = S.PvpCount;
		Out.Tick = S.Tick;
		Out.Incarnation = S.Incarnation;
		Out.bBaseline = S.Baseline;
		Out.BaselineTick = S.BaselineTick;
		return Out;
	}

	FResurrectOffer FromGrpc(const FGrpcNightfallV1ResurrectOffer& S)
	{
		FResurrectOffer Out;
		Out.Target = S.Target;
		Out.TargetIncarnation = S.TargetIncarnation;
		Out.OfferId = S.OfferId;
		if (S._caster._casterCase == EGrpcNightfallV1ResurrectOffer_caster::Caster) Out.Caster = S._caster.Caster;
		if (S._caster_incarnation._caster_incarnationCase == EGrpcNightfallV1ResurrectOffer_caster_incarnation::CasterIncarnation) Out.CasterIncarnation = S._caster_incarnation.CasterIncarnation;
		Out.RestorePercentQ = S.RestorePercentQ;
		Out.RecoverableXp = S.RecoverableXp;
		Out.ExpiryTick = S.ExpiryTick;
		Out.State = static_cast<uint32>(S.State);
		Out.Tick = S.Tick;
		Out.bBaseline = S.Baseline;
		Out.BaselineTick = S.BaselineTick;
		return Out;
	}

	FSpGained FromGrpc(const FGrpcNightfallV1SpGained& S)
	{
		FSpGained Out;
		Out.Entity = S.Entity;
		Out.Amount = S.Amount;
		Out.Total = S.Total;
		Out.Tick = S.Tick;
		Out.Incarnation = S.Incarnation;
		return Out;
	}

	FLearnedSkillRank FromGrpc(const FGrpcNightfallV1LearnedSkillRank& S)
	{
		FLearnedSkillRank Out;
		Out.SkillId = S.SkillId;
		Out.SkillLevel = S.SkillLevel;
		return Out;
	}

	FSkillReuseState FromGrpc(const FGrpcNightfallV1SkillReuseState& S)
	{
		FSkillReuseState Out;
		Out.SkillId = S.SkillId;
		Out.ReadyTick = S.ReadyTick;
		return Out;
	}

	FSkillStateSync FromGrpc(const FGrpcNightfallV1SkillStateSync& S)
	{
		FSkillStateSync Out;
		Out.Entity = S.Entity;
		for (const auto& Row : S.Learned) { Out.Learned.Add(FromGrpc(Row)); }
		for (const auto& Row : S.Reuse) { Out.Reuse.Add(FromGrpc(Row)); }
		Out.Sp = S.Sp;
		Out.Karma = S.Karma;
		Out.PkCount = S.PkCount;
		Out.PvpCount = S.PvpCount;
		Out.FlagUntilTick = S.FlagUntilTick;
		Out.DeathPenaltyLevel = S.DeathPenaltyLevel;
		Out.Tick = S.Tick;
		Out.Incarnation = S.Incarnation;
		Out.bBaseline = S.Baseline;
		Out.BaselineTick = S.BaselineTick;
		return Out;
	}

	FVitalsChanged FromGrpc(const FGrpcNightfallV1VitalsChanged& S)
	{
		FVitalsChanged Out;
		Out.Entity = S.Entity;
		Out.Hp = S.Hp;
		Out.MaxHp = S.MaxHp;
		Out.Tick = S.Tick;
		Out.Incarnation = S.Incarnation;
		Out.bBaseline = S.Baseline;
		Out.BaselineTick = S.BaselineTick;
		return Out;
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
		if (S.PendingSwing.IsValid()) Out.PendingSwing = FromGrpc(*S.PendingSwing);
		Out.Race = static_cast<uint32>(S.Race);
		Out.ClassId = S.ClassId;
		Out.Sex = static_cast<uint32>(S.Sex);
		Out.HairStyle = S.HairStyle;
		Out.HairColor = S.HairColor;
		Out.Face = S.Face;
		Out.StateTick = S.StateTick;
		Out.FlagUntilTick = S.FlagUntilTick;
		Out.Karma = S.Karma;
		for (const auto& Row : S.ActiveEffects) { if (Row.IsValid()) Out.ActiveEffects.Add(FromGrpc(*Row)); }
		if (S.PendingCast.IsValid()) Out.PendingCast = FromGrpc(*S.PendingCast);
		Out.PkCount = S.PkCount;
		Out.PvpCount = S.PvpCount;
		Out.StatusFlags = S.StatusFlags;
		Out.VisibleEquipment = S.VisibleEquipment;
		return Out;
	}

	FStatsChanged FromGrpc(const FGrpcNightfallV1StatsChanged& S)
	{
		FStatsChanged Out;
		Out.Entity = S.Entity;
		Out.Hp = S.Hp;
		Out.MaxHp = S.MaxHp;
		Out.Mp = S.Mp;
		Out.MaxMp = S.MaxMp;
		Out.Level = S.Level;
		Out.Xp = S.Xp;
		Out.Cp = S.Cp;
		Out.MaxCp = S.MaxCp;
		Out.ClassId = S.ClassId;
		Out.Sp = S.Sp;
		Out.TokenTier1Count = S.TokenTier1Count;
		Out.TokenTier2Count = S.TokenTier2Count;
		Out.Tick = S.Tick;
		Out.Karma = S.Karma;
		Out.PkCount = S.PkCount;
		Out.PvpCount = S.PvpCount;
		Out.FlagUntilTick = S.FlagUntilTick;
		Out.DeathPenaltyLevel = S.DeathPenaltyLevel;
		if (S.HasEffectiveStats) Out.EffectiveStats = FromGrpc(S.EffectiveStats);
		Out.bBaseline = S.Baseline;
		Out.BaselineTick = S.BaselineTick;
		Out.Incarnation = S.Incarnation;
		if (S.HasItemWeight) Out.ItemWeight = S.ItemWeight;
		if (S.HasItemPenalties) Out.ItemPenalties = S.ItemPenalties;
		Out.InventoryRevision = S.InventoryRevision;
		return Out;
	}

	FTargetChanged FromGrpc(const FGrpcNightfallV1TargetChanged& S)
	{
		FTargetChanged Out;
		Out.Entity = S.Entity;
		Out.Target = S.Target;
		Out.Tick = S.Tick;
		Out.bBaseline = S.Baseline;
		Out.BaselineTick = S.BaselineTick;
		if (S._target_incarnation._target_incarnationCase == EGrpcNightfallV1TargetChanged_target_incarnation::TargetIncarnation) Out.TargetIncarnation = S._target_incarnation.TargetIncarnation;
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
				Out.AttackResult = FAttackResult{ R->Attacker, R->Target, R->Tick, FromGrpc(R->Outcome), R->Damage, R->TargetHpAfter, R->TargetIncarnation, R->CpDamage };
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
				Out.StatsChanged = FromGrpc(*S);
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
				Out.TargetChanged = FromGrpc(*T);
			}
			break;
		case EGrpcNightfallV1WorldEventEvent::AttackStarted:
			if (Ev.AttackStarted.IsValid()) Out.AttackStarted = FromGrpc(*Ev.AttackStarted);
			break;
		case EGrpcNightfallV1WorldEventEvent::AttackCancelled:
			if (Ev.AttackCancelled.IsValid()) Out.AttackCancelled = FromGrpc(*Ev.AttackCancelled);
			break;
		case EGrpcNightfallV1WorldEventEvent::SkillCastStarted:
			if (Ev.SkillCastStarted.IsValid()) Out.SkillCastStarted = FromGrpc(*Ev.SkillCastStarted);
			break;
		case EGrpcNightfallV1WorldEventEvent::SkillCastResult:
			if (Ev.SkillCastResult.IsValid()) Out.SkillCastResult = FromGrpc(*Ev.SkillCastResult);
			break;
		case EGrpcNightfallV1WorldEventEvent::SkillCastCancelled:
			if (Ev.SkillCastCancelled.IsValid()) Out.SkillCastCancelled = FromGrpc(*Ev.SkillCastCancelled);
			break;
		case EGrpcNightfallV1WorldEventEvent::EffectApplied:
			if (Ev.EffectApplied.IsValid()) Out.EffectApplied = FromGrpc(*Ev.EffectApplied);
			break;
		case EGrpcNightfallV1WorldEventEvent::EffectRemoved:
			if (Ev.EffectRemoved.IsValid()) Out.EffectRemoved = FromGrpc(*Ev.EffectRemoved);
			break;
		case EGrpcNightfallV1WorldEventEvent::SkillReuse:
			if (Ev.SkillReuse.IsValid()) Out.SkillReuse = FromGrpc(*Ev.SkillReuse);
			break;
		case EGrpcNightfallV1WorldEventEvent::SkillLearned:
			if (Ev.SkillLearned.IsValid()) Out.SkillLearned = FromGrpc(*Ev.SkillLearned);
			break;
		case EGrpcNightfallV1WorldEventEvent::PvpStateChanged:
			if (Ev.PvpStateChanged.IsValid()) Out.PvpStateChanged = FromGrpc(*Ev.PvpStateChanged);
			break;
		case EGrpcNightfallV1WorldEventEvent::ResurrectOffer:
			if (Ev.ResurrectOffer.IsValid()) Out.ResurrectOffer = FromGrpc(*Ev.ResurrectOffer);
			break;
		case EGrpcNightfallV1WorldEventEvent::SpGained:
			if (Ev.SpGained.IsValid()) Out.SpGained = FromGrpc(*Ev.SpGained);
			break;
		case EGrpcNightfallV1WorldEventEvent::SkillStateSync:
			if (Ev.SkillStateSync.IsValid()) Out.SkillStateSync = FromGrpc(*Ev.SkillStateSync);
			break;
		case EGrpcNightfallV1WorldEventEvent::InventorySync:
			if (Ev.InventorySync.IsValid()) Out.InventorySync = *Ev.InventorySync;
			break;
		case EGrpcNightfallV1WorldEventEvent::InventoryChanged:
			if (Ev.InventoryChanged.IsValid()) Out.InventoryChanged = *Ev.InventoryChanged;
			break;
		case EGrpcNightfallV1WorldEventEvent::EquipmentChanged:
			if (Ev.EquipmentChanged.IsValid()) Out.EquipmentChanged = *Ev.EquipmentChanged;
			break;
		case EGrpcNightfallV1WorldEventEvent::WeightChanged:
			if (Ev.WeightChanged.IsValid()) Out.WeightChanged = *Ev.WeightChanged;
			break;
		case EGrpcNightfallV1WorldEventEvent::PenaltyChanged:
			if (Ev.PenaltyChanged.IsValid()) Out.PenaltyChanged = *Ev.PenaltyChanged;
			break;
		case EGrpcNightfallV1WorldEventEvent::EnchantResult:
			if (Ev.EnchantResult.IsValid()) Out.EnchantResult = *Ev.EnchantResult;
			break;
		case EGrpcNightfallV1WorldEventEvent::RefineResult:
			if (Ev.RefineResult.IsValid()) Out.RefineResult = *Ev.RefineResult;
			break;
		case EGrpcNightfallV1WorldEventEvent::ItemOperationResult:
			if (Ev.ItemOperationResult.IsValid()) Out.ItemOperationResult = *Ev.ItemOperationResult;
			break;
		case EGrpcNightfallV1WorldEventEvent::VitalsChanged:
			if (Ev.VitalsChanged.IsValid()) Out.VitalsChanged = FromGrpc(*Ev.VitalsChanged);
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
			MoveTo->HasDestination = true;
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

		else if (In.bStopMove)
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::StopMove;
			Msg.Intent.StopMove = MakeShared<FGrpcNightfallV1StopMoveRequest>();
		}
		else if (In.CastSkill.IsSet())
		{
			auto Req = MakeShared<FGrpcNightfallV1CastSkillRequest>();
			Req->SkillId = In.CastSkill->SkillId;
			if (In.CastSkill->Target.IsSet()) { Req->_target._targetCase = EGrpcNightfallV1CastSkillRequest_target::Target; Req->_target.Target = *In.CastSkill->Target; }
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::CastSkill;
			Msg.Intent.CastSkill = Req;
		}
		else if (In.CancelCast.IsSet())
		{
			auto Req = MakeShared<FGrpcNightfallV1CancelCastRequest>();
			if (In.CancelCast->CastId.IsSet()) { Req->_cast_id._cast_idCase = EGrpcNightfallV1CancelCastRequest_cast_id::CastId; Req->_cast_id.CastId = *In.CancelCast->CastId; }
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::CancelCast;
			Msg.Intent.CancelCast = Req;
		}
		else if (In.AcceptResurrect.IsSet())
		{
			auto Req = MakeShared<FGrpcNightfallV1AcceptResurrectRequest>();
			Req->OfferId = In.AcceptResurrect->OfferId;
			Req->Accept = In.AcceptResurrect->bAccept;
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::AcceptResurrect;
			Msg.Intent.AcceptResurrect = Req;
		}
		else if (In.EquipItem.IsSet())
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::EquipItem;
			Msg.Intent.EquipItem = MakeShared<FGrpcNightfallV1EquipItemRequest>(*In.EquipItem);
		}
		else if (In.UnequipItem.IsSet())
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::UnequipItem;
			Msg.Intent.UnequipItem = MakeShared<FGrpcNightfallV1UnequipItemRequest>(*In.UnequipItem);
		}
		else if (In.UseItem.IsSet())
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::UseItem;
			Msg.Intent.UseItem = MakeShared<FGrpcNightfallV1UseItemRequest>(*In.UseItem);
		}
		else if (In.DestroyItem.IsSet())
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::DestroyItem;
			Msg.Intent.DestroyItem = MakeShared<FGrpcNightfallV1DestroyItemRequest>(*In.DestroyItem);
		}
		else if (In.EnchantItem.IsSet())
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::EnchantItem;
			Msg.Intent.EnchantItem = MakeShared<FGrpcNightfallV1EnchantItemRequest>(*In.EnchantItem);
		}
		else if (In.ToggleAutoShot.IsSet())
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::ToggleAutoShot;
			Msg.Intent.ToggleAutoShot = MakeShared<FGrpcNightfallV1ToggleAutoShotRequest>(*In.ToggleAutoShot);
		}
		else if (In.RefineItem.IsSet())
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::RefineItem;
			Msg.Intent.RefineItem = MakeShared<FGrpcNightfallV1RefineItemRequest>(*In.RefineItem);
		}
		else if (In.CancelRefine.IsSet())
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::CancelRefine;
			Msg.Intent.CancelRefine = MakeShared<FGrpcNightfallV1CancelRefineRequest>(*In.CancelRefine);
		}
		else if (In.ExchangeSpecialAbility.IsSet())
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::ExchangeSpecialAbility;
			Msg.Intent.ExchangeSpecialAbility = MakeShared<FGrpcNightfallV1ExchangeSpecialAbilityRequest>(*In.ExchangeSpecialAbility);
		}
		else if (In.CrystallizeItem.IsSet())
		{
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::CrystallizeItem;
			Msg.Intent.CrystallizeItem = MakeShared<FGrpcNightfallV1CrystallizeItemRequest>(*In.CrystallizeItem);
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
			if (Payload.Event.IsValid()) { Out.Event = FromGrpc(*Payload.Event); NightfallWire::ItemReceiptBytes(Data,Size,Out.Event->ItemReceiptPayload); }
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
