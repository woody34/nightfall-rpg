#!/usr/bin/env python3
"""Message-scoped optional scalar patching without generating client-owned code."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'fix-proto-optionals.py'


class OptionalTests(unittest.TestCase):
    def test_duplicate_field_names_are_scoped_and_repeatable(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'Public').mkdir()
            (root / 'Private').mkdir()
            header, source = [], []
            for message in ('Cast', 'Effect'):
                for field in ('target', 'caster', 'incarnation'):
                    enum = f'EGrpc{message}_{field}'
                    case = field.title()
                    header += [f'enum class {enum} : uint8\n{{\n    {case}=0,\n}};', f'{enum} _{field}Case{{}};']
                body = ''.join(f'    switch(in->_{field}_case())\n    {{\n    case pb::Present:\n        out->{field}=in->{field}();\n        break;\n    }}\n' for field in ('target', 'caster', 'incarnation'))
                source.append(f'void GRPC_TO_TURBOLINK(const ::pb::{message}* in, FGrpc{message}* out)\n{{\n{body}}}\n')
            h = root / 'Public/TestMessage.h'
            m = root / 'Private/TestMarshaling.cpp'
            h.write_text('\n'.join(header))
            m.write_text('\n'.join(source))
            for _ in range(2):
                subprocess.run([sys.executable, str(SCRIPT), str(root)], check=True, capture_output=True)
                self.assertNotIn('_case()', m.read_text())
                self.assertEqual(m.read_text().count('if (in->has_'), 6)
                for message in ('Cast', 'Effect'):
                    for field in ('target', 'caster', 'incarnation'):
                        self.assertIn(f'= EGrpc{message}_{field}::NotSet;', m.read_text())
            self.assertEqual(h.read_text().count('NotSet=255'), 6)


if __name__ == '__main__':
    unittest.main()
