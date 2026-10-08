#include "ProtoCodec.h"

// Minimal protobuf wire-format codec. Only varint (0), 64-bit (1), length-delimited (2), and
// 32-bit (5) wire types are needed. Unknown fields are skipped, as protobuf requires.

namespace
{
	constexpr uint8 WT_VARINT = 0;
	constexpr uint8 WT_FIXED64 = 1;
	constexpr uint8 WT_LEN = 2;
	constexpr uint8 WT_FIXED32 = 5;

	struct FWriter
	{
		TArray<uint8>& Out;
		explicit FWriter(TArray<uint8>& InOut) : Out(InOut) {}

		void Varint(uint64 V)
		{
			while (V >= 0x80) { Out.Add(static_cast<uint8>(V | 0x80)); V >>= 7; }
			Out.Add(static_cast<uint8>(V));
		}
		void Tag(uint32 Field, uint8 Wire) { Varint((static_cast<uint64>(Field) << 3) | Wire); }
		void U32(uint32 Field, uint32 V) { Tag(Field, WT_VARINT); Varint(V); }
		void F32(uint32 Field, float V)
		{
			Tag(Field, WT_FIXED32);
			uint32 Bits; FMemory::Memcpy(&Bits, &V, 4);
			for (int i = 0; i < 4; ++i) Out.Add(static_cast<uint8>(Bits >> (8 * i)));
		}
		void Bytes(uint32 Field, const TArray<uint8>& V)
		{
			Tag(Field, WT_LEN); Varint(V.Num()); Out.Append(V);
		}
	};

	struct FReader
	{
		const uint8* P; const uint8* End;
		FReader(const uint8* Data, int32 Size) : P(Data), End(Data + Size) {}
		bool AtEnd() const { return P >= End; }

		bool Varint(uint64& V)
		{
			V = 0; int Shift = 0;
			while (P < End && Shift < 64)
			{
				const uint8 B = *P++;
				V |= static_cast<uint64>(B & 0x7F) << Shift;
				if (!(B & 0x80)) return true;
				Shift += 7;
			}
			return false;
		}
		bool Tag(uint32& Field, uint8& Wire)
		{
			uint64 K; if (!Varint(K)) return false;
			Field = static_cast<uint32>(K >> 3); Wire = static_cast<uint8>(K & 7); return true;
		}
		bool F32(float& V)
		{
			if (End - P < 4) return false;
			uint32 Bits = 0; for (int i = 0; i < 4; ++i) Bits |= static_cast<uint32>(P[i]) << (8 * i);
			P += 4; FMemory::Memcpy(&V, &Bits, 4); return true;
		}
		bool Len(const uint8*& Start, int32& Size)
		{
			uint64 N; if (!Varint(N) || N > static_cast<uint64>(End - P)) return false;
			Start = P; Size = static_cast<int32>(N); P += N; return true;
		}
		bool Str(FString& S)
		{
			const uint8* B; int32 N; if (!Len(B, N)) return false;
			S = FString(FUTF8ToTCHAR(reinterpret_cast<const ANSICHAR*>(B), N)); return true;
		}
		bool Skip(uint8 Wire)
		{
			switch (Wire)
			{
			case WT_VARINT: { uint64 D; return Varint(D); }
			case WT_FIXED64: if (End - P < 8) return false; P += 8; return true;
			case WT_LEN: { const uint8* B; int32 N; return Len(B, N); }
			case WT_FIXED32: if (End - P < 4) return false; P += 4; return true;
			default: return false;
			}
		}
	};

	bool DecodeVec2(const uint8* D, int32 N, FNetVec2& V)
	{
		FReader R(D, N); uint32 F; uint8 W;
		while (!R.AtEnd())
		{
			if (!R.Tag(F, W)) return false;
			if (F == 1 && W == WT_FIXED32) { if (!R.F32(V.X)) return false; }
			else if (F == 2 && W == WT_FIXED32) { if (!R.F32(V.Y)) return false; }
			else if (!R.Skip(W)) return false;
		}
		return true;
	}

	bool DecodeSpawn(const uint8* D, int32 N, FEntitySpawn& S)
	{
		FReader R(D, N); uint32 F; uint8 W;
		while (!R.AtEnd())
		{
			if (!R.Tag(F, W)) return false;
			if (F == 1 && W == WT_LEN) { if (!R.Str(S.EntityId)) return false; }
			else if (F == 2 && W == WT_LEN) { if (!R.Str(S.Name)) return false; }
			else if (F == 3 && W == WT_LEN) { const uint8* B; int32 M; if (!R.Len(B, M) || !DecodeVec2(B, M, S.Position)) return false; }
			else if (F == 4 && W == WT_VARINT) { uint64 V; if (!R.Varint(V)) return false; S.Kind = static_cast<uint32>(V); }
			else if (!R.Skip(W)) return false;
		}
		return true;
	}

