# Kitty graphics conformance requirements

This matrix tracks protocol requirements and their test evidence. Covered entries refer to exercised scenarios at the responsible public or wire boundary; they do not establish every combination of controls and state. Implementation inspection alone does not count as test coverage. Remaining coverage gaps are identified below.

## Oracles

- Kitty protocol and `kitty_tests/graphics.py` at `kovidgoyal/kitty@77630f3a6748cdf0e3675cc6b768d5dd018a5052`.
- Ghostty `src/terminal/kitty/graphics_*.zig` at `ghostty-org/ghostty@9f0e1719dc918368367d368bfe300f59bb68b5a4`.
- Runtime comparisons use Kitty 0.48.2.
- The complete rendered specification at `https://sw.kovidgoyal.net/kitty/graphics-protocol/` was re-fetched for this audit. Its temporary Markdown representation has BLAKE3 `f61ab0bae0aadf9d4bf74cb16dcf9dc4c15b8269f8586991afd7639a9d31a84a` and is not distributed with this repository.

The written protocol defines required behavior. Kitty resolves ambiguities in its documentation, and Ghostty provides an independent compatibility check. Tests derived from external scenarios use independent fixtures and native Alacritty test APIs. Do not copy Kitty's GPL test source into this repository.

Status meanings:

- `Covered`: a direct test exercises the stated boundary.
- `Partial`: some variants or only a lower-level boundary are tested.
- `Required`: no direct test establishes the requirement.
- `Implementation required`: inspection shows that behavior is absent.
- `Policy`: an intentional Alacritty behavior outside Kitty's required semantics.

## Complete written-spec audit

The audit maps every specification heading to requirements below. `A minimal example` is informative; all other headings contribute requirements or client constraints.

| Specification heading | Requirement groups |
| --- | --- |
| Getting the window size | Terminal interaction and lifecycle |
| The graphics escape code | Stream, parser, and responses |
| Transferring pixel data; RGB/RGBA; PNG; Compression | Pixel formats and compression |
| Transmission medium; Local client; Remote client | Transports and chunking |
| Querying support and available transmission mediums | Stream, parser, and responses; Transports and chunking |
| Display images on screen; Controlling displayed image layout | Identity, storage, and replacement; Classic placement and rendering |
| Unicode placeholders | Unicode placeholders |
| Relative placements | Relative placements |
| Deleting images | Deletion |
| Suppressing responses | Stream, parser, and responses |
| Requesting image IDs | Identity, storage, and replacement |
| Usage hints | Identity, storage, and replacement |
| Animation; frame transfer; control; composition | Animation |
| Image persistence and storage quotas | Identity, storage, and replacement; Robustness and performance |
| Control data reference | Every parser and action-specific group |
| Interaction with other terminal actions | Terminal interaction and lifecycle |

The audit also inventories every production call into the VTE parser. Normal input, synchronized-update completion, synchronized-update timeout, and synchronized-buffer overflow must all preserve ordered graphics barriers.

## Stream, parser, and responses

| Requirement | Status |
| --- | --- |
| APC starts, terminates with 7-bit and 8-bit ST, cancels with CAN/SUB, preserves the following input suffix, and does not change PM/SOS behavior | Covered |
| APC control and payload storage are independently bounded across arbitrary input splits; payloads accept Kitty's bounded 128 KiB client chunks | Covered |
| Unknown keys are ignored, the final duplicate key wins, integer domains are enforced, and all action-specific permissive flag values match Kitty | Covered |
| Default `a=t`, `f=32`, `t=d`, `q=0`, `m=0`, and display/frame defaults produce the documented semantics | Covered |
| A query response is ordered before a later DA response, including inside synchronized updates | Covered |
| `a=q` validates input without storing or replacing an image | Covered |
| Commands without a recoverable nonzero image ID or number, including malformed controls and anonymous queries, do not emit unsolicited or invented `i=0` responses | Covered |
| `q=1` suppresses success and `q=2` suppresses success and failure, including chunk continuations | Covered |
| Success and failure responses preserve initial image, image-number, placement, and frame identity | Covered |
| Conflicting `i` and `I` produce `EINVAL` unless quiet mode suppresses it | Covered |
| Placement and relative-placement failures emit `ENOENT`, `ENOPARENT`, `ETOODEEP`, and `ECYCLE` on the wire; missing-image responses include printable detail | Covered |
| Synchronized-update completion, timeout, and buffer overflow replay each Kitty APC only up to its ordered barrier | Covered |

