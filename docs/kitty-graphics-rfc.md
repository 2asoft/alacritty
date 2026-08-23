# RFC: Kitty Graphics Protocol Support in Alacritty

- **Feature Name:** `kitty_graphics`
- **Start Date:** 2026-08-22
- **Status:** Accepted private-fork design; implementation active
- **Base Revision:** `alacritty/alacritty@7dd7b5b`
- **Protocol Baseline:** Kitty terminal graphics protocol as published 2026-08-22
- **Scope:** Alacritty, `alacritty_terminal`, and `vte`
- **Submission Status:** Private fork only; not intended for upstream submission

This document follows the Rust RFC structure: motivation and user model first, implementation and corner cases second, then drawbacks, alternatives, prior art, unresolved questions, and future work. citeturn575486view0

The implementation target is current Alacritty `master` at commit `7dd7b5b`. citeturn575486view1

## Summary

Add complete native support for the Kitty terminal graphics protocol to Alacritty.

Support spans three existing architectural layers:

1. `vte` gains protocol-neutral streaming APC parsing.
2. `alacritty_terminal` gains Kitty command parsing, image/frame storage, placement state, image lifecycle, scrolling/reflow integration, Unicode placeholder interpretation, animation state, transport handling, quota enforcement, and protocol responses.
3. Alacritty's OpenGL renderer gains image texture caching and compositing at protocol-defined z layers.

Graphics become terminal state, not a renderer overlay. Image pixels remain canonically represented in CPU-side terminal state; GPU textures are disposable renderer caches. Placements are independent terminal objects except for Unicode-placeholder instances, which intentionally derive position and lifetime from ordinary terminal cells.

The final feature implements the current Kitty protocol surface, including:

- direct transfers;
- regular-file transfers;
- temporary-file transfers;
- shared-memory transfers;
- RGB;
- RGBA;
- PNG;
- zlib compression;
- chunked transfer;
- image IDs;
- image numbers;
- named and anonymous placements;
- source rectangles;
- cell-relative scaling;
- pixel offsets;
- z-index compositing;
- deletion selectors;
- Unicode placeholders;
- virtual placements;
- relative placements;
- image queries and responses;
- storage quotas and eviction;
- animation frame loading;
- animation control;
- frame composition;
- animation-frame deletion;
- screen/reset/scroll interactions.

Development may stage these capabilities, but partial protocol support is not the completed state described by this RFC.

The implementation does not change `$TERM`, impersonate another terminal, launch external renderers, or add unrelated graphics protocols.

The Kitty protocol itself remains authoritative for externally observable protocol behavior. This RFC specifies Alacritty architecture and resolves behavior the external protocol leaves implementation-defined, particularly storage, resize/reflow, resource management, execution ordering, and renderer organization. citeturn575486view6turn575486view7

---

# Motivation

Alacritty already provides almost every primitive needed for accelerated terminal graphics:

- a streaming VT parser;
- retained terminal/grid state;
- scrollback;
- resize/reflow;
- separate primary and alternate screens;
- an OpenGL renderer;
- damage tracking;
- a PTY thread;
- asynchronous window redraw;
- platform abstraction.

What it lacks is a protocol and state model connecting arbitrary pixel content to these facilities.

Treating graphics as merely "decode an image and draw a textured rectangle" is insufficient. Terminal graphics interact with:

- byte-stream parsing;
- command ordering;
- cursor position;
- scrolling;
- scroll regions;
- scrollback;
- history pruning;
- resize;
- line reflow;
- alternate screen transitions;
- resets;
- erase operations;
- text cells;
- z-order;
- alpha composition;
- image resource lifetime;
- protocol replies;
- memory quotas;
- animation timing;
- renderer context loss.

Consequently, correct graphics support must be designed as a terminal feature from parser through renderer.

## Current parser limitation

Kitty graphics commands are APC control strings beginning with:

```text
ESC _ G
```

and terminated by ST:

```text
ESC \
```

Alacritty's current `vte` parser recognizes APC syntactically, but APC, PM, and SOS share an ignored `SosPmApcString` state. Bytes inside such strings are discarded. The `Perform` interface exposes DCS, OSC, CSI, ESC, printing, and execution callbacks, but no APC callback. citeturn767575view1turn767575view0turn767575view2

Therefore Kitty graphics cannot currently be implemented exclusively inside `alacritty_terminal`; the parser interface must first expose APC data.

## Current execution model

Incoming PTY data is parsed while `Term` is locked. Current `event_loop.rs` reads into a 1 MiB buffer, acquires the terminal lock, and invokes `state.parser.advance(&mut **terminal, ...)`. Alacritty already limits continuous processing while locked, but image work can involve far larger computational costs than ordinary escape-sequence handling. citeturn767575view3turn767575view4

Performing PNG decoding, zlib decompression, filesystem reads, shared-memory mapping, or large allocations from inside that critical section would increase render latency and create an avoidable denial-of-service surface.

The graphics implementation therefore requires an ordered parser suspension mechanism: protocol parsing establishes a command boundary; heavyweight processing executes outside `Term`'s lock; the resulting transaction is then committed atomically before subsequent PTY bytes are interpreted.

This is also required for protocol response ordering. A graphics capability query followed immediately by another terminal query must produce replies in input order. citeturn575486view6

## Current grid limitation

A Kitty placement is attached to terminal content, not permanently to a framebuffer coordinate.

Alacritty's primary grid reflows on width changes. Growing columns can merge wrapped rows; shrinking columns can split rows. Current resize code explicitly performs these transformations. citeturn767575view5turn767575view6

A placement represented only as:

```rust
struct Placement {
    row: i32,
    column: usize,
}
```

will therefore become incorrect after reflow.

A simple immutable row ID is also insufficient because reflow can turn one row into several rows or several rows into one logical row.

Graphics anchors must participate in the same logical-cell transformation as terminal text.

## Rendering limitation

Alacritty currently collects renderable terminal state while holding the terminal lock, releases that lock, then performs GPU work. Its renderer has separate text and rectangle facilities and supports its existing OpenGL renderer paths. citeturn575486view3turn575486view4

Kitty requires image content in multiple compositing strata:

- some graphics behind all cell backgrounds;
- some behind glyphs but above non-default backgrounds;
- nonnegative z-index graphics above text.

Thus graphics cannot simply be drawn once before or once after existing cell rendering. Text background and glyph composition must become explicit layers.

## Why Kitty graphics

The Kitty graphics protocol is suitable for Alacritty because it separates:

- image transmission;
- image storage;
- placement;
- pixel source rectangles;
- destination geometry;
- cell-relative location;
- resource identity;
- protocol querying.

It additionally supplies modern composition mechanisms including alpha, z-index, reusable image IDs, Unicode placeholders, relative placements, and animation.

This maps naturally onto Alacritty's existing distinction between terminal state and renderer state while remaining fully terminal-native. citeturn575486view6

---

# Guide-level explanation

## User-visible model

Applications may send Kitty graphics APC sequences directly to Alacritty.

An application may:

1. transmit image pixels;
2. assign an image ID or image number;
3. create one or more placements;
4. move terminal content normally;
5. allow placements to enter scrollback with that content;
6. reuse stored image pixels for additional placements;
7. replace or delete images;
8. display images through Unicode placeholder cells;
9. animate stored images.

No external image viewer is involved.

## Configuration

This RFC adds:

```toml
[terminal.graphics]
storage_limit = 320000000
local_transmission = true
```

Kitty graphics support is always available. The configuration controls resource and local-transport policy, not protocol availability.

### `storage_limit`

Integer number of decoded CPU-side image bytes permitted per terminal screen buffer.

Default:

```text
320,000,000 bytes
```

This is intentionally comparable to the storage budget described by the Kitty protocol documentation. citeturn575486view6

This budget counts canonical image/frame pixel storage and pending reservations. GPU texture memory is separately managed as a renderer cache.

### `local_transmission`

Boolean.

Default:

```toml
local_transmission = true
```

When false, transfers requiring Alacritty to open local filesystem or shared-memory objects are rejected. Direct transfers continue to function.

This option exists because local-object transports extend the trust boundary beyond bytes already delivered through the PTY.

Alacritty's current `[terminal]` configuration already groups terminal-level behavioral settings, making a nested graphics configuration consistent with existing organization. citeturn575486view5

## Capability detection

No terminal identity changes are made.

In particular:

- `$TERM` is unchanged;
- no terminal name belonging to another emulator is claimed;
- no environment variable is required for Kitty graphics discovery.

Applications discover support using the Kitty graphics query mechanism.

## Transmission

Applications may transmit image data using protocol-defined direct, file, temporary-file, or shared-memory methods.

Direct transmissions pass encoded data through the PTY.

Local-object transmissions contain references to objects accessible from Alacritty's host process.

Supported pixel encodings are:

- 24-bit RGB;
- 32-bit RGBA;
- PNG.

Supported compression:

- protocol-defined zlib compression.

Successful image data is normalized into an internal canonical RGBA representation independent of source format.

## Placements

An image and a placement are different objects.

For example, an application may upload one icon once and display it at several places:

```text
Image
 ├─ Placement A
 ├─ Placement B
 └─ Placement C
```

Deleting one placement does not necessarily delete the image.

Likewise, uploading an image does not necessarily make it visible.

## Scrolling

Ordinary placements are anchored to terminal content.

When their anchor line scrolls upward into retained history, the placement follows it.

When history containing the anchor is permanently pruned, the placement is removed.

Merely changing the user's viewport does not mutate placement state. It changes which retained placements are visible.

## Resize

For the primary buffer, an ordinary placement follows its logical anchor through Alacritty's text reflow.

For example, if text before a placement changes from:

```text
aaaaaaaaaaaaaaaa[image]
```

to:

```text
aaaaaaaa
aaaaaaaa
[image]
```

because the terminal becomes narrower, the placement follows the logical position where it was originally attached.

This behavior is an Alacritty definition because terminal resize/reflow behavior is not fully prescribed by the graphics protocol.

The alternate buffer follows Alacritty's existing non-reflow behavior.

## Unicode placeholders

Kitty's Unicode placeholder mechanism uses ordinary terminal cells containing U+10EEEE plus cell attributes and combining characters.

These cells remain ordinary Alacritty cells.

Therefore:

- normal text movement moves placeholder images;
- text erasure erases them;
- scrollback stores them;
- text reflow reflows them;
- copying or inspecting cells still encounters ordinary Unicode data.

No special opaque "image cell" type is introduced.

Alacritty's current cell representation already stores the character, foreground color, background color, optional underline color, and zero-width characters required to interpret the placeholder encoding. citeturn767575view7turn767575view8

