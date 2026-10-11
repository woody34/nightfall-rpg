#!/usr/bin/env python3
"""Execute enum guards in synthetic TurboLink output without modifying generated C++."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'fix-proto-optionals.py'


class EnumTests(unittest.TestCase):
    def patch(self, root):
        return subprocess.run([sys.executable, str(SCRIPT), str(root)],
                              check=True, capture_output=True)

    def test_full_integer_membership_and_idempotence(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'Public/Imported').mkdir(parents=True)
            (root / 'Private/Consumer').mkdir(parents=True)
            header = root / 'Public/Imported/CatalogMessage.h'
            marshal = root / 'Private/Consumer/WorldMarshaling.cpp'
            # Imported type, sparse values, and aliases all come from declarations.
            header.write_text('''enum class EGrpcOperation : uint8
{
    OP_UNSPECIFIED=0,
    OP_ACTIVE=1,
    OP_ACTIVE_ALIAS=1,
    OP_PASSIVE=3,
    OP_OTHER=6,
};
enum class EGrpcState : uint8
{
    Zero=0,
    Last=255,
};
''')
            marshal.write_text('''void GRPC_TO_TURBOLINK(const pb::Catalog* in, FGrpcCatalog* out)
{
    out->Operation=StaticCast<EGrpcOperation>(in->operation());
    out->State=static_cast<EGrpcState>(in->state());
    for (int i = 0; i < 3; ++i)
        out->Repeated[i]=StaticCast<EGrpcOperation>(in->operations(i));
    if (in->has_optional())
        out->Optional=StaticCast<EGrpcOperation>(in->optional());
    switch(in->payload_case())
    {
    case 1:
        out->Payload=StaticCast<EGrpcOperation>(in->payload());
        break;
    }
}
void TURBOLINK_TO_GRPC(const FGrpcCatalog* in, pb::Catalog* out)
{
    out->set_operation(pb::Operation(static_cast<uint8>(in->Operation)));
}
''')
            self.patch(root)
            first_pass = header.read_bytes(), marshal.read_bytes()
            self.patch(root)
            self.assertEqual((header.read_bytes(), marshal.read_bytes()), first_pass)
            # Sparse declarations validate membership, not just the min/max range.
            self.assertIn('case 6:', marshal.read_text())
            self.assertNotIn('case 2:', marshal.read_text())
            cpp = root / 'enums.cpp'
            cpp.write_text('''#include <cstdint>
#include <initializer_list>
using uint8 = std::uint8_t;
template<class T, class U> T StaticCast(U value) { return static_cast<T>(value); }
#include "Public/Imported/CatalogMessage.h"
namespace pb {
enum Operation : int { Unspecified=0, Active=1, Passive=3, Other=6 };
struct Catalog {
    int raw = 0;
    Operation operation() const { return static_cast<Operation>(raw); }
    int state() const { return raw; }
    Operation operations(int) const { return operation(); }
    bool has_optional() const { return true; }
    Operation optional() const { return operation(); }
    int payload_case() const { return 1; }
    Operation payload() const { return operation(); }
    void set_operation(Operation value) { raw = static_cast<int>(value); }
};
}
struct FGrpcCatalog {
    EGrpcOperation Operation{}, Repeated[3]{}, Optional{}, Payload{};
    EGrpcState State{};
};
#include "Private/Consumer/WorldMarshaling.cpp"
int main() {
    pb::Catalog wire, encoded;
    FGrpcCatalog bridge;
    for (int raw : {0, 1, 3, 6, 2, 7, 254, 255, 256, 257, 259, 262,
                    -1, -255, -253, -250, INT32_MIN, INT32_MAX}) {
        wire.raw = raw;
        GRPC_TO_TURBOLINK(&wire, &bridge);
        const int expected = (raw == 0 || raw == 1 || raw == 3 || raw == 6) ? raw : 0;
        if (static_cast<int>(bridge.Operation) != expected) return 1;
        for (auto value : bridge.Repeated)
            if (static_cast<int>(value) != expected) return 2;
        if (static_cast<int>(bridge.Optional) != expected) return 3;
        if (static_cast<int>(bridge.Payload) != expected) return 4;
        const int state = raw == 255 ? 255 : 0;
        if (static_cast<int>(bridge.State) != state) return 5;
        TURBOLINK_TO_GRPC(&bridge, &encoded);
        if (encoded.raw != expected) return 6;
    }
    return 0;
}
''')
            binary = root / 'enums'
            subprocess.run(['c++', '-std=c++17', '-Wall', '-Wextra', '-Werror',
                            str(cpp), '-o', str(binary)], check=True, capture_output=True)
            subprocess.run([str(binary)], check=True, capture_output=True)

            # Restore fresh generated input after adding an enum member.
            header.write_text(header.read_text().replace('    OP_OTHER=6,',
                                                        '    OP_OTHER=6,\n    OP_FUTURE=7,'))
            marshal.write_text('out->Operation=StaticCast<EGrpcOperation>(in->operation());\n')
            self.patch(root)
            self.assertIn('case 7:', marshal.read_text())

    def test_missing_declaration_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'Private').mkdir()
            (root / 'Private/TestMarshaling.cpp').write_text(
                'out->Operation=StaticCast<EGrpcMissing>(in->operation());\n')
            with self.assertRaises(subprocess.CalledProcessError) as raised:
                self.patch(root)
            self.assertIn(b'missing enum declaration EGrpcMissing', raised.exception.stderr)


if __name__ == '__main__':
    unittest.main()
