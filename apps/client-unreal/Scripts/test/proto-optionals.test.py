#!/usr/bin/env python3
"""Message-scoped presence patching in disposable output, never client-owned code."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'fix-proto-optionals.py'


class OptionalTests(unittest.TestCase):
    def test_existing_oneof_repeated_and_pointer_presence_is_untouched(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'Public').mkdir()
            (root / 'Private').mkdir()
            h = root / 'Public/TestMessage.h'
            m = root / 'Private/TestMarshaling.cpp'
            h.write_text('''struct FGrpcEnvelope : public FGrpcMessage
{
    FGrpcSheet* Sheet = nullptr;
};
''')
            original = '''void GRPC_TO_TURBOLINK(const ::pb::Envelope* in, FGrpcEnvelope* out)
{
    switch(in->payload_case())
    {
    case pb::Envelope::kSheet:
        GRPC_TO_TURBOLINK(&(in->sheet()), &(out->Payload.Sheet));
        break;
    }
    for (int i = 0; i < in->sheets_size(); ++i)
    {
        GRPC_TO_TURBOLINK(&(in->sheets(i)), &(out->Sheets[i]));
    }
    if (in->has_sheet())
        GRPC_TO_TURBOLINK(&(in->sheet()), out->Sheet);
}
'''
            m.write_text(original)
            subprocess.run([sys.executable, str(SCRIPT), str(root)], check=True, capture_output=True)
            self.assertNotIn('HasSheet', h.read_text())
            self.assertEqual(m.read_text(), original)

    def test_nested_message_presence_and_reused_objects(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'Public').mkdir()
            (root / 'Private').mkdir()
            h = root / 'Public/TestMessage.h'
            m = root / 'Private/TestMarshaling.cpp'
            # Two messages deliberately share the same nested field name.
            h.write_text('''#define UPROPERTY(...)
struct FGrpcMessage {};
struct FGrpcEffectiveStats { int Value = 0; };
''' + ''.join(f'''struct FGrpc{message} : public FGrpcMessage
{{
    UPROPERTY(BlueprintReadWrite, Category = TurboLink)
    FGrpcEffectiveStats EffectiveStats;
}};
''' for message in ('StatsChanged', 'Other')))
            m.write_text(''.join(f'''void GRPC_TO_TURBOLINK(const ::pb::{message}* in, FGrpc{message}* out)
{{
    GRPC_TO_TURBOLINK(&(in->effective_stats()), &(out->EffectiveStats));
}}

void TURBOLINK_TO_GRPC(const FGrpc{message}* in, ::pb::{message}* out)
{{
    TURBOLINK_TO_GRPC(&(in->EffectiveStats), out->mutable_effective_stats());
}}

''' for message in ('StatsChanged', 'Other')))
            subprocess.run([sys.executable, str(SCRIPT), str(root)], check=True, capture_output=True)
            patched = h.read_text(), m.read_text()
            subprocess.run([sys.executable, str(SCRIPT), str(root)], check=True, capture_output=True)
            self.assertEqual((h.read_text(), m.read_text()), patched)
            self.assertEqual(h.read_text().count('bool HasEffectiveStats = false;'), 2)
            self.assertEqual(m.read_text().count('else out->clear_effective_stats();'), 2)

            # Compile and execute the patched conversion functions against a minimal
            # protobuf-shaped API. This tests generated control flow, not wire encoding;
            # byte-exact protobuf coverage belongs to the client's native bridge tests.
            cpp = root / 'presence.cpp'
            cpp.write_text('''#include "Public/TestMessage.h"
namespace pb {
struct EffectiveStats { int value = 0; };
struct StatsChanged {
    bool present = false;
    EffectiveStats sheet;
    bool has_effective_stats() const { return present; }
    const EffectiveStats& effective_stats() const { return sheet; }
    EffectiveStats* mutable_effective_stats() { present = true; return &sheet; }
    void clear_effective_stats() { present = false; sheet = {}; }
};
struct Other : StatsChanged {};
}
void GRPC_TO_TURBOLINK(const pb::EffectiveStats* in, FGrpcEffectiveStats* out) {
    out->Value = in->value;
}
void TURBOLINK_TO_GRPC(const FGrpcEffectiveStats* in, pb::EffectiveStats* out) {
    out->value = in->Value;
}
#include "Private/TestMarshaling.cpp"
int main() {
    pb::StatsChanged source, encoded;
    FGrpcStatsChanged bridge;
    if (bridge.HasEffectiveStats) return 1;
    for (int value : {0, 42, -9}) {
        source.mutable_effective_stats()->value = value;
        GRPC_TO_TURBOLINK(&source, &bridge);
        if (!bridge.HasEffectiveStats || bridge.EffectiveStats.Value != value) return 2;
        TURBOLINK_TO_GRPC(&bridge, &encoded);
        if (!encoded.has_effective_stats() || encoded.sheet.value != value) return 3;
        source.clear_effective_stats();
        GRPC_TO_TURBOLINK(&source, &bridge);
        if (bridge.HasEffectiveStats || bridge.EffectiveStats.Value != 0) return 4;
        TURBOLINK_TO_GRPC(&bridge, &encoded);
        if (encoded.has_effective_stats() || encoded.sheet.value != 0) return 5;
    }
    return 0;
}
'''.replace('#include "Public', '#include <initializer_list>\n#include "Public', 1))
            binary = root / 'presence'
            subprocess.run(['c++', '-std=c++17', '-Wall', '-Wextra', '-Werror',
                            str(cpp), '-o', str(binary)], check=True, capture_output=True)
            subprocess.run([str(binary)], check=True, capture_output=True)

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