## Layering

Conceptually, a rendered frame becomes:

```text
window clear
↓
very-negative-z graphics
↓
non-default cell backgrounds
↓
negative-z graphics
↓
glyphs and text decorations
↓
zero/positive-z graphics
↓
cursor
↓
Alacritty UI overlays
```

The exact threshold between the two negative graphics layers follows the Kitty specification.

## Failure behavior

Malformed, oversized, unsupported, or resource-exhausting graphics commands fail as protocol commands. They must not:

- panic;
- corrupt terminal state;
- partially replace an existing image;
- leave inconsistent placements;
- exhaust unbounded memory;
- block indefinitely on a local special file;
- cause integer overflow.

Where the protocol requests a response, Alacritty returns the protocol-defined success/error form, respecting quiet-mode settings.

---

# Reference-level explanation

## 1. Normative basis

Externally visible Kitty graphics behavior MUST follow the current Kitty terminal graphics protocol baseline referenced by this RFC. citeturn575486view6turn575486view7

Where the external protocol leaves implementation policy unspecified, this RFC is normative for the private Alacritty fork.

The following words describe implementation requirements:

- **MUST:** required for correctness;
- **MUST NOT:** prohibited;
- **SHOULD:** default unless a documented implementation constraint requires otherwise;
- **MAY:** optional implementation strategy compatible with externally visible requirements.

Unknown future protocol keys MUST be ignored when they are syntactically parseable and do not make an otherwise-known command ambiguous. This provides forward compatibility.

A future incompatible protocol change requires updating this protocol baseline and conformance tests.

---

## 2. Architectural decomposition

Graphics support is divided among existing components rather than centralized in a monolithic graphics subsystem.

```text
PTY bytes
    │
    ▼
vte::Parser
    │ APC stream
    ▼
ansi::Processor / Kitty APC parser
    │
    ├──────────── cheap command ────────────┐
    │                                       │
    └── deferred request                    │
            │                               │
            ▼                               │
       PTY event loop                       │
       outside Term lock                    │
            │                               │
       read/decode                          │
            │                               │
            └──────────────┐                │
                           ▼                ▼
                       Term / GraphicsState
                           │
                           ▼
                    RenderableGraphics
                           │
                     Term lock released
                           │
                           ▼
                       ImageRenderer
                           │
                           ▼
                          GPU
```

### Responsibility boundaries

`vte`:

- identifies APC string boundaries;
- streams APC bytes;
- handles generic VT cancellation and termination;
- knows nothing about Kitty graphics.

`alacritty_terminal::ansi`:

- identifies Kitty APCs;
- parses bounded control fields;
- maintains direct-transfer chunk state;
- converts completed commands into terminal operations or deferred requests.

`alacritty_terminal::graphics`:

- owns protocol semantics;
- owns canonical image/frame data;
- owns IDs and placements;
- owns animation state;
- owns quotas;
- owns screen interaction;
- emits lightweight render descriptions.

PTY event loop:

- preserves byte-stream ordering;
- executes potentially blocking or computationally expensive decode/transport work outside the terminal mutex;
- commits results in FIFO order.

Alacritty renderer:

- owns GPU resources;
- uploads image textures;
- applies crop/scaling;
- composites graphics at correct layers;
- treats GPU data as a cache of terminal-owned CPU data.

---

## 3. `vte`: streaming APC support

### Current state

Current `vte` routes APC, PM, and SOS into the ignored `SosPmApcString` parser state. `Perform` has no APC callback. citeturn767575view1turn767575view2

### Required change

APC MUST receive its own parser state.

PM and SOS remain ignored.

`Perform` gains protocol-neutral APC callbacks conceptually equivalent to:

```rust
pub trait Perform {
    // Existing callbacks...

    fn apc_start(&mut self) {}

    fn apc_put(&mut self, _byte: u8) {}

    fn apc_end(&mut self) {}

    fn apc_abort(&mut self) {}
}
```

Names may change during implementation, but semantics MUST remain streaming.

### Streaming requirement

`vte` MUST NOT collect an entire APC payload into a dynamically growing buffer.

Reasons:

- direct graphics payloads may be large across chunks;
- future APC consumers may also be large;
- generic parser memory should remain bounded independently of protocol semantics.

### Termination

ST terminates APC and invokes `apc_end`.

CAN and SUB cancel APC and invoke `apc_abort`.

An ESC beginning ST is parser syntax, not application data.

Malformed/incomplete APC input MUST leave parser memory bounded.

### Non-Kitty APC

All APC strings not beginning with the Kitty application discriminator `G` are ignored by `alacritty_terminal`.

`vte` itself remains application-neutral.

---

## 4. Kitty APC parser

Add:

```text
alacritty_terminal/src/graphics/
    animation.rs
    command.rs
    image.rs
    mod.rs
    parser.rs
    placeholder.rs
    placement.rs
    storage.rs
    transport.rs
```

Exact file boundaries MAY evolve.

### Parser states

Conceptually:

```rust
enum GraphicsApcParser {
    Prefix,
    Control,
    Payload,
    Overflow,
    Ignore,
}
```

After APC begins:

1. first application byte must be `G`;
2. bytes before `;` form control data;
3. bytes after `;` form payload;
4. ST commits parsing;
5. cancellation discards current command.

### Control representation

Control data is a comma-separated sequence of single-letter key/value pairs.

Each recognized key is parsed into checked integer or enum storage.

Protocol integers MUST NOT be parsed using unchecked casts.

Unsigned fields use checked `u32` conversion unless a narrower type is semantically required.

Signed z-index and signed offsets use checked signed conversion.

Malformed fields return `EINVAL` when a reply is applicable.

### Duplicate keys

The external specification does not provide useful semantics for arbitrary duplicate control keys.

This implementation defines:

> For duplicate syntactically valid recognized keys, the final occurrence wins.

This is deterministic, cheap to implement, and consistent with conventional property-list parsing.

### Unknown keys

Unknown syntactically valid keys are ignored.

Malformed key/value syntax is rejected.

### Control-size limit

Control data is bounded independently of image payload.

Private-fork constant:

```rust
const MAX_GRAPHICS_CONTROL_BYTES: usize = 4096;
```

Exceeding the bound places the APC parser in `Overflow`; it consumes until termination without allocating further data and then returns an error if responses are enabled.

### Payload-size limits

For direct protocol chunks, the encoded payload MUST obey the protocol chunk-size requirement.

The implementation MUST enforce this before append/copy operations.

This is important because a 2026 Kitty security advisory involved insufficient capacity handling while appending direct graphics data, demonstrating that graphics input must be treated as hostile even when chunking appears bounded. citeturn575486view8

---

## 5. Ordered command execution

### Requirement

Terminal escape sequences form an ordered byte stream.

Graphics work MUST preserve this order even when processing leaves the terminal lock.

The implementation MUST NOT submit image decode jobs to an unordered worker pool whose completion order determines terminal state.

### Parser barrier

`ansi::Processor::advance` gains an execution mode capable of returning before all supplied PTY bytes are consumed.

Conceptually:

```rust
enum AdvanceResult {
    Complete {
        consumed: usize,
    },
    Deferred {
        consumed: usize,
        request: DeferredGraphicsRequest,
    },
}
```

Exact API shape is implementation-defined.

A deferred graphics request is a barrier:

```text
bytes before command
    → processed

graphics command
    → decoded/completed

protocol response/state commit
    → completed

bytes after command
    → only then processed
```

### Why the event loop changes

Current `event_loop.rs` hands its whole unprocessed buffer to `state.parser.advance()` while holding the terminal lock and then marks the whole range processed. citeturn767575view3

The implementation MUST instead retain the unconsumed suffix when a graphics barrier is reached.

### Heavy operations

The following MUST execute without holding `Term`'s mutex:

- filesystem reads;
- shared-memory reads;
- zlib decompression of significant payloads;
- PNG decoding;
- large image canonicalization;
- large allocation/copy work.

Cheap operations MAY remain under the terminal lock:

- placement deletion;
- image lookup;
- animation state changes;
- small command validation;
- response formatting;
- metadata-only placement creation.

### FIFO implementation

Initial implementation SHOULD perform deferred work synchronously on the existing PTY reader thread after dropping `Term`'s lock.

Advantages:

- natural protocol order;
- no completion reorder buffer;
- no additional worker lifecycle;
- no concurrent mutation of graphics state;
- easier error propagation.

An ordered worker pipeline is future optimization.

### Deferred transaction

A heavyweight command is represented by an immutable request containing everything needed to perform transport/decode without consulting mutable terminal state.

Processing produces:

```rust
enum DecodedGraphicsResult {
    Image(DecodedImage),
    Frame(DecodedFrame),
    QueryResult(...),
    Error(GraphicsError),
}
```

Commit occurs under terminal lock.

The commit MUST either:

- apply the complete transaction; or
- leave prior graphics state unchanged.

No partially decoded image becomes visible.

### Cursor-dependent placement

For transmit-and-place commands, placement position is determined at the protocol-defined command completion point.

Before dropping the lock, Alacritty therefore creates a temporary tracked anchor associated with the deferred transaction.

If terminal geometry changes before decode finishes, that anchor undergoes the same grid transformations as an ordinary placement.

On failure, the pending anchor is discarded without cursor movement or placement creation.

---

## 6. Graphics state model

Each terminal screen buffer owns a distinct `GraphicsState`.

Conceptually:

```rust
pub struct GraphicsState {
    images: HashMap<ImageHandle, Image>,
    image_ids: HashMap<NonZeroU32, ImageHandle>,
    image_numbers: HashMap<u32, ImageNumberState>,

    placements: HashMap<PlacementHandle, Placement>,
    named_placements:
        HashMap<(NonZeroU32, NonZeroU32), PlacementHandle>,

    pending: Option<PendingTransmission>,

    used_bytes: usize,
    reserved_bytes: usize,
    storage_limit: usize,

    generation: u64,
    next_image_handle: u64,
    next_placement_handle: u64,
    creation_serial: u64,
}
```

The exact collections are implementation choices.

### Internal handles

Protocol IDs are insufficient as internal identities.

The protocol allows:

- anonymous images;
- anonymous placements;
- multiple placements with placement ID zero;
- image replacement using an existing external ID.

Therefore terminal state uses private monotonic handles:

```rust
struct ImageHandle(u64);
struct PlacementHandle(u64);
```

Protocol IDs are secondary indexes.

Internal handles MUST NOT be reused during a terminal session unless wraparound is safely handled.

---

## 7. Image model

