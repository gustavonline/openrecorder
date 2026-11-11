#!/usr/bin/env python3
"""Render the OpenRecorder black-disc icon to square PNGs without any image library.

The geometry is the one in assets/icon.svg (a black disc, radius 9 in a 20 unit box) and assets/icon-dark.svg (radius 8.5
with a #4a4a50 hairline ring, for dark surfaces). The output is anti-aliased by 8x8 supersampling and is deterministic.
Usage: python3 scripts/render-icons.py OUTPUT_DIR   (writes icon-48.png, icon-256.png, icon-dark-48.png, icon-dark-256.png)
"""
import pathlib, struct, sys, zlib

SS = 8


def png(path, size, pixel):
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            r = g = b = a = 0.0
            for sy in range(SS):
                for sx in range(SS):
                    pr, pg, pb, pa = pixel((x + (sx + .5) / SS) / size * 20, (y + (sy + .5) / SS) / size * 20)
                    r, g, b, a = r + pr * pa, g + pg * pa, b + pb * pa, a + pa
            n = SS * SS
            if a:
                rows += bytes((round(r / a), round(g / a), round(b / a), round(a / n * 255)))
            else:
                rows += bytes((0, 0, 0, 0))
    def chunk(kind, data):
        body = kind + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body) & 0xFFFFFFFF)
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(bytes(rows), 9)) + chunk(b'IEND', b''))


def light(x, y):
    return (0, 0, 0, 1.0) if (x - 10) ** 2 + (y - 10) ** 2 <= 81 else (0, 0, 0, 0.0)


def dark(x, y):
    d2 = (x - 10) ** 2 + (y - 10) ** 2
    if d2 <= 8.0 ** 2:
        return (0, 0, 0, 1.0)
    if d2 <= 9.0 ** 2 and d2 >= 8.0 ** 2:      # the 1 unit hairline centred on radius 8.5
        return (0x4a, 0x4a, 0x50, 1.0)
    return (0, 0, 0, 0.0)


if __name__ == '__main__':
    out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
    for size in (48, 256):
        png(out / f'icon-{size}.png', size, light)
        png(out / f'icon-dark-{size}.png', size, dark)
    print('rendered', *sorted(p.name for p in out.glob('icon*.png')))
