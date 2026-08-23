# Kitty graphics

## Responsibility

Kitty graphics support spans generic APC recognition, terminal protocol and image state, ordered heavy processing, grid-relative placement lifetime, render snapshots, and OpenGL compositing. The Kitty protocol defines wire behavior. The accepted private-fork policy is in the [RFC](../../docs/kitty-graphics-rfc.md), and phase state is in the [implementation plan](../plans/kitty-graphics.md).

## Ownership and flow

1. `vte` recognizes APC boundaries and streams bytes without protocol-specific buffering.
2. `alacritty_terminal::ansi` identifies Kitty APCs and produces typed terminal operations or deferred work.
3. Each primary and alternate screen owns separate graphics state.
4. The PTY loop executes costly transport/decode work outside `Term`'s mutex and commits in input order.
5. Display code snapshots immutable pixel handles and resolved placement geometry while locked.
6. The renderer uploads disposable, hardware-sized premultiplied-alpha texture tiles after releasing the lock, bounds cached tiles with deterministic LRU eviction, and composites three protocol image z strata around cell backgrounds, glyphs/decorations, and the cursor.

## Configuration

`[terminal.graphics]` currently defaults to disabled while conformance work is active. Its decoded storage limit defaults to 320,000,000 bytes per screen buffer, and local-object transmission defaults to allowed. Disabled APC commands are recognized and discarded without creating an ordered parser barrier.

## Invariants

- Every placement references a live image.
- Named image and placement indexes are unique where protocol IDs require uniqueness.
- Pixel reservations and retained bytes never exceed the configured screen-buffer quota.
- Tracked anchors either identify retained logical content or invalidate their placements.
- Relative placement graphs are acyclic, depth-bounded, and free of dangling parents.
- Image replacement is atomic.
- Dropping every GPU texture does not change terminal semantics.
- CPU image storage remains straight RGBA; GPU uploads premultiply RGB by alpha before linear filtering and use premultiplied-alpha blending.
- Valid rendered Unicode placeholder cells contribute image tiles and backgrounds, not visible placeholder glyphs or decorations.
- Every image pass restores shared OpenGL bindings, active texture, blend state, and scissor state before a later text batch or frame so renderer-local state caches remain coherent.
- Graphics state never crosses primary and alternate screen ownership.
- APC and command parser memory remains bounded for malformed or incomplete input.

## Failure behavior

Malformed, unsupported, oversized, exhausted, or inaccessible operations produce bounded protocol errors when quiet mode permits. They do not panic, partially commit state, block on special files, or consume unbounded memory.

## Anchors

- `vte/src/lib.rs` - generic streaming APC recognition and cancellation callbacks.
- `alacritty_terminal/src/event_loop.rs` - retained input suffix, unlocked decode, and ordered commit boundary.
- `alacritty_terminal/src/graphics/` - typed commands, bounded canonical decoding, per-screen image storage, relative placement graphs, and Unicode placeholder decoding.
- `scripts/generate-kitty-diacritics.py` - authoritative placeholder row/column table regeneration.
- `scripts/kitty-graphics-smoke.sh` - isolated headless-Sway semantic framebuffer and repeated-redraw validation, including text composited above a negative-z image.
- `scripts/kitty-graphics-benchmark.sh` - release-mode ordinary-text parser comparison with graphics disabled and enabled.
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
