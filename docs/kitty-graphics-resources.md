# Kitty graphics resource bounds

`terminal.graphics.storage_limit` is a retained pixel quota per screen buffer. It is not a process RSS limit. The primary and alternate buffers have separate quotas; working buffers, render snapshots, metadata, and GPU resources also consume memory.

Let `Q` be the configured quota, `B = 128 KiB` the encoded block size, and `M = 1 MiB` the PNG overhead allowance. Checked dimension and length calculations prevent wraparound before allocation.

## Retained pixels

Each screen retains at most `Q` canonical RGBA bytes, including animation frames. `PixelBuffer` shares an immutable `Vec<u8>` owner through `Arc<Vec<u8>>`. Its completed allocation has capacity equal to length. Freezing transfers ownership rather than copying pixels into an `Arc<[u8]>` allocation.

Image replacement and transmit-and-place operate on a transaction. Eviction includes cascades through relative placements, and replacement credit is recomputed after those cascades. A failed transaction preserves the previous images and placements.

A renderer snapshot can keep an earlier set of pixels alive after terminal state replaces or evicts it. This ownership is outside retained-state accounting. One window builds and consumes one snapshot at a time; its distinct image pixels are bounded by the active screen's quota at capture time.

## Encoded input

- Each APC accepts at most 4 KiB of controls and 128 KiB of payload.
- A chunked direct transmission accepts at most `floor(4 * Q / 3) + 4` encoded bytes.
- Empty chunks do not create stored blocks. Small chunks coalesce into blocks of size `B`.
- The final partial block can reserve up to `B` bytes. Vector header storage scales with the number of blocks, not the number of incoming APCs.
- The decoder consumes blocks without concatenating a second complete encoded body. The current APC payload can coexist with the stored blocks while it is appended.

## Processing stages

These allocations are temporary and execute outside the terminal mutex. Limits apply to each stage; the complete pipeline does not promise one aggregate working buffer.

| Stage | Owned data and allocation bound |
| --- | --- |
| Direct base64 decoding | One output with an explicit capacity ceiling, bounded by the smaller of the encoded input's decoded upper bound plus one byte and the decode limit plus one byte. Capacity grows as encoded blocks are consumed; the entire destination is not reserved alongside the entire encoded input. The extra byte detects excess data. Raw data uses its expected source size, subject to quota; compressed/PNG input uses at most `Q + M`. |
| Local transport | One source buffer of at most `Q` bytes, read through a validated regular file handle or POSIX shared-memory handle. Encoded local names are bounded by the APC payload limit. |
| Zlib expansion | Compressed source and expanded output can coexist. Raw expansion is bounded by the declared dimensions and `Q`; PNG expansion requires `S <= Q + M`. The locked miniz implementation can reserve up to twice the output limit while growing its vector; spare capacity is removed before the next stage. |
| RGB normalization | Source RGB and destination RGBA can coexist. The destination is at most `Q` bytes. Ownership conversion adds no complete RGBA copy. |
| PNG decoding | Source bytes, decoder state, a transformed output of at most `Q`, and, for non-RGBA output, a canonical RGBA destination of at most `Q` can coexist. The decoder receives its separate `Q + M` reserve allowance. Text chunks are ignored. RGBA output reuses the transformed output directly. |
| Frame composition | A transmitted source of at most `Q` and one destination of at most `Q` can coexist. A blank destination reuses its unique allocation. Editing a shared retained frame copies that destination once. Freezing the result adds no complete pixel copy. Composing between retained frames borrows the source through its existing owner. |

Allocator metadata, fragmentation, and temporary memory inside allocator reallocation are outside these byte counters. The PNG and zlib reserve policies belong to the locked dependency implementations and require reinspection when those dependencies change.

A full size edit can require two working pixel buffers in addition to the old retained frame. Preserving that operation is an explicit compatibility decision. Restricting all working buffers together to `Q` would reject some valid edits or require a different streaming composition design.

## Metadata and GPU resources

Each screen bounds images at 4,096, placements at 65,536, extra frames per image at 4,096, and extra frames across the screen at 65,536. Transactions can temporarily clone this metadata, while sharing pixel owners. Metadata limits remain independent of pixel size.

The renderer bounds its accounted texture cache by `Q`. Texture tiling respects the hardware limit, capped at 8,192 pixels per side, with overlap for filtering. Images that exceed the cache budget use temporary tiles. A temporary upload buffer and texture can coexist with the cache. Driver allocation and deferred resource release are outside cache accounting.

## Verification

Unit tests check allocation ownership transfer, capacity normalization, reuse of a unique canvas, preservation of a shared canvas, and encoded block counts under empty and small continuation streams.
