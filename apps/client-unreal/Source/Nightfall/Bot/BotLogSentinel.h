#pragma once

#include "CoreMinimal.h"
#include "Misc/OutputDevice.h"

/**
 * One allow-list entry from `<scenario>.allow` (next to the .nfs): a known noisy line that does
 * not fail the run. Format, one per line, `#` comments:
 *
 *     <category> | <substring of the message> | <reason>
 *
 * `<category>` is a log category (LogNet, LogNightfall, ...), `*` for any watched category, or
 * `ensure` for an ensure whose message contains the substring. The reason is required.
 */
struct FBotAllowEntry
{
	FString Category;
	FString Substring;
	FString Reason;
	int32 Line = 0;
	int32 Hits = 0;
};

/**
 * Plan D6 / E1.3: counts Error and Fatal lines in LogNightfall, LogTurboLink and LogNet* and every
 * ensure while a scenario runs. Anything not on the allow-list fails the scenario even when every
 * step passed. Optionally mirrors every log line to a sink (the per-scenario run log).
 *
 * Thread safe: log lines and ensures can come from any thread.
 */
class NIGHTFALL_API FBotLogSentinel : public FOutputDevice
{
public:
	using FMirrorFn = TFunction<void(const FString& Line)>;

	FBotLogSentinel() = default;
	virtual ~FBotLogSentinel() override;

	/** Parses an allow-list file's text; errors name the line. */
	static bool ParseAllowList(const FString& Text, TArray<FBotAllowEntry>& Out, TArray<FString>& OutErrors);

	/** LogNightfall, LogTurboLink, or a category starting with LogNet. */
	static bool IsWatchedCategory(const FName& Category);

	void SetAllowList(TArray<FBotAllowEntry> Entries);
	void SetMirror(FMirrorFn InMirror);

	/** Hooks GLog and FCoreDelegates::OnEnsureFailed. Stop (or the destructor) unhooks. */
	void Start();
	void Stop();

	// FOutputDevice
	virtual void Serialize(const TCHAR* V, ELogVerbosity::Type Verbosity, const FName& Category) override;
	virtual bool CanBeUsedOnAnyThread() const override { return true; }
	virtual bool CanBeUsedOnMultipleThreads() const override { return true; }

	/** What OnEnsureFailed reports; public so tests can feed a synthetic ensure. */
	void NoteEnsure(const FString& Message);

	/** Watched Error/Fatal lines and ensures not covered by the allow-list, as "<Category> Error: <text>". */
	TArray<FString> GetUnallowed() const;
	int32 NumUnallowed() const;
	int32 NumAllowed() const;
	TArray<FBotAllowEntry> GetAllowList() const;

private:
	bool Allow(const FString& Category, const FString& Text);

	mutable FCriticalSection Lock;
	TArray<FBotAllowEntry> AllowList;
	TArray<FString> Unallowed;
	int32 Allowed = 0;
	FMirrorFn Mirror;
	bool bStarted = false;
	FDelegateHandle EnsureHandle;
};
