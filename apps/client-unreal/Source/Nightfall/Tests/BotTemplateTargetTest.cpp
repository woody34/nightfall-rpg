#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "Bot/BotPredicates.h"
#include "Bot/BotSteps.h"
#include "Combat/CombatStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "HAL/IConsoleManager.h"
#include "IWebSocket.h"

#if WITH_DEV_AUTOMATION_TESTS

namespace
{
	constexpr EAutomationTestFlags Flags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter;

	const FString OwnPlayerId = TEXT("00000000-0000-4000-8000-000000000001");
	const FString SentinelUuid1 = TEXT("0b6e2f6e-0000-4000-8000-0000000000s1");
	const FString SentinelUuid2 = TEXT("0b6e2f6e-0000-4000-8000-0000000000s2");
	const FString SentinelUuidRepl = TEXT("0b6e2f6e-0000-4000-8000-0000000000sr");
	const FString KeltirUuid = TEXT("0b6e2f6e-0000-4000-8000-0000000000k1");
	const FString OtherPlayerUuid = TEXT("0b6e2f6e-0000-4000-8000-0000000000p1");
	const FString GuardUuid = TEXT("0b6e2f6e-0000-4000-8000-0000000000g1");

	/** Records every binary frame sent over the socket. */
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

		/** Intent field number of frame I: 12 SetTarget, 13 Attack, 14 StopAttack, 15 Respawn. */
		int32 IntentField(int32 I) const { return Sent.IsValidIndex(I) && Sent[I].Num() > 2 ? Sent[I][2] >> 3 : 0; }

