#!/usr/bin/env python3
"""Keep the optional catalogue's UNAVAILABLE status fail-closed without an Error log.

TurboLink 1.4.2 unary contexts log all RPC statuses at Error before the callback.
This one RPC may legitimately be unavailable (C16); other RPC/status errors keep
TurboLink's normal path. Run after generation; idempotent, fail on template drift.
"""
from pathlib import Path
import sys

START = '''void GrpcContext_GameService_GetSkillCatalog::OnRpcEvent(bool Ok, const void* EventTag)
{
'''
PATCH = '''\t// C16: optional catalogue unavailability is warning-level; deliver the failure unchanged.
\tif (Ok && RpcStatus.error_code() == grpc::StatusCode::UNAVAILABLE)
\t{
\t\tconst FGrpcResult Result = GrpcContext::MakeGrpcResult(RpcStatus);
\t\tUE_LOG(LogTurboLink, Warning, TEXT("GetSkillCatalog unavailable: %s"), *Result.GetMessageString());
\t\tUGameServiceClient* GameClient = (UGameServiceClient*)Client;
\t\tFGrpcNightfallV1GetSkillCatalogResponse Response;
\t\tGameClient->OnGetSkillCatalogResponse.Broadcast(Handle, Result, Response);
\t\tUpdateState(EGrpcContextState::Done);
\t\treturn;
\t}
'''


def patch(root):
    path = root / 'Private/SNightfallV1/GameContext.cpp'
    text = path.read_text()
    if START + PATCH in text:
        return
    if text.count(START) != 1:
        raise ValueError('GetSkillCatalog context template drift')
    path.write_text(text.replace(START, START + PATCH))


if __name__ == '__main__':
    patch(Path(sys.argv[1]))
