# Kitty graphics maintenance: wire and deferred interfaces

Parent plan: [Kitty graphics maintenance](kitty-graphics-maintenance.md)

## Stabilized module boundaries

### `graphics/parser.rs`

Own only APC byte recognition and control-field parsing. It produces a payload-owning parsed representation and does not copy payload bytes.

```rust
pub(crate) struct ParsedCommand {
    pub command: Command,
    pub payload: Vec<u8>,
}

impl GraphicsApcParser {
    pub(crate) fn start(&mut self);
    pub(crate) fn put(&mut self, byte: u8);
    pub(crate) fn end(&mut self) -> Option<Result<ParsedCommand, GraphicsError>>;
    pub(crate) fn abort(&mut self);
}
```

`GraphicsApcParser::end` must use `mem::take(&mut self.payload)`. Parsing becomes a function over borrowed control bytes and owned payload:

```rust
fn parse_command(control: &[u8], payload: Vec<u8>) -> Result<ParsedCommand, GraphicsError>;
```

`Command` remains the payload-free wire header during this maintenance series. It retains raw optional scalar fields because key presence, including simultaneous `i` and `I`, is protocol-significant. Do not normalize those fields to `Option<NonZeroU32>` at parse time.

### `graphics/transaction.rs`

Own direct chunk assembly and encoded input ownership.

```rust
pub(crate) enum EncodedPayload {
    Single(Vec<u8>),
    Chunks(Vec<Vec<u8>>),
}

pub(crate) enum GraphicsRequest {
    Invalid {
        command: Option<Command>,
        error: GraphicsError,
    },
    Command {
        command: Command,
        payload: EncodedPayload,
    },
}

pub(crate) struct PendingTransmission {
    command: Command,
    chunks: Vec<Vec<u8>>,
    encoded_bytes: usize,
}

pub(crate) struct RequestError {
    command: Option<Box<Command>>,
    error: GraphicsError,
}

pub(crate) enum PendingResult {
    Pending(PendingTransmission),
    Complete(GraphicsRequest),
}

impl PendingTransmission {
    pub(crate) fn start(
        parsed: ParsedCommand,
        encoded_limit: usize,
    ) -> Result<Self, RequestError>;

    pub(crate) fn push(
        self,
        parsed: ParsedCommand,
        encoded_limit: usize,
    ) -> Result<PendingResult, RequestError>;
}
```

`Term::take_deferred_graphics` captures the active cursor point after the parser stops at the command barrier and stores it directly in `Term::deferred_graphics_anchor`. For chunked transmission, it does this only when the final chunk completes the request, so the final chunk's cursor point becomes the placement anchor. `Command`, `ParsedCommand`, and `GraphicsRequest` do not store this terminal-derived point.

Metadata commands may arrive with payload bytes, but request classification must discard bytes before producing metadata-only prepared output.

`PendingTransmission::start` and `push` retain current continuation compatibility and identity rules. They return `RequestError` when failure can preserve the initial command identity. Encoded-size arithmetic returns `GraphicsError::TooLarge` on overflow. It must never fall back to `usize::MAX`.

If a pending transmission exists and the next command is `Action::Delete`, `Term::take_deferred_graphics` must discard the pending chunks and execute the delete as a new metadata command. It must not pass delete through continuation validation. Every other incompatible continuation fails with the initial command identity where recoverable. A successful final continuation preserves the initial command fields and applies its final `quiet` override. `Term::take_deferred_graphics` captures the cursor only after this completion result is known.

### `graphics/deferred.rs`

Add this module as the only heavy-processing boundary.

```rust
#[derive(Clone, Copy, Debug)]
pub(crate) struct ProcessingOptions {
    pub decode_limit: usize,
    pub local_transmission: bool,
}

pub(crate) enum DeferredGraphics {
    Decode(DecodeWork),
    Compose(FrameWork),
}

pub(crate) enum PreparedGraphics {
    Command(ProcessedCommand),
    Frame(PreparedFrameMutation),
}

impl DeferredGraphics {
    pub(crate) fn process(self) -> PreparedGraphics;
}
```

`FrameWork::process` converts `blank_frame` or `compose` failures into `PreparedGraphics::Command(ProcessedCommand::Error)` with the payload-free command identity. It never panics and never fabricates a frame mutation.

`DeferredGraphics::process` must not accept or access `Term`, `GraphicsState`, a terminal mutex, or an event proxy. This type-level boundary is the primary proof that heavy work can run off-lock.

`DecodeWork` owns `GraphicsRequest` and `ProcessingOptions`. `ProcessedCommand` owns a payload-free `Command` and either decoded pixels, metadata, or an error.