		FWebSocketConnectedEvent Connected;
		FWebSocketConnectionErrorEvent ConnectionError;
		FWebSocketClosedEvent Closed;
		FWebSocketMessageEvent Message;
		FWebSocketBinaryMessageEvent BinaryMessage;
		FWebSocketRawMessageEvent RawMessage;
		FWebSocketMessageSentEvent MessageSent;
	};

	static bool DecodeSetTargetPayload(const TArray<uint8>& Frame, FString& OutTargetId)
	{
		OutTargetId.Reset();
		const uint8* Ptr = Frame.GetData();
		const uint8* End = Ptr + Frame.Num();
		bool bFoundSetTarget = false;

		auto ReadVarint = [](const uint8*& P, const uint8* E, uint64& OutVal) -> bool
		{
			OutVal = 0;
			int32 Shift = 0;
			while (P < E && Shift < 64)
			{
				const uint8 B = *P++;
				OutVal |= static_cast<uint64>(B & 0x7F) << Shift;
				if ((B & 0x80) == 0) return true;
				Shift += 7;
			}
			return false;
		};

		while (Ptr < End)
		{
			uint64 Key = 0;
			if (!ReadVarint(Ptr, End, Key)) return false;
			const uint32 FieldNumber = static_cast<uint32>(Key >> 3);
			const uint32 WireType = static_cast<uint32>(Key & 0x07);

			if (WireType == 0) // varint
			{
				uint64 Val = 0;
				if (!ReadVarint(Ptr, End, Val)) return false;
			}
			else if (WireType == 2) // length-delimited
			{
				uint64 Len = 0;
				if (!ReadVarint(Ptr, End, Len) || Ptr + Len > End) return false;
				if (FieldNumber == 12) // set_target
				{
					bFoundSetTarget = true;
					const uint8* SubPtr = Ptr;
					const uint8* SubEnd = Ptr + Len;
					while (SubPtr < SubEnd)
					{
						uint64 SubKey = 0;
						if (!ReadVarint(SubPtr, SubEnd, SubKey)) return false;
						const uint32 SubField = static_cast<uint32>(SubKey >> 3);
						const uint32 SubWire = static_cast<uint32>(SubKey & 0x07);
						if (SubWire == 2)
						{
							uint64 StrLen = 0;
							if (!ReadVarint(SubPtr, SubEnd, StrLen) || SubPtr + StrLen > SubEnd) return false;
							if (SubField == 1) // entity_id
							{
								const FUTF8ToTCHAR Converted(reinterpret_cast<const ANSICHAR*>(SubPtr), static_cast<int32>(StrLen));
								OutTargetId = FString::ConstructFromPtrSize(Converted.Get(), Converted.Length());
							}
							SubPtr += StrLen;
						}
						else if (SubWire == 0)
						{
							uint64 Dummy = 0;
							if (!ReadVarint(SubPtr, SubEnd, Dummy)) return false;
						}
						else
						{
							return false;
						}
					}
				}
				Ptr += Len;
			}
			else if (WireType == 1) // 64-bit
			{
				if (Ptr + 8 > End) return false;
				Ptr += 8;
			}
			else if (WireType == 5) // 32-bit
			{
				if (Ptr + 4 > End) return false;
				Ptr += 4;
			}
			else
			{
				return false;
			}
		}
		return bFoundSetTarget;
	}

	struct FTemplateTestRig
	{
		FScopedTestGameInstance Instance;
		UNetClientSubsystem* Net = Instance.Get<UNetClientSubsystem>();
		UCombatStateSubsystem* Combat = Instance.Get<UCombatStateSubsystem>();
		TSharedPtr<FRecordingSocket> Socket;
		uint64 Tick = 0;

		FTemplateTestRig()
		{
			Net->SetSocketFactoryForTesting([this](const FWsUpgradeRequest&) -> TSharedRef<IWebSocket>
			{
				Socket = MakeShared<FRecordingSocket>();
				return Socket.ToSharedRef();
			});
			Net->SetSchedulerForTesting([](float, TFunction<void()>) {});
			Net->Connect(TEXT("ws://localhost:3000/ws"), TEXT("ticket"));
			Socket->Connected.Broadcast();
			Net->SetOwnEntityId(OwnPlayerId);

			// Admit own player entity.
			FEntitySpawn OwnSpawn;
			OwnSpawn.EntityId = OwnPlayerId;
			OwnSpawn.Name = TEXT("Hero");
			OwnSpawn.Kind = 1; // player
			OwnSpawn.SessionGeneration = 1;
			OwnSpawn.StateTick = ++Tick;
			OwnSpawn.bCombatant = true;
			OwnSpawn.Hp = 300;
			OwnSpawn.MaxHp = 300;
			OwnSpawn.Level = 3;
			FWorldEvent E;
			E.Spawn = OwnSpawn;
			Event(E);
		}

		void Event(const FWorldEvent& E)
		{
			FServerMessage M;
			M.Event = E;
			Net->DispatchServerMessage(M);
		}

		void SpawnNpc(const FString& Id, const FString& Template, uint32 Incarnation = 1, bool bAttackable = true, bool bDead = false, uint32 Hp = 100, uint32 MaxHp = 100)
		{
			FEntitySpawn S;
			S.EntityId = Id;
			S.Name = Template;
			S.Kind = 2; // NPC
			S.SessionGeneration = 1;
			S.StateTick = ++Tick;
			S.bCombatant = true;
			S.LifeIncarnation = Incarnation;
			S.bDead = bDead;
			S.bAttackable = bAttackable;
			S.Hp = Hp;
			S.MaxHp = MaxHp;
			S.Level = 1;
			S.TemplateId = Template;
			FWorldEvent E;
			E.Spawn = S;
			Event(E);
		}

		void SpawnPlayer(const FString& Id, const FString& Name, const FString& FakeTemplate = TEXT(""))
		{
			FEntitySpawn S;
			S.EntityId = Id;
			S.Name = Name;
			S.Kind = 1; // player
			S.SessionGeneration = 1;
			S.StateTick = ++Tick;
			S.bCombatant = true;
			S.bDead = false;
			S.bAttackable = false;
			S.Hp = 200;
			S.MaxHp = 200;
			S.Level = 2;
			S.TemplateId = FakeTemplate;
			FWorldEvent E;
			E.Spawn = S;
			Event(E);
		}

		void Kill(const FString& Id, uint32 Incarnation = 1)
		{
			FWorldEvent E;
			E.EntityDied = FEntityDied{ Id, ++Tick, TEXT(""), Incarnation };
			Event(E);
		}

		void Despawn(const FString& Id)
		{
			FWorldEvent E;
			E.Despawn = FEntityDespawn{ Id };
			Event(E);
		}

		FBotContext Context() const
		{
			return FBotContext{ Instance.GameInstance, nullptr };
		}

		FBotPredicateValue Eval(const FString& PredicateLine) const
		{
			FString Error;
			TArray<FString> Tokens = FBotPredicateRegistry::Tokenize(PredicateLine);
			FBotPredicateFn Fn = FBotPredicateRegistry::Get().Parse(Tokens, Error);
			if (!Fn) return { false, Error };
			return Fn(Context());
		}

		bool RunConsole(const FString& Command)
		{
			return IConsoleManager::Get().ProcessUserConsoleInput(*Command, *GLog, Instance.GameInstance->GetWorld());
		}
	};
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotTemplateTargetTest, "Nightfall.Bot.TemplateTarget", Flags)

