# Kitty graphics observational baseline

Run this suite from the repository root:

```sh
scripts/kitty-graphics-baseline.sh
```

The suite builds Alacritty, starts an isolated headless Sway compositor, and sends the same wire-level fixture to Alacritty and Kitty. It safely replaces `tests/kitty_graphics/baseline/` with:

- one screenshot and PTY response transcript for each available terminal;
- the normalized Alacritty build result, including complete diagnostics on failure;
- exact red, green, and magenta framebuffer pixel counts;
- a side-by-side comparison image;
- a machine-readable report and a short observation summary;
- the emitted byte stream and BLAKE3 hashes of every artifact.

This is an observational suite, not a pass/fail conformance gate. Source build failures and terminal behavior differences are recorded without failing the command. Broken automation still fails. Every feature-branch commit checks in the bundle produced for that commit, so Git history preserves the progression from the unsupported baseline through any non-building state to the intended result.

## Fixture meaning

The fixture emits:

1. an RGBA direct transmit-and-place command that should render a red classic placement;
2. a green virtual placement followed by a magenta Unicode placeholder cell;
3. a KGP capability query;
4. character-cell, text-grid, and primary-device-attribute queries.

The completed feature should:

- answer the KGP query with `OK`;
- render red pixels for the classic placement;
- render green pixels for the virtual placement;
- suppress the magenta placeholder glyph after resolving its virtual image;
- preserve ordered responses to the later terminal queries.

Kitty is a comparative runtime oracle, not the specification. Geometry and font configuration are matched where practical, but application-specific rasterization and window behavior can produce framebuffer differences.

## Requirements

The runner requires:

- `b3sum`
- Cargo and Rust
- Alacritty's normal build dependencies
- Kitty
- headless-capable Sway and `swaymsg`
- Grim
- ImageMagick
- `jq`
- Python 3
- DejaVu Sans Mono

Generated runtime files remain under `target/kitty-graphics-baseline`. Same-filesystem staging directories beside the baseline make replacement recoverable after interruption. Only the replace-in-place `tests/kitty_graphics/baseline/` bundle is checked in.