```rust
pub(crate) struct DecodeWork {
    request: GraphicsRequest,
    options: ProcessingOptions,
}

pub(crate) enum ProcessedCommand {
    Decoded {
        command: Command,
        image: PixelBuffer,
    },
    Metadata(Command),
    Error {
        command: Option<Command>,
        error: GraphicsError,
    },
}
```

`Term` retains `deferred_graphics_anchor: Option<Point>` while work is off-lock. `take_deferred_graphics` moves the completed request anchor into this slot before creating `DecodeWork`. `resize_grid_with_graphics` continues to track and rewrite the slot through reflow. Final commit consumes the current slot and applies it to cursor-dependent placement. Intermediate frame-composition continuation must leave it untouched. Cancellation and final errors clear it.

### Streaming encoded input

Use the existing `base64` crate. Define one reader over either one buffer or an ordered chunk iterator:

```rust
pub(crate) struct EncodedReader {
    chunks: std::vec::IntoIter<Vec<u8>>,
    current: std::io::Cursor<Vec<u8>>,
}

impl std::io::Read for EncodedReader { /* ordered, allocation-free handoff */ }
```

`EncodedPayload::Single` is converted to the same reader representation. Use `base64::read::DecoderReader`. Direct image payloads use a `GeneralPurpose` engine configured with `DecodePaddingMode::Indifferent`, preserving padded and unpadded compatibility without a retry. Local transport names retain canonical standard-base64 padding behavior. Do not decode either representation twice.

Bound decoded reads with `Read::take(limit + 1)` using checked addition. Reading one extra byte proves the source exceeded its working bound, but format-specific validation still selects the protocol error: short raw input is `NoData`, excess non-frame raw input is `Invalid`, quota overflow is `NoSpace`, and transmitted frame data retains its defined excess truncation. Invalid direct base64 maps to `GraphicsError::Decode`; invalid file, temporary-file, or shared-memory name base64 maps to `GraphicsError::Invalid`.

Move name decoding out of `load_transport`. Its stabilized boundary receives native name bytes:

```rust
pub(crate) fn load_transport(
    transmission: Transmission,
    name: Vec<u8>,
    command: &Command,
    limit: usize,
) -> Result<Vec<u8>, GraphicsError>;
```

Release encoded input before file or shared-memory reads begin. The implementation may use a small reusable read buffer, but it must not concatenate chunks.

### `Term` deferred interface

Replace the current setup methods with these crate-private methods:

```rust
impl<T> Term<T> {
    pub(crate) fn take_deferred_graphics(&mut self) -> Option<DeferredGraphics>;

    pub(crate) fn commit_deferred_graphics(
        &mut self,
        prepared: PreparedGraphics,
    ) -> Option<DeferredGraphics>
    where
        T: EventListener;
}
```

`take_deferred_graphics` performs these steps under the lock:

1. Take the parser result.
2. Capture `ProcessingOptions` and compute the checked encoded limit before calling `PendingTransmission::start` or `push`.
3. If a pending transmission exists and the new action is delete, discard the pending chunks and classify delete as a new complete request.
4. Otherwise start, continue, complete, or reject direct chunks.
5. Return `None` for an incomplete chunk sequence without setting processing state or an anchor.
6. For a complete valid request, capture the current cursor as the final command completion anchor.
7. Move that anchor into `Term::deferred_graphics_anchor` so resize can reflow it.
8. Set `graphics_processing = true` and clear stale cancellation state.
9. Return `DeferredGraphics::Decode`. Parser errors also return decode work, but leave the deferred anchor empty.

`commit_deferred_graphics` has two outcomes:

- Return `None` after final commit, response, cancellation, or error.
- Return `Some(DeferredGraphics::Compose(_))` when decoded or metadata work requires off-lock frame construction.

When returning continuation work, keep `graphics_processing = true`, preserve `deferred_graphics_anchor`, do not emit a response, do not mark final damage, and do not resume parser input. Clear processing state and the deferred anchor only on the final return of `None`.

Configuration reload retains the exact current cancellation rule: if `local_transmission` changes from true to false while any graphics request is processing, both first-stage and continuation commits discard their prepared result. Do not narrow this to requests that actually use local transport without a separate behavior decision and test change. If the active image or frame disappears because a quota change evicts it, final commit returns the same protocol error the equivalent locked operation would have returned.

### Event loop helper

Centralize the process/commit loop in `event_loop.rs`:

```rust
fn process_deferred_graphics(&self, mut work: DeferredGraphics) {
    loop {
        let prepared = work.process();
        match self.terminal.lock_unfair().commit_deferred_graphics(prepared) {
            Some(continuation) => work = continuation,
            None => break,
        }
    }
}
```

Both normal PTY input and synchronized completion, timeout, and overflow paths call this helper after dropping their terminal guard. No caller may parse the retained suffix until the helper returns.
