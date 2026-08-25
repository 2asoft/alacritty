#!/usr/bin/env python3
"""Emit deterministic Kitty graphics fixtures and record terminal responses."""

from __future__ import annotations

import argparse
import base64
import os
import select
import termios
import time
from pathlib import Path

ESC = b"\x1b"
ST = ESC + b"\\"


def apc(control: str, payload: bytes = b"") -> bytes:
    encoded = base64.b64encode(payload)
    return ESC + b"_G" + control.encode("ascii") + b";" + encoded + ST


def rgba(red: int, green: int, blue: int, alpha: int = 255) -> bytes:
    return bytes((red, green, blue, alpha))


def solid(width: int, height: int, color: bytes) -> bytes:
    return color * width * height


def stimulus() -> bytes:
    red = apc(
        "a=T,q=2,f=32,s=8,v=8,i=1,c=4,r=2,C=1",
        solid(8, 8, rgba(255, 0, 0)),
    )
    virtual = apc(
        "a=T,q=2,f=32,s=8,v=16,i=16711935,U=1,c=1,r=1,C=1",
        solid(8, 16, rgba(0, 255, 0)),
    )
    query = apc("a=q,q=0,f=32,s=1,v=1,i=31", rgba(0, 0, 0))
    placeholder = "\U0010eeee\u0305".encode("utf-8")

    return b"".join(
        (
            ESC + b"[2J" + ESC + b"[H" + ESC + b"[?25l",
            b"KITTY GRAPHICS OBSERVATIONAL BASELINE\r\n",
            b"Target: red block, green virtual tile, KGP query OK.\r\n",
            ESC + b"[5;2H",
            red,
            ESC + b"[11;2H",
            virtual,
            ESC + b"[38;2;255;0;255m" + placeholder + ESC + b"[0m",
            ESC + b"[15;2HProtocol response captured out-of-band.",
            query,
            ESC + b"[14t",
            ESC + b"[16t",
            ESC + b"[18t",
            ESC + b"[c",
        )
    )


def write_hex(path: Path, data: bytes) -> None:
    path.write_text(data.hex() + "\n", encoding="ascii")


def await_start(output: Path) -> None:
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if (output / "start").exists():
            return
        time.sleep(0.02)
    raise TimeoutError("terminal window was not prepared")


def run(output: Path) -> None:
    output.mkdir(parents=True, exist_ok=True)
    await_start(output)
    stdin_fd = 0
    original = termios.tcgetattr(stdin_fd)
    raw = termios.tcgetattr(stdin_fd)
    raw[0] = 0
    raw[1] = 0
    raw[3] = 0
    raw[6][termios.VMIN] = 0
    raw[6][termios.VTIME] = 0

    responses = bytearray()
    try:
        termios.tcsetattr(stdin_fd, termios.TCSANOW, raw)
        os.write(1, stimulus())
        deadline = time.monotonic() + 1.5
        while time.monotonic() < deadline:
            readable, _, _ = select.select([stdin_fd], [], [], 0.05)
            if readable:
                responses.extend(os.read(stdin_fd, 4096))
    finally:
        termios.tcsetattr(stdin_fd, termios.TCSANOW, original)

    write_hex(output / "transcript.hex", bytes(responses))
    (output / "ready").touch()
    time.sleep(30)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    parser.add_argument("--write-stimulus", type=Path)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if args.write_stimulus is not None:
        write_hex(args.write_stimulus, stimulus())
        return
    run(args.output)


if __name__ == "__main__":
    main()
