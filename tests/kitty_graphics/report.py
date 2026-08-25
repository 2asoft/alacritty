#!/usr/bin/env python3
"""Create the checked-in observational KGP baseline report."""

from __future__ import annotations

import argparse
import json
from dataclasses import asdict, dataclass
from pathlib import Path


@dataclass(frozen=True)
class Observation:
    terminal: str
    available: bool
    version: str
    response_sequence: tuple[str, ...]
    kgp_response: bool | None
    text_area_pixel_response: bool | None
    cell_size_response: bool | None
    screen_size_response: bool | None
    primary_device_attributes_response: bool | None
    primary_red_pixels: int | None
    virtual_green_pixels: int | None
    leaked_placeholder_magenta_pixels: int | None


def read_transcript(path: Path) -> bytes:
    value = path.read_text(encoding="ascii").strip()
    return bytes.fromhex(value) if value else b""


def read_metrics(path: Path) -> dict[str, int]:
    metrics: dict[str, int] = {}
    for line in path.read_text(encoding="ascii").splitlines():
        key, value = line.split("=", 1)
        metrics[key] = int(value)
    return metrics


def response_sequence(response: bytes) -> tuple[str, ...]:
    sequence: list[str] = []
    offset = 0
    while offset < len(response):
        if response.startswith(b"\x1b_G", offset):
            end = response.find(b"\x1b\\", offset + 3)
            if end == -1:
                sequence.append("incomplete-apc")
                break
            body = response[offset + 3 : end]
            sequence.append("kgp-ok" if b"i=31" in body and b"OK" in body else "kgp-other")
            offset = end + 2
            continue
        if response.startswith(b"\x1b[", offset):
            end = offset + 2
            while end < len(response) and not 0x40 <= response[end] <= 0x7E:
                end += 1
            if end == len(response):
                sequence.append("incomplete-csi")
                break
            body = response[offset + 2 : end + 1]
            if body.startswith(b"4;") and body.endswith(b"t"):
                sequence.append("text-area-pixels")
            elif body.startswith(b"6;") and body.endswith(b"t"):
                sequence.append("cell-size-pixels")
            elif body.startswith(b"8;") and body.endswith(b"t"):
                sequence.append("screen-size-cells")
            elif body.endswith(b"c"):
                sequence.append("primary-device-attributes")
            else:
                sequence.append("csi-other")
            offset = end + 1
            continue
        sequence.append("unparsed-byte")
        offset += 1
    return tuple(sequence)


def observe(root: Path, terminal: str, version: str, available: bool = True) -> Observation:
    directory = root / terminal
    sequence = response_sequence(read_transcript(directory / "transcript.hex")) if available else ()
    metrics = read_metrics(directory / "metrics.txt") if available else None
    return Observation(
        terminal=terminal,
        available=available,
        version=version,
        response_sequence=sequence,
        kgp_response="kgp-ok" in sequence if available else None,
        text_area_pixel_response="text-area-pixels" in sequence if available else None,
        cell_size_response="cell-size-pixels" in sequence if available else None,
        screen_size_response="screen-size-cells" in sequence if available else None,
        primary_device_attributes_response=(
            "primary-device-attributes" in sequence if available else None
        ),
        primary_red_pixels=metrics["red"] if metrics is not None else None,
        virtual_green_pixels=metrics["green"] if metrics is not None else None,
        leaked_placeholder_magenta_pixels=metrics["magenta"] if metrics is not None else None,
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--source-digest", required=True)
    parser.add_argument("--alacritty-version", required=True)
    parser.add_argument("--kitty-version", required=True)
    parser.add_argument("--rust-version", required=True)
    parser.add_argument("--alacritty-build-status", choices=("success", "failure"), required=True)
    parser.add_argument("--comparison-different-pixels", required=True)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    build_success = args.alacritty_build_status == "success"
    alacritty = observe(
        args.root, "alacritty", args.alacritty_version, available=build_success
    )
    kitty = observe(args.root, "kitty", args.kitty_version)
    different_pixels = (
        int(args.comparison_different_pixels)
        if args.comparison_different_pixels != "unavailable"
        else None
    )
    report = {
        "schema": 1,
        "purpose": "observational, not pass/fail",
        "alacritty_build_status": args.alacritty_build_status,
        "source_digest": args.source_digest,
        "toolchain": {"rustc": args.rust_version},
        "completion_expectation": {
            "kgp_response": True,
            "minimum_primary_red_pixels": 100,
            "minimum_virtual_green_pixels": 10,
            "maximum_leaked_placeholder_magenta_pixels": 0,
            "response_sequence": [
                "kgp-ok",
                "text-area-pixels",
                "cell-size-pixels",
                "screen-size-cells",
                "primary-device-attributes",
            ],
            "oracle": "Kitty is comparative; the written protocol remains authoritative.",
        },
        "observations": [asdict(alacritty), asdict(kitty)],
        "framebuffer_different_pixels": different_pixels,
    }
    (args.root / "report.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )

    lines = [
        "# Current observation",
        "",
        "This bundle records behavior. Differences do not fail the suite.",
        "",
        "| Terminal | Available | KGP query OK | Response sequence | Red classic pixels | Green virtual pixels | Leaked magenta pixels |",
        "| --- | ---: | ---: | --- | ---: | ---: | ---: |",
    ]
    for result in (alacritty, kitty):
        red = result.primary_red_pixels if result.primary_red_pixels is not None else "unavailable"
        green = result.virtual_green_pixels if result.virtual_green_pixels is not None else "unavailable"
        magenta = (
            result.leaked_placeholder_magenta_pixels
            if result.leaked_placeholder_magenta_pixels is not None
            else "unavailable"
        )
        kgp = "unavailable" if result.kgp_response is None else ("yes" if result.kgp_response else "no")
        lines.append(
            f"| {result.terminal} | {'yes' if result.available else 'no'} | "
            f"{kgp} | {', '.join(result.response_sequence)} | "
            f"{red} | {green} | {magenta} |"
        )
    comparison = (
        f"The full-frame Alacritty/Kitty comparison contains {different_pixels} differing pixels."
        if different_pixels is not None
        else "The Alacritty framebuffer comparison is unavailable because this source state did not build."
    )
    lines.extend(
        (
            "",
            "## Completion expectation",
            "",
            "Alacritty should build, answer the KGP query, render both fixtures, replace the magenta placeholder glyph with the virtual image, and preserve the query response order recorded in `report.json`.",
            "",
            comparison,
        )
    )
    (args.root / "observations.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
