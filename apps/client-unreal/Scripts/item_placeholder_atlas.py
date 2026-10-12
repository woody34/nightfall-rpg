"""Deterministic cosmetic item-kind atlas. No gameplay stats or source art."""
from pathlib import Path
import struct
import zlib


def write_atlas(path):
    width, height = 192, 64
    colours = ((90, 155, 220), (175, 150, 90), (115, 185, 120))
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            cell, px = divmod(x, 64)
            edge = px < 3 or px >= 61 or y < 3 or y >= 61
            # Sword, armour and bottle silhouettes are purely placeholders.
            shape = ((26 <= px <= 37 and 10 <= y <= 54) or (18 <= px <= 45 and 42 <= y <= 47)) if cell == 0 else ((15 <= px <= 48 and 14 <= y <= 52) and not (25 <= px <= 38 and y < 23)) if cell == 1 else (22 <= px <= 41 and 23 <= y <= 52) or (27 <= px <= 36 and 12 <= y <= 25)
            colour = colours[cell] if shape else (35, 40, 48)
            rows.extend((*((15, 18, 22) if edge else colour), 255))
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png)
    return path
