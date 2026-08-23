# Kitty graphics implementation plan

Status: active

## Outcome and acceptance

Implement the complete Kitty terminal graphics protocol described by the accepted [RFC](../../docs/kitty-graphics-rfc.md). Completion requires every item in its definition of done, including ordered queries, all transports and image formats, classic and placeholder placements, animation, bounded resources, reflow-safe anchors, layered rendering, texture tiling, context recovery, and no material steady-state regression without graphics.

## Verified current state

- Base revision is `7dd7b5b0` on `aasoft/kitty`.
- `vte` 0.15.0 recognizes APC, PM, and SOS through one ignored state and exposes no APC callback.
- `alacritty_terminal/src/event_loop.rs` advances a full PTY buffer while holding `Term`'s mutex.
- Primary grid width changes reflow cells in `alacritty_terminal/src/grid/resize.rs`; no external points participate.
- Terminal renderable state is collected under lock, while OpenGL work occurs after lock release.
- No graphics state, graphics configuration, image renderer, or local graphics transport exists.

## Accepted decisions

- The official Kitty protocol is authoritative for wire behavior. The RFC defines private-fork policy where the protocol does not.
- Canonical RGBA8 CPU data belongs to per-screen terminal graphics state. GPU textures are disposable caches.
- Generic APC recognition streams data without whole-string buffering.
- Heavy transport and decode work forms an ordered parser barrier and runs outside `Term`'s mutex.
- Classic placements are independent terminal objects with reflow-aware anchors. Placeholder instances remain ordinary cells.
- Local transports default on but have a configuration kill switch.
- Complete support defaults on only after conformance and hardening are complete.

## Ordered goals

1. [ ] Add fixtures and protocol-state test infrastructure.
2. [ ] Expose bounded streaming APC callbacks through a pinned `vte` fork and cover termination/cancellation splits.
3. [ ] Add ordered partial parser consumption and deferred transaction barriers.
4. [ ] Implement typed command parsing, direct RGB/RGBA/PNG/zlib transfer, IDs, queries, and quotas.
5. [ ] Implement classic placements, deletion, tracked anchors, scroll/reflow, screen ownership, and reset semantics.
6. [ ] Add layered OpenGL image rendering, alpha/crop/scale, tiling, cache eviction, and context recovery.
7. [ ] Add bounded regular-file, temporary-file, and shared-memory transports behind configuration.
8. [ ] Implement Unicode placeholders from authoritative generated combining-mark data.
9. [ ] Implement relative placement graph semantics.
10. [ ] Implement animation frame loading, composition, control, deletion, and deadline scheduling.
11. [ ] Complete hostile-input, property, fuzz, renderer-golden, context-loss, and performance validation; enable by default.

Each goal stops only after its focused tests, adjacent suites, formatting, linting, docs, and Lode state pass. Commit verified coherent increments separately.

## Risks and rollback

Parser changes affect all terminal input. Keep generic APC support isolated and verify existing OSC/DCS/PM/SOS behavior. Grid tracking can corrupt placement lifetime; invalid anchors must remove placements rather than clamp. Renderer layering can regress text; preserve an empty-state fast path and compare controlled framebuffers. Disable graphics through configuration for runtime rollback; source rollback remains commit-local by phase.

## Related Lode

- [Graphics domain](../graphics/summary.md)
- [Repository practices](../practices.md)