Conceptually:

```rust
struct Image {
    handle: ImageHandle,

    external_id: Option<NonZeroU32>,
    image_number: Option<u32>,

    width: u32,
    height: u32,

    frames: Vec<Frame>,
    animation: AnimationState,

    usage: UsageHints,

    content_generation: u64,
    last_used_serial: u64,
}
```

### Canonical format

Successful source images are canonicalized to RGBA8 sRGB pixels.

Conceptually:

```rust
struct PixelBuffer {
    width: u32,
    height: u32,
    bytes: Arc<[u8]>,
}
```

The implementation MAY use specialized storage internally, but renderer-visible semantics MUST be equivalent.

Advantages of a canonical representation:

- one texture-upload path;
- one alpha model;
- predictable quota calculation;
- straightforward animation composition;
- no renderer dependency on PNG/zlib;
- context-loss recovery;
- easy testing.

### Generations

`content_generation` increments whenever pixel/frame content changes.

Placement-only changes MUST NOT increment image content generation.

Renderer caches use generation to detect stale textures without hashing image memory.

---

## 8. Supported source formats

### RGB

24-bit RGB input MUST include valid width and height.

Decoded byte count MUST equal:

```text
width × height × 3
```

using checked arithmetic.

Canonicalization appends alpha `255`.

### RGBA

32-bit RGBA input MUST include valid width and height.

Decoded byte count MUST equal:

```text
width × height × 4
```

using checked arithmetic.

### PNG

PNG dimensions come from the image stream.

PNG decoding SHOULD support valid ordinary PNG forms, including:

- grayscale;
- grayscale + alpha;
- RGB;
- RGBA;
- indexed palette;
- transparency metadata;
- interlacing;
- common legal bit depths.

Output is normalized to RGBA8.

A narrow "only RGBA PNG" decoder is not sufficient for protocol-complete support.

### Dependency policy

Use a memory-safe Rust PNG implementation.

Do not introduce:

- general-purpose image-framework dependency solely for PNG;
- subprocess-based conversion;
- external decoder binaries;
- native C image libraries without a compelling later reason.

---

## 9. Compression

Protocol zlib compression is supported.

Decompression MUST be bounded.

### Raw RGB/RGBA

Expected decompressed size is known before allocation:

```text
width × height × channels
```

Anything larger or smaller than the exact expected decoded size is rejected.

### PNG plus compression

The protocol-supplied uncompressed size is used as an upper bound where required by the protocol.

Allocation and decompression limits MUST be validated before expansion.

### Required safety properties

All size arithmetic uses checked multiplication/addition.

No path may:

```rust
Vec::with_capacity(untrusted_size)
```

before validating the requested size against:

- integer range;
- configured quota;
- implementation object limits.

---

## 10. Transfer methods

The implementation supports all current transfer methods specified by Kitty. citeturn575486view6

### Direct

Data is base64-encoded inside APC.

Chunking semantics follow the protocol.

The first chunk carries transmission metadata.

Continuation chunks cannot arbitrarily redefine the transfer.

Image state is not committed until the final chunk has been received and validated.

### File

The payload identifies a host-local filesystem path.

The file MUST be opened read-only.

After opening, Alacritty MUST verify that the opened object is a regular file.

Do not rely solely on path metadata checked before opening; validate the opened handle to avoid time-of-check/time-of-use races.

FIFOs, sockets, block devices, and character devices MUST be rejected.

### Temporary file

Temporary-file transport follows regular-file validation.

After safe opening/reading, Alacritty deletes the path only when all protocol safety conditions for temporary-file deletion are met, including its expected temporary-directory location and protocol-specific path marker.

Failure to remove the temporary file MAY be logged but MUST NOT transform valid decoded image data into a corrupt terminal transaction.

### Shared memory

On POSIX systems:

1. open shared-memory object read-only;
2. validate requested range;
3. read or map it;
4. unlink according to protocol lifetime semantics;
5. close mapping/descriptor.

Equivalent platform-native behavior is used where shared-memory APIs differ.

### Offset and size

Transfer offset/size calculations MUST use checked arithmetic.

No transport uses unconstrained `read_to_end`.

### Path encoding

On Unix, path decoding SHOULD preserve non-UTF-8 native bytes.

Do not require host filesystem paths to be valid UTF-8 merely because Rust strings are convenient.

Platform-specific path conversion belongs in `transport.rs`.

---

## 11. Local-transfer security

PTY output is untrusted input.

A process capable of writing arbitrary bytes to a terminal may attempt to:

- make Alacritty open files;
- request very large files;
- request special files;
- decompress hostile content;
- trigger decoder bugs;
- allocate excessive memory;
- produce huge placement sets.

`local_transmission` provides an explicit kill switch for transports that make Alacritty open host-local objects.

### Sensitive pseudo-filesystems

File transfer SHOULD reject known pseudo-filesystem objects whose semantics are unlike ordinary user files, including sensitive portions of:

```text
/proc
/sys
/dev
```

on applicable systems.

The definitive security check remains the opened object's file type plus bounded reads.

### Logging

Image payload bytes MUST NOT appear in ordinary logs.

Full local file names SHOULD NOT be included at normal log levels.

Error logs should report protocol operation and error class rather than copying arbitrary untrusted binary/control data.

---

## 12. Chunk state

Only one direct chunked graphics transmission may be incomplete at a time where required by protocol ordering.

Conceptually:

```rust
struct PendingTransmission {
    command: TransmissionHeader,
    encoded: Vec<u8>,
    reserved_bytes: usize,
    pending_anchor: Option<TrackedAnchor>,
}
```

Actual implementation SHOULD decode base64 incrementally rather than retaining a second complete encoded copy when possible.

### Interleaving

A new incompatible graphics operation encountered while a chunked transfer is incomplete is handled according to protocol requirements rather than silently merging state.

Deletion that aborts an incomplete transfer clears pending storage and reservation.

### Failure

A failed chunked transfer leaves no partial image.

If it was intended to replace an existing image, the previous image remains valid and visible.

---

## 13. Image IDs and image numbers

Protocol image IDs and image numbers are distinct namespaces.

### Image ID

A nonzero external image ID indexes one current image.

Successful retransmission with the same ID performs atomic replacement.

Replacement behavior:

1. decode new image completely;
2. reserve required memory;
3. validate complete operation;
4. atomically remove old image/affected placements according to protocol;
5. install new image;
6. create requested new placement, if any.

If any precommit step fails, old state remains unchanged.

### Anonymous images

Protocol image ID zero does not become an internal map key shared by all anonymous images.

Each anonymous image receives its own private `ImageHandle`.

### Image number

Image numbers permit terminal-assigned image IDs and lookup of the newest matching image.

The data model MUST preserve enough ordering information to implement "newest matching image number" deterministically.

Supplying mutually exclusive ID forms is rejected according to protocol.

---

## 14. Placement model

Conceptually:

```rust
struct Placement {
    handle: PlacementHandle,

    image: ImageHandle,
    external_placement_id: Option<NonZeroU32>,

    location: PlacementLocation,

    source: PixelRect,

    pixel_offset: PixelOffset,

    columns: u32,
    rows: u32,

    z_index: i32,

    creation_serial: u64,
}
```

### Location

```rust
enum PlacementLocation {
    Direct {
        anchor: TrackedAnchor,
        scroll_clip: Option<GridClip>,
    },

    Virtual,

    Relative {
        parent: PlacementKey,
        horizontal_cells: i32,
        vertical_cells: i32,
    },
}
```

Ordinary direct placements are independent terminal state.

Virtual placements are prototypes for Unicode placeholder rendering.

Relative placements resolve through another placement.

### Why placements are not stored inside cells

Classic placement semantics differ from text-cell lifetime:

- ordinary text erase operations do not generally delete classic placements;
- several placements may overlap the same cell;
- z-order is independent;
- source rectangles and destination geometry are independent;
- placement and image resource lifetimes differ.

Therefore adding an `ImagePlacement` field to every `Cell` would encode the wrong ownership model.

A maintained implementation based on cell-associated placement metadata documents limitations around overlapping/classic placement semantics; this reinforces keeping ordinary placements independent from cells. citeturn575486view12

---

## 15. Named placements and replacement

A nonzero image ID plus nonzero placement ID defines a named placement.

An internal index:

```rust
HashMap<(NonZeroU32, NonZeroU32), PlacementHandle>
```

provides replacement lookup.

Replacing a named placement MUST be atomic from renderer perspective.

Placement ID zero does not impose uniqueness.

An image may therefore have several anonymous placements.

---

## 16. Source and destination geometry

Placement source geometry includes protocol-defined source offset and crop dimensions.

The renderer resolves a placement to:

```rust
struct ResolvedPlacement {
    source_px: Rect<u32>,
    destination_px: Rect<f32>,
    clip_px: Rect<f32>,
    z_index: i32,
    image: ImageHandle,
    frame: usize,
}
```

### Cell dimensions

Placements sized in rows/columns resolve destination dimensions from current cell size at render time.

Thus changing font size or DPI rescales cell-sized placements.

### Native-pixel placements

When protocol geometry requests native pixel size rather than a cell span, destination pixel size follows source/crop pixel dimensions.

Its terminal anchor still follows terminal content.

### Pixel offsets

Pixel offsets are validated against cell dimensions when placement is created as required by protocol.

A later font-size change does not retroactively invalidate the placement.

It may instead alter clipping/visual position according to new cell geometry.

### Aspect-ratio inference

When the protocol supplies only one destination cell dimension, the other is derived while preserving source aspect ratio.

The implementation MUST use one deterministic integer rounding algorithm and cover it with compatibility tests.

The preferred policy is ceiling to the minimum cell count required to contain the aspect-preserving result.

If compatibility testing against the reference protocol implementation demonstrates a different established rounding rule, the compatibility rule takes precedence and this RFC should be amended locally.

---

## 17. Z-order and compositing

Placement z-index is signed 32-bit.

Renderer composition is divided into these logical passes:

```text
1. clear framebuffer to default background

2. graphics with:
       z < INT32_MIN / 2

3. non-default cell backgrounds

4. graphics with:
       INT32_MIN / 2 <= z < 0

5. glyphs and text decorations

6. graphics with:
       z >= 0

7. cursor

8. Alacritty-owned UI overlays
```

This reflects the protocol distinction between extremely-negative images, ordinary negative-z images, and nonnegative images. citeturn575486view6

### Equal z-index

Where protocol ordering is defined, use that ordering.

Where it is explicitly unspecified, Alacritty uses deterministic creation serial as a final private tiebreaker after protocol-defined keys.

