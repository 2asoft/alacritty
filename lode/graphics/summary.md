# Kitty graphics

## Responsibility

Kitty graphics support spans generic APC recognition, terminal protocol and image state, ordered heavy processing, grid-relative placement lifetime, render snapshots, and OpenGL compositing. The Kitty protocol defines wire behavior. The accepted private-fork policy is in the [RFC](../../docs/kitty-graphics-rfc.md), and phase state is in the [implementation plan](../plans/kitty-graphics.md).

## Ownership and flow

1. `vte` recognizes APC boundaries and streams bytes without protocol-specific buffering.
2. `alacritty_terminal::ansi` identifies Kitty APCs and produces typed terminal operations or deferred work.
3. Each primary and alternate screen owns separate graphics state.
4. The PTY loop executes costly transport/decode work outside `Term`'s mutex and commits in input order.
5. Display code snapshots immutable pixel handles and resolved placement geometry while locked.
6. The renderer uploads disposable textures and composites them in protocol z layers after releasing the lock.

## Invariants

- Every placement references a live image.
- Named image and placement indexes are unique where protocol IDs require uniqueness.
- Pixel reservations and retained bytes never exceed the configured screen-buffer quota.
- Tracked anchors either identify retained logical content or invalidate their placements.
- Relative placement graphs are acyclic, depth-bounded, and free of dangling parents.
- Image replacement is atomic.
- Dropping every GPU texture does not change terminal semantics.
- Graphics state never crosses primary and alternate screen ownership.
- APC and command parser memory remains bounded for malformed or incomplete input.

## Failure behavior

Malformed, unsupported, oversized, exhausted, or inaccessible operations produce bounded protocol errors when quiet mode permits. They do not panic, partially commit state, block on special files, or consume unbounded memory.

## Anchors

- `vte/src/lib.rs` - generic streaming APC recognition and cancellation callbacks.
- `alacritty_terminal/src/event_loop.rs` - PTY parser lock and ordering boundary.
- `alacritty_terminal/src/term/mod.rs` - screen state, reset, resize, and render snapshot boundary.
- `alacritty_terminal/src/grid/resize.rs` - primary-buffer reflow implementation.
- `alacritty/src/display/mod.rs` - terminal snapshot and unlocked GPU-render boundary.
- `alacritty/src/renderer/` - OpenGL renderer facilities.
- `alacritty/src/config/terminal.rs` - terminal configuration table.

## Related

- [Kitty graphics implementation plan](../plans/kitty-graphics.md)
- [Kitty graphics RFC](../../docs/kitty-graphics-rfc.md)
- [Repository practices](../practices.md)