## Pixel formats and compression

| Requirement | Status |
| --- | --- |
| RGB and RGBA require nonzero dimensions, accept exact data, reject short/excess data, and normalize to straight RGBA | Covered |
| PNG supports valid grayscale, grayscale-alpha, indexed, RGB, and RGBA color types at every valid bit depth | Covered |
| PNG supports interlaced palette and non-palette images and transparency metadata | Covered |
| Zlib works for RGB, RGBA, and PNG and remains bounded under decompression bombs | Covered |
| Compressed PNG requires `S` and interprets it as the decompressed PNG byte size | Covered |
| Decoder dimensions, arithmetic, intermediate allocations, and canonical output remain within quota | Covered |

## Transports and chunking

| Requirement | Status |
| --- | --- |
| Direct transmission supports complete and chunked RGB, RGBA, PNG, and zlib payloads | Covered |
| Non-final chunks have valid base64 boundaries, padded and unpadded base64 are accepted, and total encoded data remains bounded without concatenating a second encoded body | Covered |
| Final-chunk cursor position anchors transmit-and-place | Covered |
| Any delete command aborts an incomplete upload, and the next upload starts cleanly | Covered |
| Chunk success, decode failure, continuation failure, and changing quiet levels preserve initial request identity | Covered |
| Direct chunks accept the written `m`/optional-`q` continuation form, the written `a=f` frame form, and Kitty's emitted omitted/repeated action variants | Covered, compatibility extension |
| File and shared-memory transports ignore the direct-only `m` flag | Covered |
| File and shared-memory `S`/`O` ranges accept exact ranges and reject short or overflowing ranges with request identity | Covered |
| Regular files and safe symlinks are read; symlink loops and every special-file class fail without blocking | Covered |
| Sensitive opened objects are rejected after path resolution | Covered on Linux |
| Temporary files are removed only when both the directory and marker constraints hold | Covered |
| POSIX shared memory is read and range-checked before it is unlinked and closed | Covered |
| Shared-memory transmission on non-Unix hosts is rejected with `ENOTSUP` | Policy |
| Disabling local transmission cancels in-flight local work while direct transmission remains available | Covered |

## Identity, storage, and replacement

| Requirement | Status |
| --- | --- |
| Number-based transmission allocates the smallest free nonzero client ID and replies with both ID and number; explicit `i=0` and `I=0` retain anonymous defaults | Covered |
| Number lookup and deletion select the newest image with that number | Covered |
| Successful retransmission of an ID removes its old placements and does not display the replacement implicitly | Covered |
| Failed or incomplete replacement preserves the old image and placements atomically | Covered, policy extension |
| Same nonzero `(i,p)` replaces a placement without duplication or flicker | Covered |
| `p` is ignored for image ID zero; repeated zero placement IDs remain independent | Covered |
| Canonical image and animation bytes stay within the per-screen storage quota; metadata has separate count limits and processing stages have separate working bounds | Covered by quota/transaction tests and the allocation fixtures; see resource bounds |
| Animation canvas allocation, pixel copying, and composition run outside the terminal mutex; stale revisions and quota changes fail atomically at commit | Covered |
| Usage hint `N` is parsed as a bitmask; eviction recognizes its transient bit and prioritizes transient, unplaced, and non-visible images | Covered |
| Frame-level transient hints are accepted but intentionally ignored because frames share canonical in-memory image ownership | Policy |
| Usage hints on placement commands do not mutate stored image policy | Covered |
| Replacement remains possible at metadata and byte limits without temporary double accounting | Covered |
| Repeated upload/delete and replacement restore all tracked byte and object accounting | Covered |

## Classic placement and rendering

The terminal render snapshot indexes virtual prototypes once per frame. It scans only the viewport for ordinary placeholders and scans retained history only for classic relative chains with virtual roots. The renderer splits cell backgrounds from glyphs only when the middle negative-z stratum requires it.

