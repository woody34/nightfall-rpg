#!/usr/bin/env python3
"""Byte-compare committed bindings with fresh TurboLink + committed postprocessor output."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

CLIENT = Path(__file__).resolve().parents[2]
PROTO = CLIENT.parents[1] / 'packages/proto'
TOOLS = Path(os.environ.get('NIGHTFALL_PROTO_TOOLS', Path.home() / '.cache/nightfall/proto-tools'))


class ProtoRegenerationTest(unittest.TestCase):
    def test_fresh_generation_matches_committed_bindings(self):
        with tempfile.TemporaryDirectory(prefix='nightfall-client-proto-') as directory:
            output = Path(directory)
            (output / 'Private/pb').mkdir(parents=True)
            protos = sorted(str(p.relative_to(PROTO)) for p in (PROTO / 'nightfall').rglob('*.proto'))
            subprocess.run([
                str(TOOLS / 'bin/protoc'), '-I', str(TOOLS / 'include'), '-I', '.',
                f'--cpp_out={output}/Private/pb',
                f'--plugin=protoc-gen-grpc={TOOLS}/bin/grpc_cpp_plugin',
                f'--grpc_out={output}/Private/pb',
                f'--plugin=protoc-gen-turbolink={TOOLS}/bin/protoc-gen-turbolink',
                f'--turbolink_out={output}', '--turbolink_opt=GenerateJsonCode=true',
                *protos,
            ], cwd=PROTO, check=True, capture_output=True)
            subprocess.run(['python3', str(CLIENT / 'Scripts/fix-proto-optionals.py'), str(output)],
                           check=True, capture_output=True)
            for path in (output / 'Private/pb').rglob('*'):
                if path.suffix not in ('.h', '.cc') or '.grpc.pb.' in path.name:
                    continue
                fix = 'fix_proto_h.txt' if path.name.endswith('.pb.h') else 'fix_proto_cpp.txt'
                prefix = (CLIENT / 'Plugins/TurboLink/Tools' / fix).read_bytes().replace(b'\r', b'')
                path.write_bytes(prefix + path.read_bytes())
            def files(root):
                return {str(p.relative_to(root)): p.read_bytes() for p in root.rglob('*')
                        if p.is_file() and p.name != '.ubtignore'}
            fresh = files(output)
            committed = files(CLIENT / 'Source/Nightfall/Generated')
            self.assertEqual(fresh.keys(), committed.keys(), 'generated file inventory differs')
            differences = [name for name in fresh if fresh[name] != committed[name]]
            self.assertEqual([], differences, 'run Scripts/gen-proto.sh --generate-only')


if __name__ == '__main__':
    unittest.main()
