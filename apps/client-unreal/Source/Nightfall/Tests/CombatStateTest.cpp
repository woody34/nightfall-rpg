#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "IWebSocket.h"
#include "Combat/CombatStateSubsystem.h"
#include "Game/NightfallCharacter.h"
#include "NightfallPlayerController.h"
#include "Net/NetClientSubsystem.h"
#include "UI/NightfallHud.h"
#include "Blueprint/UserWidget.h"
#include "Blueprint/WidgetTree.h"
#include "Engine/World.h"

#if WITH_DEV_AUTOMATION_TESTS

// E5.2 / E5.3 without a server: typed events are fed through UNetClientSubsystem::DispatchServerMessage
// (the same path decoded frames take) and every HUD-facing field is asserted. Wire bytes are
// hand-assembled protobuf, so a field-number mistake in the codec shows up here too.

namespace
{
	constexpr EAutomationTestFlags CombatTestFlags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::EngineFilter;

	const TCHAR* const OwnId = TEXT("0b6e2f6e-0000-4000-8000-000000000001");
	const TCHAR* const Wolf = TEXT("0b6e2f6e-0000-4000-8000-0000000000a1");
	const TCHAR* const Boar = TEXT("0b6e2f6e-0000-4000-8000-0000000000a2");
	const TCHAR* const Other = TEXT("0b6e2f6e-0000-4000-8000-0000000000b1");

	/** Records every frame the client sends. */
	class FRecordingSocket final : public IWebSocket
	{
	public:
		TArray<TArray<uint8>> Sent;
		virtual void Connect() override {}
		virtual void Close(int32 Code, const FString& Reason) override {}
		virtual bool IsConnected() override { return true; }
		virtual void Send(const FString& Data) override {}
		virtual void Send(const void* Data, SIZE_T Size, bool bIsBinary) override
		{
			Sent.Emplace_GetRef().Append(static_cast<const uint8*>(Data), static_cast<int32>(Size));
		}
		virtual void SetTextMessageMemoryLimit(uint64 Limit) override {}
		virtual FWebSocketConnectedEvent& OnConnected() override { return Connected; }
		virtual FWebSocketConnectionErrorEvent& OnConnectionError() override { return ConnectionError; }
		virtual FWebSocketClosedEvent& OnClosed() override { return Closed; }
		virtual FWebSocketMessageEvent& OnMessage() override { return Message; }
		virtual FWebSocketBinaryMessageEvent& OnBinaryMessage() override { return BinaryMessage; }
		virtual FWebSocketRawMessageEvent& OnRawMessage() override { return RawMessage; }
		virtual FWebSocketMessageSentEvent& OnMessageSent() override { return MessageSent; }

		/** The intent field number of frame I (seq is one byte in these tests): 12 SetTarget, 13 Attack, 14 StopAttack, 15 Respawn, 10 MoveTo. */
		int32 IntentField(int32 I) const { return Sent.IsValidIndex(I) && Sent[I].Num() > 2 ? Sent[I][2] >> 3 : 0; }
		int32 Count(int32 Field) const
		{
			int32 N = 0;
			for (int32 I = 0; I < Sent.Num(); ++I) N += IntentField(I) == Field;
			return N;
		}

		FWebSocketConnectedEvent Connected;
		FWebSocketConnectionErrorEvent ConnectionError;
		FWebSocketClosedEvent Closed;
		FWebSocketMessageEvent Message;
		FWebSocketBinaryMessageEvent BinaryMessage;
		FWebSocketRawMessageEvent RawMessage;
		FWebSocketMessageSentEvent MessageSent;
	};

	/** A connected client with a recording socket and the player's own entity id set. */
	struct FCombatRig
	{
		FScopedTestGameInstance Instance;
		UNetClientSubsystem* Net = Instance.Get<UNetClientSubsystem>();
		UCombatStateSubsystem* Combat = Instance.Get<UCombatStateSubsystem>();
		TSharedPtr<FRecordingSocket> Socket;

		FCombatRig()
		{
			Net->SetSocketFactoryForTesting([this](const FWsUpgradeRequest&) -> TSharedRef<IWebSocket>
			{
				Socket = MakeShared<FRecordingSocket>();
				return Socket.ToSharedRef();
			});
			Net->SetSchedulerForTesting([](float, TFunction<void()>) {});
			Net->Connect(TEXT("ws://localhost:3000/ws"), TEXT("ticket"));
			Socket->Connected.Broadcast();
			Net->SetOwnEntityId(OwnId);
		}

		void Event(const FWorldEvent& E)
		{
			FServerMessage M;
			M.Event = E;
			Net->DispatchServerMessage(M);
		}
		void Ack(uint32 Seq) { FServerMessage M; M.Ack = FAck{ Seq, 1 }; Net->DispatchServerMessage(M); }
		void Reject(uint32 Seq, uint32 Reason) { FServerMessage M; M.Rejected = FIntentRejected{ Seq, Reason, TEXT("") }; Net->DispatchServerMessage(M); }

