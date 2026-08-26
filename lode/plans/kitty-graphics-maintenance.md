# Kitty graphics maintenance architecture

Status: Implemented; acceptance coverage is recorded in the conformance matrix.

## Outcome

The maintenance architecture preserves the supported Unix Kitty Graphics Protocol behavior while bounding parser storage, decode stages, retained pixels, and metadata. It retains terminal-side protocol state, POSIX transports, the vendored VTE fork, CPU-owned canonical pixels, clone-and-swap transmit-and-place transactions, and the observational baseline suite.

## Governing invariants

1. Later PTY bytes are not interpreted until the current graphics barrier has fully committed or failed.
2. Base64 decoding, transport I/O, image decoding, canvas allocation, and pixel composition run without `Term`'s mutex.
3. Commit applies a prepared mutation completely or preserves prior terminal graphics state.
4. Primary and alternate screen graphics state remains independent.
5. Canonical retained pixels remain straight-alpha RGBA8 in an immutable `Arc<Vec<u8>>` owner with exact retained capacity; GPU resources remain disposable.
6. Stored canonical bytes stay within the configured per-screen quota. Deferred work has explicit source, decoder, and destination bounds; combined peak accounting includes concurrently live stages and snapshots. See `docs/kitty-graphics-resources.md`.
7. Non-Unix shared-memory transmission returns `ENOTSUP`.
8. Placeholder cells remain ordinary grid cells. Virtual placements remain immutable lookup prototypes under scrolling.
9. Relative placement graphs remain cycle-free and depth-bounded.
10. No-placement and no-graphics render paths do not scan history or allocate graphics snapshots.

## Architecture

The parser transfers payload ownership without cloning complete bodies. Direct input coalesces into 128 KiB blocks; empty chunks add no blocks. Base64 decoding consumes those blocks while growing its output within an explicit capacity ceiling. Direct image data accepts padded and unpadded standard base64; local transport names retain canonical padding and distinct error behavior.

One deferred processing contract serves normal PTY input and synchronized-update completion, timeout, and overflow replay. `Term` retains reflowable placement-anchor state while work runs off-lock. Decode and frame-composition continuations complete before the parser resumes later input.

Animation preparation snapshots cheap state under the lock, allocates and composes pixels off-lock, then revalidates image identity, structural revision, count, and quota before atomic commit. Store-frame eviction is planned before mutation; frame composition does not evict unrelated images. A unique canvas transfers its allocation into the result; editing a shared canvas copies it once. Plain image replacement and transmit-and-place both commit transactionally and recompute replacement credit after cascading eviction.

`Term::graphics_render_snapshot` builds one frame-local virtual-prototype index. Ordinary virtual placements scan only the viewport. A retained-history scan occurs only when a classic relative placement has a virtual root. The renderer uses separate background and glyph cell passes only for the middle negative-z image stratum.

Production code exposes the render snapshot boundary to `alacritty`. A feature-gated fuzz helper drives the production deferred pipeline without making parser, transaction, or mutation internals public.

## Preserved decisions

- Keep clone-and-swap for transmit-and-place. Metadata cloning remains the simplest proven atomic mechanism until representative measurements justify a separate redesign.
- Keep the reflowable deferred anchor in `Term`, because resize must update it while processing is off-lock.
- Decode transmitted frame data before preparing animation targets to preserve wire-error precedence.
- Use a structural frame revision instead of visible content generation to detect stale animation work.
- Index virtual prototypes by placement handle and materialize renderables only for visible placeholders.
- Keep placeholder resolution in `Term`, where grid inheritance and active-screen ownership are available.
- Keep direct-data and local-name base64 policies separate.
- Keep one feature-gated fuzz boundary that exercises production processing and commit code.
- Do not replace terminal-side protocol state with a Kitty client emitter crate.

## Dependency policy

Keep `base64`, `png`, `miniz_oxide`, `libc`, and test-only `tempfile` while they own distinct protocol boundaries and no measured replacement benefit exists.

Replace the vendored VTE fork only when a released upstream version provides all of:

- streaming APC start, put, end, and abort callbacks;
- 7-bit and 8-bit ST handling;
- ordered parser termination with consumed-byte reporting;
- retained suffix support;
- synchronized completion, timeout, and overflow replay that stops at every barrier;
- unchanged PM and SOS behavior.

If an upstream release satisfies these criteria, compare behavior under a separate accepted migration plan before changing the dependency.

## Non-goals

- No client-specific protocol subset.
- No Windows named shared-memory implementation or Windows KGP conformance claim.
- No parallel graphics worker or completion reorder queue.
- No write-time placeholder-origin index.
- No GPU-only canonical storage or delta animation-frame storage.
- No terminal identity change.
- No broad renderer framework or image-decoder migration without separate evidence and scope.

## Related Lode

- [Graphics domain](../graphics/summary.md)
- [Kitty graphics implementation](kitty-graphics.md)
- [Repository practices](../practices.md)