| Requirement | Status |
| --- | --- |
| Native, explicit, and one-axis-inferred placement spans preserve aspect ratio | Covered |
| `X`/`Y` offsets clamp to cell bounds and contribute correctly to native span and cursor movement | Covered |
| Source `x,y,w,h` is intersected with the image before scaling | Covered |
| Images truncate at terminal and scroll-region clip bounds without changing source geometry | Covered |
| Cursor movement follows documented classic placement rules and never applies to virtual or relative placements | Covered |
| `C=1` suppresses cursor movement; other Kitty-compatible values retain default movement | Covered |
| Equal-z images order by lower image ID, with deterministic creation order where the protocol is undefined | Covered |
| Very-negative, negative, and nonnegative z strata compose below backgrounds, below text, and above text as specified | Covered |
| Straight-alpha CPU pixels upload as premultiplied textures and blend transparent edges without fringes | Covered |
| Overlapping translucent classic placements produce source-over framebuffer output | Covered |
| Every image pass restores shared OpenGL bindings and state used by later text and rectangle passes | Covered |
| Oversized textures tile with overlap, cache accounting is bounded, LRU eviction is deterministic, and transient tiles do not enter the cache | Covered |
| Destroying every GPU texture or recreating the context reconstructs the same framebuffer from CPU state | Texture discard covered; complete GL context recreation is untested |

## Unicode placeholders

| Requirement | Status |
| --- | --- |
| The complete authoritative row/column diacritic table is generated and decoded | Covered |
| Indexed and RGB foreground colors encode image IDs, including the bounded high-byte diacritic | Covered |
| Indexed and RGB underline colors encode placement IDs, with zero or omitted IDs selecting any matching virtual placement | Covered |
| All documented left-to-right inheritance conditions and their mismatch boundaries are applied | Covered |
| Virtual placement geometry fits and centers the image while preserving aspect ratio | Covered |
| Partial, sparse, scrolled, clipped, and horizontally shifted placeholder grids sample the correct source tiles | Covered |
| Placeholder background color remains visible through transparent image pixels | Covered |
| Successfully resolved cells suppress placeholder glyphs and decorations; unresolved cells remain visible | Covered |
| Location-based deletion never affects virtual placements; only `i/I`, `n/N`, and `r/R` do | Covered |
| Full-screen and margin scrolling move placeholder cells without moving or removing their virtual placement prototypes | Covered |

## Relative placements

| Requirement | Status |
| --- | --- |
| Explicit and fallback parents resolve as Kitty specifies, including placement ID zero for virtual parents | Covered |
| Cell offsets accumulate through relative chains and relative placement never moves the cursor | Covered |
| Missing parents, cycles, and chains deeper than eight fail atomically with required wire errors | Covered |
| A virtual placement cannot itself be relative and returns `EINVAL` | Covered |
| A virtual parent's origin uses independent minimum X and minimum Y across all matching placeholder cells | Covered |
| Parent move, replacement, deletion, pruning, and eviction cascade through every descendant and reclaim orphan images | Covered |

## Deletion

| Requirement | Status |
| --- | --- |
| Every lowercase selector removes matching placements while retaining reusable image data | Covered |
| Every uppercase selector removes matching placements and frees only unreferenced image data | Covered |
| `i/I` and `n/N` honor an optional placement ID | Covered |
| `a/A`, `c/C`, `p/P`, `q/Q`, `x/X`, `y/Y`, and `z/Z` use visible/intersection and one-based coordinate semantics | Covered |
| `r/R` handles inclusive, omitted, reversed, and empty ranges | Covered |
| `f/F` handles root promotion, current-frame deletion, missing frames, and non-animated images | Covered |
| Every delete selector aborts incomplete direct uploads | Covered |

## Animation

| Requirement | Status |
| --- | --- |
| Frame create and edit support transparent/default and explicit `Y` canvases, prior-frame `c` canvases, offsets, overwrite, and alpha blend | Covered |
| New non-root frames default to 40 ms; zero gap preserves the prior/default gap | Covered |
| Negative gaps create gapless frames that playback skips without displaying | Covered |
| Frame responses report the created or edited frame number | Covered |
| Client-selected current frames update rendered content | Covered |
| Stop, loading, and running states follow the documented transition rules | Covered |
| Loading mode parks on the final frame and resumes when a frame arrives | Covered |
| Infinite and finite loop budgets have exact semantics, and stop resets the completed-loop counter | Covered |
| Animation deadlines wake the application scheduler without busy polling | Covered |
| Frame composition validates source/destination frames, omitted/zero full-image rectangle defaults, bounds, same-frame overlap, and exact `C=1` overwrite versus default blend semantics | Covered |
| Frame deletion promotes roots, adjusts the current frame, updates rendering, and frees accounting | Covered |
| Retransmitting the base image removes all animation state | Covered |
| A runtime framebuffer test proves automatic multi-frame playback; scheduler tests prove gapless skipping | Covered |

