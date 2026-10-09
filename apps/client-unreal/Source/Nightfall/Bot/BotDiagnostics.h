#pragma once

#include "CoreMinimal.h"
#include "BotPredicates.h"
#include "BotSteps.h"
#include "NightfallContractWire.h"

class UNetClientSubsystem;
class FJsonObject;

/** Bounded wire history and descriptor-backed coverage; active only while a bot scenario runs. */
class NIGHTFALL_API FBotDiagnostics
{
public:
	static constexpr int32 FailureSchemaVersion = 1;
	static constexpr int32 MaxEvents = 50;
	FBotDiagnostics();
	~FBotDiagnostics();
	void Bind(UNetClientSubsystem* Net);
	void Unbind();
	void ObserveFrame(bool bClient, const TArray<uint8>& Bytes, double ArrivalSeconds);
	void ObserveClose(int32 Code);
	void BeginMove(double NowSeconds);
	void ObservePosition(const FBotContext& Context, double NowSeconds);
	/** Process coverage survives loop resets; positional history is per scenario iteration. */
	void ResetIteration();
	TSharedRef<FJsonObject> Coverage() const;
	TSharedRef<FJsonObject> FailureBundle(const FString& Scenario, const FString& OwnEntity,
		const FBotContext& Context, const FBotFailedStep& Step, const FString& Message) const;
	void AddJUnitProperties(TArray<TPair<FString, FString>>& Properties) const;
	static FString Json(const TSharedRef<FJsonObject>& Object);
	const TMap<FString, TMap<FString, uint64>>& CountsForTesting() const { return Counts; }

private:
	struct FEvent
	{
		FString Type;
		FString Fields;
		TOptional<uint64> Tick;
		uint64 PrecedingTick = 0;
		double ArrivalSeconds = 0.0;
	};
	struct FPosition { double ArrivalSeconds = 0.0; uint64 Tick = 0; FVector2D Tiles; };
	TWeakObjectPtr<UNetClientSubsystem> BoundNet;
	FDelegateHandle SentHandle, ReceivedHandle, ClosedHandle;
	TMap<FString, TMap<FString, uint64>> Counts;
	TArray<FEvent> Events;
	TArray<FPosition> PositionHistory;
	TOptional<uint64> FirstTick;
	uint64 LastTick = 0;
	double MoveStartSeconds = 0.0;
	uint64 ClientFrames = 0;
	uint64 ServerFrames = 0;
};
