# Kitty graphics conformance requirements

This matrix defines the protocol-observable tests required for complete Kitty graphics protocol support. A requirement is complete only when a test exercises the responsible public or wire boundary. Implementation inspection alone does not count as coverage.

## Oracles

- Kitty protocol and `kitty_tests/graphics.py` at `kovidgoyal/kitty@77630f3a6748cdf0e3675cc6b768d5dd018a5052`.
- Ghostty `src/terminal/kitty/graphics_*.zig` at `ghostty-org/ghostty@9f0e1719dc918368367d368bfe300f59bb68b5a4`.
- Runtime comparisons use Kitty 0.48.2.

The written protocol defines required behavior. Kitty resolves ambiguities in its documentation, and Ghostty provides an independent compatibility check. Tests derived from external scenarios use independent fixtures and native Alacritty test APIs. Do not copy Kitty's GPL test source into this repository.

Status meanings:

- `Covered`: a direct test exercises the stated boundary.
- `Partial`: some variants or only a lower-level boundary are tested.
- `Required`: no direct test establishes the requirement.
- `Implementation required`: inspection shows that behavior is absent.
- `Policy`: an intentional Alacritty behavior outside Kitty's required semantics.

## Stream, parser, and responses

| Requirement | Status |
| --- | --- |
| APC starts, terminates with 7-bit and 8-bit ST, cancels with CAN/SUB, preserves the following input suffix, and does not change PM/SOS behavior | Covered |
| APC control and payload storage are independently bounded across arbitrary input splits | Covered |
| Unknown keys are ignored, the final duplicate key wins, integer domains are enforced, and all action-specific permissive flag values match Kitty | Covered |
| Default `a=t`, `f=32`, `t=d`, `q=0`, `m=0`, and display/frame defaults produce the documented semantics | Required |
| A query response is ordered before a later DA response | Covered |
| `a=q` validates input without storing or replacing an image | Required |
| Commands without an image ID or number do not emit unsolicited responses | Required |
| `q=1` suppresses success and `q=2` suppresses success and failure, including chunk continuations | Required |
| Success and failure responses preserve initial image, image-number, placement, and frame identity | Partial |
| Conflicting `i` and `I` produce `EINVAL` unless quiet mode suppresses it | Partial |
| Placement and relative-placement failures emit `ENOENT`, `ENOPARENT`, `ETOODEEP`, and `ECYCLE` on the wire | Required |

## Pixel formats and compression

| Requirement | Status |
| --- | --- |
| RGB and RGBA require nonzero dimensions, accept exact data, reject short/excess data, and normalize to straight RGBA | Partial |
| PNG supports valid grayscale, grayscale-alpha, indexed, RGB, and RGBA color types at every valid bit depth | Partial |
| PNG supports interlaced palette and non-palette images and transparency metadata | Partial |
| Zlib works for RGB, RGBA, and PNG and remains bounded under decompression bombs | Partial |
| Compressed PNG requires `S` and interprets it as the decompressed PNG byte size | Required |
| Decoder dimensions, arithmetic, intermediate allocations, and canonical output remain within quota | Covered |

## Transports and chunking

| Requirement | Status |
| --- | --- |
| Direct transmission supports complete and chunked RGB, RGBA, PNG, and zlib payloads | Partial |
| Non-final chunks have valid base64 boundaries and total assembled data remains bounded | Partial |
| Final-chunk cursor position anchors transmit-and-place | Required |
| Any delete command aborts an incomplete upload, and the next upload starts cleanly | Required |
| Chunk success, decode failure, continuation failure, and changing quiet levels preserve initial request identity | Required |
| Animation frame chunks accept Kitty-compatible `m`-only continuations and explicit `a=f` continuations | Required |
| File and shared-memory transports ignore the direct-only `m` flag | Partial |
| File and shared-memory `S`/`O` ranges accept exact ranges and reject short or overflowing ranges with request identity | Partial |
| Regular files and safe symlinks are read; symlink loops and every special-file class fail without blocking | Partial |
| Sensitive opened objects are rejected after path resolution | Covered on Linux |
| Temporary files are removed only when both the directory and marker constraints hold | Partial |
| POSIX shared memory is read, range-checked, unlinked, and closed | Partial |
| Windows named shared memory is read, range-checked, and closed without unlink semantics | Implementation required |
| Disabling local transmission cancels in-flight local work while direct transmission remains available | Partial |

