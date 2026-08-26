# Kitty graphics maintenance: ordered work items

Parent plan: [Kitty graphics maintenance](kitty-graphics-maintenance.md)

## Work items

### Work item 0: Safety references and measurements

Status: Complete.

1. Record local and remote tips and create a backup reference.
2. Confirm a clean tree.
3. Add repeatable measurements for ordinary text, chunk peak RSS, animation composition, and placeholder snapshot scaling.
4. Regenerate the observational baseline and prove a second run on the same source tree is byte-identical. A later source commit still changes the checked report source digest and manifest.

Measurement cases:

- 64 MiB direct chunked RGBA and RGB uploads.
- A 4096 by 4096 frame insertion and frame composition.
- 100,000 retained lines with no graphics, one ordinary virtual placement, and one relative virtual root.
- 65,536 virtual placements with viewport placeholders.

Stop when the harness records machine-readable results without changing production behavior.

Suggested commit: `test(graphics): measure deferred graphics working sets`

### Work item 1: Move parser payload ownership

Status: Complete.

1. Add `ParsedCommand`.
2. Move payload from parser with `mem::take`.
3. Remove payload and the terminal-derived anchor from `Command`.
4. Add `EncodedPayload`, `GraphicsRequest`, and `RequestError`, then update chunk assembly.
5. Capture the placement anchor in `Term` only when a complete request is ready.
6. In the same commit, change the transitional `begin_graphics_processing` path so it sets processing flags without overwriting the anchor captured during request completion. Work item 3 later removes this superseded method.
7. Preserve delete-aborts-pending, final quiet override, response identity, and simultaneous `i`/`I` presence semantics.
8. Copy placement scalar fields directly instead of cloning `Command`.

Focused tests cover every parser split, payload overflow, malformed control, padded and unpadded data, placement fields, and response identity.

Stop when no parser completion or placement path clones encoded payload bytes.

Suggested commit: `refactor(graphics): separate command metadata from payloads`

### Work item 2: Stream base64 input

Status: Complete.

Depends on work item 1.

1. Add `EncodedReader`.
2. Configure a padding-indifferent engine for direct data and retain canonical standard-base64 configuration for local transport names.
3. Decode direct data through a padding-indifferent `DecoderReader` into one bounded output.
4. Decode local transport names once with canonical standard-base64 behavior and preserve `EINVAL` on malformed names.
5. Remove chunk concatenation and the direct padded/unpadded retry.
6. Make encoded-limit and decoded-limit arithmetic fail closed while preserving format-specific short, excess, and quota errors.
7. Release encoded payload before transport reads and source decoding.

Focused tests cover chunk boundaries, invalid intermediate padding, final unpadded input, one-byte reads, overflow, quota errors, and all image formats.

Stop when a source inspection and peak-RSS run prove there is no second complete encoded body.

Suggested commit: `perf(graphics): stream chunked payload decoding`

### Work item 3: Establish the deferred interface

Depends on work items 1 and 2.

1. Add `graphics/deferred.rs` and the stabilized deferred types.
2. Implement `Term::take_deferred_graphics`, including delete-aborts-pending, final quiet override, final-chunk anchor capture, and reflow tracking through `deferred_graphics_anchor`.
3. Implement final-only behavior in `Term::commit_deferred_graphics` before adding frame continuations.
4. Add `EventLoop::process_deferred_graphics`.
5. Route normal PTY and every synchronized replay path through it.
6. Add the fuzz-only feature and helper.
7. Remove superseded setup methods.

Focused tests cover normal, synchronized completion, timeout, and overflow ordering; suffix retention; chunk completion; delete-aborts-pending; final quiet override; deferred-anchor reflow; both continuation-stage cancellation points; and fuzz compilation.

Stop when all ingress paths call one helper and no heavy processor accepts `Term`.

Suggested commit: `refactor(graphics): centralize deferred command processing`

### Work item 4: Prepare animation pixels off-lock

Depends on work item 3.

1. Add `frame_revision` and increment it at every structural mutation.
2. Add `FrameCanvas`, `FrameDestination`, `FrameWork`, and `PreparedFrameMutation`.
3. Split store-frame validation/snapshot from pixel processing and commit.
4. Split frame composition validation/snapshot from pixel processing and commit.
5. Add continuation behavior to `Term::commit_deferred_graphics`.
6. Revalidate handle, revision, count, and quota at final commit; preflight store-frame eviction and forbid compose-frame eviction.
7. Remove pixel-copy work from `GraphicsState::store_frame` and `compose_frames`; remove or reduce those methods after callers migrate.

Focused tests cover canvas defaults, frame bases and edits, overwrite and alpha composition, overlap rejection, stale targets, quota changes, store-frame eviction, compose-frame no-eviction, deletion, root promotion, visible-frame `content_generation`, response frame number, loading resume, and atomic failure.

Stop when `blank_frame` and `compose` have no call path under a terminal guard and lock-duration measurements confirm it.

Suggested commit: `perf(graphics): prepare animation frames outside terminal lock`

### Work item 5: Build the terminal graphics render snapshot

Independent of work items 1 through 4 after work item 0.