	bool DecodeMove(const uint8* D, int32 N, FEntityMove& M)
	{
		FReader R(D, N); uint32 F; uint8 W;
		while (!R.AtEnd())
		{
			if (!R.Tag(F, W)) return false;
			if (F == 1 && W == WT_LEN) { if (!R.Str(M.EntityId)) return false; }
			else if (F == 2 && W == WT_LEN) { const uint8* B; int32 L; if (!R.Len(B, L) || !DecodeVec2(B, L, M.Position)) return false; }
			else if (F == 3 && W == WT_LEN) { const uint8* B; int32 L; if (!R.Len(B, L) || !DecodeVec2(B, L, M.Destination)) return false; }
			else if (F == 4 && W == WT_FIXED32) { if (!R.F32(M.Speed)) return false; }
			else if (F == 5 && W == WT_VARINT) { uint64 V; if (!R.Varint(V)) return false; M.ServerTimeMs = static_cast<int64>(V); }
			else if (!R.Skip(W)) return false;
		}
		return true;
	}

	bool DecodeDespawn(const uint8* D, int32 N, FEntityDespawn& X)
	{
		FReader R(D, N); uint32 F; uint8 W;
		while (!R.AtEnd())
		{
			if (!R.Tag(F, W)) return false;
			if (F == 1 && W == WT_LEN) { if (!R.Str(X.EntityId)) return false; }
			else if (!R.Skip(W)) return false;
		}
		return true;
	}

	bool DecodeEvent(const uint8* D, int32 N, FWorldEvent& E)
	{
		FReader R(D, N); uint32 F; uint8 W;
		while (!R.AtEnd())
		{
			if (!R.Tag(F, W)) return false;
			const uint8* B; int32 L;
			if (W == WT_LEN && (F == 1 || F == 2 || F == 3))
			{
				if (!R.Len(B, L)) return false;
				if (F == 1) { FEntitySpawn S; if (!DecodeSpawn(B, L, S)) return false; E.Spawn = S; }
				else if (F == 2) { FEntityMove M; if (!DecodeMove(B, L, M)) return false; E.Move = M; }
				else { FEntityDespawn X; if (!DecodeDespawn(B, L, X)) return false; E.Despawn = X; }
			}
			else if (!R.Skip(W)) return false;
		}
		return true;
	}
}

namespace NightfallProto
{
	void Encode(const FClientMessage& In, TArray<uint8>& Out)
	{
		Out.Reset();
		FWriter W(Out);
		W.U32(1, In.Seq);
		if (In.MoveTo.IsSet())
		{
			TArray<uint8> Vec; { FWriter V(Vec); V.F32(1, In.MoveTo->Destination.X); V.F32(2, In.MoveTo->Destination.Y); }
			TArray<uint8> Move; { FWriter M(Move); M.Bytes(1, Vec); }   // MoveToRequest.destination = 1
			W.Bytes(10, Move);                                           // ClientMessage.move_to = 10
		}
	}

	bool Decode(const uint8* Data, int32 Size, FServerMessage& Out)
	{
		Out = FServerMessage();
		FReader R(Data, Size); uint32 F; uint8 W;
		while (!R.AtEnd())
		{
			if (!R.Tag(F, W)) return false;
			const uint8* B; int32 L;
			if (F == 1 && W == WT_LEN)      // Ack { seq = 1 }
			{
				if (!R.Len(B, L)) return false;
				FReader A(B, L); uint32 AF; uint8 AW;
				while (!A.AtEnd())
				{
					if (!A.Tag(AF, AW)) return false;
					if (AF == 1 && AW == WT_VARINT) { uint64 V; if (!A.Varint(V)) return false; Out.AckSeq = static_cast<uint32>(V); }
					else if (!A.Skip(AW)) return false;
				}
			}
			else if (F == 2 && W == WT_LEN) // WorldEvent
			{
				if (!R.Len(B, L)) return false;
				FWorldEvent E; if (!DecodeEvent(B, L, E)) return false; Out.Event = E;
			}
			else if (!R.Skip(W)) return false;
		}
		return true;
	}
}