## Terminal interaction and lifecycle

| Requirement | Status |
| --- | --- |
| Full-screen scrolling moves classic placements with text and prunes them with history | Covered |
| Forward and reverse scrolling inside vertical margins moves only fully contained placements and clips them at the page area | Covered |
| Primary-buffer width reflow tracks classic and deferred anchors without corrupting source pixels | Covered |
| RIS clears visible graphics according to Alacritty reset semantics | Covered |
| Entering a fresh 1049 alternate screen clears its graphics independently of the primary screen | Covered |
| `ED 2` clears visible graphics, while `ED 3`, EL, ECH, DCH, and other text erasure leave classic graphics unchanged | Covered |
| Erasing placeholder cells removes only their cell-derived image instances | Covered |
| `TIOCGWINSZ`, `CSI 14 t`, `CSI 16 t`, and `CSI 18 t` report coherent text-area pixels, cell pixels, and text-area cells | Covered |

## Robustness and performance

| Requirement | Status |
| --- | --- |
| Stateful fuzzing covers parser splits, synchronized replay, chunk transactions, decoders, state operations, grid movement, reset, and cancellation | Covered; wall-clock timeout requires separate tests |
| Random state tests assert image/placement/index/quota/graph invariants after every operation | Covered |
| Framebuffer checks cover z strata, crop/scale/offset, alpha overlap, placeholders, animation, and texture eviction/reconstruction | Covered; complete GL context recreation is untested |
| Ordinary non-graphics workloads show no material parser, memory, or redraw regression with always-on support | Partial: parser timing and empty-snapshot tests exist; comparative memory/redraw thresholds are not established |

## Runtime fixtures

`scripts/kitty-graphics-smoke.sh` checks these composed framebuffer scenarios under headless Sway:

- text remains stable across repeated graphics redraws;
- very-negative, negative, and positive images occupy the required background/text strata;
- source cropping and scaling produce the expected framebuffer color;
- overlapping translucent images use source-over composition;
- transparent Unicode placeholders expose their cell background without exposing the placeholder glyph;
- automatic animation and client-selected frames both reach the framebuffer;
- animation transitions damage their placement and trigger visible redraws;
- texture eviction stays within the configured byte limit;
- discarded image textures reconstruct from canonical CPU pixels;
- image passes leave later text rendering stable.

`scripts/kitty-graphics-geometry.sh` additionally checks exact color counts at padding 0 and 40 for native pixel sizes, single-axis aspect scaling, explicit spans with pixel offsets, negative-z placement, enlarged fractional placeholder tiles, tiny sampling extents at source coordinates beyond `2^24`, and CPU-composed alpha.

The smoke script does not run `kitten icat` or the previously reported 645-frame GIF. That real-client observation is not automated coverage in this repository.

## Regression evidence

| Contract | Executable evidence |
| --- | --- |
| Empty continuations consume no block metadata; small chunks coalesce | `graphics::transaction::tests::empty_and_small_continuations_bound_metadata_by_encoded_bytes` |
| Replacement rollback includes parent-eviction cascades | `graphics::storage::tests::replacement_accounts_for_parent_eviction_and_preserves_state_on_failure` |
| Large spans and relative offsets remain safe during deletion | `term::tests::graphics_protocol::large_rows_and_relative_offsets_do_not_overflow_deletion` |
| Padding preserves image coordinates | `scripts/kitty-graphics-geometry.sh`, padding 0/40 |
| Native dimensions survive font changes; footprints refresh in both buffers | `term::tests::graphics_protocol::native_sizing_survives_font_changes_and_updates_its_footprint`; `term::tests::font_changes_refresh_native_footprints_in_both_buffers`; geometry fixture |
| Native footprints protect partially visible images during eviction | `graphics::storage::tests::native_footprint_visibility_protects_partially_scrolled_image` |
| Inferred spans widen large pixel offsets before scaling | `graphics::storage::tests::inferred_placement_spans_widen_pixel_offsets_before_scaling` |
| Fractional placeholder sampling preserves every tile | Display geometry tests, renderer source-crop tests, geometry fixture |
| CPU alpha preserves white and composes the expected framebuffer | `graphics::animation::tests::white_over_white_remains_white_for_every_alpha`; geometry fixture |
| Omitted placement IDs resolve the correct virtual prototype and independent origin minima | `term::tests::graphics_protocol::virtual_parent_origin_uses_the_resolved_prototype` |
| Location deletion follows current placeholder origins | `term::tests::graphics_protocol::virtual_rooted_deletion_ignores_the_creation_cursor` |
| Finite and all-gapless animation avoid publishing a hidden frame or polling indefinitely | Storage finite, initial-gapless, and all-gapless tests |
| Working stages preserve ownership and obey fixture allocation thresholds | `scripts/kitty-graphics-memory.sh`; pixel/canvas ownership tests; [resource bounds](kitty-graphics-resources.md) |
| Self-parent replacement reports `ECYCLE` and preserves state | `term::tests::graphics_protocol::self_parent_returns_cycle_and_preserves_the_placement` |

