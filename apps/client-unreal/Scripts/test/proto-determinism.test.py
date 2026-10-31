#!/usr/bin/env python3
"""P5-D2: regenerating the committed TurboLink/protobuf bindings from packages/proto is deterministic.

`gen-proto.sh --generate-only` runs the pinned generators into a scratch directory
(NIGHTFALL_GEN_OUT) and the result must equal `Source/Nightfall/Generated` byte for byte. A drift
means either the protos changed without regeneration or a generator changed: both block the lane.
The pinned tools must be installed (`Scripts/install-proto-tools.sh`); without them the test FAILS
unless NIGHTFALL_PROTO_TOOLS_OPTIONAL=1 marks the host as one without the toolchain (then it skips
and says so).
"""
import filecmp
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CLIENT = Path(__file__).resolve().parents[2]
GENERATED = CLIENT / 'Source/Nightfall/Generated'
TOOLS = Path(os.environ.get('NIGHTFALL_PROTO_TOOLS', Path.home() / '.cache/nightfall/proto-tools'))


def tools_present():
    return all((TOOLS / 'bin' / tool).exists() for tool in ('protoc', 'grpc_cpp_plugin', 'protoc-gen-turbolink'))


def tree(root):
    return sorted(str(p.relative_to(root)) for p in root.rglob('*') if p.is_file() and p.name != '.ubtignore')


class Determinism(unittest.TestCase):
    def test_regenerate_and_diff(self):
        if not tools_present():
            if os.environ.get('NIGHTFALL_PROTO_TOOLS_OPTIONAL') == '1':
                self.skipTest(f'proto tools absent in {TOOLS} (NIGHTFALL_PROTO_TOOLS_OPTIONAL=1)')
            self.fail(f'proto tools absent in {TOOLS}; run Scripts/install-proto-tools.sh')
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'Generated'
            out.mkdir()
            run = subprocess.run(['bash', str(CLIENT / 'Scripts/gen-proto.sh'), '--generate-only'], env={**os.environ, 'NIGHTFALL_GEN_OUT': str(out)}, capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertEqual(tree(out), tree(GENERATED), 'generated file set differs from the committed set')
            differing = [f for f in tree(out) if not filecmp.cmp(out / f, GENERATED / f, shallow=False)]
            self.assertEqual(differing, [], 'committed Generated/ differs from a fresh generation')
            self.assertGreater(len(tree(out)), 20)


if __name__ == '__main__':
    unittest.main()
