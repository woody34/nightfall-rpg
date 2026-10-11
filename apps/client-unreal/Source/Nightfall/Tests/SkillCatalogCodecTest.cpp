#include "Misc/AutomationTest.h"
#include "NightfallWire.h"

#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillCatalogCodecTest,"Nightfall.Skill.CatalogueMetadataWire",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillCatalogCodecTest::RunTest(const FString& Parameters)
{
 using E=EGrpcNightfallV1SkillOperateType;
 // Independent wire vectors: legacy skill 3 capability, then additive tags 13/14/15.
 const TArray<uint8> Legacy={0x12,0x04,0x08,0x03,0x28,0x01};
 const TArray<uint8> Active={0x12,0x13,0x08,0x03,0x28,0x01,0x68,0x01,0x72,0x04,'S','E','L','F',0x7a,0x05,'u','n','s','e','t'};
 FGrpcNightfallV1GetSkillCatalogResponse R;
 if(!TestTrue(TEXT("legacy catalogue decodes"),NightfallWire::DecodeSkillCatalogResponse(Legacy.GetData(),Legacy.Num(),R)) || !TestEqual(TEXT("legacy row"),R.Skills.Num(),1))return false;
 TestTrue(TEXT("old operate metadata is unspecified"),R.Skills[0].OperateType==E::SKILL_OPERATE_TYPE_UNSPECIFIED);
 TestTrue(TEXT("old target metadata absent"),R.Skills[0].TargetType.IsEmpty());
 TArray<uint8> Bytes;NightfallWire::EncodeSkillCatalogResponse(R,Bytes);TestTrue(TEXT("historical catalogue bytes unchanged"),Bytes==Legacy);
 if(!TestTrue(TEXT("C7 catalogue decodes"),NightfallWire::DecodeSkillCatalogResponse(Active.GetData(),Active.Num(),R)) || !TestEqual(TEXT("C7 row"),R.Skills.Num(),1))return false;
 TestTrue(TEXT("active enum received"),R.Skills[0].OperateType==E::SKILL_OPERATE_TYPE_ACTIVE_INSTANT);
 TestTrue(TEXT("target string received"),R.Skills[0].TargetType==TArray<FString>{TEXT("SELF")});
 TestTrue(TEXT("literal unset scope preserved"),R.Skills[0].AffectScope==TArray<FString>{TEXT("unset")});
 NightfallWire::EncodeSkillCatalogResponse(R,Bytes);TestTrue(TEXT("fixed additive tags preserved"),Bytes==Active);
 for(uint8 Type:{uint8(0),uint8(1),uint8(2),uint8(3),uint8(4),uint8(5),uint8(6),uint8(254)})
 {
  auto& Row=R.Skills[0];Row.OperateType=static_cast<E>(Type);Row.MaxLevel=2;Row.TargetType={TEXT("TARGET"),TEXT("UNSUPPORTED")};Row.AffectScope={TEXT("SINGLE"),TEXT("unset")};
  NightfallWire::EncodeSkillCatalogResponse(R,Bytes);FGrpcNightfallV1GetSkillCatalogResponse Decoded;
  if(!TestTrue(TEXT("every enum binary round trip"),NightfallWire::DecodeSkillCatalogResponse(Bytes.GetData(),Bytes.Num(),Decoded)) || !TestEqual(TEXT("round trip row"),Decoded.Skills.Num(),1))return false;
  TestEqual(TEXT("known and future enum preserved"),static_cast<uint8>(Decoded.Skills[0].OperateType),Type);
  TestTrue(TEXT("rank target order/unsupported strings preserved"),Decoded.Skills[0].TargetType==Row.TargetType);
  TestTrue(TEXT("rank scope order/unset preserved"),Decoded.Skills[0].AffectScope==Row.AffectScope);
 }
 TestTrue(TEXT("legacy decoder reuse resets metadata"),NightfallWire::DecodeSkillCatalogResponse(Legacy.GetData(),Legacy.Num(),R));
 TestTrue(TEXT("previous type cannot leak into legacy response"),R.Skills[0].OperateType==E::SKILL_OPERATE_TYPE_UNSPECIFIED && R.Skills[0].TargetType.IsEmpty() && R.Skills[0].AffectScope.IsEmpty());
 return true;
}
#endif
