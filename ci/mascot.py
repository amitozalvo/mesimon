#!/usr/bin/env python3
"""Build the shin's SVG/PNG assets and installer art (stdlib only).

Run from any directory; --check verifies the checked-in outputs. The terminal
drawings are optically tuned, not downsampled from the notification geometry.
"""
import argparse
import math
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1]
FULL = """                   ▄██▄
  ▄██▄             ████
  ████            ▄███▀
  ▀███▄    ▄██▄   ████
   ████    ████  ▄███▀
   ▀███▄   ████  ████
    ████████████████
  ▄██████████████████▄
  █████  ██████  █████
  █████▄▄██████▄▄█████
   ▀████████████████▀
      ▀▀▀▀▀▀▀▀▀▀▀▀
"""
COMPACT = """                 ▄█▄
 ▄█▄    ▄▄▄     ▄███
 ▀██▄   ███    ▄███▀
  ▀███████████████▀
  ████  █████  ████
  ▀███▄▄█████▄▄███▀
    ▀▀█████████▀▀
"""
GROUND = (0x13, 0x14, 0x17)
MARK = (0xB6, 0xB2, 0xA9)
ATTN = (0xF0, 0xA9, 0x3A)


def shapes(attention):
    # Filled geometry only. Eyes reveal the tile's ground.
    return [
        (GROUND, "roundrect", (0, 0, 128, 128, 27)),
        (MARK, "capsule", (27, 39, 37, 73, 8)),
        (MARK, "capsule", (64, 51, 64, 74, 8)),
        (MARK, "capsule", (99 if attention else 105,
                             49 if attention else 25, 91, 73, 8)),
        (MARK, "roundrect", (24, 65, 80, 47, 18)),
        (GROUND, "circle", (48, 88, 5)),
        (GROUND, "circle", (79, 88, 5)),
    ] + ([(ATTN, "circle", (105, 25, 7))] if attention else [])


def inside(kind, p, x, y):
    if kind == "circle":
        cx, cy, r = p
        return (x-cx)**2 + (y-cy)**2 <= r*r
    if kind == "capsule":
        ax, ay, bx, by, r = p
        dx, dy = bx-ax, by-ay
        t = max(0, min(1, ((x-ax)*dx + (y-ay)*dy)/(dx*dx + dy*dy)))
        return (x-ax-t*dx)**2 + (y-ay-t*dy)**2 <= r*r
    ax, ay, w, h, r = p
    qx, qy = abs(x-ax-w/2)-w/2+r, abs(y-ay-h/2)-h/2+r
    return math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) <= r


def svg(attention):
    parts = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">',
             '<title>Mesimon — ' + ('needs you' if attention else 'resting') + '</title>']
    for color, kind, p in shapes(attention):
        fill = '#' + bytes(color).hex()
        if kind == "circle":
            x, y, r = p
            parts.append(f'<circle cx="{x}" cy="{y}" r="{r}" fill="{fill}"/>')
        elif kind == "roundrect":
            x, y, w, h, r = p
            parts.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}"/>')
        else:
            ax, ay, bx, by, r = p
            length = math.hypot(bx-ax, by-ay)
            dx, dy = -(by-ay)*r/length, (bx-ax)*r/length
            d = (f'M{ax+dx:.5f},{ay+dy:.5f} L{bx+dx:.5f},{by+dy:.5f} '
                 f'A{r},{r} 0 0 0 {bx-dx:.5f},{by-dy:.5f} '
                 f'L{ax-dx:.5f},{ay-dy:.5f} A{r},{r} 0 0 0 {ax+dx:.5f},{ay+dy:.5f} Z')
            parts.append(f'<path d="{d}" fill="{fill}"/>')
    return ('\n'.join(parts) + '\n</svg>\n').encode()


def png(attention):
    size, samples = 256, 2
    geometry = shapes(attention)
    pixels = bytearray()
    for y in range(size):
        pixels.append(0)  # PNG row filter: none
        for x in range(size):
            colors = []
            for sy in range(samples):
                for sx in range(samples):
                    px, py = (x+(sx+.5)/samples)*128/size, (y+(sy+.5)/samples)*128/size
                    color = None
                    for candidate, kind, p in geometry:
                        if inside(kind, p, px, py):
                            color = candidate
                    if color is not None:
                        colors.append(color)
            rgb = [round(sum(c[i] for c in colors)/len(colors)) for i in range(3)] if colors else [0]*3
            pixels.extend([*rgb, round(255*len(colors)/(samples*samples))])

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind+data))

    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(pixels, 9)) + chunk(b'IEND', b''))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    assert len(FULL.splitlines()) == 12 and max(map(len, FULL.splitlines())) <= 24
    assert len(COMPACT.splitlines()) == 7 and max(map(len, COMPACT.splitlines())) <= 20
    outputs = {'resting.txt': FULL.encode(), 'compact.txt': COMPACT.encode()}
    for attention, name in [(False, 'resting'), (True, 'needs-you')]:
        outputs[name+'.svg'] = svg(attention)
        outputs[name+'.png'] = png(attention)
    for name, data in outputs.items():
        path = ROOT / 'assets/mascot' / name
        if args.check:
            assert path.read_bytes() == data, f'{path} is stale; run python3 -B ci/mascot.py'
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    installer = ROOT / 'install.sh'
    start, end = '# mascot:start\n', '# mascot:end'
    original = installer.read_text()
    before, tail = original.split(start, 1)
    _, after = tail.split(end, 1)
    updated = before + start + "  cat <<'MESIMON_SHIN'\n" + FULL + 'MESIMON_SHIN\n' + end + after
    if args.check:
        assert updated == original, 'installer mascot is stale'
    else:
        installer.write_text(updated)


if __name__ == '__main__':
    main()
