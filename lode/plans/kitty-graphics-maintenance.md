# Kitty graphics maintenance plan

- Status: Accepted for implementation
- Date: 2026-08-26
- Base: `1f69bc9193ef55d8a77851bf2e7889e8c119aece`
- Scope owner: Alacritty Kitty graphics fork
- Independent review: Brainbuddy architecture review completed 2026-08-26
- Related completed plan: [Kitty graphics implementation](kitty-graphics.md)

## Summary

Preserve complete Unix Kitty Graphics Protocol behavior while reducing terminal-lock latency, upload peak memory, repeated render scans, duplicate deferred-processing code, and unnecessary public API. The final architecture keeps the terminal-side protocol implementation, POSIX transports, vendored VTE fork, CPU-owned canonical pixels, clone-and-swap transmit-and-place transaction, and observational baseline suite.

This plan fixes interfaces before implementation. An implementation may deviate only after updating this plan and every dependent work item in the same planning commit. Do not leave a changed signature, invariant, test gate, or dependency assumption documented only in an implementation commit.

## Problem and evidence

1. `GraphicsState::store_frame` and `GraphicsState::compose_frames` allocate and copy complete pixel buffers while `Term::commit_graphics_command` holds the terminal mutex. This violates the RFC rule that large allocation and copy work runs off-lock.
2. Display snapshot construction scans all retained history whenever any virtual placement exists. Each visible placeholder then scans every placement to resolve its prototype. The terminal remains locked for both operations.
3. APC parsing clones each payload. Chunk completion then creates a second complete encoded body. The decoded command retains encoded bytes through commit, and placement insertion clones that command again.
4. Normal PTY input and synchronized-update timeout replay duplicate deferred request setup and commit plumbing.
5. New graphics internals are broader than the application boundary requires. Some methods exist only for tests or have no callers.
6. Permanent planning text still claims Windows named shared-memory support, while code and conformance policy reject non-Unix shared memory with `ENOTSUP`.

The completed protocol behavior, conformance matrix, tmux and Zellij behavior, storage bounds, transport policy, and renderer layering are constraints. They are not simplification targets.

## Goals

- Execute base64 decoding, transport reads, image decoding, blank-frame allocation, canvas copying, and pixel composition without holding `Term`'s mutex.
- Preserve FIFO processing: commit each graphics command before parsing later PTY bytes.
- Eliminate a second complete encoded chunk body and release encoded payloads before commit.
- Resolve visible placeholders with one frame-local prototype index.
- Scan retained history only when a classic relative placement has a virtual root.
- Use one deferred processing contract for all parser ingress paths.
- Expose one render snapshot boundary to the application and one feature-gated fuzz driver instead of graphics mutation internals.
- Preserve current Unix KGP behavior and correct stale platform documentation.
- Include the current observational baseline bundle in every new or rewritten feature commit.

## Non-goals

- No protocol subset or client-specific `icat` profile.
- No Windows named shared-memory implementation or Windows KGP conformance claim.
- No parallel graphics worker or completion reorder queue.
- No write-time placeholder-origin index.
- No GPU-only canonical storage.
- No delta animation-frame storage.
- No terminal identity change.
- No replacement of clone-and-swap transactionality without benchmark evidence and a separate accepted plan update.
- No replacement of the vendored VTE fork before released upstream APIs satisfy the exit criteria in this plan.
- No broad renderer framework or image-decoder migration.

## Governing invariants

1. Later PTY bytes are not interpreted until the current graphics barrier has fully committed or failed.
2. Large I/O, decoding, allocation, and pixel-copy work runs without `Term`'s mutex.
3. Commit either applies the prepared mutation completely or preserves prior terminal graphics state.
4. Primary and alternate screen graphics state remains independent.
5. Canonical retained pixels remain straight-alpha RGBA8 in `Arc<[u8]>`; GPU resources remain disposable.
6. Stored canonical bytes stay within the configured per-screen quota. Deferred work is separately bounded by that quota.
7. Non-Unix shared-memory transmission returns `ENOTSUP`.
8. Placeholder cells remain ordinary grid cells. Virtual placements remain immutable lookup prototypes under scrolling.
9. Relative placement graphs remain cycle-free and depth-bounded.
10. The no-placement and no-graphics render paths do not scan history or allocate graphics snapshots.


## Detailed plan documents

- [Wire and deferred interfaces](kitty-graphics-maintenance-wire.md) - Parser ownership, chunk assembly, streaming decode, terminal commit loop, and event-loop contract.
- [Animation interfaces](kitty-graphics-maintenance-animation.md) - Frame snapshots, off-lock composition, revision checks, and atomic frame commit.
- [Rendering interfaces](kitty-graphics-maintenance-rendering.md) - Frame-local virtual prototype index, terminal render snapshot, renderer passes, and fuzz boundary.
- [Ordered work items](kitty-graphics-maintenance-work-items.md) - Dependencies, tests, stop conditions, documentation, and final verification.
- [Execution contract](kitty-graphics-maintenance-execution.md) - Commit workflow, dependency order, risks, rollback, and plan-change rules.

## Review decisions and rejected alternatives

The independent review found three blocking omissions in the first draft. This accepted revision keeps the reflowable anchor in `Term`, makes delete-aborts-pending explicit, and preserves distinct direct-payload and local-name base64 error behavior. It also stabilizes frame processing errors, eviction policy, visible-frame generation changes, anonymous placeholder exclusion, active-grid scan range, no-graphics early return, and the separate fuzz workspace feature.

Options reconsidered:

- Keep `deferred_graphics_anchor` in `Term`, rather than `DecodeWork`, because resize must rewrite it while processing is off-lock.
- Use a two-stage decode then compose continuation, rather than snapshotting animation before decode, to preserve current decode-first error precedence.
- Keep `EncodedPayload::{Single, Chunks}` rather than always wrapping one payload in `Vec<Vec<u8>>`; the enum avoids an outer allocation for the common case and makes chunk ownership explicit.
- Use separate base64 engine configuration for direct data and local names, rather than one padding-indifferent policy, because current wire errors and accepted padding differ.
- Add `frame_revision`, rather than reuse `content_generation`, because playback changes the visible generation without changing frame structure.
- Store `PlacementHandle` in the frame-local prototype index, rather than clone `RenderableGraphic` for every index entry; materialize renderables only for visible placeholders.
- Put `graphics_render_snapshot` on `Term`, rather than `GraphicsState` or the application, because placeholder inheritance and active-grid ownership are terminal semantics.
- Use a feature-gated fuzz helper, rather than public mutation types, so fuzzing exercises the production pipeline without defining a permanent external protocol API.
- Keep clone-and-swap for transmit-and-place. Metadata cloning remains the simplest proven atomic mechanism until measurements justify a separate redesign.
- Keep the vendored VTE fork and current narrow dependencies. Available KGP crates are client emitters, not terminal-side state and rendering implementations.
- Keep per-commit observational bundles. Re-running an unchanged tree must be byte-identical; each source commit still changes the bundle source digest and manifest.


## Dependency order

```text
0 measurements
|
+-- 1 payload ownership
|   |
|   +-- 2 streaming base64
|       |
|       +-- 3 deferred interface
|           |
|           +-- 4 off-lock animation
|
+-- 5 render snapshot
    |
    +-- 6 renderer passes

7 API narrowing depends on 3 through 6
8 documentation depends on all code work
9 dependency and VTE review follows final code shape
10 verification and history is last
```


## Open questions

None known. New evidence that changes a stabilized interface must first update this plan and its dependent work items.