1. Add `GraphicsRenderSnapshot` and `RenderablePlaceholder`.
2. Add `Term::graphics_render_snapshot`.
3. Build one virtual prototype index per snapshot.
4. Compute required virtual origin keys from relative chains.
5. Scan only the viewport for ordinary virtual placements.
6. Scan retained history once only for required virtual roots.
7. Move placeholder suppression inputs to the snapshot result.
8. Remove application-side prototype and origin scans.

Focused tests cover exact and zero placement IDs, anonymous-image exclusion, oldest prototype selection, inheritance reset, sparse grids, viewport coordinates, display offset, full active-grid origin range, no-graphics early return, independent origin axes, relative chains, scrolling, alternate screens, and tmux navigation.

Stop when visible placeholder lookup is O(viewport cells plus placement count), and ordinary virtual placements do not scan history.

Suggested commit: `perf(renderer): index virtual graphics per frame`

### Work item 6: Avoid unnecessary cell passes

Depends on work item 5 so the final snapshot path receives framebuffer coverage.

1. Add `split_cells` from the middle negative range.
2. Keep two passes only for middle negative images.
3. Preserve placeholder suppression in one-pass and split-pass paths.
4. Preserve decorations, transparency, damage, and cursor order.

Framebuffer tests cover no graphics, each stratum alone, mixed strata, transparent placeholders, and cursor overlap.

Stop when nonnegative-only and very-negative-only frames use one cell pass without framebuffer changes.

Suggested commit: `perf(renderer): avoid unnecessary graphics cell passes`

### Work item 7: Narrow APIs and optimize static lookup

Depends on work items 3 through 6.

1. Remove `Term::take_graphics_command`.
2. Remove `GraphicsState::place` and `place_handle` if no production caller remains.
3. Remove superseded processing option and commit methods.
4. Make parser, transaction, deferred, storage mutation, and animation preparation types crate-private.
5. Keep public only `PixelBuffer`, `ImageHandle`, render snapshot types, renderable types required by `alacritty`, and `Term::graphics_render_snapshot`.
6. Use binary search for the generated sorted diacritic table.
7. Move tests to production boundaries.

Stop when repository-wide usage proves every remaining public item has an external caller or intentional reusable contract.

Suggested commit: `refactor(graphics): narrow deferred protocol APIs`

### Work item 8: Reconcile permanent documentation

Depends on all code work.

1. Replace Windows named shared-memory claims with POSIX support and non-Unix `ENOTSUP` policy.
2. Update the RFC shared-memory section.
3. Update deferred pipeline diagrams and type examples to the final interfaces in this plan.
4. Update resource-bound and peak-memory text.
5. Update render snapshot and conditional history-scan descriptions.
6. Recheck every conformance `Covered` row against a direct test.
7. Update changelogs, manual page, Lode graphics summary, completed implementation plan, and this plan.
8. Mark this plan complete only after final verification.

A separate documentation-only commit may remove RFC duplication of the official wire specification. It must preserve local policy, invariants, security rationale, accepted extensions, rejected alternatives, and conformance links.

Suggested commits:

- `docs(graphics): reconcile deferred processing architecture`
- `docs(graphics): align shared memory platform policy`

### Work item 9: Recheck dependencies and VTE exit criteria

Depends on final code shape.

Keep `base64`, `png`, `miniz_oxide`, `libc`, and test-only `tempfile` unless measurements identify a concrete replacement benefit. Do not adopt KGP client emitter crates.

Replace the local VTE fork only when a released upstream version provides all of:

- streaming APC start, put, end, and abort callbacks;
- 7-bit and 8-bit ST handling;
- ordered parser termination with consumed-byte reporting;
- retained suffix support;
- synchronized completion, timeout, and overflow replay that stops at every barrier;
- unchanged PM and SOS behavior.

If upstream satisfies these criteria, create a separate migration plan and compare behavior before changing the dependency.

### Work item 10: Final verification and history

Run after the last relevant edit:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check --workspace --target x86_64-pc-windows-gnu
python3 -m unittest discover -s tests/kitty_graphics -p 'test_*.py'
shellcheck scripts/kitty-graphics-baseline.sh
scripts/kitty-graphics-baseline.sh
git diff --check
```

Also run the fuzz crate check, stateful KGP fuzzing, framebuffer smoke, ordinary-text benchmark, chunk peak-RSS benchmark, placeholder scaling benchmark, animation lock-duration benchmark, direct client, tmux, Zellij, native animation, Broot, Yazi, and treemd acceptance checks.

Acceptance gates:

- All conformance rows remain covered or retain an explicit accepted policy.
- No large animation allocation or pixel copy occurs under `Term`'s mutex.
- Chunk decoding has no second complete encoded body.
- Ordinary virtual placements do not scan retained history.
- Ordinary-text performance stays within the baseline run's measured noise band.
- No Windows KGP implementation or conformance claim appears.
- Every new or rewritten feature commit contains its generated bundle and changes at least the recorded source digest and manifest, even when protocol observations are unchanged.
- History remains linear and semantic, with no repair, fixup, revert, or merge commits.
- The working tree is clean.