		void Spawn(const TCHAR* Id, const TCHAR* Name, uint32 Kind, uint32 Gen, uint32 Inc, uint32 Hp, uint32 MaxHp, uint32 Level, bool bAttackable, bool bDead = false)
		{
			FEntitySpawn S;
			S.EntityId = Id; S.Name = Name; S.Kind = Kind; S.SessionGeneration = Gen;
			S.bCombatant = true; S.LifeIncarnation = Inc; S.bDead = bDead; S.bAttackable = bAttackable;
			S.Hp = Hp; S.MaxHp = MaxHp; S.Level = Level;
			FWorldEvent E; E.Spawn = S; Event(E);
		}
		void Stats(const TCHAR* Id, uint32 Hp, uint32 MaxHp, uint32 Mp, uint32 MaxMp, uint32 Level)
		{
			FWorldEvent E; E.StatsChanged = FStatsChanged{ Id, Hp, MaxHp, Mp, MaxMp, Level }; Event(E);
		}
		void Xp(const TCHAR* Id, uint64 Amount, uint64 Total) { FWorldEvent E; E.XpGained = FXpGained{ Id, Amount, Total }; Event(E); }
		void Target(const TCHAR* Selector, const TCHAR* Target) { FWorldEvent E; E.TargetChanged = FTargetChanged{ Selector, Target }; Event(E); }
		void Hit(const TCHAR* Attacker, const TCHAR* Target, uint64 Tick, ENetAttackOutcome O, uint32 Damage, uint32 HpAfter, uint32 Inc)
		{
			FWorldEvent E; E.AttackResult = FAttackResult{ Attacker, Target, Tick, O, Damage, HpAfter, Inc }; Event(E);
		}
		void Died(const TCHAR* Id, uint64 Tick, uint32 Inc) { FWorldEvent E; E.EntityDied = FEntityDied{ Id, Tick, TEXT(""), Inc }; Event(E); }
		void Respawned(const TCHAR* Id, uint64 Tick, uint32 Hp) { FWorldEvent E; E.EntityRespawned = FEntityRespawned{ Id, Tick, FNetVec2(), Hp }; Event(E); }
		void Despawn(const TCHAR* Id) { FWorldEvent E; E.Despawn = FEntityDespawn{ Id }; Event(E); }

		/** The player and a wolf (attackable, level 2, 100 HP) in view. */
		void StandardScene()
		{
			Spawn(OwnId, TEXT("Hero"), 1, 1, 0, 300, 300, 3, false);
			Stats(OwnId, 300, 300, 120, 150, 3);
			Spawn(Wolf, TEXT("Wolf"), 2, 1, 1, 100, 100, 2, true);
		}
	};

	/** Minimal protobuf writer for hand-built server frames. */
	struct FPb
	{
		TArray<uint8> B;
		void Varint(uint64 V) { while (V >= 0x80) { B.Add(static_cast<uint8>(V | 0x80)); V >>= 7; } B.Add(static_cast<uint8>(V)); }
		FPb& U(int32 Field, uint64 V) { Varint(static_cast<uint64>(Field) << 3); Varint(V); return *this; }
		FPb& S(int32 Field, const char* Text)
		{
			const int32 Len = FCStringAnsi::Strlen(Text);
			Varint((static_cast<uint64>(Field) << 3) | 2); Varint(Len);
			B.Append(reinterpret_cast<const uint8*>(Text), Len);
			return *this;
		}
		FPb& M(int32 Field, const FPb& Inner)
		{
			Varint((static_cast<uint64>(Field) << 3) | 2); Varint(Inner.B.Num()); B.Append(Inner.B);
			return *this;
		}
	};

	/** ServerMessage{ event{ <field>: inner } } */
	TArray<uint8> EventFrame(int32 WorldEventField, const FPb& Inner)
	{
		FPb Event; Event.M(WorldEventField, Inner);
		FPb Msg; Msg.M(2, Event);
		return Msg.B;
	}
}

// --- Codec -----------------------------------------------------------------------------------

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FCombatCodecEncodeTest, "Nightfall.Combat.Codec.EncodeIntents", CombatTestFlags)

bool FCombatCodecEncodeTest::RunTest(const FString& Parameters)
{
	TArray<uint8> Bytes;
	FClientMessage Msg;
	Msg.Seq = 1;
	Msg.SetTarget = FSetTargetIntent{ TEXT("e") };
	NightfallProto::Encode(Msg, Bytes);
	TestEqual(TEXT("SetTarget: seq=1, set_target(12){entity_id=e}"), Bytes, TArray<uint8>({ 0x08, 0x01, 0x62, 0x03, 0x0a, 0x01, 'e' }));

	Msg = FClientMessage(); Msg.Seq = 2; Msg.bAttack = true;
	NightfallProto::Encode(Msg, Bytes);
	TestEqual(TEXT("Attack: attack(13) empty"), Bytes, TArray<uint8>({ 0x08, 0x02, 0x6a, 0x00 }));

	Msg = FClientMessage(); Msg.Seq = 3; Msg.bStopAttack = true;
	NightfallProto::Encode(Msg, Bytes);
	TestEqual(TEXT("StopAttack: stop_attack(14) empty"), Bytes, TArray<uint8>({ 0x08, 0x03, 0x72, 0x00 }));

	Msg = FClientMessage(); Msg.Seq = 4; Msg.bRespawn = true;
	NightfallProto::Encode(Msg, Bytes);
	TestEqual(TEXT("Respawn: respawn(15) empty"), Bytes, TArray<uint8>({ 0x08, 0x04, 0x7a, 0x00 }));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FCombatCodecDecodeTest, "Nightfall.Combat.Codec.DecodeEvents", CombatTestFlags)

