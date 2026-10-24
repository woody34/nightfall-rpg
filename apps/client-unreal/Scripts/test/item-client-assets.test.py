#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
import zlib

scripts = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('item_atlas', scripts / 'item_placeholder_atlas.py')
atlas = importlib.util.module_from_spec(spec)
spec.loader.exec_module(atlas)


class ItemClientAssetsTest(unittest.TestCase):
    def test_metadata_matches_pinned_import(self):
        subprocess.run(['python3', str(scripts / 'gen-item-client-metadata.py'), '--check'], check=True)

    def test_atlas_decodes_and_is_repeatable(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as tmp:
            one = atlas.write_atlas(Path(tmp) / 'one.png').read_bytes()
            two = atlas.write_atlas(Path(tmp) / 'two.png').read_bytes()
        self.assertEqual(one, two)
        self.assertEqual(one[:8], b'\x89PNG\r\n\x1a\n')
        offset, pixels = 8, None
        while offset < len(one):
            size = struct.unpack('>I', one[offset:offset + 4])[0]
            kind = one[offset + 4:offset + 8]
            data = one[offset + 8:offset + 8 + size]
            crc = struct.unpack('>I', one[offset + 8 + size:offset + 12 + size])[0]
            self.assertEqual(crc, zlib.crc32(kind + data))
            if kind == b'IHDR':
                self.assertEqual(struct.unpack('>II', data[:8]), (192, 64))
            if kind == b'IDAT':
                pixels = zlib.decompress(data)
            offset += 12 + size
        self.assertEqual(len(pixels), 64 * (1 + 192 * 4))
        self.assertEqual(len({pixels[y * 769 + 1 + x * 4:y * 769 + 5 + x * 4] for y in range(64) for x in range(192)}), 5)


if __name__ == '__main__':
    unittest.main()
