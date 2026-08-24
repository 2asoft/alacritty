# Kitty graphics implementation plan

Status: complete

## Outcome and acceptance

Implement the complete Kitty terminal graphics protocol described by the accepted [RFC](../../docs/kitty-graphics-rfc.md). Completion requires every item in its definition of done, including ordered queries, all transports and image formats, classic and placeholder placements, animation, bounded resources, reflow-safe anchors, layered rendering, texture tiling, context recovery, and no material steady-state regression without graphics.

## Verified current state

- Base revision is `7dd7b5b0` on `aasoft/kitty`.
- The workspace now carries a local `vte` 0.15.0 fork. APC has its own streaming callback state; PM and SOS remain ignored.
- `alacritty_terminal` parses bounded Kitty APC data into typed commands. The PTY loop retains the suffix at a graphics barrier, accepts the published 4 KiB chunks plus Kitty 0.48.2's bounded 128 KiB and unpadded-base64 client behavior, assembles bounded direct chunks without copying image bodies under the terminal lock, decodes RGB, RGBA, PNG, and zlib data outside `Term`'s mutex, commits in order, and preserves query-response ordering. Regular-file, constrained temporary-file, and POSIX shared-memory transfers run outside the terminal lock and enforce the local-transmission gate, opened-object type and filesystem checks, post-open sensitive-path checks, ranges, and storage bounds.
- Per-screen image storage uses process-unique monotonic handles, external ID indexes, deterministic image-number lookup, atomic ID replacement, one quota-bounded deferred decode working buffer, a canonical stored-byte limit, and deterministic oldest-first eviction. New IDs can reach commit-time eviction even when canonical storage is full; peak CPU pixel memory remains bounded by stored quota plus one decode working buffer.
- Basic classic transmit-and-place and place-existing commands create independent placements. Successful placements advance the cursor using explicit, aspect-inferred, or native-pixel cell spans unless suppressed; virtual and relative placements never move it. The OpenGL renderer caches textures and draws crop/scaled RGBA content in the protocol's three image z strata around cell backgrounds, glyphs/decorations, and the cursor. Classic anchors follow width reflow and full-screen scrolling. Partial-region scrolling moves only fully contained placements and retains page-area clipping while content exits the region. Anchors are removed when retained history prunes them. Deferred placement anchors also follow resize while heavy processing runs outside the terminal lock. The renderer tiles images to the hardware texture limit with overlap borders and uses a deterministic LRU texture cache bounded by the configured graphics storage limit; oversized working images stream through transient tiles. Image passes intersect the active damage scissor with terminal content bounds, including placements anchored partially outside the viewport. The framebuffer harness discards and reconstructs texture state and verifies bounded cache eviction. Classic deletion supports the complete lowercase/uppercase selector matrix and aborts incomplete chunk streams.
- Virtual placements and explicit/inherited Unicode placeholder cells render from the complete authoritative generated diacritic table. Row and column indices cover the full table, high image-ID bytes remain byte-bounded, all inheritance conditions are tested, and location-based deletion excludes virtual placements.
- Relative placements resolve classic and virtual parents, enforce an eight-link depth bound, reject missing parents and cycles atomically, follow parent movement, and cascade deletion and replacement lifetimes.
- Animation frames load as quota-counted canonical RGBA canvases, support frame editing and alpha/overwrite composition, client-selected frames, stop/loading/run states, loop limits, visible and gapless frame gaps, frame deletion, and UI-scheduled playback deadlines. Stop resets loop progress, loading resumes when a frame arrives, animation transitions damage their placements, and runtime framebuffer tests cover automatic and client-selected playback.
- Primary grid width changes reflow cells in `alacritty_terminal/src/grid/resize.rs`; transient, serde-skipped cell markers apply the same mapping to classic and deferred graphics anchors without adding per-cell steady-state storage.
- Terminal renderable state is collected under lock, while OpenGL work occurs after lock release. Without placements, rendering skips history/viewport placeholder scans and image snapshot allocation.

## Accepted decisions

- The official Kitty protocol is authoritative for wire behavior. The RFC defines private-fork policy where the protocol does not.
- Canonical RGBA8 CPU data belongs to per-screen terminal graphics state. GPU textures are disposable caches.
- Generic APC recognition streams data without whole-string buffering.
- Heavy transport and decode work forms an ordered parser barrier and runs outside `Term`'s mutex.
- Classic placements are independent terminal objects with reflow-aware anchors. Placeholder instances remain ordinary cells.
- Local transports default on but have a configuration kill switch.
- Kitty graphics support is always available. After manual acceptance and external conformance hardening, the operator chose to remove the temporary protocol-disable option rather than retain a feature switch.