bool FCombatCodecDecodeTest::RunTest(const FString& Parameters)
{
	FServerMessage Msg;
	auto Decode = [&](const TArray<uint8>& Frame) { return NightfallProto::Decode(Frame.GetData(), Frame.Num(), Msg) && Msg.Event.IsSet(); };

	FPb Hit; Hit.S(1, "a").S(2, "t").U(3, 5).U(4, 3).U(5, 7).U(6, 9).U(7, 2);
	if (TestTrue(TEXT("AttackResult decodes"), Decode(EventFrame(4, Hit))) && TestTrue(TEXT("set"), Msg.Event->AttackResult.IsSet()))
	{
		const FAttackResult& R = *Msg.Event->AttackResult;
		TestEqual(TEXT("attacker"), R.Attacker, FString(TEXT("a")));
		TestEqual(TEXT("target"), R.Target, FString(TEXT("t")));
		TestEqual(TEXT("tick"), R.Tick, uint64(5));
		TestTrue(TEXT("outcome CRIT"), R.Outcome == ENetAttackOutcome::Crit);
		TestEqual(TEXT("damage"), R.Damage, 7u);
		TestEqual(TEXT("hp after"), R.TargetHpAfter, 9u);
		TestEqual(TEXT("incarnation"), R.TargetIncarnation, 2u);
	}

	FPb Died; Died.S(1, "d").U(2, 11).S(3, "k").U(4, 3);
	if (TestTrue(TEXT("EntityDied decodes"), Decode(EventFrame(5, Died))) && TestTrue(TEXT("set"), Msg.Event->EntityDied.IsSet()))
	{
		TestEqual(TEXT("entity"), Msg.Event->EntityDied->Entity, FString(TEXT("d")));
		TestEqual(TEXT("tick"), Msg.Event->EntityDied->Tick, uint64(11));
		TestEqual(TEXT("killer"), Msg.Event->EntityDied->Killer, FString(TEXT("k")));
		TestEqual(TEXT("incarnation"), Msg.Event->EntityDied->Incarnation, 3u);
	}

	FPb Pos; Pos.B = { 0x0d, 0x00, 0x00, 0x80, 0x3f };   // x = 1.0
	FPb Resp; Resp.S(1, "r").U(2, 12).M(3, Pos).U(4, 195);
	if (TestTrue(TEXT("EntityRespawned decodes"), Decode(EventFrame(6, Resp))) && TestTrue(TEXT("set"), Msg.Event->EntityRespawned.IsSet()))
	{
		TestEqual(TEXT("tick"), Msg.Event->EntityRespawned->Tick, uint64(12));
		TestEqual(TEXT("x"), Msg.Event->EntityRespawned->Position.X, 1.f);
		TestEqual(TEXT("hp"), Msg.Event->EntityRespawned->Hp, 195u);
	}

	FPb Stats; Stats.S(1, "o").U(2, 10).U(3, 20).U(4, 5).U(5, 8).U(6, 3);
	if (TestTrue(TEXT("StatsChanged decodes"), Decode(EventFrame(7, Stats))) && TestTrue(TEXT("set"), Msg.Event->StatsChanged.IsSet()))
	{
		const FStatsChanged& S = *Msg.Event->StatsChanged;
		TestEqual(TEXT("hp/max/mp/max/level"), FString::Printf(TEXT("%u/%u/%u/%u/%u"), S.Hp, S.MaxHp, S.Mp, S.MaxMp, S.Level), FString(TEXT("10/20/5/8/3")));
	}

	FPb Xp; Xp.S(1, "o").U(2, 40).U(3, 1040);
	if (TestTrue(TEXT("XpGained decodes"), Decode(EventFrame(8, Xp))) && TestTrue(TEXT("set"), Msg.Event->XpGained.IsSet()))
	{
		TestEqual(TEXT("amount"), Msg.Event->XpGained->Amount, uint64(40));
		TestEqual(TEXT("total"), Msg.Event->XpGained->Total, uint64(1040));
	}

	FPb Lvl; Lvl.S(1, "o").U(2, 4);
	if (TestTrue(TEXT("LevelUp decodes"), Decode(EventFrame(9, Lvl))) && TestTrue(TEXT("set"), Msg.Event->LevelUp.IsSet()))
	{
		TestEqual(TEXT("level"), Msg.Event->LevelUp->Level, 4u);
	}

	FPb Tgt; Tgt.S(1, "o").S(2, "t");
	if (TestTrue(TEXT("TargetChanged decodes"), Decode(EventFrame(10, Tgt))) && TestTrue(TEXT("set"), Msg.Event->TargetChanged.IsSet()))
	{
		TestEqual(TEXT("target"), Msg.Event->TargetChanged->Target, FString(TEXT("t")));
	}

	FPb Spawn; Spawn.S(1, "n").S(2, "Wolf").U(4, 2).U(5, 2).U(6, 1).S(7, "keltir").U(8, 3).U(10, 1).U(11, 50).U(12, 100).U(13, 5);
	if (TestTrue(TEXT("EntitySpawn combat fields decode"), Decode(EventFrame(1, Spawn))) && TestTrue(TEXT("set"), Msg.Event->Spawn.IsSet()))
	{
		const FEntitySpawn& S = *Msg.Event->Spawn;
		TestTrue(TEXT("combatant"), S.bCombatant);
		TestEqual(TEXT("template"), S.TemplateId, FString(TEXT("keltir")));
		TestEqual(TEXT("incarnation"), S.LifeIncarnation, 3u);
		TestFalse(TEXT("not dead"), S.bDead);
		TestTrue(TEXT("attackable"), S.bAttackable);
		TestEqual(TEXT("hp/max/level"), FString::Printf(TEXT("%u/%u/%u"), S.Hp, S.MaxHp, S.Level), FString(TEXT("50/100/5")));
	}
	return true;
}