## Identity, storage, and replacement

| Requirement | Status |
| --- | --- |
| Number-based transmission allocates the smallest free nonzero client ID and replies with both ID and number | Partial |
| Number lookup and deletion select the newest image with that number | Partial |
| Successful retransmission of an ID removes its old placements and does not display the replacement implicitly | Required |
| Failed or incomplete replacement preserves the old image and placements atomically | Covered, policy extension |
| Same nonzero `(i,p)` replaces a placement without duplication or flicker | Required |
| `p` is ignored for image ID zero; repeated zero placement IDs remain independent | Required |
| Image bytes, animation bytes, image count, placement count, frame count, and pending reservations are bounded | Partial |
| Eviction is deterministic and prioritizes transient, unplaced, and non-visible images as documented | Covered |
| Replacement remains possible at metadata and byte limits without temporary double accounting | Partial |
| Repeated upload/delete and replacement return all accounting and process memory to a stable baseline | Required |

## Classic placement and rendering

| Requirement | Status |
| --- | --- |
| Native, explicit, and one-axis-inferred placement spans preserve aspect ratio | Covered |
| `X`/`Y` offsets clamp to cell bounds and contribute correctly to native span and cursor movement | Required |
| Source `x,y,w,h` is intersected with the image before scaling | Required |
| Images truncate at terminal and scroll-region clip bounds without changing source geometry | Partial |
| Cursor movement follows documented classic placement rules and never applies to virtual or relative placements | Covered |
| `C=1` suppresses cursor movement; other Kitty-compatible values retain default movement | Partial |
| Equal-z images order by lower image ID, with deterministic creation order where the protocol is undefined | Required |
| Very-negative, negative, and nonnegative z strata compose below backgrounds, below text, and above text as specified | Partial |
| Straight-alpha CPU pixels upload as premultiplied textures and blend transparent edges without fringes | Covered |
| Overlapping translucent classic placements produce source-over framebuffer output | Required |
| Every image pass restores shared OpenGL bindings and state used by later text and rectangle passes | Partial |
| Oversized textures tile with overlap, cache accounting is bounded, LRU eviction is deterministic, and transient tiles do not enter the cache | Partial |
| Destroying every GPU texture or recreating the context reconstructs the same framebuffer from CPU state | Covered |

## Unicode placeholders

| Requirement | Status |
| --- | --- |
| The complete authoritative row/column diacritic table is generated and decoded | Covered |
| Indexed and RGB foreground colors encode image IDs, including the bounded high-byte diacritic | Covered |
| Indexed and RGB underline colors encode placement IDs, with zero or omitted IDs selecting any matching virtual placement | Required |
| All documented left-to-right inheritance conditions and their mismatch boundaries are applied | Covered |
| Virtual placement geometry fits and centers the image while preserving aspect ratio | Covered |
| Partial, sparse, scrolled, clipped, and horizontally shifted placeholder grids sample the correct source tiles | Required |
| Placeholder background color remains visible through transparent image pixels | Required |
| Successfully resolved cells suppress placeholder glyphs and decorations; unresolved cells remain visible | Covered |
| Location-based deletion never affects virtual placements; only `i/I`, `n/N`, and `r/R` do | Partial |

## Relative placements

| Requirement | Status |
| --- | --- |
| Explicit and fallback parents resolve as Kitty specifies, including placement ID zero for virtual parents | Covered |
| Cell offsets accumulate through relative chains and relative placement never moves the cursor | Covered |
| Missing parents, cycles, and chains deeper than eight fail atomically with required wire errors | Partial |
| A virtual placement cannot itself be relative and returns `EINVAL` | Required |
| A virtual parent's origin uses independent minimum X and minimum Y across all matching placeholder cells | Required |
| Parent move, replacement, deletion, pruning, and eviction cascade through every descendant and reclaim orphan images | Covered |