## Ordered goals

1. [x] Add fixtures and protocol-state test infrastructure.
2. [x] Expose bounded streaming APC callbacks through a pinned `vte` fork and cover termination/cancellation splits.
3. [x] Add ordered partial parser consumption and deferred transaction barriers.
4. [x] Implement typed command parsing, direct RGB/RGBA/PNG/zlib transfer, IDs, queries, quotas, Kitty-compatible transport errors, permissive flag values, and smallest-free number IDs.
5. [x] Implement classic placements, deletion, tracked anchors, scroll/reflow, screen ownership, reset semantics, pixel-offset spans, and the full selector matrix.
6. [x] Add layered OpenGL image rendering, alpha/crop/scale, tiling, cache eviction, and context recovery.
7. [x] Add bounded regular-file, temporary-file, and shared-memory transports behind configuration.
8. [x] Implement Unicode placeholders from authoritative generated combining-mark data, including fit-and-center virtual-box geometry.
9. [x] Implement relative placement graph semantics. Graph safety, fallback parent selection, and virtual parent placement ID zero pass external scenarios.
10. [x] Implement animation frame loading, composition, control, deletion, and deadline scheduling, including external root, gap, response, control, overlap, deletion, and excess-data scenarios.
11. [x] Complete every row in the [protocol conformance requirements](../../docs/kitty-graphics-conformance.md).
12. [x] Complete stream, default-value, query, quiet-mode, response-identity, and wire-error tests.
13. [x] Complete the RGB/RGBA/PNG/zlib format matrix, including compressed PNG sizing and remaining valid PNG forms.
14. [x] Complete direct chunk, file, temporary-file, POSIX shared-memory, and Windows named-shared-memory behavior and tests. The Windows-gated test cross-compiles on Linux and is ready for native Windows CI execution.
15. [x] Complete image identity, successful replacement, placement replacement, metadata-limit, and stable-accounting tests.
16. [x] Complete classic crop/scale/offset/clipping, z-order, alpha-overlap, GL-state, texture-cache, and context-recovery framebuffer tests.
17. [x] Complete placeholder placement-ID, background, sparse-grid, clipping, deletion, and virtual-parent-origin tests.
18. [x] Complete relative error responses and the full lowercase/uppercase deletion-selector matrix.
19. [x] Implement and test gapless frames and stop-time loop reset; complete animation canvas, loading, loop, composition, chunking, retransmission, scheduler, and runtime playback scenarios.
20. [x] Complete RIS, text-erasure, reverse/margin scrolling, placeholder erasure, geometry-query, fuzz-state, repeated-accounting, and always-on performance validation.
21. [x] Run the complete workspace, application, terminal, reference, VTE, fuzz, framebuffer, benchmark, Lode, and diff checks. The final Linux run passed 235 terminal tests, 84 application tests, 45 reference tests, 58 VTE tests, workspace Clippy, 1,000 instrumented fuzz runs, the expanded framebuffer smoke, and a 125.5 ms mean always-on ordinary-text benchmark. Final animation acceptance used Kitty 0.48.2 `kitten icat` with treemd's 645-frame GIF: four framebuffer captures differed across playback and playback was visible. A 64 MB quota-pressure run kept treemd's software-driven modal animation rendering beyond its previous placeholder failure point; its unique-ID frame churn, slow software encoding, and stale-placeholder reuse remain treemd client behavior rather than native KGP animation behavior.

Each goal stops only after its focused tests, adjacent suites, formatting, linting, docs, and Lode state pass. Commit verified coherent increments separately.

The source-grounded scenario matrix is [Kitty graphics conformance](../../docs/kitty-graphics-conformance.md). Its current oracles are Kitty `77630f3a6748cdf0e3675cc6b768d5dd018a5052`, Ghostty `9f0e1719dc918368367d368bfe300f59bb68b5a4`, and installed Kitty 0.48.2. The protocol remains authoritative. Tests derived from external scenarios use independent fixtures; GPL Kitty test code is not copied.

## Risks and rollback

Parser changes affect all terminal input. Keep generic APC support isolated and verify existing OSC/DCS/PM/SOS behavior. Grid tracking can corrupt placement lifetime; invalid anchors must remove placements rather than clamp. Renderer layering can regress text; preserve an empty-state fast path and compare controlled framebuffers. Graphics is always available, so rollback is source-level and remains commit-local by coherent protocol increment.

## Related Lode

- [Graphics domain](../graphics/summary.md)
- [Repository practices](../practices.md)
