#include "EntityAnimInstance.h"
#include "Animation/AnimSequenceBase.h"
#include "Animation/AnimationPoseData.h"
#include "AnimationRuntime.h"

void UEntityAnimInstance::SetClip(EProxyClip Role, UAnimSequenceBase* Sequence)
{
	if (Sequence) Clips.Add(static_cast<uint8>(Role), Sequence);
	else Clips.Remove(static_cast<uint8>(Role));
}

UAnimSequenceBase* UEntityAnimInstance::GetClip(EProxyClip Role) const
{
	const TObjectPtr<UAnimSequenceBase>* Found = Clips.Find(static_cast<uint8>(Role));
	return Found ? Found->Get() : nullptr;
}

void UEntityAnimInstance::SetPose(const FProxyAnimPose& NewPose)
{
	UAnimSequenceBase* Sequence = GetClip(NewPose.Clip);
	// A role without a clip (no run clip, say) keeps whatever is playing rather than snapping to the reference pose.
	if (Sequence == nullptr && NewPose.Clip == EProxyClip::Run) Sequence = GetClip(EProxyClip::Walk);
	if (Sequence == nullptr) Sequence = GetClip(EProxyClip::Idle);

	const bool bChanged = !bHavePose || Sequence != Current.Sequence
		|| (!NewPose.bLoop && NewPose.ClipTime + KINDA_SMALL_NUMBER < Current.Time);   // a one-shot restarted
	if (bChanged && bHavePose && Current.Sequence != nullptr)
	{
		Previous = Current;
		PreviousRate = Pose.PlayRate;
		BlendElapsed = 0.f;
	}
	Pose = NewPose;
	bHavePose = true;
	Current.Sequence = Sequence;
	Current.Time = NewPose.ClipTime;
	Current.bLoop = NewPose.bLoop;
}

void UEntityAnimInstance::NativeUpdateAnimation(float DeltaSeconds)
{
	Super::NativeUpdateAnimation(DeltaSeconds);
	if (Previous.Sequence != nullptr)
	{
		BlendElapsed += DeltaSeconds;
		Previous.Time += DeltaSeconds * PreviousRate;
		if (BlendElapsed >= BlendSeconds) Previous.Sequence = nullptr;
	}
}

FAnimInstanceProxy* UEntityAnimInstance::CreateAnimInstanceProxy()
{
	return new FEntityAnimInstanceProxy(this);
}

void UEntityAnimInstance::DestroyAnimInstanceProxy(FAnimInstanceProxy* InProxy)
{
	delete static_cast<FEntityAnimInstanceProxy*>(InProxy);
}

void FEntityAnimInstanceProxy::PreUpdate(UAnimInstance* InAnimInstance, float DeltaSeconds)
{
	FAnimInstanceProxy::PreUpdate(InAnimInstance, DeltaSeconds);
	const UEntityAnimInstance* Instance = CastChecked<UEntityAnimInstance>(InAnimInstance);
	Current = Instance->Current;
	Previous = Instance->Previous;
	CurrentWeight = (Previous.Sequence != nullptr && Instance->BlendSeconds > 0.f)
		? FMath::Clamp(Instance->BlendElapsed / Instance->BlendSeconds, 0.f, 1.f)
		: 1.f;
}

void FEntityAnimInstanceProxy::Sample(const FEntityClipSample& Clip, FPoseContext& Output)
{
	const double Length = Clip.Sequence->GetPlayLength();
	double Time = Clip.Time;
	if (Length <= 0.0) Time = 0.0;
	else if (Clip.bLoop) Time = FMath::Fmod(FMath::Max(Time, 0.0), Length);
	else Time = FMath::Clamp(Time, 0.0, Length);
	FAnimationPoseData Data(Output);
	Clip.Sequence->GetAnimationPose(Data, FAnimExtractContext(Time, false, {}, Clip.bLoop));
}

bool FEntityAnimInstanceProxy::Evaluate(FPoseContext& Output)
{
	if (Current.Sequence == nullptr)
	{
		Output.ResetToRefPose();
		return true;
	}
	Sample(Current, Output);
	if (Previous.Sequence != nullptr && CurrentWeight < 1.f)
	{
		FPoseContext Other(Output);
		Sample(Previous, Other);
		FAnimationPoseData OutData(Output);
		FAnimationRuntime::BlendTwoPosesTogetherInPlace(OutData, FAnimationPoseData(Other), CurrentWeight);
	}
	return true;
}