Undefined protocol ordering MUST NOT depend on hash-map iteration order.

### Alpha

CPU canonical storage uses straight RGBA unless implementation benchmarks justify another canonical format.

GPU uploads SHOULD use or convert to a representation that avoids filtering halos at translucent edges; premultiplied-alpha textures are preferred when compatible with renderer blending.

---

## 18. Grid anchors

### Requirement

Every classic placement anchored to terminal content MUST survive:

- line scrolling;
- scrollback insertion;
- viewport changes;
- height changes;
- width reflow;
- wrapped-row split;
- wrapped-row merge.

It MUST be invalidated when the logical anchor leaves retained terminal storage.

### `TrackedAnchor`

Conceptually:

```rust
struct TrackedAnchor {
    point: Point,
    valid: bool,
}
```

The representation alone is not sufficient; correctness comes from participation in grid transformations.

### Reflow

Alacritty MUST expose a grid-resize mechanism capable of applying the same logical mapping to an arbitrary set of tracked points.

Possible API:

```rust
fn resize_with_tracking(
    &mut self,
    reflow: bool,
    lines: usize,
    columns: usize,
    tracked: &mut [TrackedPoint],
);
```

The concrete API is not prescribed.

### Mapping invariant

For any tracked anchor `A` attached to logical text cell `C` before resize:

- if `C` remains represented after resize, `A` maps to the new physical point representing `C`;
- if `C` is permanently discarded, `A` becomes invalid.

The implementation MUST account for:

- wrapped logical lines;
- wide-character cells;
- wide-character spacers;
- leading spacers;
- rows split by shrink;
- rows joined by grow;
- history truncation.

### Complexity

Resize tracking SHOULD be:

```text
O(grid cells + tracked anchors)
```

or similar.

It MUST NOT scan the entire set of placements separately for every cell or row.

A practical implementation can sort/group anchors by old line and transform them while existing reflow code produces new rows.

### Pending anchors

Deferred graphics transactions also participate in this mechanism.

Otherwise a resize occurring during image decoding could commit a placement to stale coordinates.

---

## 19. Scroll semantics

### Full-screen scrolling

When terminal output scrolls content through the normal full-width page region, direct placements entirely associated with that content scroll with it.

Placements entering retained history remain present.

### History pruning

When an anchor's retained row is permanently removed, its placement MUST be removed.

Image data is not automatically removed unless protocol lifetime or quota policy requires it.

This distinction prevents a placement-lifetime bug class observed in another terminal implementation, where anchors referring to pruned scrollback could survive and later resolve to incorrect screen positions. citeturn575486view10

### Viewport movement

User scrolling through history does not mutate placement anchors.

The renderer resolves anchors relative to current display offset.

### Partial scrolling regions

Kitty semantics for page-area scrolling are implemented explicitly:

- placements entirely contained in the affected scrolling area move with it;
- placements partially outside the page area do not undergo an invalid whole-object move;
- clipping state is retained when protocol semantics require content moved beyond the page region to be clipped.

This must be tested independently of full-screen scrollback behavior.

---

## 20. Resize semantics

The graphics protocol does not completely define how an emulator with reflowing scrollback should reinterpret classic placements during arbitrary terminal width changes.

This RFC defines private-fork behavior.

### Primary buffer

The top-left placement anchor follows the same logical-cell reflow as text.

Placement properties remain unchanged:

- image;
- source rectangle;
- row/column span;
- pixel offsets;
- z-index;
- placement ID;
- relative relationships.

Destination pixels are recalculated from new cell metrics.

### Alternate buffer

The alternate buffer follows Alacritty's existing resize policy rather than introducing graphics-specific text reflow.

### Invalid anchors

If resize or history-capacity adjustment removes an anchor permanently, the placement is deleted.

### No framebuffer-coordinate persistence

A placement MUST NOT remain at a fixed framebuffer `(x, y)` merely because a resize occurred.

Terminal content, not previous window pixels, is its source of position.

---

## 21. Primary and alternate graphics state

Graphics state mirrors Alacritty's two terminal-screen model.

Conceptually:

```rust
struct Term<T> {
    grid: Grid<Cell>,
    inactive_grid: Grid<Cell>,

    graphics: GraphicsState,
    inactive_graphics: GraphicsState,

    ...
}
```

Exact field placement MAY differ.

On screen swap, the corresponding graphics state swaps with the corresponding grid.

### Entering a fresh alternate screen

When the terminal mode requests a cleared alternate screen, alternate graphics are cleared along with alternate text according to terminal semantics.

Primary graphics remain retained.

### Leaving alternate screen

Primary text and primary graphics resume together.

### Hard reset

Hard terminal reset clears:

- both graphics states;
- pending graphics transmissions;
- pending tracked anchors;
- animation timers associated with those states.

---

## 22. Erase behavior

Classic graphics placements are not ordinary cells.

Therefore ordinary character/line erase operations MUST NOT generically delete them.

Protocol-specified screen-clear interactions are handled explicitly.

### Clear visible display

Operations whose terminal semantics require clearing visible graphics remove visible classic placements as required by the graphics protocol.

### Scrollback purge

When scrollback is purged, any classic placements whose anchors are contained only in discarded history are deleted.

### Placeholder cells

Unicode-placeholder images follow ordinary cell semantics and are therefore affected automatically by cell erasure.

This distinction is fundamental:

```text
classic placement
    lifetime = graphics state

Unicode placeholder instance
    lifetime = terminal cells
```

---

## 23. Unicode placeholder protocol

The Kitty Unicode placeholder code point is interpreted only when a corresponding virtual placement/image exists.

Placeholder cells remain structurally ordinary `Cell` values.

Alacritty's existing `Cell` has:

- `c`;
- `fg`;
- `bg`;
- optional underline color;
- zero-width characters.

Those fields are sufficient to decode the protocol representation without enlarging every terminal cell. citeturn767575view7turn767575view8

### Virtual placement

A protocol command may create a virtual placement carrying image/crop/grid geometry.

Virtual placements are not rendered directly.

They provide metadata for subsequent placeholder cells.

### Cell decoding

Placeholder interpretation uses the cell's raw terminal attributes, before UI theme resolution.

This is important because protocol IDs encoded through colors are numeric protocol values, not visual color values.

Do not turn a palette index into its resolved theme RGB value before interpreting an encoded ID.

### Combining characters

The official row/column combining-character lookup table MUST be included or generated from the authoritative protocol data.

Do not duplicate a hand-maintained subset.

### ID reconstruction

The implementation decodes:

- image ID;
- placement ID where present;
- image row;
- image column;
- high ID component where present;

according to protocol encoding and inheritance rules.

Inheritance MUST be evaluated left-to-right using the exact protocol conditions.

### Background

The cell's ordinary background remains a terminal background.

Transparent pixels in a placeholder-rendered tile expose that background according to normal layer composition.

### Missing prototype/image

A placeholder cell referencing no resolvable virtual placement/image renders as ordinary non-image terminal content according to placeholder rules; it MUST NOT panic or retain stale GPU references.

### Reflow

Because placeholders are cells, existing grid reflow moves their code point, colors, and combining characters together.

No separate placeholder anchor transform is necessary.

---

## 24. Relative placements

Relative placements reference another placement and apply signed cell offsets.

Conceptually:

```rust
struct RelativeLocation {
    parent_image_id: NonZeroU32,
    parent_placement_id: NonZeroU32,
    horizontal_cells: i32,
    vertical_cells: i32,
}
```

### Graph

Relative relationships form a directed placement graph.

The implementation maintains enough adjacency information to support:

- parent lookup;
- cycle checks;
- depth checks;
- recursive deletion;
- efficient invalidation.

### Depth

At least eight nested relative relationships MUST be supported.

Private-fork maximum:

```rust
const MAX_RELATIVE_PLACEMENT_DEPTH: usize = 8;
```

A command exceeding this returns the protocol-defined depth error.

### Cycles

Any operation that would create a cycle is rejected atomically.

### Missing parent

Missing parent yields the protocol-defined parent error.

### Parent lifetime

Deleting a parent cascades according to protocol relative-placement semantics.

### Cursor

Relative placements do not move the cursor.

### Virtual parent

When a relative placement references a virtual placement, its resolved parent position depends on actual placeholder instances.

Correctness-first implementation MAY scan retained visible/relevant grid cells to determine the protocol-defined minimum coordinates.

A later index/cache MAY optimize this.

Because placeholders can be changed by ordinary terminal text operations, any cache MUST be invalidated by ordinary grid mutations and cannot become the source of truth.

---

## 25. Deletion

Deletion supports the full current selector set defined by the Kitty protocol.

Implementation SHOULD parse selectors into a typed enum:

```rust
enum DeleteSelector {
    Visible,
    ImageId { image: u32, placement: Option<u32> },
    ImageNumber { number: u32, placement: Option<u32> },
    CursorCell,
    AnimationFrames,
    Cell { x: u32, y: u32 },
    CellAndZ { x: u32, y: u32, z: i32 },
    ImageIdRange { first: u32, last: u32 },
    Column(u32),
    Row(u32),
    ZIndex(i32),
}
```

Exact representation may vary.

Upper/lower-case forms retain the protocol distinction between deleting placements and requesting image-data reclamation.

### Atomicity

Selectors are first resolved to internal handles, then mutations are applied.

Deleting while iterating hash maps MUST NOT produce order-dependent behavior.

### Pending transmission

Deletion semantics that abort an in-progress transmission clear:

- decoder/chunk state;
- reserved memory;
- pending tracked anchor.

---

## 26. Queries and responses

Queries MUST be processed in byte-stream order.

If input arrives as:

```text
graphics-query
terminal-query
```

Alacritty MUST enqueue the graphics response before processing the later terminal query sufficiently to generate its response.

This is one reason deferred graphics commands create parser barriers. citeturn575486view6

### Quiet modes

All responses honor protocol quiet settings:

- normal success/error replies;
- success suppression;
- complete suppression where requested.

### Response construction

Responses SHOULD be built into a bounded small buffer.

Arbitrary payload/path strings MUST NOT be echoed verbatim into response text.

### Write path

Graphics replies use the existing PTY output infrastructure rather than a second file descriptor or renderer-side channel.

---

## 27. Animation frame storage

Full animation support is part of completed protocol support.

### Correctness-first representation

Each materialized animation frame is stored as full canonical RGBA8.

This deliberately trades memory efficiency for:

- simple random access;
- deterministic frame composition;
- simple context recovery;
- straightforward texture generation;
- no dependency-chain corruption;
- easy quota accounting.

