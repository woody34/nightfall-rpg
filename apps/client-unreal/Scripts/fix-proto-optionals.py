#!/usr/bin/env python3
"""Repair TurboLink 1.4.2's proto3 presence marshaling, without patching the plugin.

protoc represents optional scalars as synthetic oneofs in descriptors, but its C++ API exposes
has_<field>(), not <synthetic_oneof>_case(). TurboLink also defaults a oneof to its first case;
optional scalars require an explicit absent default because zero is a valid present value.
Singular messages stored by value also need presence independent of their contents.

Enum decoding validates the full protobuf integer before narrowing to a uint8 UE enum.
Only values declared in the generated Message.h enums are supported; reserved gaps
and unknown positive or negative values map to the declared zero/UNSPECIFIED member.
This intentionally loses unknown values on re-encoding, without aliasing known values.
"""
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
for header in (root / 'Public').rglob('*Message.h'):
    text = header.read_text()
    # A synthetic oneof's name starts with "_" and has exactly one member.
    pattern = re.compile(r'enum class (EGrpc\w+_\w+) : uint8\n\{\n    (\w+)=0,\n\};')
    optionals = []
    for match in pattern.finditer(text):
        enum, case = match.groups()
        field = re.sub(r'(?<!^)(?=[A-Z])', '_', case).lower()
        if not enum.endswith('_' + field):
            continue
        optionals.append((enum, case, field))
        text = text.replace(match.group(0), match.group(0).replace('    ' + case + '=0,', '    ' + case + '=0,\n    NotSet=255,'))
        text = text.replace(f'{enum} _{field}Case{{}};', f'{enum} _{field}Case = {enum}::NotSet;')
    header.write_text(text)
    if not optionals:
        continue
    marshal = root / 'Private' / header.relative_to(root / 'Public').parent / header.name.replace('Message.h', 'Marshaling.cpp')
    text = marshal.read_text()
    for enum, case, field in optionals:
        pattern = re.compile(r'    switch\(in->_' + re.escape(field) + r'_case\(\)\)\n    \{\n    case [^\n]+:\n(.*?)        break;\n    \}', re.S)
        message = 'F' + enum[1:].removesuffix('_' + field)
        function = re.compile(r'(^void GRPC_TO_TURBOLINK\([^\n]*\b' + re.escape(message)
                              + r'\* out\)\n\{\n)(.*?)(?=^void |\Z)', re.M | re.S)
        def patch_function(match):
            body, count = pattern.subn('    out->_' + field + '._' + field + 'Case = ' + enum + '::NotSet;\n'
                                      '    if (in->has_' + field + '())\n    {\n\\1    }', match[2])
            if count != 1:
                raise SystemExit(f'{marshal}: expected one synthetic optional {message}.{field}, found {count}')
            return match[1] + body
        text, count = function.subn(patch_function, text)
        if count != 1:
            raise SystemExit(f'{marshal}: expected one decoder for {message}, found {count}')
    marshal.write_text(text)

