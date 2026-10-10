#!/usr/bin/env python3
"""Repair TurboLink 1.4.2's proto3 optional scalar marshaling, without patching the plugin.

protoc represents optional scalars as synthetic oneofs in descriptors, but its C++ API exposes
has_<field>(), not <synthetic_oneof>_case(). TurboLink also defaults a oneof to its first case;
optional scalars require an explicit absent default because zero is a valid present value.
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

# Generated template blank lines contain trailing spaces; normalize deterministically.
for source in root.rglob('*'):
    if source.suffix in {'.h', '.cpp', '.cc'}:
        source.write_text('\n'.join(line.rstrip() for line in source.read_text().splitlines()) + '\n')