// --- Projection and HUD model ----------------------------------------------------------------

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FCombatProjectionTest, "Nightfall.Combat.State.HudFields", CombatTestFlags)

bool FCombatProjectionTest::RunTest(const FString& Parameters)
{
	FCombatRig Rig;
	UCombatStateSubsystem* C = Rig.Combat;
	int32 Numbers = 0;
	TArray<FDamageNumber> Seen;
	C->OnDamageNumber.AddLambda([&](const FDamageNumber& N) { ++Numbers; Seen.Add(N); });

	TestFalse(TEXT("nothing known before the spawn"), C->BuildHudModel().bOwnKnown);
	Rig.StandardScene();

	FCombatHudModel M = C->BuildHudModel();
	TestTrue(TEXT("own known"), M.bOwnKnown);
	TestEqual(TEXT("own name"), M.OwnName, FString(TEXT("Hero")));
	TestEqual(TEXT("level text"), M.LevelText, FString(TEXT("Lv 3")));
	TestEqual(TEXT("own hp text"), M.OwnHpText, FString(TEXT("300 / 300")));
	TestEqual(TEXT("own hp fraction"), M.OwnHpFraction, 1.f);
	TestEqual(TEXT("own mp text"), M.OwnMpText, FString(TEXT("MP 120 / 150")));
	TestEqual(TEXT("own mp fraction"), M.OwnMpFraction, 0.8f);
	TestEqual(TEXT("xp unknown until XpGained"), M.XpText, FString(TEXT("XP --")));
	TestFalse(TEXT("no target frame"), M.bTargetVisible);
	TestFalse(TEXT("not dead"), M.bDeadOverlay);

	Rig.Target(OwnId, Wolf);
	M = C->BuildHudModel();
	TestTrue(TEXT("target frame shown"), M.bTargetVisible);
	TestEqual(TEXT("target name"), M.TargetName, FString(TEXT("Wolf")));
	TestEqual(TEXT("target level"), M.TargetLevel, 2u);
	TestEqual(TEXT("target hp text"), M.TargetHpText, FString(TEXT("100 / 100")));

	// The server says 30 damage; the client shows what it was told, it computes nothing.
	Rig.Hit(OwnId, Wolf, 10, ENetAttackOutcome::Hit, 30, 70, 1);
	M = C->BuildHudModel();
	TestEqual(TEXT("target hp after hit"), M.TargetHpText, FString(TEXT("70 / 100")));
	TestEqual(TEXT("target hp fraction"), M.TargetHpFraction, 0.7f);
	TestEqual(TEXT("one number"), Numbers, 1);
	TestEqual(TEXT("number damage"), Seen[0].Damage, 30u);
	TestTrue(TEXT("number outcome"), Seen[0].Outcome == ENetAttackOutcome::Hit);
	TestFalse(TEXT("not on own entity"), Seen[0].bTargetIsOwn);

	Rig.Hit(OwnId, Wolf, 20, ENetAttackOutcome::Miss, 0, 70, 1);
	Rig.Hit(OwnId, Wolf, 30, ENetAttackOutcome::Crit, 55, 15, 1);
	TestEqual(TEXT("three numbers"), Numbers, 3);
	TestTrue(TEXT("miss styled"), Seen[1].Outcome == ENetAttackOutcome::Miss && Seen[1].Damage == 0);
	TestTrue(TEXT("crit styled"), Seen[2].Outcome == ENetAttackOutcome::Crit && Seen[2].Damage == 55);
	TestEqual(TEXT("wolf hp 15"), C->FindEntity(Wolf)->Hp, 15u);

	// A replayed event (same tick+attacker+target) changes nothing and shows no second number.
	Rig.Hit(OwnId, Wolf, 30, ENetAttackOutcome::Crit, 55, 15, 1);
	Rig.Hit(OwnId, Wolf, 10, ENetAttackOutcome::Hit, 30, 70, 1);
	TestEqual(TEXT("replays add no numbers"), Numbers, 3);
	TestEqual(TEXT("replay does not undo hp"), C->FindEntity(Wolf)->Hp, 15u);

	// The wolf bites back: the player's own HP follows the event, and the number is flagged.
	Rig.Hit(Wolf, OwnId, 12, ENetAttackOutcome::Hit, 25, 275, 0);
	TestEqual(TEXT("own hp"), C->BuildHudModel().OwnHpText, FString(TEXT("275 / 300")));
	TestTrue(TEXT("number is on the own entity"), Seen.Last().bTargetIsOwn);

	// Kill: the wolf dies, the target frame clears, XP and level arrive for the owner only.
	Rig.Hit(OwnId, Wolf, 40, ENetAttackOutcome::Hit, 15, 0, 1);
	Rig.Died(Wolf, 40, 1);
	TestTrue(TEXT("wolf dead"), C->FindEntity(Wolf)->bDead);
	TestEqual(TEXT("wolf hp 0"), C->FindEntity(Wolf)->Hp, 0u);
	TestTrue(TEXT("target cleared on death"), C->GetTargetId().IsEmpty());
	TestFalse(TEXT("target frame gone"), C->BuildHudModel().bTargetVisible);
	TestFalse(TEXT("dead wolf is not attackable"), C->IsAttackable(Wolf));

	Rig.Xp(OwnId, 40, 1040);
	TestEqual(TEXT("xp text"), C->BuildHudModel().XpText, FString(TEXT("XP 1040")));
	TestEqual(TEXT("last gain"), C->GetOwn().LastXpGain, uint64(40));
	{ FWorldEvent E; E.LevelUp = FLevelUp{ OwnId, 4 }; Rig.Event(E); }
	TestEqual(TEXT("level up"), C->BuildHudModel().LevelText, FString(TEXT("Lv 4")));
	Rig.Stats(OwnId, 320, 320, 130, 160, 4);
	TestEqual(TEXT("stats follow"), C->BuildHudModel().OwnHpText, FString(TEXT("320 / 320")));

	// Own death: overlay up, target cleared; respawn clears it.
	Rig.Target(OwnId, Boar);
	Rig.Died(OwnId, 50, 0);
	TestTrue(TEXT("dead overlay"), C->BuildHudModel().bDeadOverlay);
	TestTrue(TEXT("own death clears the target"), C->GetTargetId().IsEmpty());
	TestEqual(TEXT("own hp 0"), C->FindOwnEntity()->Hp, 0u);
	Rig.Respawned(OwnId, 60, 208);
	M = C->BuildHudModel();
	TestFalse(TEXT("overlay cleared by EntityRespawned"), M.bDeadOverlay);
	TestEqual(TEXT("respawn hp"), M.OwnHpText, FString(TEXT("208 / 320")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FCombatPrivacyTest, "Nightfall.Combat.State.OwnerPrivacy", CombatTestFlags)

bool FCombatPrivacyTest::RunTest(const FString& Parameters)
{
	FCombatRig Rig;
	UCombatStateSubsystem* C = Rig.Combat;
	Rig.StandardScene();
	Rig.Spawn(Other, TEXT("Rival"), 1, 1, 0, 200, 200, 5, false);

	// Owner-only facts about someone else must not touch our state.
	Rig.Xp(Other, 999, 5000);
	TestFalse(TEXT("another player's XP is not ours"), C->GetOwn().bXpKnown);
	Rig.Stats(Other, 1, 2, 3, 4, 9);
	TestEqual(TEXT("another player's stats ignored (hp)"), C->FindEntity(Other)->Hp, 200u);
	TestEqual(TEXT("another player's stats ignored (level)"), C->FindEntity(Other)->Level, 5u);
	TestEqual(TEXT("our MP is untouched"), C->GetOwn().Mp, 120u);
	Rig.Target(Other, Wolf);
	TestTrue(TEXT("another player's selection is not our target"), C->GetTargetId().IsEmpty());

	// A player's public HP is visible; their MP and XP are not representable at all.
	Rig.Hit(Wolf, Other, 5, ENetAttackOutcome::Hit, 20, 180, 0);
	TestEqual(TEXT("public hp of others follows AttackResult"), C->FindEntity(Other)->Hp, 180u);

	Rig.Xp(OwnId, 10, 10);
	TestEqual(TEXT("own xp accepted"), C->BuildHudModel().XpText, FString(TEXT("XP 10")));
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FCombatStaleTest, "Nightfall.Combat.State.AoiAndStale", CombatTestFlags)

bool FCombatStaleTest::RunTest(const FString& Parameters)
{
	FCombatRig Rig;
	UCombatStateSubsystem* C = Rig.Combat;
	Rig.StandardScene();

	// AOI exit removes the entity; selecting it first, the target clears with it.
	Rig.Target(OwnId, Wolf);
	TestEqual(TEXT("targeting the wolf"), C->GetTargetId(), FString(Wolf));
	Rig.Despawn(Wolf);
	TestNull(TEXT("wolf gone"), C->FindEntity(Wolf));
	TestTrue(TEXT("despawn clears the target"), C->GetTargetId().IsEmpty());
	TestFalse(TEXT("no target frame"), C->BuildHudModel().bTargetVisible);
	TestNull(TEXT("net cache dropped it too"), Rig.Net->GetKnownEntities().Find(Wolf));

	// AOI entry: a fresh spawn carries its current public state.
	Rig.Spawn(Wolf, TEXT("Wolf"), 2, 1, 1, 60, 100, 2, true);
	TestEqual(TEXT("re-entry hp from the spawn"), C->FindEntity(Wolf)->Hp, 60u);
	TestTrue(TEXT("attackable again"), C->IsAttackable(Wolf));

	// Stale incarnation: a swing that landed on an earlier life is ignored, with no number.
	int32 Numbers = 0;
	C->OnDamageNumber.AddLambda([&](const FDamageNumber&) { ++Numbers; });
	Rig.Despawn(Wolf);
	Rig.Spawn(Wolf, TEXT("Wolf"), 2, 1, 2, 100, 100, 2, true);   // second life
	Rig.Hit(OwnId, Wolf, 70, ENetAttackOutcome::Hit, 99, 1, 1);  // landed on life 1
	TestEqual(TEXT("stale incarnation hit ignored"), C->FindEntity(Wolf)->Hp, 100u);
	TestEqual(TEXT("and shows no number"), Numbers, 0);
	Rig.Died(Wolf, 71, 1);
	TestFalse(TEXT("stale incarnation death ignored"), C->FindEntity(Wolf)->bDead);
	Rig.Hit(OwnId, Wolf, 72, ENetAttackOutcome::Hit, 10, 90, 2);
	TestEqual(TEXT("current incarnation applies"), C->FindEntity(Wolf)->Hp, 90u);
	Rig.Hit(OwnId, Wolf, 60, ENetAttackOutcome::Hit, 50, 40, 2);   // older than the newest fact
	TestEqual(TEXT("older tick ignored"), C->FindEntity(Wolf)->Hp, 90u);

	// A spawn older than a despawned entity's last one is still stale (the AOI exit forgot nothing).
	Rig.Spawn(Boar, TEXT("Boar"), 2, 5, 3, 80, 80, 2, true);
	Rig.Despawn(Boar);
	Rig.Spawn(Boar, TEXT("OldBoar"), 2, 4, 3, 80, 80, 2, true);
	TestNull(TEXT("stale generation after despawn ignored"), C->FindEntity(Boar));
	TestNull(TEXT("and not cached by the net layer"), Rig.Net->GetKnownEntities().Find(Boar));
	Rig.Spawn(Boar, TEXT("NewBoar"), 2, 5, 4, 80, 80, 2, true);
	TestNotNull(TEXT("newer life after despawn accepted"), C->FindEntity(Boar));

	// Stale spawns: a lower session generation or an earlier life never overwrites a newer one,
	// here or in the net cache the world proxies read from.
	Rig.Spawn(Other, TEXT("Rival"), 1, 3, 0, 200, 200, 5, false);
	Rig.Spawn(Other, TEXT("OldSession"), 1, 2, 0, 1, 1, 1, false);
	TestEqual(TEXT("older generation spawn ignored"), C->FindEntity(Other)->Spawn.Name, FString(TEXT("Rival")));
	TestEqual(TEXT("net cache keeps the newer generation"), Rig.Net->GetKnownEntities()[Other].Name, FString(TEXT("Rival")));
	Rig.Spawn(Other, TEXT("NewSession"), 1, 4, 0, 250, 250, 6, false);
	TestEqual(TEXT("higher generation replaces"), C->FindEntity(Other)->Spawn.Name, FString(TEXT("NewSession")));
	Rig.Spawn(Wolf, TEXT("OldWolf"), 2, 1, 1, 5, 100, 2, true);
	TestEqual(TEXT("earlier life spawn ignored"), C->FindEntity(Wolf)->Spawn.Name, FString(TEXT("Wolf")));
	TestEqual(TEXT("hp not rewound"), C->FindEntity(Wolf)->Hp, 90u);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FCombatReconnectTest, "Nightfall.Combat.State.Reconnect", CombatTestFlags)

bool FCombatReconnectTest::RunTest(const FString& Parameters)
{
	FCombatRig Rig;
	UCombatStateSubsystem* C = Rig.Combat;
	Rig.StandardScene();
	Rig.Xp(OwnId, 40, 1040);
	Rig.Target(OwnId, Wolf);
	TestEqual(TEXT("entities before"), C->NumEntities(), 2);

	// Lose the socket: the old projection must not survive into the next session.
	Rig.Socket->Closed.Broadcast(1006, TEXT("dropped"), false);
	TestEqual(TEXT("projection dropped on disconnect"), C->NumEntities(), 0);
	TestTrue(TEXT("target dropped"), C->GetTargetId().IsEmpty());
	TestFalse(TEXT("no stale HUD"), C->BuildHudModel().bOwnKnown);

	// The server re-sends spawns (own state included) and StatsChanged on the new connection.
	Rig.Socket->Connected.Broadcast();
	Rig.Spawn(OwnId, TEXT("Hero"), 1, 2, 0, 250, 300, 3, false);
	Rig.Stats(OwnId, 250, 300, 100, 150, 3);
	Rig.Spawn(Wolf, TEXT("Wolf"), 2, 1, 1, 40, 100, 2, true);
	FCombatHudModel M = C->BuildHudModel();
	TestEqual(TEXT("own hp rebuilt"), M.OwnHpText, FString(TEXT("250 / 300")));
	TestEqual(TEXT("own mp rebuilt"), M.OwnMpText, FString(TEXT("MP 100 / 150")));
	TestEqual(TEXT("xp not carried over: unknown until XpGained"), M.XpText, FString(TEXT("XP --")));
	TestFalse(TEXT("no target until TargetChanged"), M.bTargetVisible);
	TestEqual(TEXT("wolf hp from its spawn"), C->FindEntity(Wolf)->Hp, 40u);
	TestEqual(TEXT("two entities"), C->NumEntities(), 2);

	// A player who reconnects dead sees the overlay straight from the spawn.
	Rig.Socket->Closed.Broadcast(1006, TEXT("dropped"), false);
	Rig.Socket->Connected.Broadcast();
	Rig.Spawn(OwnId, TEXT("Hero"), 1, 3, 0, 0, 300, 3, false, /*bDead=*/true);
	TestTrue(TEXT("dead overlay from the spawn"), C->BuildHudModel().bDeadOverlay);
	return true;
}

// --- Click path (E5.3) -----------------------------------------------------------------------

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FCombatClickPathTest, "Nightfall.Combat.Click.SetTargetAttackOnce", CombatTestFlags)

bool FCombatClickPathTest::RunTest(const FString& Parameters)
{
	FCombatRig Rig;
	UCombatStateSubsystem* C = Rig.Combat;
	Rig.StandardScene();
	Rig.Spawn(Boar, TEXT("Boar"), 2, 1, 1, 80, 80, 2, true);
	Rig.Spawn(Other, TEXT("Rival"), 1, 1, 0, 200, 200, 5, false);
	FRecordingSocket& S = *Rig.Socket;

	TestFalse(TEXT("a player is not a valid attack click"), C->ClickEntity(Other));
	TestFalse(TEXT("an unknown entity is not"), C->ClickEntity(TEXT("nope")));
	TestEqual(TEXT("nothing sent for invalid clicks"), S.Sent.Num(), 0);

	// First click: SetTarget then Attack, in that order.
	TestTrue(TEXT("click accepted"), C->ClickEntity(Wolf));
	if (!TestEqual(TEXT("two frames"), S.Sent.Num(), 2)) return false;
	TestEqual(TEXT("first SetTarget"), S.IntentField(0), 12);
	TestEqual(TEXT("then Attack"), S.IntentField(1), 13);
	TestEqual(TEXT("attack pending"), C->GetAttackState(), EAttackState::Pending);
	TestEqual(TEXT("pending text"), C->BuildHudModel().AttackText, FString(TEXT("Attacking...")));

	// Repeated clicks while the attack is pending or acked send nothing: no way to speed attacks up.
	C->ClickEntity(Wolf);
	C->ClickEntity(Wolf);
	TestEqual(TEXT("pending: repeated clicks send nothing"), S.Sent.Num(), 2);
	Rig.Ack(1);
	Rig.Target(OwnId, Wolf);
	Rig.Ack(2);
	TestEqual(TEXT("attack acked"), C->GetAttackState(), EAttackState::Active);
	C->ClickEntity(Wolf);
	C->ClickEntity(Wolf);
	TestEqual(TEXT("acked: repeated clicks send nothing"), S.Sent.Num(), 2);
	TestEqual(TEXT("SetTarget exactly once"), S.Count(12), 1);
	TestEqual(TEXT("Attack exactly once"), S.Count(13), 1);
	TestEqual(TEXT("active text"), C->BuildHudModel().AttackText, FString(TEXT("Attacking")));

	// A different NPC: SetTarget + Attack once more.
	C->ClickEntity(Boar);
	C->ClickEntity(Boar);
	TestEqual(TEXT("second target: one more SetTarget"), S.Count(12), 2);
	TestEqual(TEXT("second target: one more Attack"), S.Count(13), 2);
	TestEqual(TEXT("four frames in all"), S.Sent.Num(), 4);

	// Overlapping clicks: B then back to A before either is confirmed. The last click wins, and a
	// late echo of the earlier selection must not end the attack queued for the newer one.
	Rig.Target(OwnId, Wolf);   // (confirms the earlier selection of the wolf)
	const int32 Mark = S.Sent.Num();
	C->ClickEntity(Boar);      // SetTarget(boar) + Attack
	C->ClickEntity(Wolf);      // last click is the wolf: SetTarget(wolf) + Attack (the boar attack ended)
	TestEqual(TEXT("overlapping clicks: last click sends its own SetTarget"), S.Count(12), 3);
	Rig.Target(OwnId, Boar);   // echo of the earlier request
	TestEqual(TEXT("echo of an earlier selection keeps the queued attack"), C->GetAttackState(), EAttackState::Pending);
	Rig.Target(OwnId, Wolf);
	TestEqual(TEXT("selection settles on the last click"), C->GetTargetId(), FString(Wolf));
	(void)Mark;

	// Ground click while attacking: StopAttack first, then MoveTo.
	UWorld* World = Rig.Instance.GameInstance->GetWorld();
	if (!TestNotNull(TEXT("test world"), World)) return false;
	ANightfallPlayerController* PC = World->SpawnActor<ANightfallPlayerController>();
	ANightfallCharacter* Pawn = World->SpawnActor<ANightfallCharacter>(FVector(100.0, 100.0, 96.0), FRotator::ZeroRotator);
	PC->Possess(Pawn);
	const int32 Before = S.Sent.Num();
	PC->ClickGroundLocation(FVector(2000.0, 2000.0, 0.0));
	if (!TestEqual(TEXT("StopAttack + MoveTo"), S.Sent.Num(), Before + 2)) return false;
	TestEqual(TEXT("StopAttack first"), S.IntentField(Before), 14);
	TestEqual(TEXT("then MoveTo"), S.IntentField(Before + 1), 10);
	TestEqual(TEXT("attack ended locally"), C->GetAttackState(), EAttackState::Idle);

	// Ground click while idle: only MoveTo.
	PC->ClickGroundLocation(FVector(3000.0, 3000.0, 0.0));
	TestEqual(TEXT("idle ground click: just MoveTo"), S.Sent.Num(), Before + 3);
	TestEqual(TEXT("MoveTo"), S.IntentField(Before + 2), 10);
	TestEqual(TEXT("one StopAttack in all"), S.Count(14), 1);

	// With the selection still the boar and the attack ended, a click re-sends Attack only.
	Rig.Target(OwnId, Boar);
	const int32 BeforeRe = S.Sent.Num();
	C->ClickEntity(Boar);
	TestEqual(TEXT("re-attack sends Attack only"), S.Sent.Num(), BeforeRe + 1);
	TestEqual(TEXT("Attack"), S.IntentField(BeforeRe), 13);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FCombatRejectionTest, "Nightfall.Combat.Click.RejectionsAndDeath", CombatTestFlags)

bool FCombatRejectionTest::RunTest(const FString& Parameters)
{
	FCombatRig Rig;
	UCombatStateSubsystem* C = Rig.Combat;
	Rig.StandardScene();
	FRecordingSocket& S = *Rig.Socket;

	// Attack rejected (server not ready): the pending state clears and the reason is shown.
	C->ClickEntity(Wolf);
	Rig.Ack(1);
	Rig.Reject(2, 12);
	TestEqual(TEXT("rejected attack is idle again"), C->GetAttackState(), EAttackState::Idle);
	TestEqual(TEXT("status shows the reason"), C->GetStatusLine(), FString(TEXT("Can't attack: not available yet")));
	Rig.Target(OwnId, Wolf);
	C->ClickEntity(Wolf);
	TestEqual(TEXT("a click after the rejection retries Attack"), S.Count(13), 2);
	TestEqual(TEXT("without a second SetTarget"), S.Count(12), 1);

	// SetTarget rejected: the pending selection clears, so the next click sends SetTarget again.
	Rig.Spawn(Boar, TEXT("Boar"), 2, 1, 1, 80, 80, 2, true);
	C->ClickEntity(Boar);                     // SetTarget seq 4, Attack seq 5
	Rig.Reject(4, 9);
	Rig.Reject(5, 3);
	TestEqual(TEXT("status for the rejected target"), C->GetStatusLine(), FString(TEXT("Can't attack: your character is not in the zone")));
	const int32 SetTargets = S.Count(12);
	C->ClickEntity(Boar);
	TestEqual(TEXT("SetTarget resent after rejection"), S.Count(12), SetTargets + 1);

	// A target that dies mid-attack clears the frame and the attack.
	Rig.Target(OwnId, Wolf);
	Rig.Died(Wolf, 9, 1);
	TestTrue(TEXT("target cleared"), C->GetTargetId().IsEmpty());
	TestEqual(TEXT("attack ended"), C->GetAttackState(), EAttackState::Idle);
	TestFalse(TEXT("clicking the corpse does nothing"), C->ClickEntity(Wolf));

	// Respawn: only while dead, once until answered, overlay cleared by EntityRespawned only.
	TestEqual(TEXT("cannot respawn while alive"), C->RequestRespawn(), 0u);
	Rig.Died(OwnId, 20, 0);
	TestTrue(TEXT("overlay"), C->BuildHudModel().bDeadOverlay);
	TestFalse(TEXT("dead player cannot click-attack"), C->ClickEntity(Boar));
	const int32 Respawns = S.Count(15);
	const uint32 Seq = C->RequestRespawn();
	TestTrue(TEXT("Respawn sent"), Seq != 0);
	TestEqual(TEXT("second press sends nothing"), C->RequestRespawn(), 0u);
	TestEqual(TEXT("exactly one Respawn"), S.Count(15), Respawns + 1);
	TestTrue(TEXT("pending shown"), C->BuildHudModel().bRespawnPending);
	Rig.Reject(Seq, 12);
	TestTrue(TEXT("overlay stays after rejection"), C->BuildHudModel().bDeadOverlay);
	TestEqual(TEXT("status"), C->GetStatusLine(), FString(TEXT("Can't respawn: not available yet")));
	TestTrue(TEXT("can press again"), C->RequestRespawn() != 0);
	Rig.Respawned(OwnId, 30, 195);
	TestFalse(TEXT("overlay cleared by EntityRespawned"), C->BuildHudModel().bDeadOverlay);
	TestFalse(TEXT("respawn no longer pending"), C->IsRespawnPending());
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FCombatHudWidgetTest, "Nightfall.Combat.Hud.WidgetBuilds", CombatTestFlags)

bool FCombatHudWidgetTest::RunTest(const FString& Parameters)
{
	FScopedTestGameInstance Instance;
	UWorld* World = Instance.GameInstance->GetWorld();
	if (!TestNotNull(TEXT("test world"), World)) return false;
	// In the game NativeOnInitialized builds the layout (it needs a local player); here, directly.
	UNightfallHud* Hud = CreateWidget<UNightfallHud>(World, UNightfallHud::StaticClass());
	if (!TestNotNull(TEXT("HUD widget"), Hud)) return false;
	Hud->EnsureLayout();
	TestNotNull(TEXT("layout built in code"), Hud->WidgetTree ? Hud->WidgetTree->RootWidget.Get() : nullptr);
	TestEqual(TEXT("no numbers yet"), Hud->NumDamageNumbers(), 0);
	TestEqual(TEXT("no bars yet"), Hud->NumFloatingBars(), 0);
	return true;
}

#endif
