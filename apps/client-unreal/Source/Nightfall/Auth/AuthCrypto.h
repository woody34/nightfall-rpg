#pragma once

#include "CoreMinimal.h"

/**
 * The few primitives the device-flow login needs. SHA-256 is implemented here because Core's
 * FPlatformMisc::GetSHA256Signature has no Linux implementation (it asserts).
 */
namespace NightfallAuthCrypto
{
	/** SHA-256 of Size bytes (FIPS 180-4). */
	NIGHTFALL_API TArray<uint8> Sha256(const uint8* Data, int64 Size);
	NIGHTFALL_API TArray<uint8> Sha256(const FString& Utf8);

	/** base64url without padding (RFC 4648 §5), as PKCE and JWTs use. */
	NIGHTFALL_API FString Base64UrlEncode(const TArray<uint8>& Bytes);
	NIGHTFALL_API bool Base64UrlDecode(const FString& Encoded, TArray<uint8>& OutBytes);

	/** Bytes from the OS CSPRNG (getrandom on Linux, via FGuid::NewGuid), hashed together. */
	NIGHTFALL_API TArray<uint8> RandomBytes(int32 Count);

	/** PKCE S256 (RFC 7636 §4.2): base64url(sha256(ascii(verifier))). */
	NIGHTFALL_API FString PkceChallenge(const FString& Verifier);

	/** A fresh 43-character PKCE verifier (32 random bytes, base64url). */
	NIGHTFALL_API FString NewPkceVerifier();

	/**
	 * AES-256-CBC with a random IV under a key derived from DeviceMaterial and Salt. Output is
	 * IV || ciphertext. The plaintext carries a magic and a length so a wrong key is detected.
	 * Obfuscation for a refresh token at rest, NOT a keychain: anyone who can read the save file
	 * and run code as this user on this machine can derive the same key.
	 */
	NIGHTFALL_API TArray<uint8> Seal(const FString& Plain, const FString& DeviceMaterial, const TArray<uint8>& Salt);
	NIGHTFALL_API bool Open(const TArray<uint8>& Sealed, const FString& DeviceMaterial, const TArray<uint8>& Salt, FString& OutPlain);

	/** Machine- and user-bound material for Seal: FPlatformMisc::GetDeviceId and GetLoginId. */
	NIGHTFALL_API FString LocalDeviceMaterial();
}
