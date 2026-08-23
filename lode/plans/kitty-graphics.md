# Kitty graphics implementation plan

Status: active

## Outcome and acceptance

Implement the complete Kitty terminal graphics protocol described by the accepted [RFC](../../docs/kitty-graphics-rfc.md). Completion requires every item in its definition of done, including ordered queries, all transports and image formats, classic and placeholder placements, animation, bounded resources, reflow-safe anchors, layered rendering, texture tiling, context recovery, and no material steady-state regression without graphics.

## Verified current state

- Base revision is `7dd7b5b0` on `aasoft/kitty`.
- The workspace now carries a local `vte` 0.15.0 fork. APC has its own streaming callback state; PM and SOS remain ignored.
- `alacritty_terminal` parses bounded Kitty APC data into typed commands. The PTY loop retains the suffix at a graphics barrier, assembles bounded direct chunks without copying image bodies under the terminal lock, decodes RGB, RGBA, PNG, and zlib data outside `Term`'s mutex, commits in order, and preserves query-response ordering. Regular-file, constrained temporary-file, and POSIX shared-memory transfers run outside the terminal lock and enforce the local-transmission gate, opened-object type and filesystem checks, post-open sensitive-path checks, ranges, and storage bounds.
- Per-screen image storage uses process-unique monotonic handles, external ID indexes, deterministic image-number lookup, atomic ID replacement, available-or-replaced byte reservations for deferred decode, a decoded-byte limit, and deterministic oldest-first eviction.
- Basic classic transmit-and-place and place-existing commands create independent placements. Successful placements advance the cursor using explicit, aspect-inferred, or native-pixel cell spans unless suppressed; virtual and relative placements never move it. The OpenGL renderer caches textures and draws crop/scaled RGBA content in the protocol's three image z strata around cell backgrounds, glyphs/decorations, and the cursor. Classic anchors follow width reflow and full-screen scrolling. Partial-region scrolling moves only fully contained placements and retains page-area clipping while content exits the region. Anchors are removed when retained history prunes them. Deferred placement anchors also follow resize while heavy processing runs outside the terminal lock. The renderer tiles images to the hardware texture limit with overlap borders and uses a deterministic LRU texture cache bounded by the configured graphics storage limit; oversized working images stream through transient tiles. Image passes intersect the active damage scissor with terminal content bounds, including placements anchored partially outside the viewport. Context-loss validation remains pending. Classic deletion supports current placement/image selectors and aborts incomplete chunk streams.
- Virtual placements and explicit/inherited Unicode placeholder cells render from the complete authoritative generated diacritic table. Row and column indices cover the full table, high image-ID bytes remain byte-bounded, all inheritance conditions are tested, and location-based deletion excludes virtual placements.
- Relative placements resolve classic and virtual parents, enforce an eight-link depth bound, reject missing parents and cycles atomically, follow parent movement, and cascade deletion and replacement lifetimes.
- Animation frames load as quota-counted canonical RGBA canvases, support frame editing and alpha/overwrite composition, client-selected frames, stop/loading/run states, loop limits, frame gaps, frame deletion, and UI-scheduled playback deadlines. Broader animation corpus and stress validation remain pending.
- Primary grid width changes reflow cells in `alacritty_terminal/src/grid/resize.rs`; no external points participate.
- Terminal renderable state is collected under lock, while OpenGL work occurs after lock release. Without placements, rendering skips history/viewport placeholder scans and image snapshot allocation.

## Accepted decisions

- The official Kitty protocol is authoritative for wire behavior. The RFC defines private-fork policy where the protocol does not.
- Canonical RGBA8 CPU data belongs to per-screen terminal graphics state. GPU textures are disposable caches.
- Generic APC recognition streams data without whole-string buffering.
- Heavy transport and decode work forms an ordered parser barrier and runs outside `Term`'s mutex.
- Classic placements are independent terminal objects with reflow-aware anchors. Placeholder instances remain ordinary cells.
- Local transports default on but have a configuration kill switch.
- Complete support defaults on only after conformance and hardening are complete.

## Ordered goals

1. [x] Add fixtures and protocol-state test infrastructure.
2. [x] Expose bounded streaming APC callbacks through a pinned `vte` fork and cover termination/cancellation splits.
3. [x] Add ordered partial parser consumption and deferred transaction barriers.
4. [x] Implement typed command parsing, direct RGB/RGBA/PNG/zlib transfer, IDs, queries, and quotas.
5. [x] Implement classic placements, deletion, tracked anchors, scroll/reflow, screen ownership, and reset semantics.
6. [x] Add layered OpenGL image rendering, alpha/crop/scale, tiling, cache eviction, and context recovery.
7. [x] Add bounded regular-file, temporary-file, and shared-memory transports behind configuration.
8. [x] Implement Unicode placeholders from authoritative generated combining-mark data.
9. [x] Implement relative placement graph semantics.
10. [x] Implement animation frame loading, composition, control, deletion, and deadline scheduling.
11. [ ] Complete hostile-input, property, fuzz, renderer-golden, context-loss, and performance validation; enable by default. Stateful stream fuzzing and an isolated repeated-redraw screenshot smoke harness are in place. A 10-run release benchmark measured the graphics-enabled ordinary-text parser at 0.909x the disabled baseline (no regression).

Each goal stops only after its focused tests, adjacent suites, formatting, linting, docs, and Lode state pass. Commit verified coherent increments separately.

## Risks and rollback

Parser changes affect all terminal input. Keep generic APC support isolated and verify existing OSC/DCS/PM/SOS behavior. Grid tracking can corrupt placement lifetime; invalid anchors must remove placements rather than clamp. Renderer layering can regress text; preserve an empty-state fast path and compare controlled framebuffers. Disable graphics through configuration for runtime rollback; source rollback remains commit-local by phase.

## Related Lode

- [Graphics domain](../graphics/summary.md)
- [Repository practices](../practices.md)