The configured storage quota bounds the cost.

A later implementation MAY store deltas internally while preserving these semantics.

### Frame model

Conceptually:

```rust
struct Frame {
    pixels: Arc<[u8]>,
    width: u32,
    height: u32,
    gap: FrameGap,
    generation: u64,
}
```

### Frame load

Frame loading supports:

- base-frame selection;
- destination offsets;
- editing/replacement;
- background initialization;
- blend vs overwrite semantics;
- frame delay;
- usage hints.

All geometry and byte sizes use checked arithmetic.

### Frame composition

Frame composition:

1. validates source/destination frames;
2. validates rectangles;
3. rejects invalid self-overlap where required;
4. reserves required storage;
5. materializes resulting destination pixels;
6. commits atomically.

Memory failure leaves original frame state unchanged.

---

## 28. Animation control

Animation state is terminal state:

```rust
struct AnimationState {
    mode: AnimationMode,
    current_frame: usize,

    loop_limit: LoopLimit,
    completed_loops: u32,

    next_deadline: Option<Instant>,
}
```

Modes include protocol-defined stopped/loading/running states.

### Scheduling

Animation does not create a busy polling loop.

The UI scheduler registers the next required animation deadline.

At a tick:

1. acquire terminal lock;
2. advance due animations;
3. update current frame;
4. mark affected rendering dirty;
5. schedule next deadline;
6. release lock.

Pixel decoding/composition is not performed during ordinary animation ticks.

### Offscreen images

Animation logical state MAY continue while placements are offscreen.

No redraw is necessary unless a changed animation is currently visible or soon becomes visible.

### Frame changes

Changing current frame does not mutate the underlying placement.

Renderer texture lookup changes from:

```text
(image, old_frame)
```

to:

```text
(image, new_frame)
```

---

## 29. Graphics resource quota

Graphics data is explicitly bounded.

Default per-screen CPU storage limit:

```text
320_000_000 bytes
```

### Counted memory

Quota accounting includes:

- canonical root image pixels;
- canonical animation frame pixels;
- pending decoded-pixel reservations;
- buffers equivalent to retained image content.

Temporary decode scratch SHOULD be minimized and separately bounded so peak memory cannot become an uncontrolled multiple of the configured quota.

### Not counted

GPU textures are renderer cache and not counted against terminal CPU quota.

They have their own renderer-side eviction policy.

### Reservation

Before decoding or allocating potentially large canonical content, the terminal reserves expected storage.

Conceptually:

```rust
fn reserve(&mut self, bytes: usize) -> Result<Reservation, GraphicsError>;
```

Dropping an uncommitted `Reservation` returns it automatically.

### Replacement

Replacing image ID `i` must preserve old content until new content validates.

Quota accounting MAY credit the old image's eventual reclaimed size when determining whether atomic replacement can succeed, but must retain enough actual memory to avoid unsafe overcommit.

If safe replacement cannot be performed, return `ENOSPC` and preserve old state.

### Metadata limits

Pixel quota alone does not prevent millions of zero-size metadata objects.

Private-fork defensive maxima:

```rust
const MAX_IMAGES_PER_BUFFER: usize = 4_096;
const MAX_PLACEMENTS_PER_BUFFER: usize = 65_536;
const MAX_RELATIVE_PLACEMENT_DEPTH: usize = 8;
```

These are implementation safety limits rather than protocol wire limits.

They may later become configurable if real workloads require it.

---

## 30. Eviction

When storage must be reclaimed, eviction is deterministic.

Preferred order:

1. unplaced images marked transient;
2. other unplaced images, least recently used;
3. non-visible referenced transient images;
4. non-visible referenced images;
5. visible images only as last resort.

When an image is evicted:

- its canonical pixel data disappears;
- its protocol placements are removed as required to preserve invariants;
- virtual prototypes referring to it cease rendering;
- ordinary placeholder text cells remain ordinary cells but no longer resolve to image pixels.

### LRU clock

Use an internal monotonic access serial rather than wall-clock time.

Eviction MUST NOT depend on hash iteration order.

---

## 31. Renderer architecture

Add a dedicated image renderer rather than adding images to the glyph atlas.

Conceptually:

```rust
pub struct Renderer {
    text_renderer: TextRenderer,
    rect_renderer: RectRenderer,
    image_renderer: ImageRenderer,
    ...
}
```

Alacritty's current renderer already separates renderer facilities and supports multiple OpenGL shader paths. citeturn575486view4

### GPU cache

Conceptual key:

```rust
struct TextureKey {
    image: ImageHandle,
    frame: u32,
    content_generation: u64,
    tile: u16,
}
```

Canonical terminal pixels are authoritative.

Textures can be discarded at any time.

### Context loss

On GL context loss/recreation:

```text
GPU image cache → drop all
terminal graphics state → unchanged
next render → lazily upload needed textures
```

No application retransmission is required.

### Texture-size limits

An image valid under terminal storage policy may exceed `GL_MAX_TEXTURE_SIZE`.

Alacritty MUST NOT reject it merely because one GL texture cannot hold it.

The renderer tiles such images into several textures while preserving:

- continuous source coordinates;
- filtering behavior at tile boundaries;
- alpha;
- crop geometry.

### Texture filtering

Scaled image rendering SHOULD use linear filtering unless compatibility or exact-pixel modes require a different protocol behavior.

Tile boundaries MUST NOT show seams.

### Texture cache eviction

Renderer-side texture memory is bounded independently.

Only textures can be evicted; canonical terminal images remain until terminal graphics policy evicts them.

---

## 32. Render snapshots and locking

GPU operations MUST occur without `Term`'s mutex.

Current Alacritty already builds renderable state under lock and performs renderer work afterwards. Graphics preserve this model. citeturn575486view3

Conceptually:

```rust
struct RenderableGraphics {
    placements: Vec<RenderablePlacement>,
    placeholder_tiles: Vec<RenderablePlaceholder>,
}
```

Each entry contains:

- image/frame handle;
- immutable pixel-data handle or cache key;
- generation;
- source rectangle;
- destination rectangle;
- clip;
- z-index/layer.

Large image bytes SHOULD be referenced with `Arc` rather than copied into the render snapshot.

When lock is released, the renderer may upload textures from immutable buffers safely.

---

## 33. Rendering placeholder cells

Visible terminal cells are scanned as they already are for text rendering.

When a cell contains the placeholder code point:

1. decode image/placement identity from raw cell attributes;
2. identify virtual placement;
3. decode row/column tile position from combining marks;
4. resolve corresponding source tile;
5. create a renderable placeholder image tile.

Placeholder rendering MUST occur in the protocol-defined layer for the associated virtual placement.

The renderer does not mutate the cell.

### No global placeholder ownership table

Terminal cells remain source of truth.

A derived short-lived per-frame map or cache is permitted.

---

## 34. Damage tracking

Initial implementation favors correctness.

### Full damage

Any graphics mutation affecting currently visible composition MAY mark full terminal damage.

Examples:

- placement creation/deletion;
- image replacement;
- animation frame change;
- z-index change;
- placeholder prototype replacement.

### No damage

Uploading image data with no visible placement need not trigger redraw.

### Future optimization

Once semantics are stable, dirty placement rectangles can be unioned into existing damage state.

This optimization is explicitly not required for initial correctness.

### Empty fast path

When active graphics state is empty and no placeholder prototypes exist, render cost SHOULD reduce to one cheap branch.

There MUST NOT be per-cell graphics allocation in the ordinary no-graphics case.

---

## 35. Renderer blending correctness

Graphics composition MUST be tested with:

- fully opaque pixels;
- fully transparent pixels;
- partial alpha;
- translucent edges;
- non-default backgrounds;
- inverse text;
- glyph antialiasing;
- overlapping graphics;
- all z layers.

Premultiplied alpha is preferred for GPU cached textures because linear interpolation of straight-alpha edge pixels can produce color fringes.

If used, conversion from canonical straight RGBA8 occurs at upload time or through an equivalent shader path.

---

## 36. Cursor composition

Cursor is rendered after Kitty image layers.

This ensures terminal cursor visibility remains under Alacritty's control even where graphics occupy the same cell.

Cursor rendering is not encoded as another arbitrary Kitty z-index object.

---

## 37. Screen clipping

Every resolved image rectangle is clipped against:

1. terminal content framebuffer bounds;
2. scrolling-region clip retained by placement semantics;
3. source crop;
4. any viewport/history clipping.

Renderer shader input must never rely on out-of-bounds texture coordinates being harmless.

Geometry is clipped on CPU where practical; shader sampling remains defensive.

---

## 38. Error model

Add:

```rust
enum GraphicsError {
    Invalid,
    TooLarge,
    NoSpace,
    NotFound,
    NoParent,
    Cycle,
    TooDeep,
    Io(io::ErrorKind),
    Decode,
    Unsupported,
}
```

Exact type names are not normative.

They map to protocol-visible errno-style responses where required.

### Internal errors

Rust panics are not protocol errors.

No malformed graphics command may intentionally panic.

### Error response bound

Human-readable error detail is bounded:

```rust
const MAX_GRAPHICS_ERROR_TEXT: usize = 256;
```

It should not contain large untrusted input fragments.

---

## 39. Reset and parser cancellation

Graphics state must define behavior under all terminal reset paths.

### Parser cancellation

CAN/SUB during APC:

- abort current APC;
- discard uncommitted control/payload state;
- do not mutate image state.

### Soft reset

Soft-reset behavior follows terminal/protocol requirements and MUST be covered explicitly by tests.

### Hard reset

Hard reset clears:

- images;
- placements;
- virtual placements;
- relative graph;
- pending transmissions;
- reservations;
- animation state;
- renderer-visible graphics generation.

GPU textures become unreachable and are lazily reclaimed.

---

## 40. Configuration reload

### `enabled: true → false`

Immediately:

- abort pending graphics transaction;
- clear both screen graphics states;
- invalidate render snapshots/cache generations;
- request full redraw.

### `false → true`

Starts with empty graphics state.

### `local_transmission: true → false`

Does not remove already-decoded images.

Future local-object transfers are rejected.

### Storage limit reduction

If new limit is below current usage, eviction runs synchronously until:

```text
used_bytes <= storage_limit
```

or graphics state is empty.

---

## 41. Dependencies

The implementation SHOULD remain predominantly Rust-native.

Expected additions:

- PNG decoder crate;
- zlib implementation through `flate2` with Rust backend or direct `miniz_oxide`;
- existing base64 functionality where already available.

No runtime dependency on another terminal emulator or terminal-state library is introduced.

No renderer subprocess is introduced.

