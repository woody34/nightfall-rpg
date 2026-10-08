#include "AuthCrypto.h"
#include "HAL/PlatformMisc.h"
#include "Misc/AES.h"
#include "Misc/Base64.h"
#include "Misc/Guid.h"

namespace NightfallAuthCrypto
{
namespace
{
	constexpr uint32 K[64] = {
		0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
		0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
		0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
		0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
		0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
		0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
		0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
		0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
	};

	constexpr uint32 Rotr(uint32 X, uint32 N) { return (X >> N) | (X << (32 - N)); }

	void Compress(uint32 H[8], const uint8 Block[64])
	{
		uint32 W[64];
		for (int32 I = 0; I < 16; ++I)
		{
			W[I] = (uint32(Block[I * 4]) << 24) | (uint32(Block[I * 4 + 1]) << 16) | (uint32(Block[I * 4 + 2]) << 8) | uint32(Block[I * 4 + 3]);
		}
		for (int32 I = 16; I < 64; ++I)
		{
			const uint32 S0 = Rotr(W[I - 15], 7) ^ Rotr(W[I - 15], 18) ^ (W[I - 15] >> 3);
			const uint32 S1 = Rotr(W[I - 2], 17) ^ Rotr(W[I - 2], 19) ^ (W[I - 2] >> 10);
			W[I] = W[I - 16] + S0 + W[I - 7] + S1;
		}
		uint32 A = H[0], B = H[1], C = H[2], D = H[3], E = H[4], F = H[5], G = H[6], Hh = H[7];
		for (int32 I = 0; I < 64; ++I)
		{
			const uint32 S1 = Rotr(E, 6) ^ Rotr(E, 11) ^ Rotr(E, 25);
			const uint32 Ch = (E & F) ^ (~E & G);
			const uint32 T1 = Hh + S1 + Ch + K[I] + W[I];
			const uint32 S0 = Rotr(A, 2) ^ Rotr(A, 13) ^ Rotr(A, 22);
			const uint32 Maj = (A & B) ^ (A & C) ^ (B & C);
			const uint32 T2 = S0 + Maj;
			Hh = G; G = F; F = E; E = D + T1; D = C; C = B; B = A; A = T1 + T2;
		}
		H[0] += A; H[1] += B; H[2] += C; H[3] += D; H[4] += E; H[5] += F; H[6] += G; H[7] += Hh;
	}

	constexpr uint32 Magic = 0x4E465254;   // "NFRT"
	constexpr int32 Block = FAES::AESBlockSize;

	FAES::FAESKey DeriveKey(const FString& DeviceMaterial, const TArray<uint8>& Salt)
	{
		TArray<uint8> Input;
		const FTCHARToUTF8 Label(TEXT("nightfall-refresh-token-v1|"));
		Input.Append(reinterpret_cast<const uint8*>(Label.Get()), Label.Length());
		const FTCHARToUTF8 Material(*DeviceMaterial);
		Input.Append(reinterpret_cast<const uint8*>(Material.Get()), Material.Length());
		Input.Add('|');
		Input.Append(Salt);
		const TArray<uint8> Digest = Sha256(Input.GetData(), Input.Num());

		FAES::FAESKey Key;
		static_assert(FAES::FAESKey::KeySize == 32, "AES-256 key is one SHA-256 digest");
		FMemory::Memcpy(Key.Key, Digest.GetData(), FAES::FAESKey::KeySize);
		return Key;
	}

	void PutU32(TArray<uint8>& Out, uint32 V)
	{
		Out.Add(uint8(V >> 24)); Out.Add(uint8(V >> 16)); Out.Add(uint8(V >> 8)); Out.Add(uint8(V));
	}

