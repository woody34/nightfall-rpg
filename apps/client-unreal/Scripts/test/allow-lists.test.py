#!/usr/bin/env python3
"""Every message-specific `.allow` substring is text the server actually sends (review-5 P1).

An allow entry `<category> | CallRpcError: <Status>:<detail> | <reason>` only matches when
<detail> is the wire text. Wire texts are the RejectReason::detail() strings
(domain/zone/command.rs), literal AppError::<Variant>("...") messages and AppError's own
#[error("...")] texts (application/error.rs). Domain-internal thiserror strings never reach
the client, so they do not count.
"""
from pathlib import Path
import re
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
REPO = SCRIPTS.parents[2]
API = REPO / 'apps/api/src'


def wire_texts():
    texts = set()
    command = (API / 'domain/zone/command.rs').read_text()
    body = command[command.index('pub const fn detail(self)'):]
    body = body[:body.index('\n    }\n')]
    texts |= set(re.findall(r'=> "([^"]+)"', body))
    for path in API.rglob('*.rs'):
        texts |= set(re.findall(r'AppError::\w+\(\s*"([^"]+)"', path.read_text()))
    texts |= set(re.findall(r'#\[error\("([^"{}]+)"\)\]', (API / 'application/error.rs').read_text()))
    return texts


def allow_entries():
    for path in sorted((SCRIPTS.parent / 'Scenarios').rglob('*.allow')):
        for number, line in enumerate(path.read_text().splitlines(), 1):
            if not line.strip() or line.lstrip().startswith('#'):
                continue
            parts = [p.strip() for p in line.split('|')]
            yield path, number, parts


class AllowLists(unittest.TestCase):
    def test_every_allow_line_has_category_substring_and_reason(self):
        entries = list(allow_entries())
        self.assertEqual(len({p for p, _, _ in entries}), 14)
        for path, number, parts in entries:
            with self.subTest(f'{path.name}:{number}'):
                self.assertEqual(len(parts), 3)
                self.assertTrue(all(parts))

    def test_message_specific_substrings_are_wire_texts(self):
        texts = wire_texts()
        self.assertIn('skill learning requires an inventory item', texts)
        self.assertNotIn('learning requires an inventory item', texts)  # internal state_learn.rs text
        checked = 0
        for path, number, (category, substring, reason) in allow_entries():
            match = re.fullmatch(r'CallRpcError: (\w+):(.+)', substring)
            if not match:
                continue
            checked += 1
            with self.subTest(f'{path.name}:{number}'):
                self.assertIn(match[2], texts, f'{path.name}:{number} allows text the server never sends')
        self.assertEqual(checked, 11)  # 3-learn-rejected x3, six 3-learn-* variants, 4-grant-item-denied, 4-item-skill-learn


if __name__ == '__main__':
    unittest.main()