No JavaScript runtime is introduced.

---

## 42. Protocol completeness

The completed feature MUST support all currently defined action families.

| Area | Required |
|---|---|
| APC `G` parsing | Yes |
| capability query | Yes |
| direct transfer | Yes |
| file transfer | Yes |
| temporary-file transfer | Yes |
| shared-memory transfer | Yes |
| RGB | Yes |
| RGBA | Yes |
| PNG | Yes |
| zlib | Yes |
| chunking | Yes |
| transmit | Yes |
| transmit + display | Yes |
| place existing image | Yes |
| source crop | Yes |
| cell span | Yes |
| native-pixel placement | Yes |
| pixel offsets | Yes |
| cursor movement/suppression | Yes |
| signed z-index | Yes |
| image IDs | Yes |
| image numbers | Yes |
| placement IDs | Yes |
| replacement | Yes |
| quiet modes | Yes |
| deletion selectors | Yes |
| usage hints | Yes |
| Unicode placeholders | Yes |
| virtual placements | Yes |
| relative placements | Yes |
| animation frame load | Yes |
| animation control | Yes |
| frame composition | Yes |
| frame deletion | Yes |
| storage eviction | Yes |
| scrollback interaction | Yes |
| alternate screen interaction | Yes |
| reset interaction | Yes |

A staged build missing one of these items may be useful but is not "protocol complete".

---

## 43. Core invariants

The implementation MUST continuously maintain these invariants.

### Image references

Every classic placement references an existing `ImageHandle`.

### Named placement uniqueness

At most one internal placement corresponds to a nonzero external `(image_id, placement_id)` pair.

### Quota

```text
used_bytes + reserved_bytes <= effective_limit
```

except transient bounded decoder scratch explicitly excluded by policy.

### Anchor validity

Every direct-placement tracked anchor is either:

- valid and maps to retained terminal content; or
- invalid and scheduled for placement removal.

No invalid anchor may silently map to `(0, 0)`.

### Relative graph

Relative-placement graph:

- contains no cycles;
- contains no chain deeper than configured/protocol limit;
- has no dangling parent reference.

### Replacement atomicity

At every externally observable point, either old image state or new image state exists; never a partially replaced hybrid.

### GPU independence

Dropping all GPU image resources does not alter terminal graphics semantics.

### Screen ownership

A graphics object belongs to exactly one terminal screen buffer.

### Parser bounds

Malformed/incomplete APC input cannot grow memory without bound.

---

## 44. Test strategy

Graphics support is not complete without a dedicated conformance suite.

### 44.1 `vte` parser tests

Test APC across every meaningful byte split:

```text
ESC
_
G
control
;
payload
ESC
\
```

Cases:

- single-byte input chunks;
- all bytes in one chunk;
- ST split across buffers;
- CAN cancellation;
- SUB cancellation;
- ESC embedded before termination;
- non-Kitty APC ignored;
- SOS remains ignored;
- PM remains ignored;
- OSC/DCS behavior unchanged.

### 44.2 Control parser tests

Cover:

- every known key;
- unknown key;
- duplicate key;
- empty value;
- negative unsigned value;
- signed overflow;
- unsigned overflow;
- missing `=`;
- extra separators;
- 4 KiB boundary;
- over-limit control data;
- invalid enum values.

### 44.3 Direct transfer tests

Cover:

- legal one-shot payload;
- maximum legal chunk;
- one byte over maximum;
- nonfinal chunk alignment;
- final chunk;
- continuation metadata;
- interrupted chunk stream;
- delete during chunk stream;
- EOF while incomplete;
- replacement failure preserving old image.

### 44.4 Base64 tests

Cover:

- valid padding;
- no-padding form where permitted;
- invalid alphabet;
- malformed final quartet;
- random incremental boundaries;
- decoded-size overflow.

Fuzz decoder integration independently from underlying base64 crate.

### 44.5 RGB/RGBA tests

Cover:

- exact byte size;
- one byte short;
- one byte long;
- zero dimensions;
- multiplication overflow;
- quota boundary.

### 44.6 PNG corpus

Include:

- grayscale;
- grayscale alpha;
- RGB;
- RGBA;
- indexed palette;
- transparency;
- interlaced;
- supported low/normal/high bit depths;
- truncated PNG;
- invalid chunks;
- CRC failure;
- pathological dimensions;
- decompression bomb;
- malformed palette;
- valid image exactly at quota.

### 44.7 zlib tests

Include:

- legal stream;
- truncated stream;
- trailing junk;
- expansion beyond expected raw size;
- expansion beyond quota;
- invalid header;
- repeated tiny blocks;
- maximum bounded result.

### 44.8 File transport tests

Test:

- ordinary file;
- empty file;
- size/offset subrange;
- offset overflow;
- range beyond EOF;
- symlink to regular file;
- symlink loop;
- directory;
- FIFO;
- socket;
- device;
- inaccessible path;
- disappearing path;
- non-UTF-8 Unix path.

Tests MUST guarantee no case blocks waiting for special-file input.

### 44.9 Temporary-file tests

Test deletion:

- allowed temporary directory + required marker;
- path lacking marker;
- path outside temp directory;
- symlink escape;
- failed decode;
- failed deletion.

### 44.10 Shared-memory tests

Test:

- valid object;
- valid offset/range;
- range overflow;
- nonexistent object;
- undersized object;
- unlink behavior;
- cleanup after decode failure.

### 44.11 Query ordering

Feed one PTY read containing:

```text
graphics capability query
immediately followed by another response-generating terminal query
```

Assert graphics reply is first.

Repeat with arbitrary buffer splitting around both commands.

### 44.12 ID tests

Cover:

- anonymous image;
- nonzero image ID;
- replacement;
- failed replacement;
- image number allocation;
- newest-number lookup;
- mutually exclusive identifiers;
- ID reuse after deletion;
- named placement;
- anonymous placement;
- several placement-ID-zero placements.

### 44.13 Geometry tests

Cover:

- full source;
- source crop;
- crop at boundaries;
- invalid crop;
- columns only;
- rows only;
- both;
- neither/native pixels;
- offsets;
- offset at cell boundary;
- cursor movement;
- cursor suppression;
- right/bottom clipping;
- alpha;
- all z strata.

### 44.14 Scroll tests

Place images at:

- top row;
- middle;
- bottom;
- first history row;
- oldest history row.

Then exercise:

- line feed;
- full-screen scroll;
- multi-line scroll;
- scrollback growth;
- history-capacity pruning;
- user viewport movement;
- history purge.

Assert invalid anchors are removed rather than remapped arbitrarily.

### 44.15 Scroll-region tests

For every placement relation to a scrolling region:

- wholly inside;
- wholly above;
- wholly below;
- intersects top boundary;
- intersects bottom boundary.

Exercise upward/downward scroll and clipping.

### 44.16 Reflow tests

Create an anchor at every column of wrapped logical lines.

Resize repeatedly:

```text
80 → 20 → 120 → 1 → 80
```

Include:

- ASCII;
- trailing blanks;
- wide characters;
- wide spacers;
- zero-width combining marks;
- wrapped rows;
- cursor wrap state;
- maximum history.

For each anchor, compare logical cell identity before and after.

### 44.17 Pending-anchor resize test

Begin a deferred graphics load.

Before commit:

1. resize terminal;
2. scroll terminal;
3. resize again.

Commit.

Assert placement appears at transformed logical anchor.

### 44.18 Screen-buffer tests

Exercise:

- enter alternate;
- graphics in alternate;
- leave alternate;
- primary graphics restored;
- re-enter cleared alternate;
- hard reset.

No graphics object may leak between screen buffers.

### 44.19 Erase tests

Test every existing text erase operation against:

- classic placement;
- placeholder cell.

Classic placement and placeholder behavior must intentionally differ where specified.

### 44.20 Placeholder tests

Cover:

- low image-ID bits;
- full 24-bit ID;
- high-byte encoding;
- placement-ID encoding;
- row diacritic;
- column diacritic;
- all inheritance combinations;
- omitted attributes;
- invalid combining mark;
- sparse placeholder matrix;
- repeated cells;
- overlapping virtual prototypes;
- text erase;
- insert/delete character;
- scroll;
- width reflow;
- font resize;
- transparent image/background interaction.

The authoritative row/column combining-character table receives exhaustive generated tests.

### 44.21 Relative-placement tests

Cover:

- direct parent;
- relative parent;
- virtual parent;
- positive offsets;
- negative offsets;
- missing parent;
- deletion cascade;
- cycle;
- chain length 8;
- chain length 9;
- parent replacement;
- parent scrolling;
- parent history pruning;
- cursor unchanged.

### 44.22 Deletion tests

Exercise every protocol deletion selector in both data-retaining and data-freeing forms.

Include:

- no matches;
- one match;
- several matches;
- virtual placements;
- relative descendants;
- pending upload;
- current animation frame.

### 44.23 Animation tests

Frame load:

- root frame;
- additional frame;
- base frame;
- edit frame;
- transparent initialization;
- background initialization;
- blend;
- overwrite;
- invalid rectangle.

Control:

- stop;
- loading mode;
- run;
- current-frame change;
- finite loops;
- infinite loop;
- gapless frame;
- loop-counter reset.

Composition:

- different source/destination;
- clipping/error boundaries;
- invalid self-overlap;
- memory exhaustion;
- replacement atomicity.

### 44.24 Quota tests

Test exact boundaries:

```text
limit - 1
limit
limit + 1
```

Exercise:

- new image;
- replacement;
- animation frame;
- pending reservation;
- transient eviction;
- unplaced eviction;
- offscreen eviction;
- visible last-resort eviction;
- lowered runtime limit.

Assert accounting returns exactly to baseline after deletion.

### 44.25 Metadata exhaustion

Attempt more than:

- maximum images;
- maximum placements;
- maximum relative edges.

Ensure bounded memory and graceful errors.

### 44.26 Renderer golden tests

Render to controlled framebuffer and compare pixels for:

- opaque image;
- translucent image;
- crop;
- scaling;
- native-pixel size;
- z below default background;
- z between cell background and glyph;
- z above glyph;
- cursor over image;
- overlapping images;
- placeholder tile;
- relative placement;
- animation frame;
- clipping.

Run against every supported renderer path.

### 44.27 Texture tiling tests

Force a test texture-size limit smaller than input image.

Assert tiled output equals untiled expected image with no seams.

### 44.28 Context-loss tests

Populate terminal graphics state, discard all GPU cache state, recreate renderer, redraw.

Output MUST match pre-loss framebuffer.

### 44.29 Fuzzing

