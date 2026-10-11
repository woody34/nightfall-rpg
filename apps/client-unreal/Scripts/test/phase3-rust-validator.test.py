#!/usr/bin/env python3
"""Execute actual API bootstrap/admission validation, without Postgres/NATS."""
from pathlib import Path
import subprocess
import unittest

REPO = Path(__file__).resolve().parents[4]


class RustValidation(unittest.TestCase):
    def test_complete_overlay_and_every_role_through_production_admission(self):
        result = subprocess.run(['cargo', 'test', '--offline', '-p', 'nightfall-api', '--test',
                                 'phase3_fixture_overlays'], cwd=REPO,
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
