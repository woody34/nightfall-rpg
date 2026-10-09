#pragma once

#include "CoreMinimal.h"
#include "Animation/AnimInstance.h"
#include "Animation/AnimInstanceProxy.h"
#include "Anim/ProxyAnimState.h"
#include "EntityAnimInstance.generated.h"

class UAnimSequenceBase;

/** One sequence sampled at one time. */
struct FEntityClipSample
{
	TObjectPtr<const UAnimSequenceBase> Sequence;
	double Time = 0.0;
	bool bLoop = true;
};

/** Worker-thread side: samples the current clip and, during a crossfade, the previous one. */
struct FEntityAnimInstanceProxy : public FAnimInstanceProxy
{
	FEntityAnimInstanceProxy() = default;
	explicit FEntityAnimInstanceProxy(UAnimInstance* Instance) : FAnimInstanceProxy(Instance) {}

	virtual void PreUpdate(UAnimInstance* InAnimInstance, float DeltaSeconds) override;
	virtual bool Evaluate(FPoseContext& Output) override;

private:
	static void Sample(const FEntityClipSample& Clip, FPoseContext& Output);

	FEntityClipSample Current;
	FEntityClipSample Previous;
	float CurrentWeight = 1.f;
};

/**
 * Native animation instance for proxies and the player pawn: no Animation Blueprint graph. The
 * owner (UEntityAnimationComponent) hands it the clip for each role and, every frame, the pose
 * FProxyAnimStateMachine computed; it crossfades over BlendSeconds whenever the clip changes or
 * a one-shot clip restarts. All timing decisions are the state machine's; this only samples.
 */
UCLASS(Transient, NotBlueprintable)
class NIGHTFALL_API UEntityAnimInstance : public UAnimInstance
{
	GENERATED_BODY()

public:
	void SetClip(EProxyClip Role, UAnimSequenceBase* Sequence);
	UAnimSequenceBase* GetClip(EProxyClip Role) const;

	/** Called by the component each frame before the animation update. */
	void SetPose(const FProxyAnimPose& Pose);

	const FProxyAnimPose& GetPose() const { return Pose; }

	UPROPERTY(EditAnywhere, Category = "Nightfall")
	float BlendSeconds = 0.15f;

protected:
	virtual void NativeUpdateAnimation(float DeltaSeconds) override;
	virtual FAnimInstanceProxy* CreateAnimInstanceProxy() override;
	virtual void DestroyAnimInstanceProxy(FAnimInstanceProxy* InProxy) override;

private:
	friend struct FEntityAnimInstanceProxy;

	UPROPERTY(Transient)
	TMap<uint8, TObjectPtr<UAnimSequenceBase>> Clips;

	FProxyAnimPose Pose;
	bool bHavePose = false;
	FEntityClipSample Current;
	FEntityClipSample Previous;
	double PreviousRate = 1.0;
	float BlendElapsed = 0.f;
};