# Oneofs and nullable recursive messages already have their own presence carrier.
# Only direct, by-value singular message conversions have this generated shape.
for header in (root / 'Public').rglob('*Message.h'):
    marshal = root / 'Private' / header.relative_to(root / 'Public').parent / header.name.replace('Message.h', 'Marshaling.cpp')
    if not marshal.exists():
        continue
    header_text = header.read_text()
    text = marshal.read_text()
    decoder = re.compile(r'(^void GRPC_TO_TURBOLINK\([^\n]*, (FGrpc\w+)\* out\)\n\{\n)(.*?)(?=^void |\Z)', re.M | re.S)
    conversion = re.compile(r'^    GRPC_TO_TURBOLINK\(&\(in->(\w+)\(\)\), &\(out->(\w+)\)\);$', re.M)

    def patch_messages(match):
        global header_text, text
        message = match[2]
        body = match[3]
        for line in conversion.finditer(match[3]):
            field, member = line.groups()
            struct = re.compile(r'(^struct ' + re.escape(message)
                                + r' : public FGrpcMessage\n\{\n)(.*?)(^\};)', re.M | re.S)
            declaration = re.compile(r'^    FGrpc\w+ ' + re.escape(member) + r';$', re.M)
            def patch_struct(struct_match):
                declaration_match = declaration.search(struct_match[2])
                if declaration_match is None:
                    raise SystemExit(f'{header}: missing by-value message {message}.{member}')
                presence = ('\n\n    // Proto3 message presence is independent of a present, zero-valued sheet.\n'
                            '    UPROPERTY(BlueprintReadWrite, Category = TurboLink)\n'
                            f'    bool Has{member} = false;')
                struct_body = struct_match[2]
                if f'bool Has{member} = false;' not in struct_body:
                    end = declaration_match.end()
                    struct_body = struct_body[:end] + presence + struct_body[end:]
                return struct_match[1] + struct_body + struct_match[3]
            header_text, count = struct.subn(patch_struct, header_text)
            if count != 1:
                raise SystemExit(f'{header}: expected one struct {message}, found {count}')
            body = body.replace(line[0], f'    out->Has{member} = in->has_{field}();\n'
                                f'    out->{member} = {{}};\n'
                                f'    if (out->Has{member}) ' + line[0].strip())
            encoder = re.compile(r'(^void TURBOLINK_TO_GRPC\(const ' + re.escape(message)
                                 + r'\* in, [^\n]+\n\{\n)(.*?)(?=^void |\Z)', re.M | re.S)
            encode_line = f'    TURBOLINK_TO_GRPC(&(in->{member}), out->mutable_{field}());'
            def patch_encoder(encode_match):
                if encode_match[2].count(encode_line) != 1:
                    raise SystemExit(f'{marshal}: expected one encoder conversion {message}.{field}')
                return encode_match[1] + encode_match[2].replace(
                    encode_line, f'    if (in->Has{member}) ' + encode_line.strip()
                    + f'\n    else out->clear_{field}();')
            text, count = encoder.subn(patch_encoder, text)
            if count != 1:
                raise SystemExit(f'{marshal}: expected one encoder for {message}, found {count}')
        return match[1] + body

    # Encoder edits affect the same buffer: patch decoders individually rather than
    # overwriting those edits with a whole-file regex substitution result.
    for match in list(decoder.finditer(text)):
        replacement = patch_messages(match)
        text = text.replace(match[0], replacement, 1)
    header.write_text(header_text)
    marshal.write_text(text)

# Enum types can be imported from another generated Message.h (e.g. World uses Race).
# Read all declarations before patching conversions; do not hard-code current maxima.
enum_members = {}
enum_declaration = re.compile(r'enum class (EGrpc\w+) : uint8\n\{\n(.*?)\n\};', re.S)
member_declaration = re.compile(r'\s*(\w+)\s*=\s*(-?\d+)\s*,\s*')
for header in sorted((root / 'Public').rglob('*Message.h')):
    for declaration in enum_declaration.finditer(header.read_text()):
        enum, body = declaration.groups()
        members = list(member_declaration.finditer(body))
        if not members or member_declaration.sub('', body).strip():
            raise SystemExit(f'{header}: unsupported enum declaration {enum}')
        values = sorted({int(member[2]) for member in members})
        zero = next((member[1] for member in members if int(member[2]) == 0), None)
        if zero is None or values[0] < 0 or values[-1] > 255:
            raise SystemExit(f'{header}: {enum} requires a zero member and uint8 values')
        if enum in enum_members:
            raise SystemExit(f'{header}: duplicate enum declaration {enum}')
        enum_members[enum] = (values, zero)

enum_conversion = re.compile(r'(?:StaticCast|static_cast)<(EGrpc\w+)>\((in->\w+\([^()\n]*\))\)')
for marshal in sorted((root / 'Private').rglob('*Marshaling.cpp')):
    def patch_enum(match):
        enum, expression = match.groups()
        if enum not in enum_members:
            raise SystemExit(f'{marshal}: missing enum declaration {enum}')
        values, zero = enum_members[enum]
        # An int parameter preserves the protobuf integer. Switch membership also
        # rejects holes in sparse enums; the cast is reached only for known values.
        cases = ' '.join(f'case {value}:' for value in values)
        return (f'[](int value) {{ switch (value) {{ {cases} '
                f'return StaticCast<{enum}>(value); '
                f'default: return {enum}::{zero}; }} }}(static_cast<int>({expression}))')
    marshal.write_text(enum_conversion.sub(patch_enum, marshal.read_text()))

# Generated template blank lines contain trailing spaces; normalize deterministically.
for source in root.rglob('*'):
    if source.suffix in {'.h', '.cpp', '.cc'}:
        source.write_text('\n'.join(line.rstrip() for line in source.read_text().splitlines()) + '\n')