## Deletion

| Requirement | Status |
| --- | --- |
| Every lowercase selector removes matching placements while retaining reusable image data | Partial |
| Every uppercase selector removes matching placements and frees only unreferenced image data | Partial |
| `i/I` and `n/N` honor an optional placement ID | Partial |
| `a/A`, `c/C`, `p/P`, `q/Q`, `x/X`, `y/Y`, and `z/Z` use visible/intersection and one-based coordinate semantics | Partial |
| `r/R` handles inclusive, omitted, reversed, and empty ranges | Partial |
| `f/F` handles root promotion, current-frame deletion, missing frames, and non-animated images | Covered |
| Every delete selector aborts incomplete direct uploads | Required |

## Animation

| Requirement | Status |
| --- | --- |
| Frame create and edit support transparent/default and explicit `Y` canvases, prior-frame `c` canvases, offsets, overwrite, and alpha blend | Partial |
| New non-root frames default to 40 ms; zero gap preserves the prior/default gap | Partial |
| Negative gaps create gapless frames that playback skips without displaying | Implementation required |
| Frame responses report the created or edited frame number | Covered |
| Client-selected current frames update rendered content | Covered |
| Stop, loading, and running states follow the documented transition rules | Partial |
| Loading mode parks on the final frame and resumes when a frame arrives | Required |
| Infinite and finite loop budgets have exact semantics, and stop resets the completed-loop counter | Implementation required |
| Animation deadlines wake the application scheduler without busy polling | Partial |
| Frame composition validates source/destination frames, bounds, same-frame overlap, alpha/overwrite semantics, and quota failure | Partial |
| Frame deletion promotes roots, adjusts the current frame, updates rendering, and frees accounting | Covered |
| Retransmitting the base image removes all animation state | Required |
| A runtime framebuffer test proves automatic multi-frame playback and gapless skipping | Required |

## Terminal interaction and lifecycle

| Requirement | Status |
| --- | --- |
| Full-screen scrolling moves classic placements with text and prunes them with history | Covered |
| Forward and reverse scrolling inside vertical and horizontal margins moves only fully contained placements and clips them at the page area | Partial |
| Primary-buffer width reflow tracks classic and deferred anchors without corrupting source pixels | Covered |
| RIS clears visible graphics according to Alacritty reset semantics | Required |
| Entering a fresh 1049 alternate screen clears its graphics independently of the primary screen | Covered |
| `ED 2` clears visible graphics, while EL, ECH, DCH, and other text erasure leave classic graphics unchanged | Partial |
| Erasing placeholder cells removes only their cell-derived image instances | Required |
| Window pixel and cell query mechanisms used by graphics clients report coherent nonzero geometry | Required |

## Robustness and performance

| Requirement | Status |
| --- | --- |
| Stateful fuzzing covers parser splits, chunk transactions, decoders, state operations, grid movement, reset, and cancellation | Partial |
| Random state tests assert image/placement/index/quota/graph invariants after every operation | Covered |
| Framebuffer smoke covers all z strata, crop/scale/offset, alpha overlap, placeholders, animation, texture eviction, and context reconstruction | Partial |
| Ordinary non-graphics workloads show no material parser, memory, or redraw regression with always-on support | Partial |

## Runtime fixtures

`scripts/kitty-graphics-smoke.sh` currently checks these composed framebuffer scenarios under headless Sway:

- text remains stable across repeated graphics redraws;
- a negative-z image does not corrupt text renderer state;
- transparent Unicode placeholder tiles do not expose placeholder glyphs;
- image textures reconstruct from canonical CPU pixels;
- an opaque image reaches the expected framebuffer color.

The completion plan expands this harness to cover the remaining renderer and animation requirements above. Real-application review uses `treemd` with transparent PNGs and Unicode placeholders. Kitty serves as the comparison terminal with matched window, font, and color configuration where geometry matters.