Fuzz targets:

- generic APC parser;
- Kitty control parser;
- chunk-state machine;
- base64 adapter;
- zlib bounded adapter;
- PNG integration;
- local-transfer metadata;
- delete selector parser;
- relative graph operations;
- animation command parser.

A stateful terminal fuzz target SHOULD generate:

- arbitrary PTY chunk boundaries;
- graphics commands;
- scroll operations;
- resize;
- erase;
- screen swap;
- reset.

### 44.30 Property tests

Continuously assert core invariants listed in §43 after random command sequences.

### 44.31 Performance tests

Measure:

- parser throughput with no graphics;
- renderer frame time with no graphics;
- one large image;
- thousands of placements;
- repeated upload/delete;
- scrolling image-heavy history;
- repeated resize/reflow;
- 60 Hz animation;
- GPU-cache eviction/reupload.

Acceptance criterion for non-graphics workloads:

> No persistent per-frame allocation or meaningful hot-path complexity increase when graphics state is empty.

---

## 45. Development phases

Protocol completeness is the destination; phases exist only to keep implementation reviewable and testable.

### Phase 0 — test infrastructure

- protocol fixtures;
- framebuffer golden harness;
- graphics state property-test harness;
- hostile-input corpus.

### Phase 1 — APC transport

- dedicated `vte` APC state;
- streaming callbacks;
- APC cancellation tests;
- non-Kitty regression tests.

No graphics rendering yet.

### Phase 2 — parser barrier and transactional pipeline

- partial-consumption API;
- deferred requests;
- FIFO continuation;
- response ordering;
- no heavy processing under `Term` lock.

### Phase 3 — image core

- direct transmission;
- RGB;
- RGBA;
- PNG;
- zlib;
- image IDs;
- image numbers;
- queries;
- quota/reservations.

### Phase 4 — classic placements

- placement IDs;
- source crop;
- row/column geometry;
- offsets;
- cursor semantics;
- deletion;
- tracked anchors;
- scrolling;
- history pruning;
- resize/reflow;
- screen swap/reset.

### Phase 5 — renderer

- image texture cache;
- texture tiling;
- layered text/background pipeline;
- alpha;
- z-index;
- clipping;
- context recovery.

At this point basic `icat`-style use is functional but the feature is still incomplete.

### Phase 6 — local transports

- regular file;
- temporary file;
- shared memory;
- security gates;
- bounded IO.

### Phase 7 — Unicode placeholders

- virtual placements;
- generated combining-character lookup;
- ID decoding;
- inheritance;
- cell-derived rendering.

### Phase 8 — relative placements

- graph;
- parent lookup;
- virtual-parent resolution;
- cycle/depth validation;
- cascading lifetime.

### Phase 9 — animation

- frame loading;
- frame editing;
- frame composition;
- control;
- scheduling;
- frame deletion.

### Phase 10 — hardening

- fuzzing;
- adversarial corpus;
- quota stress;
- renderer golden tests;
- context loss;
- performance;
- documentation.

After Phase 10, remove the temporary protocol-disable option so completed Kitty graphics support is always available.

---

# Drawbacks

## Complexity

This is not a small renderer feature.

It adds state and invariants across:

- parser;
- PTY event loop;
- terminal grid;
- screen switching;
- resize/reflow;
- CPU memory management;
- filesystem/shared-memory IO;
- renderer;
- scheduling.

The largest maintenance burden is not image decoding; it is maintaining placement correctness as terminal state mutates.

## Attack surface

Graphics parsers process large attacker-controlled binary inputs.

PNG, zlib, base64, local transports, arithmetic involving image geometry, and GPU texture handling all enlarge attack surface.

The 2026 Kitty graphics security advisory demonstrates that apparently simple payload growth logic can become memory-safety relevant in lower-level implementations. Rust reduces classes of memory corruption but does not remove denial-of-service, overflow, excessive allocation, parser-state, or logic risks. citeturn575486view8

## Memory consumption

The default protocol-compatible storage budget is large relative to traditional terminal state.

Animation frames represented as full RGBA consume especially significant memory.

The explicit quota is therefore not optional implementation polish; it is part of the architecture.

## Rendering complexity

Existing text rendering must be split enough to place negative-z graphics correctly between backgrounds and glyphs.

That makes renderer ordering more complex even when ordinary terminals historically had a simpler background/text model.

## Reflow complexity

Anchoring classic placements through Alacritty's reflow introduces a general tracked-position facility into a grid implementation that previously did not need to preserve arbitrary external objects.

This is likely the most subtle part of the implementation.

## Local-object transport semantics

File and shared-memory transfer allow a terminal escape sequence to trigger host-local reads.

Although required for complete protocol support, this warrants explicit configuration, strict file-type checks, and bounded handling.

---

# Rationale and alternatives

## Why native Kitty support instead of external image display

An external viewer cannot implement terminal semantics correctly.

It cannot independently know or preserve:

- scrollback anchoring;
- terminal resize/reflow;
- alternate-screen lifetime;
- cell-relative geometry;
- text backgrounds;
- z-index;
- placeholder cells;
- protocol image IDs;
- protocol responses.

The outer terminal emulator is the component that owns all required state.

## Why Kitty rather than SIXEL in this RFC

SIXEL is useful, established, and independently implementable, but represents a different terminal-graphics model.

Kitty provides abstractions directly useful for modern terminal applications:

- image identity;
- image reuse;
- placement identity;
- alpha;
- z-order;
- arbitrary crop;
- efficient local transmission;
- Unicode placeholders;
- relative placement;
- animation.

Adding SIXEL would neither implement nor remove the need for these semantics.

Other graphics protocols remain independent future work.

## Why not renderer-only state

Rejected.

A renderer-only placement cannot correctly follow:

- scrolling;
- history;
- history pruning;
- resize/reflow;
- screen switching;
- terminal reset.

Renderer resources also disappear on context loss.

Protocol state belongs in `alacritty_terminal`.

## Why not GPU-only pixels

Rejected.

GPU-only ownership would make:

- context loss destructive;
- animation frame composition difficult;
- image replacement transactional semantics difficult;
- renderer-independent tests impossible;
- non-visible retained images dependent on GPU lifetime.

CPU canonical data is source of truth.

## Why not store classic placements in cells

Rejected.

Classic graphics do not share cell erase/lifetime semantics, and several placements may overlap one cell.

Cell ownership is correct only for the Unicode placeholder mechanism specifically designed around cells.

Prior implementations demonstrate that a cell-centric shortcut imposes limitations once overlapping, z-order, and newer placement semantics are required. citeturn575486view12

## Why not replace Alacritty terminal state with another terminal library

Rejected.

A foreign terminal core would duplicate or replace:

- VT parsing;
- grid;
- scrollback;
- resize;
- selections;
- cursor;
- modes;
- damage;
- screen switching.

Synchronizing two terminal models solely to obtain graphics support would be substantially more complex than implementing graphics against Alacritty's existing state.

Other terminals are useful as design references and behavioral comparison targets, not runtime dependencies.

## Why streaming APC instead of buffering whole APC

Streaming keeps the generic `vte` parser protocol-neutral and bounded.

The protocol-specific layer can enforce its own limits and chunking rules.

This also leaves APC useful for future applications without committing generic VT parsing to arbitrary large allocation.

## Why synchronous deferred work on PTY thread initially

It preserves byte-stream ordering without a reorder system.

The expensive work is still moved outside `Term`'s lock, which is the critical latency requirement.

A worker pool would improve throughput only when commands can safely execute in parallel, while introducing:

- sequencing;
- cancellation;
- resource reservation races;
- completion buffering;
- shutdown concerns.

That optimization is premature before protocol correctness.

## Why full protocol rather than an `icat` subset

Partial implementations have a tendency to become permanent compatibility profiles.

Applications then need emulator-specific capability tables despite ostensibly using one protocol.

For a private fork there is little benefit in defining a deliberately incomplete final interface.

Implementation is phased, but the specification describes complete support.

## Why no terminal-identity change

Protocol support can be queried directly.

Changing `$TERM` would conflate graphics support with the much larger terminfo and terminal-compatibility surface.

## Why preserve placeholder cells as ordinary cells

The protocol deliberately encodes placeholder semantics using Unicode and terminal attributes.

Alacritty's existing cell already stores the relevant information. citeturn767575view7turn767575view8

Special-casing storage would defeat properties that make placeholders useful:

- normal text movement;
- text editing;
- scrollback;
- reflow;
- ordinary terminal transport.

## Why full RGBA animation frames

Correctness and simplicity.

Delta graphs could save RAM but complicate:

- frame edits;
- frame composition;
- random current-frame selection;
- texture reconstruction;
- quota accounting;
- eviction;
- testability.

Storage quota bounds full-frame cost.

Optimization can follow measurement.

---

# Prior art

## Kitty

Kitty is normative protocol reference.

Its implementation and specification establish the externally observable model for:

- transfers;
- image/placement identity;
- Unicode placeholders;
- relative placements;
- animation;
- z-order;
- resource queries.

This RFC intentionally does not infer protocol semantics from another implementation when the published specification is explicit. citeturn575486view6turn575486view7

## Ghostty

Ghostty is useful architectural prior art because its graphics state separates stored images from placements and treats graphics as terminal state rather than merely renderer state. Its current implementation also demonstrates generation tracking, pending image state, quota management, and viewport-resolved placements. citeturn575486view9

A recent graphics/scrollback defect in Ghostty is particularly instructive: placements whose anchors were pruned from scrollback could survive with incorrect resolved position. The fix reinforces this RFC's requirement that anchor invalidation be explicit and tested as part of grid pruning. citeturn575486view10

The `ghostling` example further demonstrates a useful terminal/rendering boundary: graphics placements and pixel data can be exposed from terminal state to an independent renderer without making GPU state authoritative. citeturn575486view11

Ghostty remains prior art, not a runtime dependency and not an alternative source of normative protocol semantics.

## st-graphics

`st-graphics` demonstrates that a relatively small terminal can add Kitty-protocol graphics, but its documented architecture/limitations also illustrate the compromises of representing broader graphics semantics through cell-oriented machinery.

This RFC therefore borrows the proof of feasibility, not the cell-ownership model. citeturn575486view12

## Contour and other native terminals

Contour has shipped native Kitty graphics functionality, providing additional evidence that the protocol can be integrated into a terminal renderer without adopting another terminal's architecture. citeturn575486view13

Prior art collectively suggests the recurring hard problems are not texture upload itself, but:

- placement lifetime;
- scrolling;
- state replacement;
- memory bounds;
- z composition;
- protocol breadth.

