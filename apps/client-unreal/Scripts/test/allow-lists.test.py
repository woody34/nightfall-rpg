#!/usr/bin/env python3
"""Every message-specific `.allow` substring is a (status, text) the server actually sends.

An allow entry `<category> | CallRpcError: <Status>:<detail> | <reason>` only matches when the
gRPC status and <detail> are what the client receives (review-5 P1, review-6 nits). The wire
pairs come from the API source on this branch, outside test code:
- RejectReason::detail() strings (domain/zone/command.rs, arms may wrap to the next line),
  with the status that application/zone_actor.rs class_error() maps each reason to;
- literal AppError::<Variant>("...") messages, with that variant's status
  (interface/grpc/status.rs maps each variant to the same-named code);
- AppError::IdempotencyConflict, sent as FailedPrecondition with its #[error] text.
Domain-internal thiserror strings never reach the client, so they do not count.
"""
from pathlib import Path
import re
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
REPO = SCRIPTS.parents[2]
API = REPO / 'apps/api/src'


def production(text):
    """Source without its #[cfg(test)] items: `mod x;` declarations and brace-delimited
    modules/functions (brace counting; test code does not leak wire texts into the set)."""
    out, rest = [], text
    while '#[cfg(test)]' in rest:
        head, rest = rest.split('#[cfg(test)]', 1)
        out.append(head)
        item = re.match(r'(?:\s*#\[[^\]]*\])*\s*[^;{]*', rest)
        end = item.end()
        if rest[end:end + 1] == ';':
            rest = rest[end + 1:]
            continue
        depth, index = 0, end
        while index < len(rest):
            depth += {'{': 1, '}': -1}.get(rest[index], 0)
            index += 1
            if depth == 0 and rest[index - 1] == '}':
                break
        rest = rest[index:]
    return ''.join(out) + rest


def wire_pairs():
    pairs = set()
    command = production((API / 'domain/zone/command.rs').read_text())
    body = command[command.index('pub const fn detail(self)'):]
    body = body[:body.index('\n    }\n')]
    details = {}
    for names, text in re.findall(r'((?:Self::\w+\s*\|\s*)*Self::\w+)\s*=>\s*\{?\s*"([^"]+)"', body):
        for name in re.findall(r'Self::(\w+)', names):
            details[name] = text
    actor = production((API / 'application/zone_actor.rs').read_text())
    mapping = actor[actor.index('pub(crate) fn class_error('):]
    mapping = mapping[:mapping.index('\n}\n')]
    for names, variant in re.findall(r'((?:RejectReason::\w+\s*\|\s*)*RejectReason::\w+)\s*=>\s*super::AppError::(\w+)', mapping):
        for name in re.findall(r'RejectReason::(\w+)', names):
            if variant == 'IdempotencyConflict':
                continue  # sent with the #[error] text, below
            pairs.add((variant, details[name]))
    for path in API.rglob('*.rs'):
        if path.name.endswith('_tests.rs') or 'tests' in path.relative_to(API).parts:
            continue
        for variant, text in re.findall(r'AppError::(\w+)\(\s*"([^"]+)"', production(path.read_text())):
            pairs.add((variant, text))
    errors = (API / 'application/error.rs').read_text()
    conflict = re.search(r'#\[error\("([^"]+)"\)\]\s*IdempotencyConflict', errors)
    pairs.add(('FailedPrecondition', conflict[1]))
    return pairs


def allow_entries():
    for path in sorted((SCRIPTS.parent / 'Scenarios').rglob('*.allow')):
        for number, line in enumerate(path.read_text().splitlines(), 1):
            if not line.strip() or line.lstrip().startswith('#'):
                continue
            yield path, number, [p.strip() for p in line.split('|')]


class AllowLists(unittest.TestCase):
    def test_every_allow_line_has_category_substring_and_reason(self):
        entries = list(allow_entries())
        self.assertEqual(len({p for p, _, _ in entries}), 14)
        for path, number, parts in entries:
            with self.subTest(f'{path.name}:{number}'):
                self.assertEqual(len(parts), 3)
                self.assertTrue(all(parts))

    def test_wire_pairs_are_derived_exactly(self):
        pairs = wire_pairs()
        self.assertIn(('FailedPrecondition', 'skill learning requires an inventory item'), pairs)
        self.assertNotIn(('FailedPrecondition', 'learning requires an inventory item'), pairs)  # internal text
        self.assertIn(('FailedPrecondition', 'insufficient SP'), pairs)
        self.assertNotIn(('InvalidArgument', 'insufficient SP'), pairs)  # wrong status prefix
        self.assertIn(('InvalidArgument', 'unknown skill_id'), pairs)
        self.assertIn(('FailedPrecondition', 'idempotency key reused with a different request'), pairs)
        texts = {text for _, text in pairs}
        self.assertFalse({'x', 'full', 'stopped'} & texts)  # strings from test code are excluded

    def test_message_specific_substrings_are_wire_pairs(self):
        pairs = wire_pairs()
        checked = 0
        for path, number, (category, substring, reason) in allow_entries():
            match = re.fullmatch(r'CallRpcError: (\w+):(.+)', substring)
            if not match:
                continue
            checked += 1
            with self.subTest(f'{path.name}:{number}'):
                self.assertIn((match[1], match[2]), pairs, f'{path.name}:{number} allows a status/text the server never sends')
        # 3-learn-rejected x3, six 3-learn-* variants, 4-grant-item-denied, 4-item-skill-learn.
        self.assertEqual(checked, 11)


if __name__ == '__main__':
    unittest.main()
