#include "BotLogSentinel.h"
#include "Misc/CoreDelegates.h"
#include "Misc/OutputDeviceRedirector.h"

FBotLogSentinel::~FBotLogSentinel()
{
	Stop();
}

bool FBotLogSentinel::ParseAllowList(const FString& Text, TArray<FBotAllowEntry>& Out, TArray<FString>& OutErrors)
{
	TArray<FString> Lines;
	Text.ParseIntoArrayLines(Lines, /*CullEmpty*/ false);
	for (int32 I = 0; I < Lines.Num(); ++I)
	{
		FString Line = Lines[I];
		Line.TrimStartAndEndInline();
		if (Line.IsEmpty() || Line.StartsWith(TEXT("#"))) continue;
		TArray<FString> Parts;
		Line.ParseIntoArray(Parts, TEXT("|"), /*CullEmpty*/ false);
		for (FString& Part : Parts) Part.TrimStartAndEndInline();
		if (Parts.Num() != 3 || Parts[0].IsEmpty() || Parts[1].IsEmpty() || Parts[2].IsEmpty())
		{
			OutErrors.Add(FString::Printf(TEXT("line %d: expected '<category> | <substring> | <reason>', all three non-empty: %s"), I + 1, *Line));
			continue;
		}
		FBotAllowEntry& Entry = Out.AddDefaulted_GetRef();
		Entry.Category = Parts[0];
		Entry.Substring = Parts[1];
		Entry.Reason = Parts[2];
		Entry.Line = I + 1;
	}
	return OutErrors.IsEmpty();
}

bool FBotLogSentinel::IsWatchedCategory(const FName& Category)
{
	const FString Name = Category.ToString();
	return Name == TEXT("LogNightfall") || Name == TEXT("LogTurboLink") || Name.StartsWith(TEXT("LogNet"));
}

void FBotLogSentinel::SetAllowList(TArray<FBotAllowEntry> Entries)
{
	FScopeLock Guard(&Lock);
	AllowList = MoveTemp(Entries);
}

void FBotLogSentinel::SetMirror(FMirrorFn InMirror)
{
	FScopeLock Guard(&Lock);
	Mirror = MoveTemp(InMirror);
}

void FBotLogSentinel::Start()
{
	if (bStarted) return;
	bStarted = true;
	GLog->AddOutputDevice(this);
	EnsureHandle = FCoreDelegates::OnEnsureFailed.AddLambda([this](const ANSICHAR* Expr, const ANSICHAR* File, int32 Line, const TCHAR* Msg, const TCHAR* Combined)
	{
		NoteEnsure(FString::Printf(TEXT("ensure(%hs) at %hs:%d: %s"), Expr ? Expr : "", File ? File : "", Line, Msg ? Msg : TEXT("")));
	});
}

void FBotLogSentinel::Stop()
{
	if (!bStarted) return;
	bStarted = false;
	FCoreDelegates::OnEnsureFailed.Remove(EnsureHandle);
	if (GLog) GLog->RemoveOutputDevice(this);
}

bool FBotLogSentinel::Allow(const FString& Category, const FString& Text)
{
	for (FBotAllowEntry& Entry : AllowList)
	{
		const bool bCategory = Entry.Category == TEXT("*") ? !Category.Equals(TEXT("ensure"), ESearchCase::IgnoreCase) : Entry.Category.Equals(Category, ESearchCase::IgnoreCase);
		if (bCategory && Text.Contains(Entry.Substring))
		{
			++Entry.Hits;
			++Allowed;
			return true;
		}
	}
	return false;
}

namespace
{
	// A line logged while the mirror writes (e.g. the file writer reporting a full disk) must not
	// re-enter the archive mid-write: it is still counted, just not mirrored.
	thread_local bool bInMirror = false;

	void MirrorOnce(const FBotLogSentinel::FMirrorFn& Mirror, const FString& Line)
	{
		if (!Mirror || bInMirror) return;
		bInMirror = true;
		Mirror(Line);
		bInMirror = false;
	}
}

void FBotLogSentinel::Serialize(const TCHAR* V, ELogVerbosity::Type Verbosity, const FName& Category)
{
	const ELogVerbosity::Type Level = static_cast<ELogVerbosity::Type>(Verbosity & ELogVerbosity::VerbosityMask);
	FScopeLock Guard(&Lock);
	MirrorOnce(Mirror, FString::Printf(TEXT("%s: %s: %s"), *Category.ToString(), ToString(Level), V));
	if ((Level != ELogVerbosity::Error && Level != ELogVerbosity::Fatal) || !IsWatchedCategory(Category)) return;
	const FString CategoryName = Category.ToString();
	const FString Text(V);
	if (!Allow(CategoryName, Text))
	{
		Unallowed.Add(FString::Printf(TEXT("%s %s: %s"), *CategoryName, ToString(Level), *Text));
	}
}

void FBotLogSentinel::NoteEnsure(const FString& Message)
{
	FScopeLock Guard(&Lock);
	MirrorOnce(Mirror, FString::Printf(TEXT("ensure: %s"), *Message));
	if (!Allow(TEXT("ensure"), Message)) Unallowed.Add(FString::Printf(TEXT("ensure: %s"), *Message));
}

TArray<FString> FBotLogSentinel::GetUnallowed() const { FScopeLock Guard(&Lock); return Unallowed; }
int32 FBotLogSentinel::NumUnallowed() const { FScopeLock Guard(&Lock); return Unallowed.Num(); }
int32 FBotLogSentinel::NumAllowed() const { FScopeLock Guard(&Lock); return Allowed; }
TArray<FBotAllowEntry> FBotLogSentinel::GetAllowList() const { FScopeLock Guard(&Lock); return AllowList; }
