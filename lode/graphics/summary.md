# Kitty graphics

## Responsibility

Kitty graphics support spans generic APC recognition, terminal protocol and image state, ordered heavy processing, grid-relative placement lifetime, render snapshots, and OpenGL compositing. The Kitty protocol defines wire behavior. The accepted private-fork policy is in the [RFC](../../docs/kitty-graphics-rfc.md), and durable implementation decisions are in the [implementation record](../plans/kitty-graphics.md).

## Ownership and flow

1. `vte` recognizes APC boundaries and streams bytes without protocol-specific buffering.
2. `alacritty_terminal::ansi` identifies Kitty APCs and produces typed terminal operations or deferred work.
3. Each primary and alternate screen owns separate graphics state.
4. The PTY loop streams encoded chunks and executes costly transport, decode, frame allocation, and composition outside `Term`'s mutex, then commits in input order.
5. `Term` builds one render snapshot with a frame-local virtual-prototype index while locked. Ordinary virtual placements scan only the viewport; relative virtual roots trigger one retained-history scan.
6. The renderer uploads disposable, hardware-sized premultiplied-alpha texture tiles after releasing the lock, bounds cached tiles with deterministic LRU eviction, and composites three protocol image z strata around cell backgrounds, glyphs/decorations, and the cursor.

## Configuration

Kitty graphics support is always available. `[terminal.graphics]` controls resource policy: its decoded storage limit defaults to 320,000,000 bytes per screen buffer, and local-object transmission defaults to allowed.

Capability discovery uses the Kitty query rather than terminal identity. Alacritty also reports text-area pixels, character-cell pixels, and text-area cells through `CSI 14 t`, `CSI 16 t`, and `CSI 18 t`. tmux clients must use enabled DCS passthrough; tmux does not retain KGP payloads for a fresh terminal attachment, and its initial incremental placeholder output may require `refresh-client`. Zellij 0.45.0 proxies Kitty graphics inside synchronized updates; completion, timeout, and overflow replay preserve every deferred barrier before parsing a later command.

## Invariants

- Every placement references a live image.
- Named image and placement indexes are unique where protocol IDs require uniqueness.
- Canonical retained pixels never exceed the configured screen-buffer quota. Working source, decoder, and destination allocations have separate bounds documented in `docs/kitty-graphics-resources.md`; a full-size frame edit can hold two working pixel buffers. `Arc<Vec<u8>>` preserves immutable sharing and transfers uniquely owned allocations. Encoded input coalesces into 128 KiB blocks; empty chunks add no blocks.
- Tracked anchors follow retained logical content. Normal capacity pruning invalidates placements; text-only scrollback erasure leaves detached anchors non-visible without remapping them.
- Relative placement graphs are acyclic, depth-bounded, and free of dangling parents.
- Image replacement is atomic.
- Dropping every GPU texture does not change terminal semantics.
- CPU image storage remains straight RGBA; GPU uploads premultiply RGB by alpha before linear filtering and use premultiplied-alpha blending.
- Valid rendered Unicode placeholder cells contribute image tiles and backgrounds, not visible placeholder glyphs or decorations. Scrolling moves or erases those cells without moving or removing their immutable virtual placement prototypes.
- Every image pass uses the full-window GL viewport and restores the previous viewport, bindings, active texture, blend state, and scissor state before later rendering.
- Graphics state never crosses primary and alternate screen ownership.
- APC and command parser memory remains bounded for malformed or incomplete input.
- Every parser ingress, including synchronized completion, timeout, and overflow replay, commits all decode and frame-composition continuations before interpreting later buffered bytes.
- Non-Unix shared-memory transmission returns `ENOTSUP`; this fork makes no Windows KGP shared-memory conformance claim.
- Usage hints remain bitmasks, and only explicitly defined flag values change cursor or composition policy.
- Requested classic `c/r` axes remain separate from cursor/deletion `cell_span`. Native sizing preserves pixels; inferred sizing preserves aspect. Font metric changes refresh classic footprints. Explicit destination spans end at cell boundaries after pixel offsets.
- Virtual origins use the resolved `PlacementHandle`, including omitted placement IDs. Rendering and location deletion collect the same current origins from grid placeholders.
- Deletion and relative offsets widen arithmetic before addition. Self-parent replacement reports `ECYCLE`.
- Fractional source sampling preserves enlarged placeholder tiles. Global source coordinates use `f64`; intersection preserves extents relative to the source origin before GPU conversion, including tiny source fractions at coordinates beyond `2^24`. CPU composition retains full alpha precision; finite playback skips gapless frames before publication.

## Failure behavior

Malformed, unsupported, oversized, exhausted, or inaccessible operations produce bounded protocol errors when quiet mode permits. They do not panic, partially commit state, block on special files, or consume unbounded memory.

## Anchors

- `vte/src/lib.rs` - generic streaming APC recognition and cancellation callbacks.
- `alacritty_terminal/src/event_loop.rs` - retained input suffix, unlocked decode, and ordered commit boundary.
- `alacritty_terminal/src/graphics/` - typed commands, bounded canonical decoding, per-screen image storage, relative placement graphs, and Unicode placeholder decoding.
- `scripts/generate-kitty-diacritics.py` - authoritative placeholder row/column table regeneration.
- `scripts/kitty-graphics-smoke.sh` - isolated headless-Sway framebuffer validation for z strata, crop/scale, alpha overlap, placeholder backgrounds, automatic animation, bounded cache eviction, texture reconstruction, and repeated text redraws.
- `scripts/kitty-graphics-benchmark.sh` - release-mode ordinary-text parser benchmark.
- `scripts/kitty-graphics-geometry.sh` - framebuffer pixel counts for native/inferred/offset sizing, padding, fractional placeholders, large source coordinates, and CPU composition.
- `scripts/kitty-graphics-memory.sh` - combined allocation peaks and pixels through production deferred processing.
- `docs/kitty-graphics-resources.md` - retained, working, snapshot, metadata, and GPU resource accounting.
- `fuzz/fuzz_targets/kitty_stream.rs` - bounded stateful PTY/parser/decoder/grid fuzz target.
- `alacritty_terminal/src/term/mod.rs` - screen state, reset, resize, and render snapshot boundary.
- `alacritty_terminal/src/grid/resize.rs` - primary-buffer reflow implementation.
- `alacritty/src/display/mod.rs` - terminal snapshot and unlocked GPU-render boundary.
- `alacritty/src/renderer/image.rs` - RGBA texture cache and image quad renderer.
- `alacritty/src/display/mod.rs` - placement-to-framebuffer geometry and render ordering.
- `alacritty/src/config/terminal.rs` - terminal configuration table.

## Related

- [Kitty graphics implementation plan](../plans/kitty-graphics.md)
- [Kitty graphics RFC](../../docs/kitty-graphics-rfc.md)
- [Repository practices](../practices.md)
