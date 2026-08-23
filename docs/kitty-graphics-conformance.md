# Kitty graphics conformance matrix

This matrix tracks protocol-observable behavior against the official Kitty implementation and Ghostty's independent implementation. It supplements Appendix E of [the RFC](kitty-graphics-rfc.md). A row is complete only when an Alacritty test exercises the stated boundary.

## Oracles

- Kitty protocol and `kitty_tests/graphics.py` at `kovidgoyal/kitty@77630f3a6748cdf0e3675cc6b768d5dd018a5052`.
- Ghostty `src/terminal/kitty/graphics_*.zig` at `ghostty-org/ghostty@9f0e1719dc918368367d368bfe300f59bb68b5a4`.
- Runtime comparisons use Kitty 0.48.2.

The protocol defines required behavior. External tests identify scenarios and expected wire observations; Alacritty tests use independent fixtures and native test APIs. Do not copy Kitty's GPL test source into this repository.

## Matrix

| Area | Scenario | Status |
| --- | --- | --- |
| Transport | Direct RGB, RGBA, PNG, and zlib decode | Covered |
| Transport | Chunk assembly and cancellation by delete | Covered |
| Transport | Short direct, file, and shared-memory data return `ENODATA` with request identity | Covered |
| Transport | Non-regular files return `EBADF` without blocking | Covered |
| Parser | `f=0`, unknown formats, and out-of-range flag semantics match Kitty | Covered |
| Parser | Quiet levels suppress success and error responses | Covered |
| Identity | Number-based transmission allocates and reuses the smallest free client ID | Covered |
| Replacement | First replacement chunk removes old placements and data | Gap |
| Placement | Native spans and cursor movement include pixel offsets | Covered |
| Placement | Cell offsets clamp to cell bounds | Covered |
| Placement | Full deletion-selector matrix, including empty ranges | Covered |
| Placement | RIS removes visible placements but preserves placements in scrollback | Gap |
| Relative | `P` without `Q` resolves the protocol fallback parent | Covered |
| Relative | Virtual placement ID zero can act as a parent | Covered |
| Relative | Cycles, depth, lifetime, and virtual-child rejection | Covered |
| Placeholder | Color IDs, third diacritic, inheritance, and placement IDs | Covered |
| Placeholder | Fit-and-center source geometry for a virtual placement box | Covered |
| Placeholder | Resolved placeholder glyphs remain invisible through transparent pixels | Covered |
| Animation | Frame create, edit, control, composition, and scheduling | Partial |
| Animation | Root editing, gap preservation, frame-number replies, and root promotion | Covered |
| Animation | Frame deletion edge cases and excess-data behavior | Partial |
| Quota | Canonical bytes, reservations, metadata, and deterministic eviction | Covered |
| Quota | Transient and unplaced images receive deterministic eviction priority | Covered |
| Quota | Non-visible placements are evicted before visible placements | Gap |
| Renderer | Z strata, transparent-edge filtering, placeholder transparency, and GL-state isolation | Covered |
| Renderer | Hardware-sized tiling, bounded cache, and texture reconstruction | Covered |
| Robustness | Stateful fuzzing, random state invariants, and ordinary-text benchmark | Covered |

## Runtime fixtures

`scripts/kitty-graphics-smoke.sh` checks these composed framebuffer scenarios under headless Sway:

- text remains stable across repeated graphics redraws;
- negative-z images do not corrupt text renderer state;
- transparent Unicode placeholder tiles do not expose placeholder glyphs;
- image textures reconstruct from canonical CPU pixels;
- an opaque image reaches the expected framebuffer color.

Real-application review also uses `treemd` with transparent PNGs and Unicode placeholders. Kitty serves as the comparison terminal with matched window, font, and color configuration where geometry matters.