This RFC designs those concerns first.

---

# Unresolved questions

No unresolved question blocks implementation.

The following choices should remain measurable implementation decisions rather than protocol-design blockers.

## Animation storage optimization

Full RGBA frame materialization is specified initially.

If real workloads regularly encounter quota pressure, later storage could use:

- base frame + deltas;
- deduplicated tiles;
- compressed inactive frames;
- temporary backing.

Any replacement must preserve deterministic random access and quota semantics.

## Ordered background decode

The synchronous PTY-thread deferred model is simplest.

If image decoding materially delays processing unrelated PTY input, an ordered worker pipeline may be justified.

Such a pipeline would require explicit sequence numbers and in-order commit.

## Fine-grained damage

Full damage on visible graphics mutation is acceptable initially.

Rectangle-level graphics damage should be added only after placement/clipping semantics are fully tested.

## GPU memory budget

CPU graphics quota is specified.

Exact GPU cache budget can initially be heuristic based on visible data and renderer limits, then become configurable if necessary.

## Aspect-ratio rounding

The RFC specifies deterministic aspect-preserving sizing with a preferred ceiling rule, subject to compatibility testing where the published protocol leaves rounding ambiguous.

This should be settled by test corpus before Phase 10 completion.

---

# Future possibilities

## Fine-grained graphics damage

Each placement mutation could compute old/new framebuffer rectangles and union them into Alacritty's normal damage tracker.

Animation then redraws only affected regions.

## Ordered parallel decoding

A later pipeline could assign sequence numbers:

```text
parse
  ↓
decode workers
  ↓
ordered completion queue
  ↓
commit sequence N
  ↓
commit sequence N+1
```

This could improve throughput for large PNG workloads while retaining terminal ordering.

## Sparse animation representation

Frames could internally retain protocol delta form and materialize lazily.

A content-addressed tile cache could further reduce memory where frames differ only locally.

## Persistent texture staging

Modern renderer paths could use staging buffers or asynchronous texture upload to reduce frame stalls caused by large first-time image uploads.

Such optimization must remain invisible to `alacritty_terminal`.

## Better placeholder indexes

Virtual-parent relative placement currently permits correctness-first cell scans.

A grid-maintained index from virtual placement identity to occupied placeholder extents could make relative resolution near O(1).

The index must remain derived state because cells are authoritative.

## Generic tracked grid positions

The tracked-anchor machinery introduced for graphics may eventually support other Alacritty features that need logical positions stable across reflow.

Graphics should not expose its private placement types into generic grid code; the reusable abstraction is "tracked point through grid transformation."

## Additional graphics protocols

Other terminal graphics protocols may later use:

- the renderer image cache;
- bounded decode infrastructure;
- tracked anchors;
- APC/DCS parser streaming;
- image texture compositing.

They should receive independent protocol state and independent specifications rather than being forced into Kitty objects.

---

# Appendix A: Proposed module ownership

```text
vte/
└── src/
    └── lib.rs
        APC parser state
        Perform APC callbacks

alacritty_terminal/
└── src/
    ├── ansi.rs
    │   Kitty APC dispatch
    │   parser barriers
    │
    ├── event_loop.rs
    │   partial input consumption
    │   ordered deferred graphics work
    │
    ├── graphics/
    │   ├── mod.rs
    │   │   GraphicsState
    │   │
    │   ├── parser.rs
    │   │   Kitty control/payload parser
    │   │
    │   ├── command.rs
    │   │   typed commands
    │   │
    │   ├── transport.rs
    │   │   direct/file/temp/shm
    │   │
    │   ├── image.rs
    │   │   canonical pixels/IDs
    │   │
    │   ├── placement.rs
    │   │   placements/relative graph
    │   │
    │   ├── placeholder.rs
    │   │   Unicode placeholder decoding
    │   │
    │   ├── animation.rs
    │   │   frames/control/composition
    │   │
    │   └── storage.rs
    │       quotas/reservations/eviction
    │
    ├── grid/
    │   tracked-point reflow support
    │
    └── term/
        graphics/screen/reset integration

alacritty/
└── src/
    ├── config/
    │   graphics configuration
    │
    ├── display/
    │   graphics render snapshots
    │   animation scheduling
    │
    └── renderer/
        └── image/
            shaders
            texture cache
            tiling
            compositing
```

---

# Appendix B: Transaction examples

## B.1 Successful transmit and place

```text
PTY
 │
 │ APC
 ▼
vte
 │ streamed bytes
 ▼
GraphicsApcParser
 │ complete command
 ▼
parser barrier
 │
 ├─ capture cursor anchor
 └─ release Term lock
       │
       ▼
    decode PNG
       │
       ▼
   reserve storage
       │
       ▼
 reacquire Term lock
       │
       ├─ validate ID state
       ├─ commit image
       ├─ commit placement
       ├─ update cursor
       ├─ emit reply
       └─ mark damage
       │
       ▼
 resume later PTY bytes
```

## B.2 Failed replacement

```text
old image i=42
old placements
        │
        │ receive replacement
        ▼
decode new bytes
        │
        ├─ failure
        ▼
discard transaction

result:
    old image i=42      preserved
    old placements      preserved
    new image           absent
```

## B.3 Context loss

```text
GraphicsState
    canonical RGBA
       │
       ├──── GPU texture cache ─── X
       │                          context lost
       │
       ▼
 next render snapshot
       │
       ▼
 re-upload RGBA
       │
       ▼
 identical terminal state
```

## B.4 Scrollback pruning

```text
placement
    │
    ▼
TrackedAnchor ──► retained grid row
                    │
                    │ history pruning
                    ▼
                 discarded
                    │
                    ▼
TrackedAnchor = invalid
                    │
                    ▼
placement removed
```

Never:

```text
invalid anchor → clamp to top-left
```

---

# Appendix C: Security checklist

Before considering graphics complete:

- [x] APC buffers bounded.
- [x] Direct chunk buffers bounded.
- [x] All arithmetic checked.
- [x] Base64 decode bounded.
- [x] zlib output bounded.
- [x] PNG dimensions checked before canonical allocation.
- [x] CPU quota enforced before commit.
- [x] Metadata object counts bounded.
- [x] File reads bounded.
- [x] Opened object verified as regular file.
- [x] Special files rejected.
- [x] Temporary deletion path constrained.
- [x] Shared-memory ranges checked.
- [x] Shared-memory lifetime cleaned correctly.
- [x] No heavyweight IO/decode occurs under `Term` lock.
- [x] Replacement atomic.
- [x] Parser cancellation releases reservation.
- [x] Graphics payload never logged.
- [x] Error messages bounded.
- [x] Renderer texture dimensions checked.
- [x] Oversized textures tiled.
- [x] Relative graphs cycle checked.
- [x] Relative depth bounded.
- [x] History pruning invalidates anchors.
- [x] Fuzz targets run under sanitizers where applicable.
- [x] Repeated upload/delete has stable memory usage.
- [x] Context-loss reconstruction tested.

---

# Appendix D: Performance invariants

For workloads containing no Kitty graphics:

1. APC additions MUST NOT allocate.
2. graphics parser state MUST remain dormant.
3. graphics renderer MUST return through an empty fast path.
4. no texture-cache lookup occurs per ordinary glyph.
5. no extra heap object is added to every `Cell`.
6. grid resize overhead from anchor tracking is approximately zero when anchor set is empty.

For graphics workloads:

1. large decode does not monopolize `Term` mutex;
2. renderer uploads only missing/stale textures;
3. image placement does not copy image pixels;
4. one stored image may feed arbitrarily many placements within placement-count limits;
5. scrolling normally changes geometry, not image pixels;
6. animation ticks do not decode source formats;
7. context recreation causes lazy, not eager, re-upload of all retained history images.

---

# Appendix E: Definition of done

The feature is complete only when every row in [the Kitty graphics conformance requirements](kitty-graphics-conformance.md) is `Covered` or records an accepted policy difference. In particular:

1. every current action, key, default, response, quiet level, error identity, and parser boundary has a wire-level test;
2. RGB, RGBA, every valid PNG form, and zlib for each format have bounded success and failure tests;
3. direct chunks, regular files, constrained temporary files, POSIX shared memory, and non-Unix shared-memory rejection have lifecycle and range tests;
4. queries validate without mutation and remain ordered before later terminal responses;
5. image IDs, image numbers, placement IDs, successful replacement, failed atomic replacement, and generated responses are correct;
6. classic crop, scale, offset, clipping, cursor movement, scroll, reverse scroll, margins, history, and primary-buffer reflow are correct;
7. pruned anchors never survive as corrupted positions;
8. Unicode placeholders pass exhaustive identity, inheritance, placement-ID, background, sparse-grid, clipping, scrolling, and deletion tests;
9. relative placements pass parent selection, virtual-parent origin, offset, cursor, cycle, depth, error-response, replacement, deletion, pruning, and eviction tests;
10. every lowercase and uppercase deletion selector, optional placement qualifier, range boundary, virtual-placement rule, and upload-cancellation rule works;
11. animation frame loading, editing, default and gapless timing, canvases, chunking, control states, finite and infinite loops, stop reset, composition, deletion, retransmission, scheduling, and visible playback work;
12. primary, alternate, RIS, clear-screen, text-erasure, and placeholder-erasure interactions are correct;
13. image bytes, frame bytes, pending data, metadata, GPU cache, and transient uploads remain bounded and repeated lifecycle operations have stable accounting;
14. no large transport, decode, or allocation operation holds the terminal lock;
15. framebuffer tests reproduce all z strata, equal-z ordering, alpha overlap, transparent backgrounds, crop/scale/offset, placeholder suppression, animation, texture eviction, GL-state restoration, and context reconstruction;
16. parser, transaction, state machine, image decoders, animation, and random grid interaction are fuzz-tested;
17. graphics-client pixel and cell geometry queries remain coherent;
18. ordinary non-graphics Alacritty workloads show no material steady-state parser, memory, or redraw regression.

---

# References

Primary specification and project sources:

- [Kitty terminal graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
- [Alacritty source repository](https://github.com/alacritty/alacritty)

























Security and prior-art sources:

- [Kitty security advisories](https://github.com/kovidgoyal/kitty/security/advisories)
- [Ghostty source repository](https://github.com/ghostty-org/ghostty)
- [st-graphics source repository](https://github.com/sergei-grechanik/st-graphics)
- [Contour source repository](https://github.com/contour-terminal/contour)













RFC structure:

- [Rust RFC template](https://github.com/rust-lang/rfcs/blob/master/0000-template.md)

