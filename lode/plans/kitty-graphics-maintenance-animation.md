# Kitty graphics maintenance: animation interfaces

Parent plan: [Kitty graphics maintenance](kitty-graphics-maintenance.md)

### Animation preparation

`graphics/animation.rs` continues to own pure pixel operations. `graphics/storage.rs` owns cheap state snapshot and atomic commit.

Add a structural frame revision to each image:

```rust
struct Image {
    // Existing fields.
    frame_revision: u64,
}
```

Increment `frame_revision` with checked arithmetic whenever root/frame pixels or frame topology changes: frame insertion, frame replacement, frame composition, frame deletion, and root promotion. A base retransmission creates a new image handle with revision zero. Animation playback that only changes `current_frame` does not change `frame_revision`.

Use these internal types:

```rust
pub(crate) enum FrameCanvas {
    Existing(PixelBuffer),
    Blank {
        width: u32,
        height: u32,
        rgba: u32,
    },
}

pub(crate) enum FrameDestination {
    Insert {
        gap_ms: i32,
    },
    ReplaceRoot {
        gap_ms: i32,
    },
    Replace {
        index: usize,
        gap_ms: i32,
    },
    ComposeRoot,
    Compose {
        index: usize,
    },
}

pub(crate) struct FrameWork {
    command: Command,
    image: ImageHandle,
    expected_revision: u64,
    canvas: FrameCanvas,
    source: PixelBuffer,
    composition: FrameComposition,
    destination: FrameDestination,
}

pub(crate) struct PreparedFrameMutation {
    command: Command,
    image: ImageHandle,
    expected_revision: u64,
    destination: FrameDestination,
    pixels: PixelBuffer,
}

pub(crate) enum FrameCommit {
    Stored { frame_number: u32 },
    Composed,
}
```

Add storage methods:

```rust
impl GraphicsState {
    pub(crate) fn prepare_store_frame(
        &self,
        command: Command,
        source: PixelBuffer,
    ) -> Result<FrameWork, GraphicsError>;

    pub(crate) fn prepare_compose_frames(
        &self,
        command: Command,
    ) -> Result<FrameWork, GraphicsError>;

    pub(crate) fn commit_frame(
        &mut self,
        prepared: PreparedFrameMutation,
    ) -> Result<FrameCommit, GraphicsError>;
}
```

`prepare_store_frame` and `prepare_compose_frames` may perform lookup, checked coordinate validation, frame-limit validation, and `Arc` cloning. They must not allocate a canvas or copy pixel bytes.

`FrameWork::process` materializes a blank canvas when needed and calls `compose`. `commit_frame` revalidates image handle and `frame_revision`, rechecks frame count and byte quota, installs pixels, updates accounting and animation state, and increments `frame_revision`. A stale handle returns `NotFound`. A revision mismatch returns `Invalid` and preserves existing state.

Store-frame destinations retain current eviction policy; compose-frame destinations never evict. Before store-frame eviction, compute a complete candidate plan and prove that the target mutation will fit:

```rust
fn plan_evictions(
    &self,
    incoming: usize,
    credit: usize,
    excluded: Option<ImageHandle>,
) -> Result<Vec<ImageHandle>, GraphicsError>;
```

The plan uses the existing deterministic priority and LRU order without mutating state. `commit_frame` computes every checked serial, revision, count, and byte result before applying the returned removals. Apply planned evictions only after every fallible validation has passed, so a failed frame commit preserves prior state. Preserve existing `content_generation` behavior: update it only when the visible frame's pixels change. New frame insertion does not invalidate the currently displayed frame. `commit_frame` computes an inserted frame's 1-based number only after successful insertion. `FrameCommit::Stored` provides that response number; `FrameCommit::Composed` emits the existing compose response behavior. Before calling a consuming prepare method, `commit_deferred_graphics` clones the payload-free `Command` for error response identity.

Preserve error precedence by decoding transmitted frame data first. Only after successful decode may `commit_deferred_graphics` call `prepare_store_frame`. Metadata `a=c` proceeds directly to `prepare_compose_frames`.
