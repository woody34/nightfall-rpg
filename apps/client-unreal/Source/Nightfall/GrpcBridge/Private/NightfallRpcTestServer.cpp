#include "NightfallRpcTestServer.h"
#if WITH_DEV_AUTOMATION_TESTS
#include "nightfall/v1/game.grpc.pb.h"
#include <grpcpp/grpcpp.h>

namespace
{
 class FStatusService final : public ::nightfall::v1::GameService::Service
 {
 public:
  explicit FStatusService(bool bUnavailable) : bCatalogUnavailable(bUnavailable) {}
  grpc::Status GetSkillCatalog(grpc::ServerContext*,const ::nightfall::v1::GetSkillCatalogRequest*,::nightfall::v1::GetSkillCatalogResponse*) override
  {
   return bCatalogUnavailable
    ? grpc::Status(grpc::StatusCode::UNAVAILABLE,"catalogue fixture unavailable")
    : grpc::Status(grpc::StatusCode::FAILED_PRECONDITION,"catalogue fixture precondition");
  }
  grpc::Status LearnSkill(grpc::ServerContext*,const ::nightfall::v1::LearnSkillRequest*,::nightfall::v1::LearnSkillResponse*) override
  {
   return grpc::Status(grpc::StatusCode::UNAVAILABLE,"learning fixture unavailable");
  }
 private:
  const bool bCatalogUnavailable;
 };
 class FStatusServer final : public INightfallRpcTestServer
 {
 public:
  explicit FStatusServer(bool bUnavailable) : Service(bUnavailable)
  {
   grpc::ServerBuilder Builder;
   Builder.AddListeningPort("127.0.0.1:0",grpc::InsecureServerCredentials(),&Port);
   Builder.RegisterService(&Service);
   Server=Builder.BuildAndStart();
  }
  ~FStatusServer() override {if(Server){Server->Shutdown();Server->Wait();}}
  FString Endpoint() const override {return Server && Port>0?FString::Printf(TEXT("127.0.0.1:%d"),Port):FString();}
 private:
  FStatusService Service;
  int Port=0;
  std::unique_ptr<grpc::Server> Server;
 };
}
TUniquePtr<INightfallRpcTestServer> MakeNightfallRpcTestServer(bool bCatalogUnavailable)
{
 return MakeUnique<FStatusServer>(bCatalogUnavailable);
}
#endif
