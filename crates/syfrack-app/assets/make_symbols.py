"""Writes symbols.ttf: a tiny TrueType font holding the studio's symbols
missing from egui's default fonts, drawn here as polygons (no third-party
outline, no licence to carry):

  U+2192 → (Point → Point in the graph messages)
  U+25CF ● (the « modified » mark of the title bar)

Run from this folder: python make_symbols.py
"""
import math
import struct

UPM = 1000
ASC, DESC = 800, -200
MID = 270  # centre of the symbols, near the middle of the lower case


def arrow():
    # Shaft then head, one closed polygon, clockwise.
    t, h = 40, 150  # shaft half-thickness, head half-height
    x0, x1, x2 = 90, 600, 760
    return [[(x0, MID - t), (x0, MID + t), (x1, MID + t), (x1, MID + h),
             (x2, MID), (x1, MID - h), (x1, MID - t)]]


def bullet():
    r, cx, n = 250, 380, 32
    pts = [(round(cx + r * math.cos(-2 * math.pi * k / n)),
            round(MID + r * math.sin(-2 * math.pi * k / n))) for k in range(n)]
    return [pts]


GLYPHS = [  # (codepoint, advance, contours)
    (None, 500, []),       # .notdef
    (0x2192, 850, arrow()),
    (0x25CF, 760, bullet()),
]


def glyf_entry(contours):
    if not contours:
        return b""
    pts = [p for c in contours for p in c]
    xs, ys = [p[0] for p in pts], [p[1] for p in pts]
    out = struct.pack(">hhhhh", len(contours), min(xs), min(ys), max(xs), max(ys))
    end = -1
    for c in contours:
        end += len(c)
        out += struct.pack(">H", end)
    out += struct.pack(">H", 0)  # no instructions
    out += bytes([1] * len(pts))  # on-curve, 16-bit deltas
    px = py = 0
    dx, dy = b"", b""
    for x, y in pts:
        dx += struct.pack(">h", x - px)
        dy += struct.pack(">h", y - py)
        px, py = x, y
    out += dx + dy
    if len(out) % 2:
        out += b"\0"
    return out


def cmap():
    codes = sorted((g[0], i) for i, g in enumerate(GLYPHS) if g[0] is not None)
    segs = [(c, c, gid) for c, gid in codes] + [(0xFFFF, 0xFFFF, 0)]
    n = len(segs)
    sr = 2 * 2 ** int(math.log2(n))
    sub = struct.pack(">HHHHHHH", 4, 0, 0, n * 2, sr, int(math.log2(sr // 2)), n * 2 - sr)
    sub += b"".join(struct.pack(">H", e) for _, e, _ in segs)
    sub += struct.pack(">H", 0)
    sub += b"".join(struct.pack(">H", s) for s, _, _ in segs)
    sub += b"".join(struct.pack(">h", (gid - s) if gid else 1) for s, _, gid in segs)
    sub += b"".join(struct.pack(">H", 0) for _ in segs)
    length = len(sub)
    sub = sub[:2] + struct.pack(">H", length) + sub[4:]
    return struct.pack(">HHHHI", 0, 1, 3, 1, 12) + sub


def build():
    glyf = b""
    loca = []
    for _, _, c in GLYPHS:
        loca.append(len(glyf))
        glyf += glyf_entry(c)
    loca.append(len(glyf))
    allpts = [p for _, _, cs in GLYPHS for c in cs for p in c]
    xmin = min(p[0] for p in allpts); xmax = max(p[0] for p in allpts)
    ymin = min(p[1] for p in allpts); ymax = max(p[1] for p in allpts)
    n = len(GLYPHS)
    maxpts = max((sum(len(c) for c in cs) for _, _, cs in GLYPHS), default=0)
    tables = {
        b"head": struct.pack(">IIIIHHqqhhhhHHhhh", 0x00010000, 0x00010000, 0, 0x5F0F3CF5,
                             0x000B, UPM, 0, 0, xmin, ymin, xmax, ymax, 0, 8, 2, 0, 0),
        b"hhea": struct.pack(">IhhhHhhhhhhhhhhhH", 0x00010000, ASC, DESC, 0,
                             max(a for _, a, _ in GLYPHS), 0, 0, xmax, 1, 0, 0,
                             0, 0, 0, 0, 0, n),
        b"maxp": struct.pack(">IHHHHHHHHHHHHHH", 0x00010000, n, maxpts, 1, 0, 0, 2, 0, 0,
                             0, 0, 0, 0, 0, 0),
        b"hmtx": b"".join(struct.pack(">Hh", a, min((p[0] for c in cs for p in c), default=0))
                          for _, a, cs in GLYPHS),
        b"loca": b"".join(struct.pack(">H", o // 2) for o in loca),
        b"glyf": glyf,
        b"cmap": cmap(),
        b"post": struct.pack(">IIhhIIIII", 0x00030000, 0, -100, 50, 0, 0, 0, 0, 0),
        b"OS/2": struct.pack(">HhHHHhhhhhhhhhhh10sIIII4sHHHhhhHH", 1, 600, 400, 5, 0,
                             650, 600, 0, 75, 650, 600, 0, 350, 50, 250, 0,
                             b"\0" * 10, 0, 0, 0, 0, b"SYFK", 0x40, 0x2192, 0x25CF,
                             ASC, DESC, 0, ASC, -DESC) + struct.pack(">II", 1, 0),
        b"name": name_table(),
    }
    tags = sorted(tables)
    num = len(tags)
    sr = 16 * 2 ** int(math.log2(num))
    out = struct.pack(">IHHHH", 0x00010000, num, sr, int(math.log2(sr // 16)), num * 16 - sr)
    offset = 12 + 16 * num
    body = b""
    for tag in tags:
        data = tables[tag]
        out += struct.pack(">4sIII", tag, checksum(data), offset + len(body), len(data))
        body += data + b"\0" * (-len(data) % 4)
    return out + body


def name_table():
    names = {1: "SyFrack Symbols", 2: "Regular", 4: "SyFrack Symbols", 6: "SyFrackSymbols"}
    recs, strings = b"", b""
    for nid, text in names.items():
        raw = text.encode("utf-16-be")
        recs += struct.pack(">HHHHHH", 3, 1, 0x409, nid, len(raw), len(strings))
        strings += raw
    head = struct.pack(">HHH", 0, len(names), 6 + 12 * len(names))
    return head + recs + strings


def checksum(data):
    data += b"\0" * (-len(data) % 4)
    return sum(struct.unpack(">%dI" % (len(data) // 4), data)) & 0xFFFFFFFF


if __name__ == "__main__":
    with open("symbols.ttf", "wb") as f:
        f.write(build())
    print("symbols.ttf written")