bool FBotTemplateTargetTest::RunTest(const FString& Parameters)
{
	FTemplateTestRig Rig;
	const FBotContext Context = Rig.Context();

	// -------------------------------------------------------------------------------------------
	// 1. Unknown / empty template resolution
	// -------------------------------------------------------------------------------------------
	TestTrue(TEXT("empty template string rejected"), BotPredicates::ResolveTemplateTarget(Context, TEXT("")).IsEmpty());
	TestTrue(TEXT("whitespace template string rejected"), BotPredicates::ResolveTemplateTarget(Context, TEXT("   ")).IsEmpty());
	TestTrue(TEXT("unknown template returns empty"), BotPredicates::ResolveTemplateTarget(Context, TEXT("nonexistent_boss")).IsEmpty());

	FBotPredicateValue UnknownVal = Rig.Eval(TEXT("template_target_available nonexistent_boss"));
	TestFalse(TEXT("unknown template predicate is false"), UnknownVal.bTrue);
	TestEqual(TEXT("unknown template observed is none"), UnknownVal.Observed, FString(TEXT("none")));

	const int32 InitialSentCount = Rig.Socket->Sent.Num();
	Rig.RunConsole(TEXT("nf.Target template:nonexistent_boss"));
	TestEqual(TEXT("unknown template nf.Target sends nothing"), Rig.Socket->Sent.Num(), InitialSentCount);

	Rig.RunConsole(TEXT("nf.Target template:"));
	TestEqual(TEXT("empty template nf.Target sends nothing"), Rig.Socket->Sent.Num(), InitialSentCount);

	// -------------------------------------------------------------------------------------------
	// 2. Unique received UUID resolution and wire SetTarget
	// -------------------------------------------------------------------------------------------
	Rig.SpawnNpc(SentinelUuid1, TEXT("sentinel"), 1, true, false, 150, 150);

	const FString ResolvedSentinel = BotPredicates::ResolveTemplateTarget(Context, TEXT("sentinel"));
	TestEqual(TEXT("resolves unique received UUID exactly"), ResolvedSentinel, SentinelUuid1);

	FBotPredicateValue AvailableVal = Rig.Eval(TEXT("template_target_available sentinel"));
	TestTrue(TEXT("template_target_available is true for admitted living sentinel"), AvailableVal.bTrue);
	TestEqual(TEXT("template_target_available observed matches received UUID"), AvailableVal.Observed, SentinelUuid1);

	// nf.Target template:sentinel sends SetTarget intent for SentinelUuid1.
	Rig.RunConsole(TEXT("nf.Target template:sentinel"));
	TestEqual(TEXT("nf.Target template sends one intent"), Rig.Socket->Sent.Num(), InitialSentCount + 1);
	TestEqual(TEXT("nf.Target template sent SetTarget frame"), Rig.Socket->IntentField(Rig.Socket->Sent.Num() - 1), 12);
	FString DecodedTargetId;
	TestTrue(TEXT("initial SetTarget payload decodes"), DecodeSetTargetPayload(Rig.Socket->Sent.Last(), DecodedTargetId));
	TestEqual(TEXT("SetTarget payload contains initial sentinel UUID"), DecodedTargetId, SentinelUuid1);
	TestFalse(TEXT("sent initial UUID is not empty"), DecodedTargetId.IsEmpty());

	// -------------------------------------------------------------------------------------------
	// 3. Other template and players exclusion
	// -------------------------------------------------------------------------------------------
	// Spawn another NPC with a different template ("keltir").
	Rig.SpawnNpc(KeltirUuid, TEXT("keltir"), 1, true, false, 50, 50);

	// Other template exclusion: sentinel still resolves sentinel, keltir resolves keltir.
	TestEqual(TEXT("sentinel resolution excludes other template"), BotPredicates::ResolveTemplateTarget(Context, TEXT("sentinel")), SentinelUuid1);
	TestEqual(TEXT("keltir resolution excludes sentinel"), BotPredicates::ResolveTemplateTarget(Context, TEXT("keltir")), KeltirUuid);

	// Players exclusion: spawn another player (Kind 1) with template set to "sentinel".
	Rig.SpawnPlayer(OtherPlayerUuid, TEXT("RivalPlayer"), TEXT("sentinel"));

	// Player is excluded; SentinelUuid1 is still the unique admitted NPC.
	TestEqual(TEXT("player with same template excluded"), BotPredicates::ResolveTemplateTarget(Context, TEXT("sentinel")), SentinelUuid1);

	// If the sentinel NPC despawns, the player must NOT be resolved as a template target.
	Rig.Despawn(SentinelUuid1);
	TestTrue(TEXT("player alone is rejected as template target"), BotPredicates::ResolveTemplateTarget(Context, TEXT("sentinel")).IsEmpty());
	TestFalse(TEXT("template_target_available false when only player has template"), Rig.Eval(TEXT("template_target_available sentinel")).bTrue);

	// -------------------------------------------------------------------------------------------
	// 4. Dead / returning or nonattackable exclusion and intermediate lethal AttackResult
	// -------------------------------------------------------------------------------------------
	// Re-spawn sentinel NPC.
	Rig.SpawnNpc(SentinelUuid1, TEXT("sentinel"), 1, true, false, 150, 150);
	TestEqual(TEXT("re-spawned sentinel resolved"), BotPredicates::ResolveTemplateTarget(Context, TEXT("sentinel")), SentinelUuid1);

	// Intermediate state: dispatch a lethal AttackResult that reduces HP to 0 before EntityDied arrives.
	FAttackResult LethalHit;
	LethalHit.Attacker = OwnPlayerId;
	LethalHit.Target = SentinelUuid1;
	LethalHit.Outcome = ENetAttackOutcome::Hit;
	LethalHit.Damage = 150;
	LethalHit.TargetHpAfter = 0;
	LethalHit.TargetIncarnation = 1;
	Rig.Tick = 10;
	LethalHit.Tick = Rig.Tick;
	FWorldEvent LethalEvent;
	LethalEvent.AttackResult = LethalHit;
	Rig.Event(LethalEvent);

	const FCombatEntity* IntermediateCE = Rig.Combat->FindEntity(SentinelUuid1);
	TestTrue(TEXT("intermediate entity found in combat"), IntermediateCE != nullptr);
	if (IntermediateCE)
	{
		TestEqual(TEXT("intermediate HP is 0"), IntermediateCE->Hp, 0u);
		TestFalse(TEXT("intermediate bDead remains false before EntityDied"), IntermediateCE->bDead);
	}

	// Intermediate rejection: resolver, predicate, and console command must reject HP == 0
	TestTrue(TEXT("intermediate HP0 rejected by template resolver"), BotPredicates::ResolveTemplateTarget(Context, TEXT("sentinel")).IsEmpty());
	const FBotPredicateValue IntermediateVal = Rig.Eval(TEXT("template_target_available sentinel"));
	TestFalse(TEXT("template_target_available false during intermediate HP0"), IntermediateVal.bTrue);
	TestEqual(TEXT("template_target_available observed none during intermediate HP0"), IntermediateVal.Observed, FString(TEXT("none")));

	const int32 BeforeIntermediateCmd = Rig.Socket->Sent.Num();
	Rig.RunConsole(TEXT("nf.Target template:sentinel"));
	TestEqual(TEXT("nf.Target template sends nothing during intermediate HP0"), Rig.Socket->Sent.Num(), BeforeIntermediateCmd);

	// Dead exclusion: wire EntityDied fact arrives.
	Rig.Kill(SentinelUuid1, 1);
	const FCombatEntity* DeadSentinel = Rig.Combat->FindEntity(SentinelUuid1);
	TestTrue(TEXT("EntityDied admitted after lethal tick 10"), DeadSentinel && DeadSentinel->bDead && DeadSentinel->LastFactTick == 11);
	TestTrue(TEXT("dead NPC excluded from template resolution after EntityDied"), BotPredicates::ResolveTemplateTarget(Context, TEXT("sentinel")).IsEmpty());
	TestFalse(TEXT("template_target_available false for dead NPC"), Rig.Eval(TEXT("template_target_available sentinel")).bTrue);

	const int32 BeforeDeadTargetSent = Rig.Socket->Sent.Num();
	Rig.RunConsole(TEXT("nf.Target template:sentinel"));
	TestEqual(TEXT("nf.Target template sends nothing when target dead"), Rig.Socket->Sent.Num(), BeforeDeadTargetSent);

	// Recovery after fresh living admission
	Rig.Despawn(SentinelUuid1);
	Rig.SpawnNpc(SentinelUuid1, TEXT("sentinel"), 2, true, false, 150, 150);
	TestEqual(TEXT("resolves living NPC after fresh admission"), BotPredicates::ResolveTemplateTarget(Context, TEXT("sentinel")), SentinelUuid1);
	const FBotPredicateValue LivingVal = Rig.Eval(TEXT("template_target_available sentinel"));
	TestTrue(TEXT("template_target_available true after fresh living admission"), LivingVal.bTrue);
	TestEqual(TEXT("template_target_available observed living UUID"), LivingVal.Observed, SentinelUuid1);

	// Returning or nonattackable exclusion: spawn NPC with bAttackable = false.
	Rig.SpawnNpc(GuardUuid, TEXT("guard"), 1, false, false, 500, 500);
	TestTrue(TEXT("nonattackable or returning NPC excluded"), BotPredicates::ResolveTemplateTarget(Context, TEXT("guard")).IsEmpty());
	TestFalse(TEXT("template_target_available false for nonattackable NPC"), Rig.Eval(TEXT("template_target_available guard")).bTrue);

	Rig.RunConsole(TEXT("nf.Target template:guard"));
	TestEqual(TEXT("nf.Target template sends nothing when nonattackable"), Rig.Socket->Sent.Num(), BeforeDeadTargetSent);

	// Clean up sentinel and guard.
	Rig.Despawn(SentinelUuid1);
	Rig.Despawn(GuardUuid);

	// -------------------------------------------------------------------------------------------
	// 5. Ambiguity rejection
	// -------------------------------------------------------------------------------------------
	// Spawn two living attackable sentinels with distinct UUIDs.
	Rig.SpawnNpc(SentinelUuid1, TEXT("sentinel"), 3, true, false, 150, 150);
	Rig.SpawnNpc(SentinelUuid2, TEXT("sentinel"), 1, true, false, 150, 150);
	const FCombatEntity* AmbiguousSentinel1 = Rig.Combat->FindEntity(SentinelUuid1);
	const FCombatEntity* AmbiguousSentinel2 = Rig.Combat->FindEntity(SentinelUuid2);
	TestTrue(TEXT("first ambiguous NPC is a current living incarnation 3 admission"),
		Rig.Net->GetKnownEntities().Contains(SentinelUuid1) && AmbiguousSentinel1
		&& AmbiguousSentinel1->Incarnation == 3 && !AmbiguousSentinel1->bDead && AmbiguousSentinel1->Hp > 0);
	TestTrue(TEXT("second ambiguous NPC is a current living incarnation 1 admission"),
		Rig.Net->GetKnownEntities().Contains(SentinelUuid2) && AmbiguousSentinel2
		&& AmbiguousSentinel2->Incarnation == 1 && !AmbiguousSentinel2->bDead && AmbiguousSentinel2->Hp > 0);

	// Multiple matching living attackable NPCs must be rejected as ambiguous (no guessing).
	TestTrue(TEXT("ambiguous template matches rejected"), BotPredicates::ResolveTemplateTarget(Context, TEXT("sentinel")).IsEmpty());
	TestFalse(TEXT("template_target_available false when ambiguous"), Rig.Eval(TEXT("template_target_available sentinel")).bTrue);

	const int32 BeforeAmbiguousTargetSent = Rig.Socket->Sent.Num();
	Rig.RunConsole(TEXT("nf.Target template:sentinel"));
	TestEqual(TEXT("nf.Target template sends nothing when ambiguous"), Rig.Socket->Sent.Num(), BeforeAmbiguousTargetSent);

	// Ambiguity clears when one dies: SentinelUuid2 dies.
	Rig.Kill(SentinelUuid2, 1);
	TestEqual(TEXT("ambiguity resolves to single living NPC after death"), BotPredicates::ResolveTemplateTarget(Context, TEXT("sentinel")), SentinelUuid1);
	TestTrue(TEXT("template_target_available true once unambiguous"), Rig.Eval(TEXT("template_target_available sentinel")).bTrue);

	// nf.Target template:sentinel now targets the living SentinelUuid1.
	Rig.RunConsole(TEXT("nf.Target template:sentinel"));
	TestEqual(TEXT("nf.Target template succeeds once unambiguous"), Rig.Socket->Sent.Num(), BeforeAmbiguousTargetSent + 1);
	TestEqual(TEXT("nf.Target template sent SetTarget frame"), Rig.Socket->IntentField(Rig.Socket->Sent.Num() - 1), 12);

	// Clean up dead SentinelUuid2.
	Rig.Despawn(SentinelUuid2);

	// -------------------------------------------------------------------------------------------
	// 6. Despawn and replacement admission IDs
	// -------------------------------------------------------------------------------------------
	// Despawn SentinelUuid1.
	Rig.Despawn(SentinelUuid1);
	TestTrue(TEXT("despawned NPC no longer resolved"), BotPredicates::ResolveTemplateTarget(Context, TEXT("sentinel")).IsEmpty());
	TestFalse(TEXT("template_target_available false after despawn"), Rig.Eval(TEXT("template_target_available sentinel")).bTrue);

	// Replacement admission: a new sentinel NPC spawns with a brand new UUID (SentinelUuidRepl).
	Rig.SpawnNpc(SentinelUuidRepl, TEXT("sentinel"), 2, true, false, 150, 150);

	// Must resolve the new replacement admission UUID, not the old despawned UUID.
	const FString ResolvedRepl = BotPredicates::ResolveTemplateTarget(Context, TEXT("sentinel"));
	TestEqual(TEXT("resolves new replacement admission UUID"), ResolvedRepl, SentinelUuidRepl);
	TestTrue(TEXT("new UUID differs from previous admission"), ResolvedRepl != SentinelUuid1);

	FBotPredicateValue ReplVal = Rig.Eval(TEXT("template_target_available sentinel"));
	TestTrue(TEXT("template_target_available true for replacement admission"), ReplVal.bTrue);
	TestEqual(TEXT("template_target_available observed has replacement UUID"), ReplVal.Observed, SentinelUuidRepl);

	const int32 BeforeReplTargetSent = Rig.Socket->Sent.Num();
	Rig.RunConsole(TEXT("nf.Target template:sentinel"));
	TestEqual(TEXT("nf.Target sends SetTarget for replacement"), Rig.Socket->Sent.Num(), BeforeReplTargetSent + 1);
	TestEqual(TEXT("SetTarget frame sent for replacement"), Rig.Socket->IntentField(Rig.Socket->Sent.Num() - 1), 12);
	FString DecodedReplId;
	TestTrue(TEXT("replacement SetTarget payload decodes"), DecodeSetTargetPayload(Rig.Socket->Sent.Last(), DecodedReplId));
	TestEqual(TEXT("replacement SetTarget transmits replacement UUID"), DecodedReplId, SentinelUuidRepl);
	TestTrue(TEXT("replacement UUID differs from initial UUID"), DecodedReplId != SentinelUuid1);
	TestFalse(TEXT("replacement UUID is not empty"), DecodedReplId.IsEmpty());

	// -------------------------------------------------------------------------------------------
	// 7. Predicate parser validation
	// -------------------------------------------------------------------------------------------
	FString Error;
	const FBotPredicateFn ParsedNoArgs = FBotPredicateRegistry::Get().Parse(FBotPredicateRegistry::Tokenize(TEXT("template_target_available")), Error);
	TestFalse(TEXT("template_target_available without args rejected at parse"), ParsedNoArgs.IsSet());
	TestTrue(TEXT("error mentions usage"), Error.Contains(TEXT("template_target_available <id>")));

	Error.Reset();
	const FBotPredicateFn ParsedMultiArgs = FBotPredicateRegistry::Get().Parse(FBotPredicateRegistry::Tokenize(TEXT("template_target_available id1 id2")), Error);
	TestFalse(TEXT("template_target_available with multiple args rejected at parse"), ParsedMultiArgs.IsSet());

	// -------------------------------------------------------------------------------------------
	// 8. Normal nf.Target arguments and none preserved
	// -------------------------------------------------------------------------------------------
	const int32 BeforeNormalArgs = Rig.Socket->Sent.Num();
	Rig.RunConsole(TEXT("nf.Target none"));
	TestEqual(TEXT("nf.Target none clears target intent"), Rig.Socket->Sent.Num(), BeforeNormalArgs + 1);
	FString DecodedNoneId;
	TestTrue(TEXT("nf.Target none SetTarget payload decodes"), DecodeSetTargetPayload(Rig.Socket->Sent.Last(), DecodedNoneId));
	TestTrue(TEXT("nf.Target none payload entity id is empty"), DecodedNoneId.IsEmpty());

	Rig.RunConsole(*FString::Printf(TEXT("nf.Target %s"), *KeltirUuid));
	TestEqual(TEXT("nf.Target with raw UUID sends SetTarget"), Rig.Socket->Sent.Num(), BeforeNormalArgs + 2);
	FString DecodedRawId;
	TestTrue(TEXT("nf.Target raw UUID SetTarget payload decodes"), DecodeSetTargetPayload(Rig.Socket->Sent.Last(), DecodedRawId));
	TestEqual(TEXT("nf.Target raw UUID transmits exact raw UUID"), DecodedRawId, KeltirUuid);
	TestFalse(TEXT("nf.Target raw UUID is not empty"), DecodedRawId.IsEmpty());

	return true;
}

#endif