	uint32 GetU32(const uint8* P)
	{
		return (uint32(P[0]) << 24) | (uint32(P[1]) << 16) | (uint32(P[2]) << 8) | uint32(P[3]);
	}
}

TArray<uint8> Sha256(const uint8* Data, int64 Size)
{
	uint32 H[8] = { 0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19 };
	int64 Offset = 0;
	for (; Offset + 64 <= Size; Offset += 64)
	{
		Compress(H, Data + Offset);
	}
	// Final block(s): remaining bytes, 0x80, zero pad, 64-bit big-endian bit length.
	uint8 Tail[128] = {};
	const int64 Rest = Size - Offset;
	if (Rest > 0)
	{
		FMemory::Memcpy(Tail, Data + Offset, Rest);
	}
	Tail[Rest] = 0x80;
	const int32 TailLen = (Rest + 1 + 8 <= 64) ? 64 : 128;
	const uint64 Bits = uint64(Size) * 8;
	for (int32 I = 0; I < 8; ++I)
	{
		Tail[TailLen - 1 - I] = uint8(Bits >> (8 * I));
	}
	Compress(H, Tail);
	if (TailLen == 128)
	{
		Compress(H, Tail + 64);
	}

	TArray<uint8> Out;
	Out.Reserve(32);
	for (uint32 Word : H)
	{
		PutU32(Out, Word);
	}
	return Out;
}

TArray<uint8> Sha256(const FString& Utf8)
{
	const FTCHARToUTF8 Bytes(*Utf8);
	return Sha256(reinterpret_cast<const uint8*>(Bytes.Get()), Bytes.Length());
}

FString Base64UrlEncode(const TArray<uint8>& Bytes)
{
	return FBase64::Encode(Bytes, EBase64Mode::UrlSafe).Replace(TEXT("="), TEXT(""));
}

bool Base64UrlDecode(const FString& Encoded, TArray<uint8>& OutBytes)
{
	FString Padded = Encoded;
	while (Padded.Len() % 4 != 0)
	{
		Padded.AppendChar(TEXT('='));
	}
	return FBase64::Decode(Padded, OutBytes, EBase64Mode::UrlSafe);
}

TArray<uint8> RandomBytes(int32 Count)
{
	// FGuid::NewGuid draws from getrandom/BCryptGenRandom on desktop platforms (>= 74 random bits
	// per GUID). Hashing several together gives uniformly distributed output of any length.
	TArray<uint8> Out;
	while (Out.Num() < Count)
	{
		TArray<uint8> Pool;
		for (int32 I = 0; I < 4; ++I)
		{
			const FGuid G = FGuid::NewGuid();
			Pool.Append(reinterpret_cast<const uint8*>(&G), sizeof(FGuid));
		}
		Out.Append(Sha256(Pool.GetData(), Pool.Num()));
	}
	Out.SetNum(Count);
	return Out;
}

FString PkceChallenge(const FString& Verifier)
{
	return Base64UrlEncode(Sha256(Verifier));
}

FString NewPkceVerifier()
{
	return Base64UrlEncode(RandomBytes(32));
}

TArray<uint8> Seal(const FString& Plain, const FString& DeviceMaterial, const TArray<uint8>& Salt)
{
	const FTCHARToUTF8 Utf8(*Plain);
	TArray<uint8> Body;
	PutU32(Body, Magic);
	PutU32(Body, uint32(Utf8.Length()));
	Body.Append(reinterpret_cast<const uint8*>(Utf8.Get()), Utf8.Length());
	Body.SetNumZeroed(Align(Body.Num(), Block));

	const FAES::FAESKey Key = DeriveKey(DeviceMaterial, Salt);
	TArray<uint8> Out = RandomBytes(Block);   // IV
	Out.Reserve(Block + Body.Num());
	// CBC on top of FAES's single-block primitive: XOR with the previous ciphertext block.
	for (int32 Offset = 0; Offset < Body.Num(); Offset += Block)
	{
		const uint8* Prev = Out.GetData() + Offset;
		uint8 Chunk[Block];
		for (int32 I = 0; I < Block; ++I)
		{
			Chunk[I] = Body[Offset + I] ^ Prev[I];
		}
		FAES::EncryptData(Chunk, Block, Key);
		Out.Append(Chunk, Block);
	}
	FMemory::Memzero(Body.GetData(), Body.Num());
	return Out;
}

bool Open(const TArray<uint8>& Sealed, const FString& DeviceMaterial, const TArray<uint8>& Salt, FString& OutPlain)
{
	if (Sealed.Num() < 2 * Block || Sealed.Num() % Block != 0)
	{
		return false;
	}
	const FAES::FAESKey Key = DeriveKey(DeviceMaterial, Salt);
	TArray<uint8> Body;
	Body.SetNumUninitialized(Sealed.Num() - Block);
	for (int32 Offset = Block; Offset < Sealed.Num(); Offset += Block)
	{
		uint8 Chunk[Block];
		FMemory::Memcpy(Chunk, Sealed.GetData() + Offset, Block);
		FAES::DecryptData(Chunk, Block, Key);
		for (int32 I = 0; I < Block; ++I)
		{
			Body[Offset - Block + I] = Chunk[I] ^ Sealed[Offset - Block + I];
		}
	}
	const uint32 Len = GetU32(Body.GetData() + 4);
	if (GetU32(Body.GetData()) != Magic || Len > uint32(Body.Num() - 8))
	{
		return false;   // wrong key (another machine or user), or a corrupted file
	}
	OutPlain = FString(FUTF8ToTCHAR(reinterpret_cast<const ANSICHAR*>(Body.GetData() + 8), Len));
	FMemory::Memzero(Body.GetData(), Body.Num());
	return true;
}

FString LocalDeviceMaterial()
{
	// GetDeviceId is empty on desktop Linux; GetLoginId is /etc/machine-id plus the uid there.
	return FString::Printf(TEXT("%s|%s"), *FPlatformMisc::GetDeviceId(), *FPlatformMisc::GetLoginId());
}
}