## Multiplexer interoperability

Isolated runtime checks use separate tmux sockets and Zellij socket directories so existing sessions remain untouched.

- tmux 3.7c passes classic Kitty graphics commands to Alacritty when the pane's `allow-passthrough` option is `on`. A zsh child rendered a controlled PNG through the tmux DCS passthrough wrapper.
- Zellij 0.45.0 probes the host with a Kitty query and requires `CSI 16 t` before enabling its graphics proxy. Alacritty answers both requests, and a zsh child receives `OK` from Zellij for query and transmit-and-place commands.
- Zellij's emitted 4 KiB chunks and placement are wrapped in `CSI ?2026 h/l` synchronized updates. Barrier-aware synchronized replay preserves transmit/placement and query/DA order. An isolated `Alacritty -> Zellij -> zsh` framebuffer test renders a controlled image.
- Kitty 0.48.2 `kitten icat` has client-specific multiplexer limits. Under tmux it forces Unicode-placeholder output. tmux redraws pane text but only forwards KGP payloads to an attached client; it neither retains nor replays image data. Alacritty retains virtual prototypes through pane-margin scrolling, so switching panes and windows in one attached terminal preserves resolvable images. tmux's initial incremental output can expose placeholders until a full `refresh-client`, and a fresh terminal attachment cannot reconstruct images without client retransmission. Kitty 0.48.2 itself shows fragmented placeholders under the same isolated pane/window sequence. Under Zellij, `kitten icat` can read zero `TIOCGWINSZ` pixel fields before Zellij's asynchronous host geometry arrives; an immediate retry then succeeds. Raw protocol fixtures remain the deterministic multiplexer acceptance boundary.

## Written-spec ambiguities and accepted extensions

- The remote-client section requires 4 KiB chunks and narrowly constrained continuation controls. Alacritty also accepts bounded 128 KiB chunks, unpadded base64, omitted `a=f`, and repeated transmission actions because Kitty 0.48.2 emits these forms. This is an accepting-terminal extension, not a weaker resource bound.
- The frame-composition prose/example and control-reference table reverse the meanings of frame keys `r` and `c`. Alacritty follows the prose/example and Kitty runtime: `r` selects the source frame and `c` selects the destination frame. The prose sentence for pixel offsets conflicts with the worked example, control-reference table, and Kitty runtime; Alacritty follows the latter three, with `X/Y` as the source origin and `x/y` as the destination origin.
- The specification requires clients to keep placement `X/Y` below cell dimensions but does not define terminal failure behavior. Alacritty follows Kitty by clamping offsets to the cell.
- Kitty-compatible permissive values are accepting extensions: `f=0` means default RGBA, `q>2` clamps to complete suppression, and nonzero `U` requests a virtual placement. Other noncanonical flags use default behavior unless the defined value is present. In particular, only `C=1` selects no cursor movement or overwrite.
- Frame-level transient hints are accepted and ignored. The specification explicitly permits terminals to ignore usage hints.
- Failed replacement preserves prior image state atomically. This strengthens behavior where the written protocol specifies only successful replacement.

Real-application review uses `treemd` with transparent PNGs, Unicode placeholders, and modal GIF playback. Kitty serves as the comparison terminal with matched window, font, and color configuration where geometry matters. Treemd's software GIF path creates a new virtual image ID for every frame and does not delete stale image data; under sustained quota pressure this can evict IDs that Treemd later reuses through retained placeholders. Native terminal animation is therefore the acceptance boundary for Alacritty animation, while Treemd's unique-ID software playback remains client-owned behavior.
