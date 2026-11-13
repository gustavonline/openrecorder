#!/usr/bin/env python3
"""Write the macOS app icon (an .icns file) for OpenRecorder: the black disc of assets/icon.svg on a transparent square.

Usage: python3 scripts/render-app-icon.py OUTPUT.icns
Standard library only and deterministic (the same bytes every run). The geometry is the one in assets/icon.svg and
scripts/render-icons.py: a black disc of radius 9 in a 20 unit box. The edge is anti-aliased from the exact distance to the circle.
The container holds PNG images for 16 to 1024 pixels, the sizes macOS asks for (icp4-icp6, ic07-ic14).
"""
import pathlib, struct, sys, zlib

SIZES = (16, 32, 64, 128, 256, 512, 1024)
# icns entry type -> pixel size of the PNG it holds (the @2x types hold the doubled size).
ENTRIES = (('icp4', 16), ('icp5', 32), ('icp6', 64), ('ic07', 128), ('ic08', 256), ('ic09', 512), ('ic10', 1024),
           ('ic11', 32), ('ic12', 64), ('ic13', 256), ('ic14', 512))


def disc_png(size):
    centre, radius = size / 2, size * 9 / 20
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        dy = y + .5 - centre
        for x in range(size):
            dx = x + .5 - centre
            coverage = min(1.0, max(0.0, radius - (dx * dx + dy * dy) ** .5 + .5))
            rows += bytes((0, 0, 0, round(coverage * 255)))

    def chunk(kind, data):
        body = kind + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body) & 0xFFFFFFFF)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(bytes(rows), 9)) + chunk(b'IEND', b'')


def build_icns():
    pngs = {size: disc_png(size) for size in SIZES}
    body = b''.join(kind.encode('ascii') + struct.pack('>I', 8 + len(pngs[size])) + pngs[size] for kind, size in ENTRIES)
    return b'icns' + struct.pack('>I', 8 + len(body)) + body


if __name__ == '__main__':
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    pathlib.Path(sys.argv[1]).write_bytes(build_icns())
