"""The macOS app icon: a valid icns of the project's black disc, the same bytes every run, with every size macOS asks for.

Usage: python3 tests/app-icon.py     (pure file checks; nothing is installed or signed)
"""
import importlib.util, pathlib, struct, sys, zlib

root = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('render_app_icon', root / 'scripts/render-app-icon.py')
icon = importlib.util.module_from_spec(spec); spec.loader.exec_module(icon)

data = icon.build_icns()
assert data == icon.build_icns(), 'the icon must be deterministic'
assert data[:4] == b'icns' and struct.unpack('>I', data[4:8])[0] == len(data), 'icns header and total length'


def entries(blob):
    pos = 8
    while pos < len(blob):
        kind, length = blob[pos:pos + 4].decode('ascii'), struct.unpack('>I', blob[pos + 4:pos + 8])[0]
        assert length >= 8 and pos + length <= len(blob), f'{kind} runs past the file'
        yield kind, blob[pos + 8:pos + length]; pos += length


def decode(png):
    """RGBA rows of a PNG this script wrote (8-bit RGBA, filter 0 only)."""
    assert png[:8] == b'\x89PNG\r\n\x1a\n'
    pos, idat, size = 8, b'', None
    while pos < len(png):
        length, kind = struct.unpack('>I', png[pos:pos + 4])[0], png[pos + 4:pos + 8]
        body = png[pos + 8:pos + 8 + length]
        assert struct.unpack('>I', png[pos + 8 + length:pos + 12 + length])[0] == zlib.crc32(kind + body) & 0xFFFFFFFF, f'{kind} crc'
        if kind == b'IHDR': size = struct.unpack('>II', body[:8]); assert body[8:10] == bytes((8, 6))
        if kind == b'IDAT': idat += body
        pos += 12 + length
    raw, w = zlib.decompress(idat), size[0]
    rows = [raw[i * (w * 4 + 1) + 1:(i + 1) * (w * 4 + 1)] for i in range(size[1])]
    assert all(raw[i * (w * 4 + 1)] == 0 for i in range(size[1])), 'only filter 0'
    return size, rows


seen = dict(entries(data))
assert set(seen) == {k for k, _ in icon.ENTRIES}, sorted(seen)
for kind, pixels in icon.ENTRIES:
    (w, h), rows = decode(seen[kind])
    assert (w, h) == (pixels, pixels), f'{kind} is {w}x{h}, expected {pixels}'
    px = lambda x, y: rows[y][x * 4:x * 4 + 4]
    mid = pixels // 2
    assert px(mid, mid) == bytes((0, 0, 0, 255)), f'{kind}: the centre is an opaque black disc'
    assert px(0, 0)[3] == 0 and px(pixels - 1, pixels - 1)[3] == 0, f'{kind}: the corners are transparent'
    # The disc spans 90% of the width (radius 9 of 20 units): the outermost row is mostly empty, 20% in is solid.
    assert px(mid, 0)[3] < 128 and px(mid, int(pixels * 0.2))[3] == 255 and px(int(pixels * 0.2), mid)[3] == 255, f'{kind}: disc size'
# The build script must ship it and say so in the bundle metadata; the system the app needs comes from the files, not a typed number.
build = (root / 'scripts/build-native.py').read_text()
for needle in ('render-app-icon.py', '"CFBundleIconFile":"OpenRecorder"', 'contents/"Resources"', 'LSMinimumSystemVersion":needed', 'LC_BUILD_VERSION'):
    assert needle in build, needle
assert '"LSMinimumSystemVersion":"12.3"' not in build
print('app icon: ok')
