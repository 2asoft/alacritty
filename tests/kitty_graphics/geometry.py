#!/usr/bin/env python3
"""Emit geometry and CPU-composition fixtures with exact framebuffer color counts."""

from __future__ import annotations

import argparse
import fcntl
import os
import struct
import termios
import time
import zlib
from dataclasses import dataclass
from pathlib import Path

from client import apc, await_start, rgba, solid


@dataclass(frozen=True)
class ExpectedColor:
    hex_rgb: str
    pixels: int


def solid_png_row(width: int, color: bytes) -> bytes:
    compressor = zlib.compressobj()
    compressed = compressor.compress(b"\0") + compressor.compress(color * width) + compressor.flush()
    chunks = (
        (b"IHDR", struct.pack(">IIBBBBB", width, 1, 8, 6, 0, 0, 0)),
        (b"IDAT", compressed),
        (b"IEND", b""),
    )
    return b"\x89PNG\r\n\x1a\n" + b"".join(
        struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
        for kind, data in chunks
    )


def run(output: Path) -> None:
    output.mkdir(parents=True, exist_ok=True)
    await_start(output)
    rows, columns, width, height = struct.unpack(
        "HHHH", fcntl.ioctl(1, termios.TIOCGWINSZ, bytes(8))
    )
    if rows < 20 or columns < 12 or width == 0 or height == 0:
        raise ValueError("geometry fixture requires 20 rows, 12 columns, and reported pixel sizes")
    cell_width, cell_height = width // columns, height // rows
    square = min(2 * cell_width, 2 * cell_height)
    source_origin = 1 << 24
    grid_scale = (2**32 - 1) // max(cell_width, cell_height)
    virtual_columns, virtual_rows = grid_scale * cell_height, grid_scale * cell_width
    expected = (
        ExpectedColor("FF0000", 6),
        ExpectedColor("00FF00", 8 * cell_width * cell_width),
        ExpectedColor("00FFFF", 8 * cell_height * cell_height),
        ExpectedColor("FF00FF", square * square),
        ExpectedColor("808080", 3 * cell_width * cell_height),
        ExpectedColor("FF8000", 20 * cell_width * cell_height),
        ExpectedColor("0000FF", 0),
        ExpectedColor("112233", (4 * cell_width - 2) * (2 * cell_height - 3)),
        ExpectedColor("445566", cell_width * cell_height),
    )
    (output / "expected.tsv").write_text(
        "".join(f"{item.hex_rgb}\t{item.pixels}\n" for item in expected), encoding="ascii"
    )
    (output / "cell-size.txt").write_text(f"{cell_width} {cell_height}\n", encoding="ascii")
    placeholder = "\U0010eeee\u0305\u0305\U0010eeee\u0305\u030d".encode()
    second_row = "\U0010eeee\u030d\u0305\U0010eeee\u030d\u030d".encode()
    stimulus = b"".join((
        b"\x1b[?25l\x1b[2J\x1b[H",
        apc("a=T,q=2,i=1,s=3,v=2,C=1", solid(3, 2, rgba(255, 0, 0))),
        b"\x1b[3;1H",
        apc("a=T,q=2,i=2,s=100,v=50,c=4,C=1", solid(100, 50, rgba(0, 255, 0))),
        b"\x1b[6;1H",
        apc("a=T,q=2,i=3,s=100,v=50,r=2,C=1", solid(100, 50, rgba(0, 255, 255))),
        apc("a=T,q=2,i=4,s=1,v=1,U=1,c=2,r=2", rgba(255, 0, 255)),
        b"\x1b[9;1H\x1b[38;5;4m", placeholder,
        b"\x1b[10;1H", second_row, b"\x1b[0m",
        apc("a=t,q=2,i=5,s=1,v=1", rgba(255, 255, 255, 1)),
        apc("a=f,q=2,i=5,s=1,v=1,c=1", rgba(255, 255, 255, 128)),
        apc("a=a,q=2,i=5,c=2,s=1"),
        b"\x1b[12;1H", apc("a=p,q=2,i=5,c=3,r=1,C=1"),
        b"\x1b[15;1H\x1b[44m          \x1b[16;1H          \x1b[0m\x1b[15;1H",
        apc("a=T,q=2,i=6,s=1,v=1,c=10,r=2,z=-1,C=1", rgba(255, 128, 0)),
        b"\x1b[18;1H",
        apc("a=T,q=2,i=7,s=1,v=1,c=4,r=2,X=2,Y=3,C=1", rgba(17, 34, 51)),
        apc(
            f"a=T,q=2,i=8,f=100,U=1,c={virtual_columns},r={virtual_rows},"
            f"x={source_origin},w=1,h=1,C=1",
            solid_png_row(source_origin + 1, rgba(68, 85, 102)),
        ),
        "\x1b[20;1H\x1b[38;5;8m\U0010eeee\u0305\u0305\x1b[0m".encode(),
    ))
    # Write the entire fixture even if the PTY accepts only a prefix per syscall.
    pending = memoryview(stimulus)
    while pending:
        pending = pending[os.write(1, pending):]
    (output / "ready").touch()
    time.sleep(30)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    run(parser.parse_args().output)
