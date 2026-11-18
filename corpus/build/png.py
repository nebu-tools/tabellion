"""Writes small PNG pictures for the corpus: a bird shape on a plain ground.

    python3 png.py OUT.png WIDTH HEIGHT R G B
"""
import struct
import sys
import zlib


def chunk(kind, data):
    c = struct.pack(">I", len(data)) + kind + data
    return c + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)


def main():
    out, w, h, r, g, b = sys.argv[1], *map(int, sys.argv[2:7])
    rows = []
    for y in range(h):
        row = bytearray([0])
        for x in range(w):
            # An ellipse for the body and a smaller one for the head.
            body = ((x - w * 0.45) / (w * 0.28)) ** 2 + ((y - h * 0.6) / (h * 0.2)) ** 2 <= 1
            head = ((x - w * 0.72) / (w * 0.1)) ** 2 + ((y - h * 0.42) / (h * 0.12)) ** 2 <= 1
            row += bytes((40, 40, 40) if body or head else (r, g, b))
        rows.append(bytes(row))
    data = zlib.compress(b"".join(rows), 9)
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", data)
    png += chunk(b"IEND", b"")
    with open(out, "wb") as f:
        f.write(png)


main()
